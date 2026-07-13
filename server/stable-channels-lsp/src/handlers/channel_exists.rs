//! ChannelExists handler: app-facing restore guard. A wallet restoring from a
//! bare seed asks whether its node_id still has channels with the LSP before
//! reestablishing with wiped LDK state — which would force-close them (the
//! reset-to-zero channel_reestablish class). Unsigned JSON like the legacy
//! push route: whether a node_id has a channel with this LSP is not sensitive
//! enough to justify blocking the recovery UX behind a signature the wallet
//! cannot produce until its node is already running.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use ldk_server_client::ldk_server_grpc::api::ListChannelsRequest;

use crate::stable_manager::LdkServerCalls;
use crate::state::AppState;

#[derive(serde::Deserialize)]
pub struct ChannelExistsRequest {
    pub node_id: String,
}

fn json_response(status: axum::http::StatusCode, body: String) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body))
        .unwrap()
}

/// Returns whether the node_id is a valid 33-byte compressed pubkey hex string.
pub fn is_valid_node_id(node_id: &str) -> bool {
    node_id.len() == 66
        && node_id.chars().all(|c| c.is_ascii_hexdigit())
        && (node_id.starts_with("02") || node_id.starts_with("03"))
}

pub async fn channel_exists(State(state): State<AppState>, body: Bytes) -> Response {
    let req: ChannelExistsRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return json_response(
                axum::http::StatusCode::BAD_REQUEST,
                serde_json::json!({ "error": e.to_string() }).to_string(),
            );
        }
    };
    let node_id = req.node_id.to_lowercase();
    if !is_valid_node_id(&node_id) {
        return json_response(
            axum::http::StatusCode::BAD_REQUEST,
            serde_json::json!({ "error": "invalid node_id" }).to_string(),
        );
    }

    let ldk = state.ldk_server.as_ref() as &dyn LdkServerCalls;
    let count = match ldk.list_channels(ListChannelsRequest {}).await {
        Ok(r) => r
            .channels
            .iter()
            .filter(|c| c.counterparty_node_id.to_lowercase() == node_id)
            .count(),
        Err(e) => {
            return json_response(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                serde_json::json!({ "error": e.to_string() }).to_string(),
            );
        }
    };

    stable_channels::audit::audit_event(
        "CHANNEL_EXISTS_CHECK",
        serde_json::json!({ "node_id": node_id, "count": count }),
    );
    json_response(
        axum::http::StatusCode::OK,
        serde_json::json!({ "exists": count > 0, "count": count }).to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_validation() {
        let valid = format!("02{}", "ab".repeat(32));
        assert!(is_valid_node_id(&valid));
        let valid3 = format!("03{}", "cd".repeat(32));
        assert!(is_valid_node_id(&valid3));

        assert!(!is_valid_node_id("")); // empty
        assert!(!is_valid_node_id("02abcd")); // too short
        assert!(!is_valid_node_id(&format!("04{}", "ab".repeat(32)))); // bad prefix
        assert!(!is_valid_node_id(&format!("02{}zz", "ab".repeat(31)))); // non-hex
    }

    #[test]
    fn request_body_parses() {
        let req: ChannelExistsRequest =
            serde_json::from_str(r#"{"node_id":"02abcd"}"#).unwrap();
        assert_eq!(req.node_id, "02abcd");
        assert!(serde_json::from_str::<ChannelExistsRequest>(r#"{}"#).is_err());
    }
}
