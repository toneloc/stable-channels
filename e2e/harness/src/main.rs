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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
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
use ldk_node::payment::PaymentStatus;
use ldk_node::{Builder, Node};
use serde_json::{json, Value};

struct AppState {
    node: Arc<Node>,
    /// BTC/USD price as f64 bits — the mocked feed value.
    price_bits: AtomicU64,
    rpc_url: String,
    rpc_auth: String, // "Basic <b64>"
    lsp_node_id: Option<String>,
    lsp_p2p_addr: String,
}

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

    // Drain the event queue so it never wedges; log for debugging.
    {
        let node = node.clone();
        std::thread::spawn(move || loop {
            let event = node.wait_next_event();
            println!("[harness] event: {:?}", event);
            let _ = node.event_handled();
        });
    }

    let auth_b64 =
        base64::engine::general_purpose::STANDARD.encode(format!("{rpc_user}:{rpc_pass}"));
    let state = Arc::new(AppState {
        node,
        price_bits: AtomicU64::new(100_000.0f64.to_bits()),
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
        .route("/bootstrap", post(bootstrap))
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
async fn feed_bitstamp(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({"last": format!("{:.2}", price(&st))}))
}
async fn feed_coingecko(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({"bitcoin": {"usd": price(&st)}}))
}
async fn feed_kraken(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({"result": {"XXBTZUSD": {"c": [format!("{:.5}", price(&st)), "1.0"]}}}))
}
async fn feed_coinbase(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({"data": {"amount": format!("{:.2}", price(&st))}}))
}
async fn feed_blockchain(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({"USD": {"last": price(&st)}}))
}

/// Ask ldk-server (gRPC, TLS + api_key from the shared volume) for an onchain
/// address, so bootstrap can fund the LSP's JIT-channel wallet. None if the
/// cert/api_key aren't readable (volume not mounted) — funding is skipped.
async fn lsp_onchain_address() -> Option<String> {
    let url = env_or("LDK_GRPC_URL", "ldk-server:3536");
    let cert = std::fs::read(env_or("LDK_CERT_PATH", "/data/ldk-server/tls.crt")).ok()?;
    let key = std::fs::read(env_or("LDK_API_KEY_PATH", "/data/ldk-server/regtest/api_key")).ok()?;
    let api_key: String = key.iter().map(|b| format!("{b:02x}")).collect();
    let client = ldk_server_client::client::LdkServerClient::new(url, api_key, &cert).ok()?;
    let resp = client
        .onchain_receive(ldk_server_client::ldk_server_grpc::api::OnchainReceiveRequest {})
        .await
        .ok()?;
    Some(resp.address)
}

/// POST /bootstrap {"channel_sats": N, "push_msat": M, "lsp_fund_sats": F}
/// Funds the counterparty (mines its own coinbases mature), funds the LSP's
/// ONCHAIN wallet (required for JIT channel opens — an unfunded LSP fails
/// LSPS2 with "insufficient funds", the exact prod incident of 2026-06/07),
/// and opens a channel to the LSP so /pay has a route.
async fn bootstrap(State(st): State<Arc<AppState>>, Json(body): Json<Value>) -> Resp {
    let lsp_fund_addr = lsp_onchain_address().await;
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
        let mut lsp_funded = false;
        if let Some(ref addr_str) = lsp_fund_addr {
            let addr = Address::from_str(addr_str)
                .map_err(bad_req)?
                .require_network(Network::Regtest)
                .map_err(bad_req)?;
            node.onchain_payment().send_to_address(&addr, lsp_fund_sats, None).map_err(err500)?;
            lsp_funded = true;
            println!("[harness] funded LSP onchain: {lsp_fund_sats} sats -> {addr_str}");
        } else {
            println!("[harness] WARNING: could not fetch LSP onchain address — JIT opens will fail");
        }

        // 3) Channel to the LSP.
        if !node.list_channels().iter().any(|c| c.is_channel_ready) {
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
                if node.list_channels().iter().any(|c| c.is_channel_ready) {
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
            "lsp_funded_sats": if lsp_funded { lsp_fund_sats } else { 0 },
        })))
    })
    .await
    .map_err(err500)?
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
