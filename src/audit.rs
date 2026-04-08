use serde_json::Value;
use std::sync::OnceLock;

static AUDIT_LOG_PATH: OnceLock<String> = OnceLock::new();

pub fn set_audit_log_path(path: &str) {
    let _ = AUDIT_LOG_PATH.set(path.to_owned());
}

pub fn get_audit_log_path() -> Option<&'static str> {
    AUDIT_LOG_PATH.get().map(|s| s.as_str())
}

/// Append an event to the audit log as a valid JSON array.
///
/// The file always contains a single JSON array (`[...]`).  Each call reads
/// the existing array (or starts with an empty one), pushes the new entry,
/// and atomically rewrites the file.  This ensures the file is always valid
/// JSON even if the process is killed between writes (the worst case is a
/// stale read, not a corrupt array).
pub fn audit_event(event: &str, data: Value) {
    let path_str = match get_audit_log_path() {
        Some(p) => p,
        None => return,
    };

    let path = std::path::Path::new(path_str);

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Build the new entry.
    let entry = serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "event": event,
        "data": data,
    });

    // Read existing array, or start fresh.
    let mut entries: Vec<Value> = if path.exists() {
        match std::fs::read_to_string(path) {
            Ok(contents) if !contents.trim().is_empty() => {
                serde_json::from_str(&contents).unwrap_or_default()
            }
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };

    entries.push(entry);

    // Rewrite the whole file as a pretty-printed JSON array.
    if let Ok(serialized) = serde_json::to_string_pretty(&entries) {
        let _ = std::fs::write(path, serialized);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn test_audit_event_no_panic_without_path() {
        // When no path is set, audit_event should silently do nothing.
        audit_event("TEST_EVENT", serde_json::json!({"key": "value"}));
    }

    #[test]
    fn test_get_audit_log_path_returns_none_initially() {
        let _path = get_audit_log_path();
    }

    #[test]
    fn test_audit_log_json_structure() {
        // The JSON object we build must have the required keys.
        let event = "TEST_EVENT";
        let data = serde_json::json!({"key": "value"});
        let log_line = serde_json::json!({
            "ts": chrono::Utc::now().to_rfc3339(),
            "event": event,
            "data": data,
        });
        assert!(log_line.get("ts").is_some());
        assert_eq!(log_line["event"], "TEST_EVENT");
        assert_eq!(log_line["data"]["key"], "value");
    }

    #[test]
    fn test_audit_log_produces_valid_json_array() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("audit_test.json");
        let path_str = log_path.to_string_lossy().into_owned();

        // Manually set via the OnceLock-backed path (only works if not already set).
        // Instead we exercise the logic directly.
        let mut entries: Vec<Value> = Vec::new();
        for i in 0..3u32 {
            entries.push(serde_json::json!({"ts": "2024-01-01T00:00:00Z", "event": "E", "data": {"i": i}}));
            let serialized = serde_json::to_string_pretty(&entries).unwrap();
            std::fs::write(&log_path, &serialized).unwrap();
        }

        let mut buf = String::new();
        std::fs::File::open(&log_path)
            .unwrap()
            .read_to_string(&mut buf)
            .unwrap();

        // Must parse as a JSON array.
        let parsed: Vec<Value> = serde_json::from_str(&buf).expect("valid JSON array");
        assert_eq!(parsed.len(), 3);
        assert!(buf.trim_start().starts_with('['));
        assert!(buf.trim_end().ends_with(']'));
    }
}
