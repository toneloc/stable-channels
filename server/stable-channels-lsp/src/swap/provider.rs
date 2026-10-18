//! FixedFloat-compatible swap provider client, ported from OpenReceive's `fixedfloat*.ts`:
//! HMAC-signed `POST /api/v2/<path>` calls (`ccies`, `price`, `create`, `order`, `emergency`),
//! the public `GET /rates/fixed.xml` for per-pair limits (`/ccies` carries none), and the
//! provider status → `SwapState` table from `spec/data/swap-state-table.json`.
//! Amounts stay decimal strings or integer msat; never f64.

use std::time::Duration;

use bitcoin_hashes::hmac::{Hmac, HmacEngine};
use bitcoin_hashes::{sha256, Hash, HashEngine};
use serde_json::{json, Value};

use super::{is_lightning_network, network_matches, PayInAsset, SwapState};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const SATS_PER_BTC: u128 = 100_000_000;
/// Decimal scales above this are not amounts a provider sends; refusing them keeps math in u128.
const MAX_DECIMAL_SCALE: u32 = 18;

#[derive(Debug)]
pub enum ProviderError {
    /// Transport failure, non-2xx, or unparsable body: the provider may be down.
    Unavailable(String),
    /// The provider answered with an API error envelope (`code != 0`).
    Api(String),
    /// The provider answered but broke the response contract.
    Invalid(String),
    /// HTTP 429. Providers meter calls by weight (a create costs far more than a status poll),
    /// so a 429 must not be retried in a loop.
    RateLimited(String),
    /// HTTP 409. On `emergency` it means a refund address is already set for the order.
    Conflict(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(m) => write!(f, "provider unavailable: {m}"),
            Self::Api(m) => write!(f, "provider error: {m}"),
            Self::Invalid(m) => write!(f, "provider response invalid: {m}"),
            Self::RateLimited(m) => write!(f, "provider rate limited: {m}"),
            Self::Conflict(m) => write!(f, "provider conflict: {m}"),
        }
    }
}

/// One raw provider HTTP answer, kept apart from parsing so a dev tool can show the body when the
/// real API does not match what we expect. Bodies can carry an order token: never log them as-is.
pub struct RawResponse {
    pub status: u16,
    pub body: String,
}

/// One stablecoin → Lightning rail the provider currently offers, with invoice-side limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rail {
    pub asset: PayInAsset,
    pub code: String,
    pub min_msat: u64,
    pub max_msat: u64,
}

#[derive(Debug, Clone)]
pub struct Catalog {
    pub lightning_code: String,
    pub rails: Vec<Rail>,
}

impl Catalog {
    pub fn rail(&self, asset: PayInAsset) -> Option<&Rail> {
        self.rails.iter().find(|r| r.asset == asset)
    }
}

/// Provider state plus the reasons OpenReceive derives alongside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderStatus {
    pub state: SwapState,
    pub refund_reason: Option<&'static str>,
    pub attention_reason: Option<&'static str>,
}

/// USD valuations of both sides; their gap is the swap fee. Never an amount to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fee {
    pub pay_in_usd: String,
    pub payout_usd: String,
}

/// A created order. `token` is the provider credential for this order (OpenReceive `swap_data`):
/// it must never leave the LSP, so this type is deliberately not `Serialize`/`Debug`.
pub struct ProviderOrder {
    pub id: String,
    pub token: String,
    pub deposit_address: String,
    pub deposit_amount: String,
    pub deposit_memo: Option<String>,
    pub expires_at: u64,
    pub status: OrderStatus,
    pub fee: Option<Fee>,
}

pub struct FixedFloatClient {
    pub id: String,
    base_url: String,
    key: String,
    secret: String,
    http: reqwest::Client,
}

impl FixedFloatClient {
    pub fn new(id: &str, base_url: &str, key: &str, secret: &str) -> Self {
        Self {
            id: id.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            key: key.to_string(),
            secret: secret.to_string(),
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_default(),
        }
    }

    /// Signed `POST {base}/api/v2/{path}`, transport only. The key and signature travel in headers
    /// and are never logged; bodies may carry the order token, so they are not logged either.
    pub async fn post_raw(&self, path: &str, body: &Value) -> Result<RawResponse, ProviderError> {
        let body = body.to_string();
        let response = self
            .http
            .post(format!("{}/api/v2/{}", self.base_url, path))
            .header("Content-Type", "application/json; charset=UTF-8")
            .header("X-API-KEY", &self.key)
            .header("X-API-SIGN", sign(&self.secret, &body))
            .body(body)
            .send()
            .await
            .map_err(|e| ProviderError::Unavailable(format!("{path}: {}", e.without_url())))?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|e| ProviderError::Unavailable(format!("{path}: {}", e.without_url())))?;
        Ok(RawResponse { status, body })
    }

    /// The public, unsigned rates export (`GET {base}/rates/fixed.xml`), transport only.
    pub async fn rates_raw(&self) -> Result<RawResponse, ProviderError> {
        let response = self
            .http
            .get(format!("{}/rates/fixed.xml", self.base_url))
            .send()
            .await
            .map_err(|e| ProviderError::Unavailable(format!("rates: {}", e.without_url())))?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|e| ProviderError::Unavailable(format!("rates: {}", e.without_url())))?;
        Ok(RawResponse { status, body })
    }

    /// Signed POST; returns the envelope's `data`.
    async fn post(&self, path: &str, body: Value) -> Result<Value, ProviderError> {
        envelope_data(path, &self.post_raw(path, &body).await?)
    }

    /// `/ccies` resolution joined with the public rates XML: the stablecoin rails this provider
    /// accepts deposits for, each with invoice-side min/max derived from the pair rate.
    pub async fn catalog(&self) -> Result<Catalog, ProviderError> {
        let ccies = self.post("ccies", json!({})).await?;
        let rates = self.rates_raw().await?;
        if !(200..300).contains(&rates.status) {
            return Err(ProviderError::Unavailable(format!(
                "rates HTTP {}",
                rates.status
            )));
        }
        build_catalog(&ccies, &rates.body)
    }

    pub async fn create(
        &self,
        rail: &Rail,
        lightning_code: &str,
        bolt11: &str,
        amount_msat: u64,
    ) -> Result<ProviderOrder, ProviderError> {
        let request = create_request(&rail.code, lightning_code, bolt11, amount_msat);
        let mut order = parse_order(&self.post("create", request).await?)?;
        if order.fee.is_none() {
            order.fee = self.price(rail, lightning_code, amount_msat).await;
        }
        Ok(order)
    }

    /// Best-effort fee lookup for the same trade; a failure just leaves the fee off.
    pub async fn price(&self, rail: &Rail, lightning_code: &str, amount_msat: u64) -> Option<Fee> {
        let request = price_request(&rail.code, lightning_code, amount_msat);
        read_fee(&self.post("price", request).await.ok()?)
    }

    /// Current provider status, or `None` for a thin body with no `status` (keep the persisted one).
    pub async fn order(&self, id: &str, token: &str) -> Result<Option<OrderStatus>, ProviderError> {
        let data = self
            .post("order", json!({ "id": id, "token": token }))
            .await?;
        Ok(coerced_string(&data["status"]).map(|_| map_status(&data)))
    }

    pub async fn emergency_refund(
        &self,
        id: &str,
        token: &str,
        address: &str,
    ) -> Result<(), ProviderError> {
        self.post(
            "emergency",
            json!({ "id": id, "token": token, "choice": "REFUND", "address": address }),
        )
        .await
        .map(|_| ())
    }
}

/// Unwrap a `{code, msg, data}` envelope: non-2xx is `Unavailable` (429 is `RateLimited`, 409 is
/// `Conflict`), a non-zero `code` is `Api`, otherwise `data`.
pub fn envelope_data(path: &str, raw: &RawResponse) -> Result<Value, ProviderError> {
    let envelope: Option<Value> = if raw.body.is_empty() {
        Some(json!({}))
    } else {
        serde_json::from_str(&raw.body).ok()
    };
    let msg = envelope
        .as_ref()
        .and_then(|e| coerced_string(&e["msg"]))
        .unwrap_or_default();
    if raw.status == 429 {
        return Err(ProviderError::RateLimited(format!("{path} HTTP 429 {msg}")));
    }
    if raw.status == 409 {
        return Err(ProviderError::Conflict(format!("{path} HTTP 409 {msg}")));
    }
    if !(200..300).contains(&raw.status) {
        return Err(ProviderError::Unavailable(format!(
            "{path} HTTP {} {msg}",
            raw.status
        )));
    }
    let envelope = envelope
        .ok_or_else(|| ProviderError::Unavailable(format!("{path} returned invalid JSON")))?;
    if envelope["code"].as_i64() != Some(0) {
        return Err(ProviderError::Api(if msg.is_empty() {
            format!("{path} failed")
        } else {
            msg
        }));
    }
    Ok(envelope["data"].clone())
}

/// `/create` body: fixed rate, `direction: to` = exactly this much BTC must arrive at the bolt11.
pub fn create_request(
    from_code: &str,
    lightning_code: &str,
    bolt11: &str,
    amount_msat: u64,
) -> Value {
    json!({
        "type": "fixed",
        "fromCcy": from_code,
        "toCcy": lightning_code,
        "direction": "to",
        "amount": msat_to_btc_string(amount_msat),
        "toAddress": bolt11,
    })
}

pub fn price_request(from_code: &str, lightning_code: &str, amount_msat: u64) -> Value {
    json!({
        "type": "fixed",
        "fromCcy": from_code,
        "toCcy": lightning_code,
        "direction": "to",
        "amount": msat_to_btc_string(amount_msat),
    })
}

/// Lowercase hex HMAC-SHA256(secret, body): the `X-API-SIGN` header.
pub fn sign(secret: &str, body: &str) -> String {
    let mut engine: HmacEngine<sha256::Hash> = HmacEngine::new(secret.as_bytes());
    engine.input(body.as_bytes());
    Hmac::<sha256::Hash>::from_engine(engine).to_string()
}

/// A string field that may arrive as a JSON number.
fn coerced_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn required(value: &Value, label: &str) -> Result<String, ProviderError> {
    coerced_string(value).ok_or_else(|| ProviderError::Invalid(format!("missing {label}")))
}

pub fn read_fee(record: &Value) -> Option<Fee> {
    Some(Fee {
        pay_in_usd: coerced_string(&record["from"]["usd"])?,
        payout_usd: coerced_string(&record["to"]["usd"])?,
    })
}

/// Parse a `/create` body. Every deposit field is required: no invented deadline or amount.
pub fn parse_order(data: &Value) -> Result<ProviderOrder, ProviderError> {
    let deposit_amount = required(&data["from"]["amount"], "from.amount")?;
    if parse_decimal(&deposit_amount).is_none() {
        return Err(ProviderError::Invalid(
            "from.amount is not a decimal amount".into(),
        ));
    }
    let expires_at = match &data["time"]["expiration"] {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
    .ok_or_else(|| ProviderError::Invalid("missing time.expiration".into()))?;
    Ok(ProviderOrder {
        id: required(&data["id"], "id")?,
        token: required(&data["token"], "token")?,
        deposit_address: required(&data["from"]["address"], "from.address")?,
        deposit_amount,
        deposit_memo: coerced_string(&data["from"]["tag"]),
        expires_at,
        status: map_status(data),
        fee: read_fee(data),
    })
}

/// Provider status + emergency block + refund-tx presence → state, first match wins
/// (swap-state-table.json `status_rows`, in order).
pub fn map_status(record: &Value) -> OrderStatus {
    let status = coerced_string(&record["status"])
        .unwrap_or_else(|| "NEW".into())
        .to_uppercase();
    let refund_tx = coerced_string(&record["back"]["tx"]["id"])
        .or_else(|| coerced_string(&record["refund"]["tx"]["id"]))
        .is_some();
    let choice = coerced_string(&record["emergency"]["choice"]).map(|c| c.to_uppercase());
    let reason = || refund_reason(&record["emergency"]["status"]);
    let plain = |state| OrderStatus {
        state,
        refund_reason: None,
        attention_reason: None,
    };
    match status.as_str() {
        "DONE" | "FINISHED" if refund_tx => plain(SwapState::Refunded),
        "NEW" => plain(SwapState::AwaitingDeposit),
        "PENDING" => plain(SwapState::Confirming),
        "EXCHANGE" => plain(SwapState::Exchanging),
        "WITHDRAW" => plain(SwapState::PayingInvoice),
        "DONE" => plain(SwapState::Completed),
        "EXPIRED" => plain(SwapState::Expired),
        "EMERGENCY" => match choice.as_deref() {
            Some("REFUND") => OrderStatus {
                state: if refund_tx {
                    SwapState::Refunded
                } else {
                    SwapState::RefundPending
                },
                refund_reason: reason(),
                attention_reason: None,
            },
            Some("EXCHANGE") => OrderStatus {
                state: SwapState::Attention,
                refund_reason: None,
                attention_reason: Some("provider_reported_emergency"),
            },
            _ => OrderStatus {
                state: SwapState::RefundRequired,
                refund_reason: reason(),
                attention_reason: None,
            },
        },
        s if s.contains("FAIL") => plain(SwapState::Failed),
        // Unrecognized is not an emergency: never guess a state for it.
        _ => OrderStatus {
            state: SwapState::Attention,
            refund_reason: None,
            attention_reason: Some("provider_status_unrecognized"),
        },
    }
}

/// Emergency status list → refund reason (`refund_reason_rows`, OVER/OVERPAID alias MORE).
fn refund_reason(statuses: &Value) -> Option<&'static str> {
    let list: Vec<String> = match statuses {
        Value::Array(items) => items.iter().filter_map(coerced_string).collect(),
        other => coerced_string(other).into_iter().collect(),
    };
    let has = |wanted: &str| {
        list.iter().any(|s| {
            let s = s.to_uppercase();
            s == wanted || (wanted == "MORE" && (s == "OVER" || s == "OVERPAID"))
        })
    };
    match (has("LESS"), has("MORE"), has("EXPIRED")) {
        (true, _, true) => Some("underpaid_and_late"),
        (_, true, true) => Some("overpaid_and_late"),
        (true, _, _) => Some("underpaid"),
        (_, true, _) => Some("overpaid"),
        (_, _, true) => Some("late_deposit"),
        _ => None,
    }
}

/// Resolve `/ccies` + rates XML into the stablecoin rails (OpenReceive `resolveFixedFloatCurrencies`
/// + `invoiceLimitsFromFixedFloatRate`). A rail with no rate pair is temporarily unavailable.
pub fn build_catalog(ccies: &Value, rates_xml: &str) -> Result<Catalog, ProviderError> {
    let (lightning_code, codes) = resolve_currencies(ccies)?;
    let pairs = parse_rates_xml(rates_xml);
    let rails = codes
        .into_iter()
        .filter_map(|(asset, code)| {
            let pair = pairs.iter().find(|p| {
                p.from.eq_ignore_ascii_case(&code) && p.to.eq_ignore_ascii_case(&lightning_code)
            })?;
            Some(Rail {
                asset,
                min_msat: pay_to_invoice_msat(&pair.min, pair, true)?,
                max_msat: pay_to_invoice_msat(&pair.max, pair, false)?,
                code,
            })
        })
        .collect();
    Ok(Catalog {
        lightning_code,
        rails,
    })
}

/// `/ccies` alone: the Lightning payout code and the provider code of each stablecoin rail that
/// accepts deposits (`recv=false` is omitted rather than failing at create). No limits here.
pub fn resolve_currencies(
    ccies: &Value,
) -> Result<(String, Vec<(PayInAsset, String)>), ProviderError> {
    let items = ccies
        .as_array()
        .or_else(|| ccies["ccies"].as_array())
        .or_else(|| ccies["currencies"].as_array())
        .cloned()
        .unwrap_or_default();
    let field = |item: &Value, names: &[&str]| names.iter().find_map(|n| coerced_string(&item[*n]));
    let currencies: Vec<(String, String, String, &Value)> = items
        .iter()
        .filter_map(|item| {
            Some((
                field(item, &["code", "ticker"])?,
                field(item, &["coin", "currency", "symbol"])?.to_uppercase(),
                field(item, &["network", "chain", "networkName", "name"])?,
                item,
            ))
        })
        .collect();

    let lightning_code = currencies
        .iter()
        .find(|(_, coin, network, item)| {
            coin == "BTC" && is_lightning_network(network) && item["send"].as_bool() != Some(false)
        })
        .map(|(code, ..)| code.clone())
        .ok_or_else(|| {
            ProviderError::Invalid("/ccies has no BTC Lightning payout currency".into())
        })?;

    let codes = PayInAsset::ALL
        .into_iter()
        .filter_map(|asset| {
            let (code, ..) = currencies.iter().find(|(_, coin, network, item)| {
                coin == asset.coin()
                    && network_matches(asset.network(), network)
                    && item["recv"].as_bool() != Some(false)
            })?;
            Some((asset, code.clone()))
        })
        .collect();
    Ok((lightning_code, codes))
}

#[derive(Debug)]
struct RatePair {
    from: String,
    to: String,
    rate_in: String,
    rate_out: String,
    min: String,
    max: String,
}

/// Reader for the flat `<rates><item><from>..</from>..</item></rates>` export, matching
/// OpenReceive's `parseFixedFloatRatesXml`: tags may carry attributes and any case, text is
/// trimmed, and amounts keep only their leading number ("10 USDT" and "10USDT" → "10").
fn parse_rates_xml(xml: &str) -> Vec<RatePair> {
    let text = |item: &str, name: &str| -> Option<String> {
        let value = tag_contents(item, name).into_iter().next()?.trim();
        (!value.is_empty()).then(|| value.to_string())
    };
    let number = |item: &str, name: &str| -> Option<String> {
        let value = text(item, name)?;
        let digits: String = value
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        Some(if digits.is_empty() { value } else { digits })
    };
    tag_contents(xml, "item")
        .into_iter()
        .filter_map(|item| {
            let to = text(item, "to")?;
            if !is_lightning_network(&to) {
                return None;
            }
            Some(RatePair {
                from: text(item, "from")?,
                to,
                rate_in: number(item, "in")?,
                rate_out: number(item, "out")?,
                min: number(item, "minamount")?,
                max: number(item, "maxamount")?,
            })
        })
        .collect()
}

/// Contents of every `<name …>…</name>` in `xml`, case-insensitive and attribute-tolerant
/// (OpenReceive `matchTags`). `<to>` never matches `<tofee>`: the name must end at `>` or a space.
fn tag_contents<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    // ASCII lowercasing keeps byte offsets, so indexes into `lower` are valid in `xml`.
    let lower = xml.to_ascii_lowercase();
    let open = format!("<{}", name.to_ascii_lowercase());
    let close = format!("</{}>", name.to_ascii_lowercase());
    let mut found = Vec::new();
    let mut pos = 0;
    while let Some(i) = lower[pos..].find(&open) {
        let after_name = pos + i + open.len();
        match lower[after_name..].chars().next() {
            Some(c) if c == '>' || c.is_whitespace() => {}
            _ => {
                pos = after_name;
                continue;
            }
        }
        let Some(gt) = lower[after_name..].find('>') else {
            break;
        };
        let start = after_name + gt + 1;
        let Some(len) = lower[start..].find(&close) else {
            break;
        };
        found.push(&xml[start..start + len]);
        pos = start + len + close.len();
    }
    found
}

/// Positive decimal string → (units, scale).
fn parse_decimal(value: &str) -> Option<(u128, u32)> {
    let (whole, frac) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole
            .bytes()
            .chain(frac.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    if value.ends_with('.') || frac.len() as u32 > MAX_DECIMAL_SCALE {
        return None;
    }
    let units: u128 = format!("{whole}{frac}").parse().ok()?;
    (units > 0).then_some((units, frac.len() as u32))
}

/// invoice_sats = pay * out * 1e8 / in; min rounds up and max rounds down, so a borderline
/// invoice is never reported inside a range the provider would reject.
fn pay_to_invoice_msat(pay: &str, pair: &RatePair, ceil: bool) -> Option<u64> {
    let (pay, pay_scale) = parse_decimal(pay)?;
    let (rate_in, in_scale) = parse_decimal(&pair.rate_in)?;
    let (rate_out, out_scale) = parse_decimal(&pair.rate_out)?;
    let numerator = pay
        .checked_mul(rate_out)?
        .checked_mul(SATS_PER_BTC)?
        .checked_mul(10u128.checked_pow(in_scale)?)?;
    let denominator = 10u128
        .checked_pow(pay_scale + out_scale)?
        .checked_mul(rate_in)?;
    let sats = if ceil {
        numerator.div_ceil(denominator)
    } else {
        numerator / denominator
    };
    u64::try_from(sats.checked_mul(1000)?)
        .ok()
        .filter(|m| *m > 0)
}

/// msat → whole sats (rounded up) as a BTC decimal string, e.g. 150_000_500 → "0.00150001".
pub fn msat_to_btc_string(amount_msat: u64) -> String {
    let sats = amount_msat.div_ceil(1000) as u128;
    let fraction = format!("{:08}", sats % SATS_PER_BTC);
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        (sats / SATS_PER_BTC).to_string()
    } else {
        format!("{}.{}", sats / SATS_PER_BTC, fraction)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(body: Value) -> (SwapState, Option<&'static str>, Option<&'static str>) {
        let s = map_status(&body);
        (s.state, s.refund_reason, s.attention_reason)
    }

    #[test]
    fn status_table_matches_openreceive_rows() {
        use SwapState::*;
        assert_eq!(status(json!({"status": "NEW"})).0, AwaitingDeposit);
        assert_eq!(status(json!({"status": "pending"})).0, Confirming);
        assert_eq!(status(json!({"status": "EXCHANGE"})).0, Exchanging);
        assert_eq!(status(json!({"status": "WITHDRAW"})).0, PayingInvoice);
        assert_eq!(status(json!({"status": "DONE"})).0, Completed);
        assert_eq!(
            status(json!({"status": "DONE", "back": {"tx": {"id": "r"}}})).0,
            Refunded
        );
        assert_eq!(status(json!({"status": "EXPIRED"})).0, Expired);
        assert_eq!(status(json!({"status": "PAYOUT_FAILED"})).0, Failed);
        assert_eq!(
            status(json!({"status": "SOMETHING_NEW"})),
            (Attention, None, Some("provider_status_unrecognized"))
        );
        // FINISHED without a refund tx is not a known status.
        assert_eq!(status(json!({"status": "FINISHED"})).0, Attention);
    }

    #[test]
    fn emergency_rows_and_refund_reasons() {
        use SwapState::*;
        let em = |choice: &str, statuses: Value, refund_tx: bool| {
            let mut body =
                json!({"status": "EMERGENCY", "emergency": {"choice": choice, "status": statuses}});
            if refund_tx {
                body["back"] = json!({"tx": {"id": "r"}});
            }
            status(body)
        };
        assert_eq!(
            em("NONE", json!(["LESS"]), false),
            (RefundRequired, Some("underpaid"), None)
        );
        assert_eq!(em("NONE", json!(["OVERPAID"]), false).1, Some("overpaid"));
        assert_eq!(em("NONE", json!("EXPIRED"), false).1, Some("late_deposit"));
        assert_eq!(
            em("NONE", json!(["LESS", "EXPIRED"]), false).1,
            Some("underpaid_and_late")
        );
        assert_eq!(
            em("NONE", json!(["MORE", "EXPIRED", "LIMIT"]), false).1,
            Some("overpaid_and_late")
        );
        assert_eq!(em("REFUND", json!(["LESS"]), false).0, RefundPending);
        assert_eq!(em("REFUND", json!(["LESS"]), true).0, Refunded);
        assert_eq!(
            em("EXCHANGE", json!(["LESS"]), true),
            (Attention, None, Some("provider_reported_emergency"))
        );
    }

    #[test]
    fn envelope_maps_http_and_api_errors() {
        let raw = |status, body: &str| RawResponse {
            status,
            body: body.to_string(),
        };
        let ok = envelope_data(
            "order",
            &raw(200, r#"{"code":0,"msg":"OK","data":{"id":"A"}}"#),
        );
        assert_eq!(ok.unwrap()["id"], "A");
        assert!(matches!(
            envelope_data("create", &raw(429, "Too Many Requests")),
            Err(ProviderError::RateLimited(_))
        ));
        assert!(matches!(
            envelope_data("emergency", &raw(409, r#"{"code":409,"msg":"address already set"}"#)),
            Err(ProviderError::Conflict(m)) if m.contains("address already set")
        ));
        assert!(matches!(
            envelope_data("create", &raw(200, r#"{"code":1,"msg":"bad amount"}"#)),
            Err(ProviderError::Api(m)) if m == "bad amount"
        ));
        assert!(matches!(
            envelope_data("ccies", &raw(200, "<html>")),
            Err(ProviderError::Unavailable(_))
        ));
    }

    #[test]
    fn hmac_signature_is_lower_hex_sha256() {
        // RFC 4231 test case 2.
        assert_eq!(
            sign("Jefe", "what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn btc_amount_string_rounds_up_to_whole_sats() {
        assert_eq!(msat_to_btc_string(150_000_000), "0.0015");
        assert_eq!(msat_to_btc_string(150_000_001), "0.00150001");
        assert_eq!(msat_to_btc_string(100_000_000_000), "1");
    }

    const RATES: &str = "<rates>\n<item><from>USDTTRC</from><to>BTCLN</to><in>1.01</in>\
        <out>0.00001</out><amount>10 BTC</amount><minamount>10 USDT</minamount>\
        <maxamount>10000 USDT</maxamount><tofee>0.0000 BTC</tofee></item>\n\
        <item><from>USDTETH</from><to>BTC</to><in>1</in><out>0.00001</out><amount>1</amount>\
        <minamount>10</minamount><maxamount>100</maxamount></item>\n</rates>";

    fn ccies() -> Value {
        json!([
            {"code": "USDTTRC", "coin": "USDT", "network": "TRC20", "recv": true, "send": true},
            {"code": "USDCSOL", "coin": "USDC", "network": "SOL", "recv": false, "send": true},
            {"code": "USDTETH", "coin": "USDT", "network": "ERC20", "recv": true, "send": true},
            {"code": "BTCLN", "coin": "BTC", "network": "Lightning", "recv": true, "send": true}
        ])
    }

    #[test]
    fn catalog_keeps_only_receivable_rails_with_lightning_pairs() {
        let catalog = build_catalog(&ccies(), RATES).unwrap();
        assert_eq!(catalog.lightning_code, "BTCLN");
        // USDC_SOL is recv=false; USDT_ETH has no Lightning pair in the rates export.
        assert_eq!(catalog.rails.len(), 1);
        let rail = catalog.rail(PayInAsset::UsdtTron).unwrap();
        assert_eq!(rail.code, "USDTTRC");
        // 10 USDT * 0.00001 / 1.01 BTC = 9900.99 sats → ceil 9901; max 10000 USDT → floor 9900990.
        assert_eq!(rail.min_msat, 9_901_000);
        assert_eq!(rail.max_msat, 9_900_990_000);
    }

    #[test]
    fn rates_xml_accepts_attributes_case_and_glued_units() {
        // Same pair as RATES, written the way other FixedFloat-compatible exports may be.
        let variant = "<?xml version=\"1.0\"?><Rates>\n<ITEM id=\"1\">\n  <From> USDTTRC </From>\
            <TO>BTCLN</TO><tofee>0.0001 BTC</tofee><In>1.01</In><Out>0.00001 BTC</Out>\
            <MinAmount>10USDT</MinAmount><MaxAmount type=\"x\">10000 USDT</MaxAmount>\n</ITEM></Rates>";
        let catalog = build_catalog(&ccies(), variant).unwrap();
        let rail = catalog.rail(PayInAsset::UsdtTron).unwrap();
        // Identical limits to the plain FixedFloat shape in RATES.
        assert_eq!(rail.min_msat, 9_901_000);
        assert_eq!(rail.max_msat, 9_900_990_000);
        // <tofee> appearing before <to> must not be read as the payout code.
        assert_eq!(
            tag_contents("<tofee>1</tofee><to>BTCLN</to>", "to"),
            ["BTCLN"]
        );
    }

    #[test]
    fn catalog_without_lightning_payout_is_invalid() {
        let ccies = json!([{"code": "USDTTRC", "coin": "USDT", "network": "TRC20"}]);
        assert!(matches!(
            build_catalog(&ccies, RATES),
            Err(ProviderError::Invalid(_))
        ));
    }

    #[test]
    fn create_body_parses_and_requires_deposit_fields() {
        let body = json!({
            "id": "ABC123", "token": "tok", "status": "NEW",
            "time": {"expiration": 1_700_000_600},
            "from": {"address": "T9y", "amount": "15.01", "tag": null, "usd": "15.01"},
            "to": {"usd": "15.00"}
        });
        let order = parse_order(&body).unwrap();
        assert_eq!(order.deposit_amount, "15.01");
        assert_eq!(order.deposit_memo, None);
        assert_eq!(order.expires_at, 1_700_000_600);
        assert_eq!(order.status.state, SwapState::AwaitingDeposit);
        assert_eq!(order.fee.unwrap().payout_usd, "15.00");

        let mut no_expiry = body.clone();
        no_expiry["time"] = json!({});
        assert!(parse_order(&no_expiry).is_err());
        let mut bad_amount = body;
        bad_amount["from"]["amount"] = json!("1e3");
        assert!(parse_order(&bad_amount).is_err());
    }
}
