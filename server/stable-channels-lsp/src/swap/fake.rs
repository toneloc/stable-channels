//! In-process FixedFloat-compatible fake provider for tests (axum on 127.0.0.1:0), modelled on
//! OpenReceive's `FakeLscProviderCore`: signed `/api/v2` subset + `/rates/fixed.xml`, with a
//! control to force an order into any swap state. No real network.

use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use super::provider::{sign, FixedFloatClient};

pub const KEY: &str = "fake-key";
pub const SECRET: &str = "fake-secret";
pub const TRON_ADDRESS: &str = "T9yD14Nj9j7xAB4dbGeiX9h8unkKHxuWwb";
pub const ETH_ADDRESS: &str = "0x1111111111111111111111111111111111111111";

#[derive(Clone)]
pub struct FakeOrder {
    pub id: String,
    pub token: String,
    pub bolt11: String,
    pub from_ccy: String,
    pub state: &'static str,
    pub refund_address: Option<String>,
}

#[derive(Default)]
pub struct FakeState {
    pub orders: Mutex<Vec<FakeOrder>>,
    pub creates: Mutex<usize>,
}

pub struct FakeProvider {
    pub base_url: String,
    pub state: Arc<FakeState>,
}

impl FakeProvider {
    pub async fn start() -> Self {
        let state = Arc::new(FakeState::default());
        let app = Router::new()
            .route("/rates/fixed.xml", get(rates))
            .route("/api/v2/{path}", post(api))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            base_url: format!("http://{addr}"),
            state,
        }
    }

    pub fn client(&self, id: &str) -> FixedFloatClient {
        FixedFloatClient::new(id, &self.base_url, KEY, SECRET)
    }

    /// Force every order into an OpenReceive state (rendered back as FixedFloat statuses).
    pub fn force_state(&self, state: &'static str) {
        for order in self.state.orders.lock().unwrap().iter_mut() {
            order.state = state;
        }
    }

    pub fn orders(&self) -> Vec<FakeOrder> {
        self.state.orders.lock().unwrap().clone()
    }
}

async fn rates() -> String {
    let item = |code: &str| {
        format!(
            "<item><from>{code}</from><to>BTCLN</to><in>1.01</in><out>0.00001</out>\
             <amount>10 BTC</amount><minamount>10 USDT</minamount>\
             <maxamount>10000 USDT</maxamount><tofee>0 BTC</tofee></item>"
        )
    };
    format!("<rates>{}{}</rates>", item("USDTTRC"), item("USDTETH"))
}

fn envelope(status: StatusCode, code: i64, msg: &str, data: Value) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(json!({ "code": code, "msg": msg, "data": data })),
    )
}

async fn api(
    State(state): State<Arc<FakeState>>,
    Path(path): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    let body = String::from_utf8_lossy(&body).to_string();
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
    };
    if header("X-API-KEY") != KEY || header("X-API-SIGN") != sign(SECRET, &body) {
        return envelope(
            StatusCode::UNAUTHORIZED,
            1,
            "Invalid signature",
            Value::Null,
        );
    }
    let req: Value = serde_json::from_str(&body).unwrap_or(json!({}));
    let ok = |data| envelope(StatusCode::OK, 0, "OK", data);
    match path.as_str() {
        "ccies" => ok(json!([
            {"code": "USDTTRC", "coin": "USDT", "network": "TRC20", "recv": true, "send": true},
            {"code": "USDTETH", "coin": "USDT", "network": "ERC20", "recv": true, "send": true},
            {"code": "USDCSOL", "coin": "USDC", "network": "SOL", "recv": false, "send": true},
            {"code": "BTCLN", "coin": "BTC", "network": "Lightning", "recv": true, "send": true}
        ])),
        "price" => ok(json!({"from": {"usd": "15.16"}, "to": {"usd": "15.00"}})),
        "create" => {
            let mut creates = state.creates.lock().unwrap();
            *creates += 1;
            let order = FakeOrder {
                id: format!("ORD{creates}"),
                token: format!("provider-token-{creates}"),
                bolt11: req["toAddress"].as_str().unwrap_or_default().to_string(),
                from_ccy: req["fromCcy"].as_str().unwrap_or_default().to_string(),
                state: "awaiting_deposit",
                refund_address: None,
            };
            let rendered = render(&order);
            state.orders.lock().unwrap().push(order);
            ok(rendered)
        }
        "order" | "emergency" => {
            let mut orders = state.orders.lock().unwrap();
            let Some(order) = orders.iter_mut().find(|o| {
                Some(o.id.as_str()) == req["id"].as_str()
                    && Some(o.token.as_str()) == req["token"].as_str()
            }) else {
                return envelope(StatusCode::OK, 1, "Order not found", Value::Null);
            };
            if path == "emergency" {
                if req["choice"] != "REFUND"
                    || req["address"].as_str().unwrap_or_default().is_empty()
                {
                    return envelope(StatusCode::OK, 1, "Invalid emergency request", Value::Null);
                }
                order.refund_address = req["address"].as_str().map(str::to_string);
                order.state = "refund_pending";
                return ok(json!(true));
            }
            ok(render(order))
        }
        _ => envelope(StatusCode::NOT_FOUND, 404, "Not found", Value::Null),
    }
}

fn render(order: &FakeOrder) -> Value {
    let (status, choice) = match order.state {
        "awaiting_deposit" => ("NEW", "NONE"),
        "confirming" => ("PENDING", "NONE"),
        "exchanging" => ("EXCHANGE", "NONE"),
        "paying_invoice" => ("WITHDRAW", "NONE"),
        "completed" => ("DONE", "NONE"),
        "expired" => ("EXPIRED", "NONE"),
        "refund_required" => ("EMERGENCY", "NONE"),
        "refund_pending" | "refunded" => ("EMERGENCY", "REFUND"),
        _ => ("SOMETHING_NEW", "NONE"),
    };
    let address = if order.from_ccy == "USDTETH" {
        ETH_ADDRESS
    } else {
        TRON_ADDRESS
    };
    let mut body = json!({
        "id": order.id,
        "type": "fixed",
        "token": order.token,
        "status": status,
        "time": {"reg": 1_700_000_000u64, "expiration": 1_700_000_600u64},
        "from": {"code": order.from_ccy, "address": address, "tag": null, "amount": "15.16"},
        "to": {"code": "BTCLN", "address": order.bolt11, "amount": "0.0015"},
        "emergency": {"status": if status == "EMERGENCY" { json!(["LESS"]) } else { json!([]) }, "choice": choice, "repeat": 0}
    });
    if order.state == "refunded" {
        body["back"] = json!({"address": order.refund_address, "tx": {"id": "fake-refund"}});
    }
    body
}
