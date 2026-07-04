//! AuditLog handler. Returns the tail of the SC daemon's audit_log.txt.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use sc_protos::stable::{LogRequest, LogResponse};

use crate::handlers::{decode_body, ok_response};
use crate::state::AppState;

const DEFAULT_MAX_LINES: usize = 200;
const FULL_HISTORY_CAP: usize = 5000;

/// Keep lines matching `filter` (substring; all when empty); `full` returns the whole matching history oldest-first capped at FULL_HISTORY_CAP, else the last `max_lines`.
pub(crate) fn filter_tail(content: &str, filter: &str, max_lines: usize, full: bool) -> String {
    let lines: Vec<&str> = if filter.is_empty() {
        content.lines().collect()
    } else {
        content.lines().filter(|l| l.contains(filter)).collect()
    };
    if full {
        let start = lines.len().saturating_sub(FULL_HISTORY_CAP);
        let body = lines[start..].join("\n");
        return if start > 0 {
            format!("[… {} older matching line(s) truncated — narrow the filter to see them …]\n{}", start, body)
        } else {
            body
        };
    }
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

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
        Ok(s) => filter_tail(&s, &req.filter, max_lines, req.full),
        Err(e) => format!("Error reading {}: {}", path.display(), e),
    };

    ok_response(LogResponse { content })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_filter_returns_tail() {
        let c = "a\nb\nc\nd";
        assert_eq!(filter_tail(c, "", 2, false), "c\nd");
        assert_eq!(filter_tail(c, "", 10, false), "a\nb\nc\nd");
    }

    #[test]
    fn filter_matches_whole_content_then_tails() {
        let c = "x1\ny\nx2\nz\nx3";
        assert_eq!(filter_tail(c, "x", 2, false), "x2\nx3"); // last 2 of the 3 matches, from across the file
        assert_eq!(filter_tail(c, "x", 10, false), "x1\nx2\nx3");
    }

    #[test]
    fn filter_no_match_is_empty() {
        assert_eq!(filter_tail("a\nb", "zzz", 5, false), "");
    }

    #[test]
    fn full_returns_all_matches_oldest_first_ignoring_max_lines() {
        let c = "x1\ny\nx2\nz\nx3";
        // full mode returns every match oldest-first regardless of the (tail) max_lines.
        assert_eq!(filter_tail(c, "x", 2, true), "x1\nx2\nx3");
        assert_eq!(filter_tail(c, "", 1, true), c);
    }

    #[test]
    fn full_caps_and_prepends_truncation_notice() {
        let content = (0..FULL_HISTORY_CAP + 3).map(|i| format!("m{i}")).collect::<Vec<_>>().join("\n");
        let out = filter_tail(&content, "m", 200, true);
        assert!(out.starts_with("[… 3 older matching line(s) truncated"));
        // keeps the most-recent CAP matches, newest line last.
        assert!(out.ends_with(&format!("m{}", FULL_HISTORY_CAP + 2)));
        assert!(!out.contains("\nm2\n")); // the 3 oldest are dropped
    }
}
