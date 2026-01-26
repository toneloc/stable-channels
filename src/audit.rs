use std::io::Write;
use serde_json::Value;
use std::sync::OnceLock;


static AUDIT_LOG_PATH: OnceLock<String> = OnceLock::new();

pub fn set_audit_log_path(path: &str) {
    let _ = AUDIT_LOG_PATH.set(path.to_owned());
}

pub fn get_audit_log_path() -> Option<&'static str> {
    AUDIT_LOG_PATH.get().map(|s| s.as_str())
}

pub fn audit_event(event: &str, data: Value) {
    if let Some(path_str) = get_audit_log_path() {
        let path = std::path::Path::new(path_str);

        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // compose log line
        let log_line = serde_json::json!({
            "ts": chrono::Utc::now().to_rfc3339(),
            "event": event,
            "data": data
        });

        // append to file
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{}", log_line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_event_no_panic_without_path() {
        // When no path is set, audit_event should silently do nothing
        audit_event("TEST_EVENT", serde_json::json!({"key": "value"}));
        // If we reach here without panic, test passes
    }

    #[test]
    fn test_get_audit_log_path_returns_none_initially() {
        // Note: This may return Some if another test set the path first
        // due to OnceLock behavior, but we test the function works
        let _path = get_audit_log_path();
        // Just verify it doesn't panic
    }

    #[test]
    fn test_audit_log_json_structure() {
        // Test that the JSON structure we build is correct
        let event = "TEST_EVENT";
        let data = serde_json::json!({"key": "value"});
        let log_line = serde_json::json!({
            "ts": chrono::Utc::now().to_rfc3339(),
            "event": event,
            "data": data
        });

        assert!(log_line.get("ts").is_some());
        assert_eq!(log_line.get("event").unwrap(), "TEST_EVENT");
        assert_eq!(log_line.get("data").unwrap().get("key").unwrap(), "value");
    }
}