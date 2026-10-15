//! Onboarding-swap handlers: public JSON routes (outside the HMAC layer, like
//! `/api/channel-exists`) that let a wallet without a channel pay its LSPS2 JIT invoice in
//! USDT/USDC through the configured swap provider. Create / status / refund are signed by the
//! wallet's node key (ONBOARDING_SWAP_V1); settled is proven by the payment preimage instead.
//! Responses carry only public deposit fields: the provider order token never leaves the LSP.
//! Onboarding itself still continues only when LDK reports the invoice paid.

use std::str::FromStr;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use bitcoin_hashes::{sha256, Hash};
use ldk_node::lightning_invoice::Bolt11Invoice;
use ldk_server_client::ldk_server_grpc::api::{GetNodeInfoRequest, VerifySignatureRequest};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::handlers::channel_exists::is_valid_node_id;
use crate::stable_manager::LdkServerCalls;
use crate::state::AppState;
use crate::swap::address::refund_address_error;
use crate::swap::provider::{Fee, ProviderError};
use crate::swap::store::SwapRow;
use crate::swap::{OnboardingSwaps, PayInAsset, SwapState};

/// Freshness window for the signed timestamp, in seconds (same as RegisterPush).
const SIG_WINDOW_SECS: u64 = 300;
/// Remaining invoice life the provider needs: deposit window 10m + settlement SLA 15m + margin
/// 5m (OpenReceive fixedfloat.ts). A shorter invoice could expire after the user has paid.
pub const MIN_INVOICE_EXPIRY_SECS: u64 = (10 + 15 + 5) * 60;

#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);

fn err(status: StatusCode, message: impl Into<String>) -> ApiError {
    ApiError(status, message.into())
}

fn bad(message: impl Into<String>) -> ApiError {
    err(StatusCode::BAD_REQUEST, message)
}

fn store_err(e: rusqlite::Error) -> ApiError {
    tracing::error!("[swap] store error: {}", e);
    err(StatusCode::INTERNAL_SERVER_ERROR, "swap store unavailable")
}

fn provider_err(e: ProviderError) -> ApiError {
    match e {
        ProviderError::Api(m) => err(StatusCode::BAD_GATEWAY, m),
        other => err(StatusCode::SERVICE_UNAVAILABLE, other.to_string()),
    }
}

#[derive(Deserialize)]
pub struct CreateRequest {
    pub node_id: String,
    pub bolt11: String,
    pub pay_in_asset: String,
    pub ts: u64,
    pub signature: String,
}

#[derive(Deserialize)]
pub struct StatusRequest {
    pub node_id: String,
    pub payment_hash: String,
    pub ts: u64,
    pub signature: String,
}

#[derive(Deserialize)]
pub struct SettledRequest {
    pub payment_hash: String,
    pub preimage: String,
}

#[derive(Deserialize)]
pub struct RefundRequest {
    pub node_id: String,
    pub payment_hash: String,
    pub refund_address: String,
    pub ts: u64,
    pub signature: String,
}

/// Verify an ONBOARDING_SWAP_V1 node-key signature and the timestamp window.
#[allow(clippy::too_many_arguments)]
async fn verify_signed(
    ldk: &dyn LdkServerCalls,
    action: &str,
    node_id: &str,
    payment_hash: &str,
    pay_in_asset: &str,
    refund_address: &str,
    ts: u64,
    signature: &str,
    now: u64,
) -> Result<(), ApiError> {
    let unauthorized = || err(StatusCode::UNAUTHORIZED, "invalid signature");
    if !is_valid_node_id(node_id) || now.abs_diff(ts) > SIG_WINDOW_SECS {
        return Err(unauthorized());
    }
    let message = crate::messages::onboarding_swap_signed_bytes(
        action,
        node_id,
        payment_hash,
        pay_in_asset,
        refund_address,
        ts,
    );
    let valid = ldk
        .verify_signature(VerifySignatureRequest {
            message: message.into(),
            signature: signature.to_string(),
            public_key: node_id.to_string(),
        })
        .await
        .map(|r| r.valid)
        .unwrap_or(false);
    if !valid {
        stable_channels::audit::audit_event(
            "ONBOARDING_SWAP_SIGNATURE_INVALID",
            json!({ "node_id": node_id, "action": action, "payment_hash": payment_hash }),
        );
        return Err(unauthorized());
    }
    Ok(())
}

/// The only shape a swap row ever leaves the LSP in. Built field by field (SwapRow is not
/// Serialize), so provider_token cannot slip into a response by accident.
pub fn public_json(row: &SwapRow, fee: Option<&Fee>) -> Value {
    let asset = row.pay_in_asset;
    let (warning_title, warning) = asset.network_warning(&row.deposit_amount);
    let mut body = json!({
        "payment_hash": row.payment_hash,
        "pay_in_asset": asset.as_str(),
        "coin": asset.coin(),
        "network": asset.network_label(),
        "amount_msat": row.amount_msat,
        "provider_order_id": row.provider_order_id,
        "deposit_address": row.deposit_address,
        "deposit_amount": row.deposit_amount,
        "deposit_memo": row.deposit_memo,
        "deposit_risk": asset.deposit_risk().as_str(),
        "network_warning_title": warning_title,
        "network_warning": warning,
        "provider_state": row.provider_state.as_str(),
        "state_reason": row.state_reason,
        "expires_at": row.expires_at,
        "settled": row.settled_at.is_some(),
        "refund_address": row.refund_address,
    });
    if let Some(fee) = fee {
        body["fee"] = json!({ "pay_in_usd": fee.pay_in_usd, "payout_usd": fee.payout_usd });
    }
    body
}

/// Rails a wallet may offer: only USDT/USDC rails the provider currently accepts and prices.
pub async fn options(swaps: Option<&OnboardingSwaps>) -> Value {
    let Some(swaps) = swaps else {
        return json!({ "enabled": false });
    };
    match swaps.catalog().await {
        Ok((_, catalog)) => {
            let rails: Vec<Value> = catalog
                .rails
                .iter()
                .map(|r| {
                    json!({
                        "asset": r.asset.as_str(),
                        "coin": r.asset.coin(),
                        "network": r.asset.network_label(),
                        "deposit_risk": r.asset.deposit_risk().as_str(),
                        "min_msat": r.min_msat,
                        "max_msat": r.max_msat,
                    })
                })
                .collect();
            json!({ "enabled": !rails.is_empty(), "rails": rails })
        }
        Err(e) => {
            tracing::warn!("[swap] options: {}", e);
            json!({ "enabled": false, "reason": "provider_unreachable" })
        }
    }
}

pub async fn create(
    swaps: &OnboardingSwaps,
    ldk: &dyn LdkServerCalls,
    lsp_node_id: &str,
    req: CreateRequest,
    now: u64,
) -> Result<Value, ApiError> {
    let node_id = req.node_id.to_lowercase();
    let asset =
        PayInAsset::parse(&req.pay_in_asset).ok_or_else(|| bad("unsupported pay_in_asset"))?;
    let bolt11 = req.bolt11.trim().to_string();
    let invoice = Bolt11Invoice::from_str(&bolt11).map_err(|_| bad("invalid bolt11"))?;
    let payment_hash = hex::encode(invoice.payment_hash().0);
    verify_signed(
        ldk,
        "create",
        &node_id,
        &payment_hash,
        asset.as_str(),
        "",
        req.ts,
        &req.signature,
        now,
    )
    .await?;

    let _guard = swaps.create_lock.lock().await;
    // Idempotent per invoice: a retry (or an app restart) gets the live attempt, not a second order.
    if let Some(row) = swaps.store.get(&payment_hash).map_err(store_err)? {
        if row.node_id != node_id || row.pay_in_asset != asset {
            return Err(err(
                StatusCode::CONFLICT,
                "a different swap already exists for this invoice",
            ));
        }
        return Ok(public_json(&row, None));
    }

    if invoice.get_payee_pub_key().to_string() != node_id {
        return Err(bad("invoice payee does not match node_id"));
    }
    if invoice.currency() != swaps.currency {
        return Err(bad("invoice is for a different network"));
    }
    // Real LSPS2 JIT invoices always route-hint through this LSP. Requiring it stops the endpoint
    // from being used to open provider orders for arbitrary invoices.
    let via_lsp = invoice.route_hints().iter().any(|hint| {
        hint.0
            .iter()
            .any(|hop| hop.src_node_id.to_string() == lsp_node_id)
    });
    if !via_lsp {
        return Err(bad("invoice is not a JIT invoice through this LSP"));
    }
    let amount_msat = invoice
        .amount_milli_satoshis()
        .ok_or_else(|| bad("invoice has no amount"))?;
    let expires_at = invoice.expires_at().map(|d| d.as_secs()).unwrap_or(0);
    if expires_at < now + MIN_INVOICE_EXPIRY_SECS {
        return Err(bad(format!(
            "invoice must stay valid for at least {MIN_INVOICE_EXPIRY_SECS}s"
        )));
    }

    let (provider_id, catalog) = swaps.catalog().await.map_err(provider_err)?;
    let rail = catalog
        .rail(asset)
        .ok_or_else(|| bad("rail temporarily unavailable"))?;
    if amount_msat < rail.min_msat {
        return Err(bad("amount_too_small"));
    }
    if amount_msat > rail.max_msat {
        return Err(bad("amount_too_large"));
    }
    // Counted only here, right before a provider order would exist: rejected requests and
    // idempotent retries cost nothing.
    let limited = swaps
        .rate_limit
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .try_acquire(&node_id, now);
    if let Err(message) = limited {
        stable_channels::audit::audit_event(
            "ONBOARDING_SWAP_RATE_LIMITED",
            json!({ "node_id": node_id, "payment_hash": payment_hash, "reason": message }),
        );
        return Err(err(StatusCode::TOO_MANY_REQUESTS, message));
    }
    let provider = swaps
        .provider(&provider_id)
        .ok_or_else(|| err(StatusCode::SERVICE_UNAVAILABLE, "provider not configured"))?;
    let order = match provider
        .create(rail, &catalog.lightning_code, &bolt11, amount_msat)
        .await
    {
        Ok(order) => order,
        Err(e) => {
            if matches!(e, ProviderError::Unavailable(_)) {
                swaps.invalidate_catalog().await;
            }
            stable_channels::audit::audit_event(
                "ONBOARDING_SWAP_CREATE_FAILED",
                json!({ "payment_hash": payment_hash, "node_id": node_id, "provider_id": provider_id, "error": e.to_string() }),
            );
            return Err(provider_err(e));
        }
    };

    let row = SwapRow {
        payment_hash: payment_hash.clone(),
        node_id: node_id.clone(),
        bolt11,
        amount_msat,
        pay_in_asset: asset,
        provider_id: provider_id.clone(),
        provider_order_id: order.id,
        provider_token: order.token,
        deposit_address: order.deposit_address,
        deposit_amount: order.deposit_amount,
        deposit_memo: order.deposit_memo,
        provider_state: order.status.state,
        state_reason: order
            .status
            .refund_reason
            .or(order.status.attention_reason)
            .map(str::to_string),
        created_at: now,
        expires_at: order.expires_at,
        settled_at: None,
        refund_address: None,
    };
    swaps.store.insert(&row).map_err(store_err)?;
    stable_channels::audit::audit_event(
        "ONBOARDING_SWAP_CREATED",
        json!({
            "payment_hash": payment_hash,
            "node_id": node_id,
            "pay_in_asset": asset.as_str(),
            "amount_msat": amount_msat,
            "provider_id": provider_id,
            "provider_order_id": row.provider_order_id,
            "deposit_amount": row.deposit_amount,
            "provider_state": row.provider_state.as_str(),
        }),
    );
    Ok(public_json(&row, order.fee.as_ref()))
}

/// Load a row the signer owns; another node's attempt looks exactly like a missing one.
fn owned_row(
    swaps: &OnboardingSwaps,
    payment_hash: &str,
    node_id: &str,
) -> Result<SwapRow, ApiError> {
    swaps
        .store
        .get(payment_hash)
        .map_err(store_err)?
        .filter(|row| row.node_id == node_id)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "unknown swap"))
}

/// Resume after restart: answers from the store, which the poller keeps fresh.
pub async fn status(
    swaps: &OnboardingSwaps,
    ldk: &dyn LdkServerCalls,
    req: StatusRequest,
    now: u64,
) -> Result<Value, ApiError> {
    let node_id = req.node_id.to_lowercase();
    let payment_hash = req.payment_hash.to_lowercase();
    verify_signed(
        ldk,
        "status",
        &node_id,
        &payment_hash,
        "",
        "",
        req.ts,
        &req.signature,
        now,
    )
    .await?;
    Ok(public_json(
        &owned_row(swaps, &payment_hash, &node_id)?,
        None,
    ))
}

/// The wallet reports LDK settled the invoice. sha256(preimage) == payment_hash is the proof, so
/// no signature is needed. Settlement is final and stops provider polling.
pub async fn settled(
    swaps: &OnboardingSwaps,
    req: SettledRequest,
    now: u64,
) -> Result<Value, ApiError> {
    let payment_hash = req.payment_hash.to_lowercase();
    let preimage = hex::decode(req.preimage.trim()).map_err(|_| bad("invalid preimage"))?;
    if preimage.len() != 32
        || hex::encode(sha256::Hash::hash(&preimage).to_byte_array()) != payment_hash
    {
        return Err(bad("preimage does not match payment_hash"));
    }
    let row = swaps
        .store
        .get(&payment_hash)
        .map_err(store_err)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "unknown swap"))?;
    if swaps
        .store
        .mark_settled(&payment_hash, now)
        .map_err(store_err)?
    {
        stable_channels::audit::audit_event(
            "ONBOARDING_SWAP_SETTLED",
            json!({
                "payment_hash": payment_hash,
                "node_id": row.node_id,
                "provider_order_id": row.provider_order_id,
                "last_provider_state": row.provider_state.as_str(),
            }),
        );
    }
    Ok(json!({ "ok": true, "payment_hash": payment_hash }))
}

/// Request a refund of an unconverted deposit. The provider state is refreshed first and only
/// `refund_required` is accepted (OpenReceive rule), so a stale UI cannot trigger a refund.
pub async fn refund(
    swaps: &OnboardingSwaps,
    ldk: &dyn LdkServerCalls,
    req: RefundRequest,
    now: u64,
) -> Result<Value, ApiError> {
    let node_id = req.node_id.to_lowercase();
    let payment_hash = req.payment_hash.to_lowercase();
    let address = req.refund_address.trim();
    verify_signed(
        ldk,
        "refund",
        &node_id,
        &payment_hash,
        "",
        &req.refund_address,
        req.ts,
        &req.signature,
        now,
    )
    .await?;
    let row = owned_row(swaps, &payment_hash, &node_id)?;
    // Checked against the attempt's own rail: a valid address on another chain loses the refund.
    if let Some(message) = refund_address_error(row.pay_in_asset, address) {
        return Err(bad(message));
    }
    if row.settled_at.is_some() {
        return Err(err(StatusCode::CONFLICT, "swap already settled"));
    }
    let (state, _) = swaps.refresh(&row).await.map_err(provider_err)?;
    if state != SwapState::RefundRequired {
        return Err(err(
            StatusCode::CONFLICT,
            format!("refund not available in state {}", state.as_str()),
        ));
    }
    let provider = swaps
        .provider(&row.provider_id)
        .ok_or_else(|| err(StatusCode::SERVICE_UNAVAILABLE, "provider not configured"))?;
    provider
        .emergency_refund(&row.provider_order_id, &row.provider_token, address)
        .await
        .map_err(provider_err)?;
    swaps
        .store
        .set_refund_address(&payment_hash, address)
        .map_err(store_err)?;
    stable_channels::audit::audit_event(
        "ONBOARDING_SWAP_REFUND_REQUESTED",
        json!({
            "payment_hash": payment_hash,
            "node_id": node_id,
            "provider_order_id": row.provider_order_id,
            "refund_address": address,
        }),
    );
    // Best effort: pick up refund_pending now rather than on the next poll.
    if let Err(e) = swaps.refresh(&row).await {
        tracing::warn!("[swap] post-refund refresh failed: {}", e);
    }
    Ok(public_json(
        &owned_row(swaps, &payment_hash, &node_id)?,
        None,
    ))
}

// ---- axum wrappers ----

fn json_response(status: StatusCode, body: Value) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap()
}

fn respond(result: Result<Value, ApiError>) -> Response {
    match result {
        Ok(body) => json_response(StatusCode::OK, body),
        Err(ApiError(status, message)) => json_response(status, json!({ "error": message })),
    }
}

fn parse<T: DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|e| bad(e.to_string()))
}

fn enabled(state: &AppState) -> Result<&OnboardingSwaps, ApiError> {
    state
        .onboarding_swaps
        .as_deref()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "onboarding swaps disabled"))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub async fn options_handler(State(state): State<AppState>) -> Response {
    json_response(
        StatusCode::OK,
        options(state.onboarding_swaps.as_deref()).await,
    )
}

/// This LSP's node id, asked of LDK Server once and cached (it never changes while running).
async fn lsp_node_id<'a>(
    state: &AppState,
    swaps: &'a OnboardingSwaps,
) -> Result<&'a str, ApiError> {
    swaps
        .lsp_node_id
        .get_or_try_init(|| async {
            match state.ldk_server.get_node_info(GetNodeInfoRequest {}).await {
                Ok(info) if !info.node_id.is_empty() => Ok(info.node_id.to_lowercase()),
                Ok(_) => Err(err(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "LSP node id unavailable",
                )),
                Err(e) => Err(err(StatusCode::SERVICE_UNAVAILABLE, e.to_string())),
            }
        })
        .await
        .map(String::as_str)
}

pub async fn create_handler(State(state): State<AppState>, body: Bytes) -> Response {
    let ldk = state.ldk_server.as_ref() as &dyn LdkServerCalls;
    respond(
        async {
            let swaps = enabled(&state)?;
            let lsp = lsp_node_id(&state, swaps).await?;
            create(swaps, ldk, lsp, parse(&body)?, now_secs()).await
        }
        .await,
    )
}

pub async fn status_handler(State(state): State<AppState>, body: Bytes) -> Response {
    let ldk = state.ldk_server.as_ref() as &dyn LdkServerCalls;
    respond(async { status(enabled(&state)?, ldk, parse(&body)?, now_secs()).await }.await)
}

pub async fn settled_handler(State(state): State<AppState>, body: Bytes) -> Response {
    respond(async { settled(enabled(&state)?, parse(&body)?, now_secs()).await }.await)
}

pub async fn refund_handler(State(state): State<AppState>, body: Bytes) -> Response {
    let ldk = state.ldk_server.as_ref() as &dyn LdkServerCalls;
    respond(async { refund(enabled(&state)?, ldk, parse(&body)?, now_secs()).await }.await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swap::fake::{FakeProvider, ETH_ADDRESS, TRON_ADDRESS};
    use crate::swap::store::SwapStore;
    use async_trait::async_trait;
    use ldk_node::bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use ldk_node::lightning_invoice::{Currency, InvoiceBuilder};
    use ldk_node::lightning_types::payment::{PaymentHash, PaymentSecret};
    use ldk_node::lightning_types::routing::{RouteHint, RouteHintHop, RoutingFees};
    use ldk_server_client::error::LdkServerError;
    use ldk_server_client::ldk_server_grpc::api::{
        ListChannelsRequest, ListChannelsResponse, SignMessageRequest, SignMessageResponse,
        SpontaneousSendRequest, SpontaneousSendResponse, VerifySignatureResponse,
    };
    use std::time::Duration;

    const NOW: u64 = 1_800_000_000;
    const PREIMAGE: [u8; 32] = [7; 32];
    const AMOUNT_MSAT: u64 = 150_000_000;

    /// Accepts exactly the signature "sig:<sha256 of canonical bytes>" from the claimed key, so
    /// a test that signs the wrong action, ts or field is rejected like a real bad signature.
    struct SigFake;

    fn fake_sign(message: &[u8]) -> String {
        format!("sig:{}", sha256::Hash::hash(message))
    }

    #[async_trait]
    impl LdkServerCalls for SigFake {
        async fn list_channels(
            &self,
            _r: ListChannelsRequest,
        ) -> Result<ListChannelsResponse, LdkServerError> {
            Ok(ListChannelsResponse { channels: vec![] })
        }
        async fn spontaneous_send(
            &self,
            _r: SpontaneousSendRequest,
        ) -> Result<SpontaneousSendResponse, LdkServerError> {
            Ok(SpontaneousSendResponse {
                payment_id: String::new(),
            })
        }
        async fn sign_message(
            &self,
            _r: SignMessageRequest,
        ) -> Result<SignMessageResponse, LdkServerError> {
            Ok(SignMessageResponse {
                signature: String::new(),
            })
        }
        async fn verify_signature(
            &self,
            r: VerifySignatureRequest,
        ) -> Result<VerifySignatureResponse, LdkServerError> {
            Ok(VerifySignatureResponse {
                valid: r.signature == fake_sign(&r.message),
            })
        }
    }

    fn user_key() -> (SecretKey, String) {
        let sk = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let pk = PublicKey::from_secret_key(&Secp256k1::new(), &sk);
        (sk, pk.to_string())
    }

    fn lsp_key() -> PublicKey {
        PublicKey::from_secret_key(
            &Secp256k1::new(),
            &SecretKey::from_slice(&[0x33; 32]).unwrap(),
        )
    }

    fn lsp() -> String {
        lsp_key().to_string()
    }

    /// A real JIT invoice's shape: regtest (the test LSP's network), route hint through the LSP.
    fn invoice_with(
        sk: &SecretKey,
        preimage: [u8; 32],
        amount_msat: Option<u64>,
        expiry_secs: u64,
    ) -> Bolt11Invoice {
        invoice_custom(
            sk,
            preimage,
            amount_msat,
            expiry_secs,
            Currency::Regtest,
            Some(lsp_key()),
        )
    }

    fn invoice_custom(
        sk: &SecretKey,
        preimage: [u8; 32],
        amount_msat: Option<u64>,
        expiry_secs: u64,
        currency: Currency,
        hint_via: Option<PublicKey>,
    ) -> Bolt11Invoice {
        let hash = PaymentHash(sha256::Hash::hash(&preimage).to_byte_array());
        let builder = InvoiceBuilder::new(currency)
            .description("onboarding".into())
            .payment_hash(hash)
            .payment_secret(PaymentSecret([42; 32]))
            .duration_since_epoch(Duration::from_secs(NOW))
            .min_final_cltv_expiry_delta(144)
            .expiry_time(Duration::from_secs(expiry_secs));
        let builder = match hint_via {
            Some(src_node_id) => builder.private_route(RouteHint(vec![RouteHintHop {
                src_node_id,
                short_channel_id: 42,
                fees: RoutingFees {
                    base_msat: 0,
                    proportional_millionths: 0,
                },
                cltv_expiry_delta: 144,
                htlc_minimum_msat: None,
                htlc_maximum_msat: None,
            }])),
            None => builder,
        };
        let builder = match amount_msat {
            Some(a) => builder.amount_milli_satoshis(a),
            None => builder,
        };
        builder
            .build_signed(|m| Secp256k1::new().sign_ecdsa_recoverable(m, sk))
            .unwrap()
    }

    fn create_req(node_id: &str, invoice: &Bolt11Invoice, asset: &str) -> CreateRequest {
        let hash = hex::encode(invoice.payment_hash().0);
        let msg =
            crate::messages::onboarding_swap_signed_bytes("create", node_id, &hash, asset, "", NOW);
        CreateRequest {
            node_id: node_id.into(),
            bolt11: invoice.to_string(),
            pay_in_asset: asset.into(),
            ts: NOW,
            signature: fake_sign(&msg),
        }
    }

    fn status_req(node_id: &str, hash: &str) -> StatusRequest {
        let msg =
            crate::messages::onboarding_swap_signed_bytes("status", node_id, hash, "", "", NOW);
        StatusRequest {
            node_id: node_id.into(),
            payment_hash: hash.into(),
            ts: NOW,
            signature: fake_sign(&msg),
        }
    }

    fn refund_req(node_id: &str, hash: &str, address: &str) -> RefundRequest {
        let msg = crate::messages::onboarding_swap_signed_bytes(
            "refund", node_id, hash, "", address, NOW,
        );
        RefundRequest {
            node_id: node_id.into(),
            payment_hash: hash.into(),
            refund_address: address.into(),
            ts: NOW,
            signature: fake_sign(&msg),
        }
    }

    async fn setup() -> (FakeProvider, OnboardingSwaps, tempfile::TempDir) {
        let fake = FakeProvider::start().await;
        let dir = tempfile::tempdir().unwrap();
        let swaps = OnboardingSwaps::new(
            SwapStore::open(dir.path()).unwrap(),
            vec![fake.client("fake")],
            Currency::Regtest,
        );
        (fake, swaps, dir)
    }

    fn assert_no_token(value: &Value) {
        let text = value.to_string();
        assert!(
            !text.contains("provider-token") && !text.contains("provider_token"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn options_lists_only_supported_stablecoin_rails() {
        assert_eq!(options(None).await, json!({ "enabled": false }));
        let (_fake, swaps, _dir) = setup().await;
        let body = options(Some(&swaps)).await;
        assert_eq!(body["enabled"], true);
        let rails = body["rails"].as_array().unwrap();
        // USDC_SOL is recv=false at the fake; only USDT on Tron and Ethereum are offered.
        let assets: Vec<&str> = rails.iter().map(|r| r["asset"].as_str().unwrap()).collect();
        assert_eq!(assets, ["USDT_TRON", "USDT_ETH"]);
        assert_eq!(rails[0]["deposit_risk"], "asset_only");
        assert_eq!(rails[1]["deposit_risk"], "chain_ambiguous");
        assert_eq!(rails[0]["min_msat"], 9_901_000);
    }

    #[tokio::test]
    async fn create_happy_path_returns_public_fields_only() {
        let (fake, swaps, _dir) = setup().await;
        let (sk, node_id) = user_key();
        let invoice = invoice_with(&sk, PREIMAGE, Some(AMOUNT_MSAT), 3600);
        let body = create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&node_id, &invoice, "USDT_TRON"),
            NOW,
        )
        .await
        .unwrap();

        assert_eq!(body["payment_hash"], hex::encode(invoice.payment_hash().0));
        assert_eq!(body["deposit_address"], TRON_ADDRESS);
        assert_eq!(body["deposit_amount"], "15.16");
        assert_eq!(body["deposit_risk"], "asset_only");
        assert_eq!(body["provider_state"], "awaiting_deposit");
        assert_eq!(body["fee"]["payout_usd"], "15.00");
        assert!(body["network_warning"]
            .as_str()
            .unwrap()
            .contains("15.16 USDT on the Tron network"));
        assert_no_token(&body);
        // The provider was asked to pay exactly the wallet's bolt11.
        assert_eq!(fake.orders()[0].bolt11, invoice.to_string());
    }

    #[tokio::test]
    async fn eth_rail_is_chain_ambiguous() {
        let (_fake, swaps, _dir) = setup().await;
        let (sk, node_id) = user_key();
        let invoice = invoice_with(&sk, PREIMAGE, Some(AMOUNT_MSAT), 3600);
        let body = create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&node_id, &invoice, "USDT_ETH"),
            NOW,
        )
        .await
        .unwrap();
        assert_eq!(body["deposit_address"], ETH_ADDRESS);
        assert_eq!(body["deposit_risk"], "chain_ambiguous");
        assert!(body["network_warning"].as_str().unwrap().contains("EVM"));
    }

    #[tokio::test]
    async fn create_rejects_bad_signature_payee_amount_and_expiry() {
        let (fake, swaps, _dir) = setup().await;
        let (sk, node_id) = user_key();
        let invoice = invoice_with(&sk, PREIMAGE, Some(AMOUNT_MSAT), 3600);

        let mut forged = create_req(&node_id, &invoice, "USDT_TRON");
        forged.signature = "sig:forged".into();
        assert_eq!(
            create(&swaps, &SigFake, &lsp(), forged, NOW)
                .await
                .unwrap_err()
                .0,
            StatusCode::UNAUTHORIZED
        );

        let stale = create_req(&node_id, &invoice, "USDT_TRON");
        assert_eq!(
            create(&swaps, &SigFake, &lsp(), stale, NOW + 301)
                .await
                .unwrap_err()
                .0,
            StatusCode::UNAUTHORIZED
        );

        // Signed by someone else's node: the invoice pays a different node.
        let other = PublicKey::from_secret_key(
            &Secp256k1::new(),
            &SecretKey::from_slice(&[0x22; 32]).unwrap(),
        );
        let err = create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&other.to_string(), &invoice, "USDT_TRON"),
            NOW,
        )
        .await
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("payee"));

        let short = invoice_with(&sk, [8; 32], Some(AMOUNT_MSAT), MIN_INVOICE_EXPIRY_SECS - 1);
        let err = create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&node_id, &short, "USDT_TRON"),
            NOW,
        )
        .await
        .unwrap_err();
        assert!(err.1.contains("valid for at least"));

        let no_amount = invoice_with(&sk, [9; 32], None, 3600);
        let err = create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&node_id, &no_amount, "USDT_TRON"),
            NOW,
        )
        .await
        .unwrap_err();
        assert!(err.1.contains("no amount"));

        let tiny = invoice_with(&sk, [10; 32], Some(1_000_000), 3600);
        let err = create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&node_id, &tiny, "USDT_TRON"),
            NOW,
        )
        .await
        .unwrap_err();
        assert_eq!(err.1, "amount_too_small");

        let err = create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&node_id, &invoice, "USDC_SOL"),
            NOW,
        )
        .await
        .unwrap_err();
        assert!(err.1.contains("unavailable"));

        assert_eq!(
            *fake.state.creates.lock().unwrap(),
            0,
            "no provider order for a rejected request"
        );
    }

    #[tokio::test]
    async fn create_is_idempotent_per_payment_hash() {
        let (fake, swaps, _dir) = setup().await;
        let (sk, node_id) = user_key();
        let invoice = invoice_with(&sk, PREIMAGE, Some(AMOUNT_MSAT), 3600);
        let first = create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&node_id, &invoice, "USDT_TRON"),
            NOW,
        )
        .await
        .unwrap();
        let second = create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&node_id, &invoice, "USDT_TRON"),
            NOW,
        )
        .await
        .unwrap();
        assert_eq!(first["provider_order_id"], second["provider_order_id"]);
        assert_eq!(*fake.state.creates.lock().unwrap(), 1);
        let err = create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&node_id, &invoice, "USDT_ETH"),
            NOW,
        )
        .await
        .unwrap_err();
        assert_eq!(err.0, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn status_follows_poller_and_refund_flow() {
        let (fake, swaps, _dir) = setup().await;
        let (sk, node_id) = user_key();
        let invoice = invoice_with(&sk, PREIMAGE, Some(AMOUNT_MSAT), 3600);
        let hash = hex::encode(invoice.payment_hash().0);
        stable_channels::audit::enable_test_capture();
        create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&node_id, &invoice, "USDT_TRON"),
            NOW,
        )
        .await
        .unwrap();

        // Refund refused while the provider still awaits the deposit (state refreshed first).
        let err = refund(
            &swaps,
            &SigFake,
            refund_req(&node_id, &hash, TRON_ADDRESS),
            NOW,
        )
        .await
        .unwrap_err();
        assert_eq!(err.0, StatusCode::CONFLICT);

        fake.force_state("confirming");
        assert_eq!(crate::swap_poller::poll_once(&swaps, NOW).await, 1);
        let body = status(&swaps, &SigFake, status_req(&node_id, &hash), NOW)
            .await
            .unwrap();
        assert_eq!(body["provider_state"], "confirming");

        fake.force_state("refund_required");
        crate::swap_poller::poll_once(&swaps, NOW).await;
        let body = status(&swaps, &SigFake, status_req(&node_id, &hash), NOW)
            .await
            .unwrap();
        assert_eq!(body["provider_state"], "refund_required");
        assert_eq!(body["state_reason"], "underpaid");

        // A refund signed for a different address than the one submitted is rejected.
        let mut tampered = refund_req(&node_id, &hash, TRON_ADDRESS);
        tampered.refund_address = "TAttackerAddress".into();
        assert_eq!(
            refund(&swaps, &SigFake, tampered, NOW).await.unwrap_err().0,
            StatusCode::UNAUTHORIZED
        );

        let body = refund(
            &swaps,
            &SigFake,
            refund_req(&node_id, &hash, TRON_ADDRESS),
            NOW,
        )
        .await
        .unwrap();
        assert_eq!(body["provider_state"], "refund_pending");
        assert_eq!(body["refund_address"], TRON_ADDRESS);
        assert_eq!(
            fake.orders()[0].refund_address.as_deref(),
            Some(TRON_ADDRESS)
        );
        assert_no_token(&body);

        let events = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let names: Vec<&str> = events.iter().map(|(e, _)| e.as_str()).collect();
        for expected in [
            "ONBOARDING_SWAP_CREATED",
            "ONBOARDING_SWAP_STATE",
            "ONBOARDING_SWAP_SIGNATURE_INVALID",
            "ONBOARDING_SWAP_REFUND_REQUESTED",
        ] {
            assert!(names.contains(&expected), "missing {expected} in {names:?}");
        }
        for (_, data) in &events {
            assert_no_token(data);
        }

        // Another node cannot read this attempt.
        let other = PublicKey::from_secret_key(
            &Secp256k1::new(),
            &SecretKey::from_slice(&[0x22; 32]).unwrap(),
        );
        let err = status(&swaps, &SigFake, status_req(&other.to_string(), &hash), NOW)
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn settled_requires_matching_preimage_and_stops_polling() {
        let (fake, swaps, _dir) = setup().await;
        let (sk, node_id) = user_key();
        let invoice = invoice_with(&sk, PREIMAGE, Some(AMOUNT_MSAT), 3600);
        let hash = hex::encode(invoice.payment_hash().0);
        create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&node_id, &invoice, "USDT_TRON"),
            NOW,
        )
        .await
        .unwrap();

        let wrong = SettledRequest {
            payment_hash: hash.clone(),
            preimage: hex::encode([8u8; 32]),
        };
        assert_eq!(
            settled(&swaps, wrong, NOW).await.unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
        let garbage = SettledRequest {
            payment_hash: hash.clone(),
            preimage: "zz".into(),
        };
        assert_eq!(
            settled(&swaps, garbage, NOW).await.unwrap_err().0,
            StatusCode::BAD_REQUEST
        );

        // Provider "completed" alone settles nothing.
        fake.force_state("completed");
        crate::swap_poller::poll_once(&swaps, NOW).await;
        assert!(swaps
            .store
            .get(&hash)
            .unwrap()
            .unwrap()
            .settled_at
            .is_none());

        let ok = SettledRequest {
            payment_hash: hash.clone(),
            preimage: hex::encode(PREIMAGE),
        };
        assert_eq!(settled(&swaps, ok, NOW).await.unwrap()["ok"], true);
        assert_eq!(
            crate::swap_poller::poll_once(&swaps, NOW).await,
            0,
            "settled rows are not polled"
        );
        let body = status(&swaps, &SigFake, status_req(&node_id, &hash), NOW)
            .await
            .unwrap();
        assert_eq!(body["settled"], true);

        // Refund is impossible once settled, whatever the provider says.
        fake.force_state("refund_required");
        let err = refund(
            &swaps,
            &SigFake,
            refund_req(&node_id, &hash, TRON_ADDRESS),
            NOW,
        )
        .await
        .unwrap_err();
        assert_eq!(err.0, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn provider_signature_mismatch_is_reported_unavailable() {
        let fake = FakeProvider::start().await;
        let dir = tempfile::tempdir().unwrap();
        let wrong_secret = crate::swap::provider::FixedFloatClient::new(
            "fake",
            &fake.base_url,
            "fake-key",
            "wrong",
        );
        let swaps = OnboardingSwaps::new(
            SwapStore::open(dir.path()).unwrap(),
            vec![wrong_secret],
            Currency::Regtest,
        );
        assert_eq!(options(Some(&swaps)).await["enabled"], false);
    }

    #[tokio::test]
    async fn create_rejects_wrong_network_and_non_jit_invoices() {
        let (fake, swaps, _dir) = setup().await;
        let (sk, node_id) = user_key();

        let mainnet = invoice_custom(
            &sk,
            [20; 32],
            Some(AMOUNT_MSAT),
            3600,
            Currency::Bitcoin,
            Some(lsp_key()),
        );
        let err = create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&node_id, &mainnet, "USDT_TRON"),
            NOW,
        )
        .await
        .unwrap_err();
        assert!(err.1.contains("different network"), "{}", err.1);

        let no_hint = invoice_custom(
            &sk,
            [21; 32],
            Some(AMOUNT_MSAT),
            3600,
            Currency::Regtest,
            None,
        );
        let err = create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&node_id, &no_hint, "USDT_TRON"),
            NOW,
        )
        .await
        .unwrap_err();
        assert!(err.1.contains("JIT invoice through this LSP"), "{}", err.1);

        // A route hint through some other LSP is not one of ours either.
        let stranger = PublicKey::from_secret_key(
            &Secp256k1::new(),
            &SecretKey::from_slice(&[0x44; 32]).unwrap(),
        );
        let other_lsp = invoice_custom(
            &sk,
            [22; 32],
            Some(AMOUNT_MSAT),
            3600,
            Currency::Regtest,
            Some(stranger),
        );
        let err = create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&node_id, &other_lsp, "USDT_TRON"),
            NOW,
        )
        .await
        .unwrap_err();
        assert!(err.1.contains("JIT invoice through this LSP"));

        assert_eq!(*fake.state.creates.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn create_is_rate_limited_per_node_and_globally() {
        let (fake, mut swaps, _dir) = setup().await;
        swaps.rate_limit = std::sync::Mutex::new(crate::swap::CreateRateLimit::new(3600, 2, 3));
        let (sk, node_id) = user_key();
        let other_sk = SecretKey::from_slice(&[0x55; 32]).unwrap();
        let other_id = PublicKey::from_secret_key(&Secp256k1::new(), &other_sk).to_string();

        for i in 0..2u8 {
            let inv = invoice_with(&sk, [30 + i; 32], Some(AMOUNT_MSAT), 3600);
            create(
                &swaps,
                &SigFake,
                &lsp(),
                create_req(&node_id, &inv, "USDT_TRON"),
                NOW,
            )
            .await
            .unwrap();
        }
        // Idempotent retry of an existing attempt is not a new order and is never limited.
        let first = invoice_with(&sk, [30; 32], Some(AMOUNT_MSAT), 3600);
        create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&node_id, &first, "USDT_TRON"),
            NOW,
        )
        .await
        .unwrap();

        let third = invoice_with(&sk, [32; 32], Some(AMOUNT_MSAT), 3600);
        let err = create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&node_id, &third, "USDT_TRON"),
            NOW,
        )
        .await
        .unwrap_err();
        assert_eq!(err.0, StatusCode::TOO_MANY_REQUESTS);
        assert!(err.1.contains("this wallet"));

        // Another wallet gets the last global slot, then the global cap applies to everyone.
        let a = invoice_with(&other_sk, [40; 32], Some(AMOUNT_MSAT), 3600);
        create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&other_id, &a, "USDT_TRON"),
            NOW,
        )
        .await
        .unwrap();
        let b = invoice_with(&other_sk, [41; 32], Some(AMOUNT_MSAT), 3600);
        let err = create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&other_id, &b, "USDT_TRON"),
            NOW,
        )
        .await
        .unwrap_err();
        assert_eq!(err.0, StatusCode::TOO_MANY_REQUESTS);
        assert!(err.1.contains("right now"));

        assert_eq!(*fake.state.creates.lock().unwrap(), 3);
    }

    #[tokio::test]
    async fn refund_address_must_match_the_attempt_rail() {
        let (fake, swaps, _dir) = setup().await;
        let (sk, node_id) = user_key();
        let invoice = invoice_with(&sk, PREIMAGE, Some(AMOUNT_MSAT), 3600);
        let hash = hex::encode(invoice.payment_hash().0);
        create(
            &swaps,
            &SigFake,
            &lsp(),
            create_req(&node_id, &invoice, "USDT_TRON"),
            NOW,
        )
        .await
        .unwrap();
        fake.force_state("refund_required");

        // A valid Ethereum address is the wrong chain for a Tron deposit.
        let err = refund(
            &swaps,
            &SigFake,
            refund_req(&node_id, &hash, ETH_ADDRESS),
            NOW,
        )
        .await
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("starting with T"), "{}", err.1);
        let typo = TRON_ADDRESS.replace("D14", "D15");
        let err = refund(&swaps, &SigFake, refund_req(&node_id, &hash, &typo), NOW)
            .await
            .unwrap_err();
        assert!(err.1.contains("checksum"), "{}", err.1);
        assert!(
            fake.orders()[0].refund_address.is_none(),
            "no emergency call for a bad address"
        );
    }
}
