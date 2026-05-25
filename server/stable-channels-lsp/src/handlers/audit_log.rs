//! AuditLog handler. Returns the tail of the SC daemon's audit_log.txt.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use sc_protos::stable::{LogRequest, LogResponse};

use crate::handlers::{decode_body, ok_response};
use crate::state::AppState;

const DEFAULT_MAX_LINES: usize = 200;

pub async fn audit_log(State(state): State<AppState>, body: Bytes) -> Response {
    let req: LogRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    let max_lines = if req.max_lines == 0 {
        DEFAULT_MAX_LINES
    } else {
        req.max_lines as usize
    };

    let path = state.data_dir.join("audit_log.txt");
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => {
            let lines: Vec<&str> = s.lines().collect();
            let start = lines.len().saturating_sub(max_lines);
            lines[start..].join("\n")
        },
        Err(e) => format!("Error reading {}: {}", path.display(), e),
    };

    ok_response(LogResponse { content })
}
