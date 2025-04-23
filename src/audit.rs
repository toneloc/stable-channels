use std::fs::{OpenOptions, create_dir_all};
use std::io::Write;
use std::path::PathBuf;
use chrono::Utc;
use serde_json::{json, Value};

static mut AUDIT_LOG_PATH: Option<PathBuf> = None;

/// Must be called once per mode to set the log path (e.g. at node init)
pub fn set_audit_log_path(path: &str) {
    unsafe {
        AUDIT_LOG_PATH = Some(PathBuf::from(path));
    }
}


/// Logs a structured event to the configured audit log path
pub fn audit_event(event: &str, data: Value) {
    let log_path = unsafe {
        AUDIT_LOG_PATH.as_ref().cloned()
    };

    if let Some(path) = log_path {
        if let Some(parent) = path.parent() {
            let _ = create_dir_all(parent);
        }

        let timestamp = Utc::now().to_rfc3339();
        let log_line = json!({
            "ts": timestamp,
            "event": event,
            "data": data
        });

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(file, "{}", log_line);
        }
    }
}
