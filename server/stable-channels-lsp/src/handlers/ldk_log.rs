//! LdkLog handler: tails LDK Server's log file and returns the last N lines.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use tracing::warn;

use sc_protos::stable::{LogRequest, LogResponse};

use crate::handlers::{decode_body, ok_response};
use crate::state::AppState;

pub async fn ldk_log(State(state): State<AppState>, body: Bytes) -> Response {
    let req: LogRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let Some(path) = state.ldk_log_file.as_ref() else {
        return ok_response(LogResponse { content: String::new() });
    };

    let content = read_tail_lines(path, req.max_lines as usize);
    ok_response(LogResponse { content })
}

/// Read the last `max_lines` lines from `path`. Returns empty string on any error.
fn read_tail_lines(path: &Path, max_lines: usize) -> String {
    if max_lines == 0 {
        return String::new();
    }
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            warn!("[ldk-log] cannot open {}: {}", path.display(), e);
            return String::new();
        }
    };
    let reader = BufReader::new(file);
    let mut buf: VecDeque<String> = VecDeque::with_capacity(max_lines);
    for line in reader.lines().map_while(Result::ok) {
        if buf.len() == max_lines {
            buf.pop_front();
        }
        buf.push_back(line);
    }
    buf.into_iter().collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn empty_max_lines_returns_empty() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "line1").unwrap();
        assert_eq!(read_tail_lines(tmp.path(), 0), "");
    }

    #[test]
    fn fewer_lines_than_requested_returns_all() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "line1").unwrap();
        writeln!(tmp, "line2").unwrap();
        let out = read_tail_lines(tmp.path(), 10);
        assert_eq!(out, "line1\nline2");
    }

    #[test]
    fn more_lines_than_requested_returns_tail() {
        let mut tmp = NamedTempFile::new().unwrap();
        for i in 1..=20 {
            writeln!(tmp, "line{}", i).unwrap();
        }
        let out = read_tail_lines(tmp.path(), 3);
        assert_eq!(out, "line18\nline19\nline20");
    }

    #[test]
    fn missing_file_returns_empty() {
        assert_eq!(read_tail_lines(Path::new("/nope/does/not/exist.log"), 5), "");
    }
}
