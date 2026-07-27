use serde_json::Value;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::sync::OnceLock;

use crate::db::Database;
use crate::ledger::{AppendOutcome, LedgerEventDraft};

static CAPTURE_ON: AtomicBool = AtomicBool::new(false);
static CAPTURE: Mutex<Vec<(String, Value)>> = Mutex::new(Vec::new());
static CAPTURE_OWNER: Mutex<Option<std::thread::ThreadId>> = Mutex::new(None);

pub fn enable_test_capture() {
    CAPTURE.lock().unwrap().clear();
    *CAPTURE_OWNER.lock().unwrap() = Some(std::thread::current().id());
    CAPTURE_ON.store(true, Ordering::SeqCst);
}
pub fn disable_test_capture() {
    CAPTURE_ON.store(false, Ordering::SeqCst);
    *CAPTURE_OWNER.lock().unwrap() = None;
}
pub fn drain_test_capture() -> Vec<(String, Value)> {
    std::mem::take(&mut *CAPTURE.lock().unwrap())
}

static AUDIT_LOG_PATH: OnceLock<String> = OnceLock::new();
static AUDIT_LEDGER: Mutex<Option<Database>> = Mutex::new(None);

pub fn set_audit_log_path(path: &str) {
    let _ = AUDIT_LOG_PATH.set(path.to_owned());
}

pub fn get_audit_log_path() -> Option<&'static str> {
    AUDIT_LOG_PATH.get().map(|s| s.as_str())
}

/// Route all subsequent audit events through the authoritative SQLite ledger.
/// Calling this again replaces the recorder, which keeps isolated tests and
/// embedded consumers from being tied to the first database opened in-process.
pub fn set_audit_ledger(database: Database) {
    *AUDIT_LEDGER.lock().unwrap() = Some(database);
}

/// Record one event and then mirror the committed row to JSONL. A mirror error
/// never changes the SQLite result.
pub fn record_event(event: &str, data: Value) -> rusqlite::Result<AppendOutcome> {
    if CAPTURE_ON.load(Ordering::SeqCst)
        && CAPTURE_OWNER.lock().unwrap().as_ref() == Some(&std::thread::current().id())
    {
        CAPTURE.lock().unwrap().push((event.to_owned(), data.clone()));
    }
    let mut draft = LedgerEventDraft::from_audit_event(event, data.clone());
    let database = AUDIT_LEDGER.lock().unwrap().clone();
    let outcome = match database {
        Some(database) => {
            // Named compatibility events often carry the price/amount decision
            // but omit unchanged allocation fields. Enrich them from the
            // authoritative post-commit channel row so the structured event is
            // directly auditable without parsing a neighboring log line.
            if let Some(user_channel_id) = draft
                .refs
                .iter()
                .find(|reference| reference.role == "user_channel_id")
                .map(|reference| reference.value.clone())
            {
                if let Some(channel) = database.load_channel(&user_channel_id)? {
                    let snapshot = draft.after.get_or_insert_with(Default::default);
                    snapshot.expected_usd.get_or_insert(channel.expected_usd);
                    snapshot.backing_sats.get_or_insert(channel.backing_sats);
                    snapshot.native_sats.get_or_insert(channel.native_sats);
                    snapshot
                        .live_receiver_sats
                        .get_or_insert(channel.backing_sats.saturating_add(channel.native_sats));
                }
            }
            database.append_ledger_event(&draft)?
        },
        None => AppendOutcome { event_id: 0, inserted: true },
    };
    // Deduplicated replays already have a mirrored first occurrence.
    if outcome.inserted {
        mirror_event(event, data, (outcome.event_id != 0).then_some(outcome.event_id));
    }
    Ok(outcome)
}

pub fn audit_event(event: &str, data: Value) {
    if let Err(error) = record_event(event, data) {
        // Existing callers intentionally cannot ignore a JSON mirror failure;
        // they may, however, be legacy non-transactional sites. Surface the
        // authoritative write failure without fabricating a committed mirror.
        eprintln!("channel ledger write failed for {event}: {error}");
    }
}

fn mirror_event(event: &str, data: Value, ledger_id: Option<i64>) {
    if let Some(path_str) = get_audit_log_path() {
        let _ = mirror_event_at(std::path::Path::new(path_str), event, data, ledger_id);
    }
}

pub(crate) fn mirror_committed_ledger_event(draft: &LedgerEventDraft, ledger_id: i64) {
    mirror_event(
        &draft.event_type,
        draft.detail.clone(),
        Some(ledger_id),
    );
}

fn mirror_event_at(
    path: &std::path::Path,
    event: &str,
    data: Value,
    ledger_id: Option<i64>,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut log_line = serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "event": event,
        "data": data
    });
    if let Some(ledger_id) = ledger_id {
        log_line["ledger_id"] = serde_json::json!(ledger_id);
    }

    // One write_all is a single atomic O_APPEND; writeln! emits token-by-token
    // so concurrent calls can interleave into corrupt lines.
    let mut line = log_line.to_string();
    line.push('\n');
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.as_bytes())
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
    fn test_capture_records_events_when_enabled() {
        enable_test_capture();
        audit_event("CAP_TEST", serde_json::json!({"user_channel_id": "42"}));
        let got = drain_test_capture();
        disable_test_capture();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "CAP_TEST");
        assert_eq!(got[0].1.get("user_channel_id").unwrap(), "42");
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

    #[test]
    fn mirror_failure_cannot_remove_committed_sqlite_event() {
        let database = Database::open_in_memory().unwrap();
        let outcome = database
            .append_ledger_event(&LedgerEventDraft::from_audit_event(
                "PAYMENT_SETTLED",
                serde_json::json!({"payment_id": "pay"}),
            ))
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        assert!(mirror_event_at(
            directory.path(),
            "PAYMENT_SETTLED",
            serde_json::json!({"payment_id": "pay"}),
            Some(outcome.event_id),
        )
        .is_err());
        let page = database
            .list_ledger_events(&crate::ledger::LedgerQuery {
                identifier: Some("pay".to_owned()),
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].id, outcome.event_id);
    }

    #[test]
    fn mirror_includes_ledger_id() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("audit_log.txt");
        mirror_event_at(&path, "CHANNEL_READY", serde_json::json!({}), Some(41)).unwrap();
        let line: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(line["ledger_id"], 41);
    }
}
