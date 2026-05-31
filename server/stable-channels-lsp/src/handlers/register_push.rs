//! RegisterPush handler: wallets POST an APNs/FCM device token and Lightning node id, stored in push_tokens.db.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use sc_protos::stable::{RegisterPushRequest, RegisterPushResponse};

use crate::handlers::{decode_body, ok_response};
use crate::state::AppState;

pub async fn register_push(State(state): State<AppState>, body: Bytes) -> Response {
    let req: RegisterPushRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    {
        let push = state.push.lock().await;
        push.register_token(&req.token, &req.platform, &req.node_id, &req.environment);
    }

    ok_response(RegisterPushResponse { ok: true })
}
