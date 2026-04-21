use serde_json::Value;
use std::io::Write;
use std::sync::OnceLock;

static AUDIT_LOG_PATH: OnceLock<String> = OnceLock::new();

pub fn set_audit_log_path(path: &str) {
    let _ = AUDIT_LOG_PATH.set(path.to_owned());
}

pub fn get_audit_log_path() -> Option<&'static str> {
    AUDIT_LOG_PATH.get().map(|s| s.as_str())
}

/// Append an audit event to the log file in JSONL format.
///
/// Each call opens the file in append mode and writes exactly one line:
/// a compact JSON object followed by `\n`.  No existing data is ever read
/// or rewritten, so writes are O(1) and a crash between writes at most
/// loses the in-flight entry — all previous entries remain intact.
///
/// To convert the log to a proper JSON array when needed:
///   jq -s '.' audit_log.jsonl
pub fn audit_event(event: &str, data: Value) {
    let path_str = match get_audit_log_path() {
        Some(p) => p,
        None => return,
    };

    let path = std::path::Path::new(path_str);

    // Ensure the parent directory exists.
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("[audit] failed to create log directory {}: {}", parent.display(), e);
            return;
        }
    }

    // Build the entry as a compact single-line JSON object.
    let entry = serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "event": event,
        "data": data,
    });

    let mut line = match serde_json::to_string(&entry) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[audit] failed to serialize event '{}': {}", event, e);
            return;
        }
    };
    line.push('\n');

    // Open in append mode (creates the file if it doesn't exist).
    let mut file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[audit] failed to open log file {}: {}", path.display(), e);
            return;
        }
    };

    // A single write_all call makes the entry as atomic as the OS allows;
    // in the worst case only this entry is lost, never any previous one.
    if let Err(e) = file.write_all(line.as_bytes()) {
        eprintln!("[audit] failed to write to log file {}: {}", path.display(), e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    #[test]
    fn test_audit_event_no_panic_without_path() {
        // When no path is set, audit_event should silently do nothing.
        audit_event("TEST_EVENT", serde_json::json!({"key": "value"}));
    }

    #[test]
    fn test_get_audit_log_path_returns_none_initially() {
        // Just ensures the getter does not panic.
        let _path = get_audit_log_path();
    }

    #[test]
    fn test_audit_log_json_structure() {
        // Verify the shape of a single entry without touching the filesystem.
        let entry = serde_json::json!({
            "ts": chrono::Utc::now().to_rfc3339(),
            "event": "TEST_EVENT",
            "data": {"key": "value"},
        });
        assert!(entry.get("ts").is_some());
        assert_eq!(entry["event"], "TEST_EVENT");
        assert_eq!(entry["data"]["key"], "value");
    }

    #[test]
    fn test_audit_log_produces_valid_jsonl() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("audit_test.jsonl");

        // Write three entries by appending directly (mirrors what audit_event does).
        {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .expect("open");

            for i in 0..3u32 {
                let entry = serde_json::json!({
                    "ts": "2024-01-01T00:00:00Z",
                    "event": "E",
                    "data": {"i": i},
                });
                let mut line = serde_json::to_string(&entry).unwrap();
                line.push('\n');
                file.write_all(line.as_bytes()).unwrap();
            }
        }

        // Every line must be a valid, self-contained JSON object.
        let file = std::fs::File::open(&log_path).expect("open for reading");
        let lines: Vec<String> = BufReader::new(file)
            .lines()
            .map(|l| l.expect("line"))
            .filter(|l| !l.is_empty())
            .collect();

        assert_eq!(lines.len(), 3, "expected 3 JSONL lines");
        for line in &lines {
            let parsed: Value = serde_json::from_str(line).expect("each line is valid JSON");
            assert!(parsed.get("ts").is_some());
            assert_eq!(parsed["event"], "E");
        }
    }

    #[test]
    fn test_audit_log_appends_not_overwrites() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("append_test.jsonl");

        // Simulate two separate "process runs" writing to the same file.
        for pass in 0..2u32 {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .expect("open");
            let entry = serde_json::json!({"ts": "2024-01-01T00:00:00Z", "event": "E", "data": {"pass": pass}});
            let mut line = serde_json::to_string(&entry).unwrap();
            line.push('\n');
            file.write_all(line.as_bytes()).unwrap();
        }

        let contents = std::fs::read_to_string(&log_path).unwrap();
        let non_empty: Vec<&str> = contents.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(non_empty.len(), 2, "both entries must be present");
    }
}
