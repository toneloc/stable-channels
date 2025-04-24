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