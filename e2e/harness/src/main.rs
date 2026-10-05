//! Regtest control harness for the Stable Channels E2E suite.
//!
//! One process plays every off-app role in the demo-script flows:
//!   - the counterparty wallet ("another app"):  /pay /invoice /address /send
//!   - the miner:                                /mine
//!   - the price feed:                           /price (set) + /feeds/* (serve)
//! plus /bootstrap (fund self + open a channel to the LSP) and /info.
//!
//! Config via env (defaults match e2e/harness/docker-compose.yml):
//!   HARNESS_LISTEN     0.0.0.0:9737
//!   DATA_DIR           ./harness-data
//!   ESPLORA_URL        http://127.0.0.1:30000
//!   BITCOIND_RPC       http://127.0.0.1:18443
//!   BITCOIND_RPC_USER  sc
//!   BITCOIND_RPC_PASS  sc
//!   P2P_LISTEN         0.0.0.0:9736
//!   LSP_NODE_ID        (required for /bootstrap)
//!   LSP_P2P_ADDR       127.0.0.1:9735

use std::str::FromStr;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use ldk_node::bitcoin::secp256k1::PublicKey;
use ldk_node::bitcoin::{Address, Network};
use ldk_node::config::EsploraSyncConfig;
use ldk_node::lightning::ln::msgs::SocketAddress;
use ldk_node::lightning_invoice::{Bolt11Invoice, Bolt11InvoiceDescription, Description};
use ldk_node::bitcoin::hashes::{sha256, Hash};
use ldk_node::lightning_types::payment::{PaymentHash, PaymentPreimage};
use ldk_node::payment::PaymentStatus;
use ldk_node::{Builder, Node};
use serde_json::{json, Value};

struct AppState {
    node: Arc<Node>,
    /// BTC/USD price as f64 bits — the mocked feed value.
    price_bits: AtomicU64,
    /// When set, every /feeds/* endpoint returns 503 — a full price-feed outage
    /// for BOTH the app and the LSP (both read the mock feeds in E2E).
    feeds_down: AtomicBool,
    /// Hold invoices created by /hold-invoice, keyed by payment hash.
    holds: Holds,
    rpc_url: String,
    rpc_auth: String, // "Basic <b64>"
    lsp_node_id: Option<String>,
    lsp_p2p_addr: String,
}

/// Mode of a harness hold invoice: `Hold` keeps an arriving HTLC pending until
/// /hold-claim or /hold-fail; `Fail` rejects it the moment it arrives, so the
/// payer sees a deterministic recipient-rejected failure.
#[derive(Clone, Copy, PartialEq)]
enum HoldMode {
    Hold,
    Fail,
}

struct Hold {
    preimage: [u8; 32],
    mode: HoldMode,
    /// "open" (no HTLC yet), "held", "claimed", "failed"
    state: &'static str,
    claimable_amount_msat: Option<u64>,
}

type Holds = Arc<Mutex<HashMap<[u8; 32], Hold>>>;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn main() {
    let listen = env_or("HARNESS_LISTEN", "0.0.0.0:9737");
    let data_dir = env_or("DATA_DIR", "./harness-data");
    let esplora = env_or("ESPLORA_URL", "http://127.0.0.1:30000");
    let rpc_url = env_or("BITCOIND_RPC", "http://127.0.0.1:18443");
    let rpc_user = env_or("BITCOIND_RPC_USER", "sc");
    let rpc_pass = env_or("BITCOIND_RPC_PASS", "sc");
    let p2p_listen = env_or("P2P_LISTEN", "0.0.0.0:9736");

    let seed_path = format!("{data_dir}/keys_seed");

    // Counterparty node: plain regtest ldk-node against the local esplora.
    let mut builder = Builder::new();
    builder.set_network(Network::Regtest);
    builder.set_chain_source_esplora(esplora.clone(), Some(EsploraSyncConfig::default()));
    builder.set_storage_dir_path(data_dir);
    builder
        .set_listening_addresses(vec![p2p_listen.parse().expect("bad P2P_LISTEN")])
        .expect("set_listening_addresses");
    let _ = builder.set_node_alias("sc-e2e-counterparty".to_string());

    // Deterministic-but-persistent entropy: keys_seed file under DATA_DIR so
    // the counterparty keeps its identity/funds across harness restarts.
    let entropy = ldk_node::entropy::NodeEntropy::from_seed_path(seed_path)
        .expect("load/create keys seed");
    let node = Arc::new(builder.build(entropy).expect("build ldk-node"));
    node.start().expect("start ldk-node");
    println!("[harness] counterparty node: {}", node.node_id());

    // Drain the event queue so it never wedges; log for debugging. Also drives
    // hold invoices: record an arriving HTLC, or fail it at once in `Fail` mode.
    let holds: Holds = Arc::new(Mutex::new(HashMap::new()));
    {
        let node = node.clone();
        let holds = holds.clone();
        std::thread::spawn(move || loop {
            let event = node.wait_next_event();
            println!("[harness] event: {:?}", event);
            if let ldk_node::Event::PaymentClaimable { payment_hash, claimable_amount_msat, .. } = &event {
                let mut map = holds.lock().unwrap();
                if let Some(hold) = map.get_mut(&payment_hash.0) {
                    hold.claimable_amount_msat = Some(*claimable_amount_msat);
                    if hold.mode == HoldMode::Fail {
                        match node.bolt11_payment().fail_for_hash(*payment_hash) {
                            Ok(()) => hold.state = "failed",
                            Err(e) => println!("[harness] fail_for_hash {payment_hash}: {e}"),
                        }
                    } else {
                        hold.state = "held";
                    }
                }
            }
            let _ = node.event_handled();
        });
    }

    let auth_b64 =
        base64::engine::general_purpose::STANDARD.encode(format!("{rpc_user}:{rpc_pass}"));
    let state = Arc::new(AppState {
        node,
        price_bits: AtomicU64::new(100_000.0f64.to_bits()),
        feeds_down: AtomicBool::new(false),
        holds,
        rpc_url,
        rpc_auth: format!("Basic {auth_b64}"),
        lsp_node_id: std::env::var("LSP_NODE_ID").ok(),
        lsp_p2p_addr: env_or("LSP_P2P_ADDR", "127.0.0.1:9735"),
    });

    let app = Router::new()
        .route("/pay", post(pay))
        .route("/invoice", post(invoice))
        .route("/address", post(address))
        .route("/send", post(send_onchain))
        .route("/mine", post(mine))
        .route("/price", post(set_price))
        .route("/feeds/bitstamp", get(feed_bitstamp))
        .route("/feeds/coingecko", get(feed_coingecko))
        .route("/feeds/kraken", get(feed_kraken))
        .route("/feeds/coinbase", get(feed_coinbase))
        .route("/feeds/blockchain", get(feed_blockchain))
        .route("/feeds/outage", post(set_feed_outage))
        .route("/hold-invoice", post(hold_invoice))
        .route("/hold-status", get(hold_status))
        .route("/hold-claim", post(hold_claim))
        .route("/hold-fail", post(hold_fail))
        .route("/bootstrap", post(bootstrap))
        .route("/audit-tail", get(audit_tail))
        .route("/info", get(info))
        .with_state(state);

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(&listen).await.expect("bind");
        println!("[harness] listening on {listen}");
        axum::serve(listener, app).await.expect("serve");
    });
}

type Resp = Result<Json<Value>, (axum::http::StatusCode, String)>;

fn err500(e: impl std::fmt::Display) -> (axum::http::StatusCode, String) {
    (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn bad_req(e: impl std::fmt::Display) -> (axum::http::StatusCode, String) {
    (axum::http::StatusCode::BAD_REQUEST, e.to_string())
}

/// POST /pay {"invoice": "lnbcrt..."} — pay and BLOCK until settled/failed,
/// so a flow's next assertion can rely on the payment being done.
async fn pay(State(st): State<Arc<AppState>>, Json(body): Json<Value>) -> Resp {
    let inv_str = body["invoice"].as_str().ok_or_else(|| bad_req("missing invoice"))?.to_string();
    let node = st.node.clone();
    tokio::task::spawn_blocking(move || {
        let invoice = Bolt11Invoice::from_str(&inv_str).map_err(bad_req)?;
        let payment_id = node.bolt11_payment().send(&invoice, None).map_err(err500)?;
        // Poll to a terminal state (JIT-channel opens can take a while).
        for _ in 0..120 {
            match node.payment(&payment_id).map(|p| p.status) {
                Some(PaymentStatus::Succeeded) => {
                    return Ok(Json(json!({"status": "succeeded", "payment_id": format!("{payment_id}")})))
                }
                Some(PaymentStatus::Failed) => return Err(err500("payment failed")),
                _ => std::thread::sleep(Duration::from_secs(1)),
            }
        }
        Err(err500("payment still pending after 120s"))
    })
    .await
    .map_err(err500)?
}

/// POST /invoice {"amount_msat": N} -> {"invoice": ...}
async fn invoice(State(st): State<Arc<AppState>>, Json(body): Json<Value>) -> Resp {
    let amount_msat = body["amount_msat"].as_u64().ok_or_else(|| bad_req("missing amount_msat"))?;
    let node = st.node.clone();
    tokio::task::spawn_blocking(move || {
        let desc = Bolt11InvoiceDescription::Direct(
            Description::new("sc-e2e".to_string()).map_err(err500)?,
        );
        let inv = node.bolt11_payment().receive(amount_msat, &desc, 3600).map_err(err500)?;
        Ok(Json(json!({"invoice": inv.to_string()})))
    })
    .await
    .map_err(err500)?
}

/// POST /address {} -> {"address": "bcrt1..."}
async fn address(State(st): State<Arc<AppState>>, Json(_body): Json<Value>) -> Resp {
    let node = st.node.clone();
    tokio::task::spawn_blocking(move || {
        let addr = node.onchain_payment().new_address().map_err(err500)?;
        Ok(Json(json!({"address": addr.to_string()})))
    })
    .await
    .map_err(err500)?
}

/// POST /send {"address": ..., "amount_sats": N} — counterparty pays onchain.
async fn send_onchain(State(st): State<Arc<AppState>>, Json(body): Json<Value>) -> Resp {
    let addr_str = body["address"].as_str().ok_or_else(|| bad_req("missing address"))?.to_string();
    let amount_sats = body["amount_sats"].as_u64().ok_or_else(|| bad_req("missing amount_sats"))?;
    let node = st.node.clone();
    tokio::task::spawn_blocking(move || {
        let addr = Address::from_str(&addr_str)
            .map_err(bad_req)?
            .require_network(Network::Regtest)
            .map_err(bad_req)?;
        let txid = node.onchain_payment().send_to_address(&addr, amount_sats, None).map_err(err500)?;
        Ok(Json(json!({"txid": txid.to_string()})))
    })
    .await
    .map_err(err500)?
}

/// POST /mine {"blocks": N} — mines to the counterparty's own address (which
/// also funds it once coinbases mature).
async fn mine(State(st): State<Arc<AppState>>, Json(body): Json<Value>) -> Resp {
    let blocks = body["blocks"].as_u64().unwrap_or(6);
    let st2 = st.clone();
    tokio::task::spawn_blocking(move || {
        let addr = st2.node.onchain_payment().new_address().map_err(err500)?;
        let hashes = rpc(&st2, "generatetoaddress", json!([blocks, addr.to_string()]))?;
        let _ = st2.node.sync_wallets();
        Ok(Json(json!({"mined": blocks, "tip": hashes.as_array().and_then(|a| a.last()).cloned()})))
    })
    .await
    .map_err(err500)?
}

/// POST /price {"price": 100000.0}
async fn set_price(State(st): State<Arc<AppState>>, Json(body): Json<Value>) -> Resp {
    let price = body["price"].as_f64().ok_or_else(|| bad_req("missing price"))?;
    st.price_bits.store(price.to_bits(), Ordering::SeqCst);
    println!("[harness] price set to {price}");
    Ok(Json(json!({"price": price})))
}

fn price(st: &AppState) -> f64 {
    f64::from_bits(st.price_bits.load(Ordering::SeqCst))
}

// Feed shapes mirror src/price_feeds.rs / the mobile Constants feed list, so a
// test build can point each feed URL at this harness unchanged.
fn feed(st: &AppState, body: Value) -> Resp {
    if st.feeds_down.load(Ordering::SeqCst) {
        return Err((axum::http::StatusCode::SERVICE_UNAVAILABLE, "feed outage (harness)".into()));
    }
    Ok(Json(body))
}
async fn feed_bitstamp(State(st): State<Arc<AppState>>) -> Resp {
    feed(&st, json!({"last": format!("{:.2}", price(&st))}))
}
async fn feed_coingecko(State(st): State<Arc<AppState>>) -> Resp {
    feed(&st, json!({"bitcoin": {"usd": price(&st)}}))
}
async fn feed_kraken(State(st): State<Arc<AppState>>) -> Resp {
    feed(&st, json!({"result": {"XXBTZUSD": {"c": [format!("{:.5}", price(&st)), "1.0"]}}}))
}
async fn feed_coinbase(State(st): State<Arc<AppState>>) -> Resp {
    feed(&st, json!({"data": {"amount": format!("{:.2}", price(&st))}}))
}
async fn feed_blockchain(State(st): State<Arc<AppState>>) -> Resp {
    feed(&st, json!({"USD": {"last": price(&st)}}))
}

/// POST /feeds/outage {"down": true|false} — take every mock price feed down or
/// back up. Both the app and the LSP price from these feeds in E2E.
async fn set_feed_outage(State(st): State<Arc<AppState>>, Json(body): Json<Value>) -> Resp {
    let down = body["down"].as_bool().ok_or_else(|| bad_req("missing down"))?;
    st.feeds_down.store(down, Ordering::SeqCst);
    println!("[harness] price feeds {}", if down { "DOWN" } else { "up" });
    Ok(Json(json!({"down": down})))
}

fn parse_hash(hex: &str) -> Result<[u8; 32], (axum::http::StatusCode, String)> {
    let hex = hex.trim();
    if hex.len() != 64 {
        return Err(bad_req("payment_hash must be 64 hex chars"));
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(bad_req)?;
    }
    Ok(out)
}

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// POST /hold-invoice {"amount_msat": N, "mode": "hold"|"fail"} ->
/// {"invoice", "payment_hash"}. `hold` keeps the payer's HTLC pending until
/// /hold-claim or /hold-fail; `fail` rejects it on arrival (recipient-rejected).
async fn hold_invoice(State(st): State<Arc<AppState>>, Json(body): Json<Value>) -> Resp {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let amount_msat = body["amount_msat"].as_u64().ok_or_else(|| bad_req("missing amount_msat"))?;
    let mode = match body["mode"].as_str().unwrap_or("hold") {
        "hold" => HoldMode::Hold,
        "fail" => HoldMode::Fail,
        other => return Err(bad_req(format!("unknown mode {other}"))),
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(err500)?
        .as_nanos();
    let seed = format!("sc-e2e-hold:{nanos}:{}", COUNTER.fetch_add(1, Ordering::SeqCst));
    let preimage = sha256::Hash::hash(seed.as_bytes()).to_byte_array();
    let hash = sha256::Hash::hash(&preimage).to_byte_array();
    let node = st.node.clone();
    let inv = tokio::task::spawn_blocking(move || {
        let desc = Bolt11InvoiceDescription::Direct(
            Description::new("sc-e2e-hold".to_string()).map_err(err500)?,
        );
        node.bolt11_payment()
            .receive_for_hash(amount_msat, &desc, 3600, PaymentHash(hash))
            .map_err(err500)
    })
    .await
    .map_err(err500)??;
    st.holds.lock().unwrap().insert(
        hash,
        Hold { preimage, mode, state: "open", claimable_amount_msat: None },
    );
    Ok(Json(json!({"invoice": inv.to_string(), "payment_hash": hex32(&hash)})))
}

/// GET /hold-status?hash=<hex> -> {"state", "claimable_amount_msat"}
async fn hold_status(
    State(st): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>,
) -> Resp {
    let hash = parse_hash(q.get("hash").map(String::as_str).unwrap_or(""))?;
    let map = st.holds.lock().unwrap();
    let hold = map.get(&hash).ok_or_else(|| bad_req("unknown hold invoice"))?;
    Ok(Json(json!({"state": hold.state, "claimable_amount_msat": hold.claimable_amount_msat})))
}

/// POST /hold-claim {"payment_hash"} — settle a held HTLC.
async fn hold_claim(State(st): State<Arc<AppState>>, Json(body): Json<Value>) -> Resp {
    let hash = parse_hash(body["payment_hash"].as_str().unwrap_or(""))?;
    let (preimage, amount) = {
        let map = st.holds.lock().unwrap();
        let hold = map.get(&hash).ok_or_else(|| bad_req("unknown hold invoice"))?;
        if hold.state != "held" {
            return Err(bad_req(format!("hold is {}, not held", hold.state)));
        }
        (hold.preimage, hold.claimable_amount_msat.unwrap_or(0))
    };
    let node = st.node.clone();
    tokio::task::spawn_blocking(move || {
        node.bolt11_payment()
            .claim_for_hash(PaymentHash(hash), amount, PaymentPreimage(preimage))
            .map_err(err500)
    })
    .await
    .map_err(err500)??;
    if let Some(hold) = st.holds.lock().unwrap().get_mut(&hash) {
        hold.state = "claimed";
    }
    Ok(Json(json!({"state": "claimed", "amount_msat": amount})))
}

/// POST /hold-fail {"payment_hash"} — reject a held HTLC back to the payer.
async fn hold_fail(State(st): State<Arc<AppState>>, Json(body): Json<Value>) -> Resp {
    let hash = parse_hash(body["payment_hash"].as_str().unwrap_or(""))?;
    let node = st.node.clone();
    tokio::task::spawn_blocking(move || {
        node.bolt11_payment().fail_for_hash(PaymentHash(hash)).map_err(err500)
    })
    .await
    .map_err(err500)??;
    if let Some(hold) = st.holds.lock().unwrap().get_mut(&hash) {
        hold.state = "failed";
    }
    Ok(Json(json!({"state": "failed"})))
}

/// Ask ldk-server (gRPC, TLS + api_key from the shared volume) for an onchain
/// address, so bootstrap can fund the LSP's JIT-channel wallet.
async fn lsp_onchain_address_once() -> Result<String, String> {
    let url = env_or("LDK_GRPC_URL", "ldk-server:3536");
    let cert_path = env_or("LDK_CERT_PATH", "/data/ldk-server/tls.crt");
    let key_path = env_or("LDK_API_KEY_PATH", "/data/ldk-server/regtest/api_key");
    let cert = std::fs::read(&cert_path).map_err(|e| format!("read LDK cert {cert_path}: {e}"))?;
    let key = std::fs::read(&key_path).map_err(|e| format!("read LDK api key {key_path}: {e}"))?;
    let api_key: String = key.iter().map(|b| format!("{b:02x}")).collect();
    let client = ldk_server_client::client::LdkServerClient::new(url, api_key, &cert)
        .map_err(|e| format!("connect to ldk-server grpc: {e}"))?;
    let resp = client
        .onchain_receive(ldk_server_client::ldk_server_grpc::api::OnchainReceiveRequest {})
        .await
        .map_err(|e| format!("ldk-server OnchainReceive: {e}"))?;
    Ok(resp.address)
}

async fn lsp_onchain_address() -> Result<String, String> {
    let mut last_err = String::new();
    for attempt in 1..=30 {
        match lsp_onchain_address_once().await {
            Ok(address) => return Ok(address),
            Err(err) => {
                last_err = err;
                println!("[harness] waiting for LSP onchain address ({attempt}/30): {last_err}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
    Err(format!(
        "could not fetch LSP onchain address after 60s: {last_err}"
    ))
}

/// POST /bootstrap {"channel_sats": N, "push_msat": M, "lsp_fund_sats": F}
/// Funds the counterparty (mines its own coinbases mature), funds the LSP's
/// ONCHAIN wallet (required for JIT channel opens — an unfunded LSP fails
/// LSPS2 with "insufficient funds", the exact prod incident of 2026-06/07),
/// and opens a channel to the LSP so /pay has a route.
async fn bootstrap(State(st): State<Arc<AppState>>, Json(body): Json<Value>) -> Resp {
    let lsp_fund_addr = lsp_onchain_address().await.map_err(err500)?;
    let channel_sats = body["channel_sats"].as_u64().unwrap_or(5_000_000);
    // Default: push HALF the channel to the LSP at open, so the LSP has
    // outbound liquidity toward the counterparty from the start. Without it,
    // app -> LSP -> counterparty payments (Step 6) have no route on a fresh
    // harness until something first flows counterparty -> LSP.
    let push_msat = Some(body["push_msat"].as_u64().unwrap_or(channel_sats / 2 * 1000));
    let lsp_id = st.lsp_node_id.clone().ok_or_else(|| bad_req("LSP_NODE_ID env not set"))?;
    let st2 = st.clone();
    tokio::task::spawn_blocking(move || {
        let node = &st2.node;

        // 1) Fund: mine 101 blocks to our own address (coinbase maturity).
        if node.list_balances().spendable_onchain_balance_sats < channel_sats + 50_000 {
            let addr = node.onchain_payment().new_address().map_err(err500)?;
            rpc(&st2, "generatetoaddress", json!([101, addr.to_string()]))?;
            for _ in 0..60 {
                let _ = node.sync_wallets();
                if node.list_balances().spendable_onchain_balance_sats >= channel_sats + 50_000 {
                    break;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        }
        let spendable = node.list_balances().spendable_onchain_balance_sats;
        if spendable < channel_sats + 50_000 {
            return Err(err500(format!("funding did not land: spendable={spendable}")));
        }

        // 2) Fund the LSP's onchain wallet for JIT channel opens.
        let lsp_fund_sats = body["lsp_fund_sats"].as_u64().unwrap_or(10_000_000);
        let addr = Address::from_str(&lsp_fund_addr)
            .map_err(bad_req)?
            .require_network(Network::Regtest)
            .map_err(bad_req)?;
        node.onchain_payment()
            .send_to_address(&addr, lsp_fund_sats, None)
            .map_err(err500)?;
        println!("[harness] funded LSP onchain: {lsp_fund_sats} sats -> {lsp_fund_addr}");

        // 3) Channel to the LSP — liquidity-aware. Every lifecycle run drains
        // ~90k sats from the counterparty's outbound side (onboard $85 + $10
        // receive + ...) and nothing flows back over Lightning (offboards
        // return onchain). "A ready channel exists" therefore eventually means
        // "a DRY channel exists" and /pay dies with 500 "Failed to send the
        // given payment". Instead: keep total outbound above a floor, opening
        // an additional channel whenever it drops below.
        let outbound_total = |node: &ldk_node::Node| -> u64 {
            node.list_channels()
                .iter()
                .filter(|c| c.is_channel_ready)
                .map(|c| c.outbound_capacity_msat)
                .sum()
        };
        let min_outbound_msat = body["min_outbound_sats"].as_u64().unwrap_or(500_000) * 1000;
        let outbound_before = outbound_total(&node);
        if outbound_before < min_outbound_msat {
            println!(
                "[harness] outbound {}k sats below {}k floor — opening a top-up channel",
                outbound_before / 1_000_000,
                min_outbound_msat / 1_000_000
            );
            let lsp_pk = PublicKey::from_str(&lsp_id).map_err(bad_req)?;
            let lsp_addr = SocketAddress::from_str(&st2.lsp_p2p_addr)
                .map_err(|e| bad_req(format!("bad LSP_P2P_ADDR: {e:?}")))?;
            node.open_channel(lsp_pk, lsp_addr, channel_sats, push_msat, None)
                .map_err(err500)?;
            // Confirm it.
            let addr = node.onchain_payment().new_address().map_err(err500)?;
            rpc(&st2, "generatetoaddress", json!([6, addr.to_string()]))?;
            for _ in 0..60 {
                let _ = node.sync_wallets();
                if outbound_total(&node) >= min_outbound_msat {
                    break;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        }
        // Confirm the LSP funding even when the channel already existed.
        let addr = st2.node.onchain_payment().new_address().map_err(err500)?;
        rpc(&st2, "generatetoaddress", json!([6, addr.to_string()]))?;
        let _ = st2.node.sync_wallets();

        let ready = st2.node.list_channels().iter().any(|c| c.is_channel_ready);
        Ok(Json(json!({
            "node_id": st2.node.node_id().to_string(),
            "spendable_onchain_sats": st2.node.list_balances().spendable_onchain_balance_sats,
            "channel_ready": ready,
            "lsp_funded_sats": lsp_fund_sats,
        })))
    })
    .await
    .map_err(err500)?
}

/// GET /audit-tail?n=50[&event=A,B][&exclude=C,D][&since=ISO] — last N lines of
/// the SC daemon's audit log (mounted read-only), so flows can assert LSP-side
/// effects (settlements, trades).
///
/// Filters apply BEFORE the last-N cut: `event` keeps only the named events,
/// `exclude` drops named events (default `SYNC_MESSAGE_FAILED`, which the LSP
/// sprays at closed channels fast enough to push every real event out of any
/// fixed window — that made "absent" asserts pass vacuously), and `since` keeps
/// lines whose `ts` sorts after the given ISO timestamp. `exclude=` (empty)
/// disables the default.
async fn audit_tail(
    axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>,
) -> Resp {
    let n: usize = q.get("n").and_then(|v| v.parse().ok()).unwrap_or(50);
    let names = |key: &str, default: &str| -> Vec<String> {
        q.get(key)
            .map(String::as_str)
            .unwrap_or(default)
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let only = names("event", "");
    let exclude = names("exclude", "SYNC_MESSAGE_FAILED");
    let since = q.get("since").cloned();
    let path = env_or("SC_LSP_AUDIT", "/data/sc-lsp/audit_log.txt");
    let bytes = std::fs::read(&path).map_err(|e| err500(format!("read {path}: {e}")))?;
    let text = String::from_utf8_lossy(&bytes);
    let event_of = |line: &str| -> Option<String> {
        let at = line.find("\"event\":\"")? + 9;
        let end = line[at..].find('"')?;
        Some(line[at..at + end].to_string())
    };
    let ts_of = |line: &str| -> Option<String> {
        let at = line.find("\"ts\":\"")? + 6;
        let end = line[at..].find('"')?;
        Some(line[at..at + end].to_string())
    };
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| {
            let ev = event_of(line);
            if !only.is_empty() && !ev.as_ref().is_some_and(|e| only.contains(e)) {
                return false;
            }
            if ev.as_ref().is_some_and(|e| exclude.contains(e)) {
                return false;
            }
            match (&since, ts_of(line)) {
                (Some(since), Some(ts)) => ts.as_str() > since.as_str(),
                (Some(_), None) => false,
                _ => true,
            }
        })
        .collect();
    let start = kept.len().saturating_sub(n);
    Ok(Json(json!({ "lines": &kept[start..] })))
}

/// GET /info
async fn info(State(st): State<Arc<AppState>>) -> Json<Value> {
    let balances = st.node.list_balances();
    let channels: Vec<Value> = st
        .node
        .list_channels()
        .iter()
        .map(|c| {
            json!({
                "counterparty": c.counterparty_node_id.to_string(),
                "ready": c.is_channel_ready,
                "value_sats": c.channel_value_sats,
                "outbound_msat": c.outbound_capacity_msat,
            })
        })
        .collect();
    Json(json!({
        "node_id": st.node.node_id().to_string(),
        "price": price(&st),
        "spendable_onchain_sats": balances.spendable_onchain_balance_sats,
        "lightning_sats": balances.total_lightning_balance_sats,
        "channels": channels,
    }))
}

/// Minimal bitcoind JSON-RPC call.
fn rpc(st: &AppState, method: &str, params: Value) -> Result<Value, (axum::http::StatusCode, String)> {
    let body = json!({"jsonrpc": "1.0", "id": "harness", "method": method, "params": params});
    let resp = ureq::post(&st.rpc_url)
        .set("Authorization", &st.rpc_auth)
        .send_json(body)
        .map_err(|e| err500(format!("bitcoind rpc {method}: {e}")))?;
    let v: Value = resp.into_json().map_err(err500)?;
    if !v["error"].is_null() {
        return Err(err500(format!("bitcoind rpc {method}: {}", v["error"])));
    }
    Ok(v["result"].clone())
}
