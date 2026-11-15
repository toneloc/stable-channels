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
    if AUDIT_LOG_PATH.set(path.to_owned()).is_ok() {
        restrict_existing_logs(std::path::Path::new(path));
    }
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

/// Record one event: channel state changes go to the SQLite ledger and are then mirrored to
/// JSONL; operational events go to JSONL only and come back with `event_id` 0, so a caller
/// that needs a committed record can tell. A mirror error never changes the SQLite result.
pub fn record_event(event: &str, data: Value) -> rusqlite::Result<AppendOutcome> {
    if CAPTURE_ON.load(Ordering::SeqCst)
        && CAPTURE_OWNER.lock().unwrap().as_ref() == Some(&std::thread::current().id())
    {
        CAPTURE.lock().unwrap().push((event.to_owned(), data.clone()));
    }
    if !crate::ledger::records_channel_state(event) {
        mirror_event(event, data, None);
        return Ok(AppendOutcome { event_id: 0, inserted: true });
    }
    let draft = LedgerEventDraft::from_audit_event(event, data.clone());
    let database = AUDIT_LEDGER.lock().unwrap().clone();
    let outcome = match database {
        Some(database) => match database.append_ledger_event(&draft) {
            Ok(outcome) => outcome,
            Err(error) => {
                // The uncommitted JSONL line still records it, so an alarm about a failing DB is not lost with the write.
                mirror_event(event, data, None);
                return Err(error);
            },
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
        create_log_dir_all(parent)?;
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
    let rotated_permissions = rotate_if_oversize(path);
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    // Best effort: a mode that cannot be changed must not cost the line.
    #[cfg(unix)]
    match rotated_permissions {
        // The new file takes the old one's mode under the log rule in one step, so it is never briefly readable by others.
        Some(permissions) => {
            use std::os::unix::fs::PermissionsExt;
            let mode = (permissions.mode() & 0o777 & !LOG_MODE_REMOVED) | 0o600;
            let _ = file.set_permissions(std::fs::Permissions::from_mode(mode));
        },
        None => {
            let _ = restrict_log_mode(&file);
        },
    }
    #[cfg(not(unix))]
    let _ = rotated_permissions;
    (&file).write_all(line.as_bytes())
}

/// Create the log's directory and any missing parents; on Unix they are readable by their owner only.
fn create_log_dir_all(dir: &std::path::Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Bits a log file loses: it holds payment details, so no access for other users, and only the daemon writes it, so no group write; group read stays for log readers.
#[cfg(unix)]
const LOG_MODE_REMOVED: u32 = 0o027;

/// Narrow the live log and its rotated generation, which an older version created readable by other users.
fn restrict_existing_logs(path: &std::path::Path) {
    #[cfg(unix)]
    for log in [path.to_path_buf(), rotated_path(path)] {
        let _ = restrict_log_file_at(&log);
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(unix)]
fn restrict_log_file_at(path: &std::path::Path) -> std::io::Result<()> {
    // Only a regular file: opening a FIFO would block startup, and a symlinked `.1` points at a file the log does not own.
    if !std::fs::symlink_metadata(path)?.is_file() {
        return Ok(());
    }
    restrict_log_mode(&std::fs::File::open(path)?)
}

#[cfg(unix)]
fn restrict_log_mode(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = file.metadata()?;
    let mode = metadata.permissions().mode() & 0o777;
    if metadata.is_file() && mode & LOG_MODE_REMOVED != 0 {
        file.set_permissions(std::fs::Permissions::from_mode(mode & !LOG_MODE_REMOVED))?;
    }
    Ok(())
}

fn rotated_path(path: &std::path::Path) -> std::path::PathBuf {
    let mut rotated = path.as_os_str().to_owned();
    rotated.push(".1");
    std::path::PathBuf::from(rotated)
}

/// Cap on the live audit log before it rotates. The file grows one line per event and events can
/// be driven by any Lightning peer (inbound TLVs), so without a bound the disk fills — which stops
/// the daemon and its stability payments. One previous generation is kept (`<path>.1`), so on-disk
/// audit history is bounded to roughly 2× this figure.
const AUDIT_LOG_MAX_BYTES: u64 = 50 * 1024 * 1024;

/// Rotate `path` to `<path>.1` when it reaches the size cap, replacing any prior `.1`. Best-effort:
/// any error leaves the current file in place so logging simply continues appending. The rename is
/// atomic on a single filesystem; a couple of lines may land in the pre-rotation file if another
/// thread appends between the size check and the rename, which is harmless. Returns the rotated
/// file's permissions when it rotated, so the new live file can keep them.
fn rotate_if_oversize(path: &std::path::Path) -> Option<std::fs::Permissions> {
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() < AUDIT_LOG_MAX_BYTES {
        return None;
    }
    let rotated = rotated_path(path);
    std::fs::rename(path, &rotated).ok()?;
    #[cfg(unix)]
    let _ = restrict_log_file_at(&rotated);
    Some(metadata.permissions())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests that install the process-wide audit ledger must not interleave.
    static GLOBAL_LEDGER_LOCK: Mutex<()> = Mutex::new(());

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

    #[test]
    fn generic_audit_events_do_not_infer_an_after_snapshot() {
        let _guard = GLOBAL_LEDGER_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let database = Database::open_in_memory().unwrap();
        database
            .save_channel("physical", "stable", 10.0, 10_000, 5_000, None)
            .unwrap();
        set_audit_ledger(database.clone());

        record_event(
            "PEER_CONNECTED",
            serde_json::json!({"user_channel_id": "stable", "node_id": "peer"}),
        )
        .unwrap();
        let event = database
            .list_ledger_events(&crate::ledger::LedgerQuery {
                identifier: Some("peer".to_owned()),
                limit: 10,
                ..Default::default()
            })
            .unwrap()
            .events
            .into_iter()
            .find(|event| event.event_type == "PEER_CONNECTED")
            .unwrap();
        assert!(event.after.is_none());

        *AUDIT_LEDGER.lock().unwrap() = None;
    }

    #[test]
    fn callers_can_tell_a_durable_record_from_a_jsonl_mirror() {
        let _guard = GLOBAL_LEDGER_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let database = Database::open_in_memory().unwrap();
        set_audit_ledger(database.clone());
        let mirrored = record_event("SYNC_MESSAGE_FAILED", serde_json::json!({"user_channel_id": "u"})).unwrap();
        assert_eq!(mirrored.event_id, 0, "an operational event never reaches SQLite");
        let committed = record_event("RECONCILIATION_RESULT", serde_json::json!({"status": "completed"})).unwrap();
        assert!(committed.event_id > 0, "the health guard relies on this record");
        let refund = record_event("TRADE_FEE_REFUND_SENT", serde_json::json!({"trade_payment_id": "t"})).unwrap();
        assert!(refund.event_id > 0);
        *AUDIT_LEDGER.lock().unwrap() = None;
        let unrouted = record_event("RECONCILIATION_RESULT", serde_json::json!({})).unwrap();
        assert_eq!(unrouted.event_id, 0, "no ledger attached: nothing was committed");
    }

    #[test]
    fn integrity_alarms_reach_the_ledger_and_jsonl_keeps_them_when_the_write_fails() {
        let _guard = GLOBAL_LEDGER_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let directory = tempfile::tempdir().unwrap().keep();
        set_audit_log_path(directory.join("audit_log.txt").to_str().unwrap());
        let database = Database::open(&directory).unwrap();
        set_audit_ledger(database.clone());
        for alarm in ["STABILITY_PAYMENT_STATE_DIVERGENCE", "STABILITY_PAYMENT_REPLAY_CONFLICT", "STABILITY_PAYMENT_PERSIST_FAILED", "DB_WRITE_FAILED"] {
            assert!(record_event(alarm, serde_json::json!({"error": "x"})).unwrap().event_id > 0, "{alarm}");
        }
        rusqlite::Connection::open(directory.join(crate::db::DB_FILENAME)).unwrap().execute_batch(
            "CREATE TRIGGER refuse BEFORE INSERT ON ledger_events BEGIN SELECT RAISE(ABORT, 'disk full'); END;"
        ).unwrap();
        assert!(record_event("DB_WRITE_FAILED", serde_json::json!({"error": "disk full"})).is_err());
        let log = std::fs::read_to_string(get_audit_log_path().unwrap()).unwrap();
        let last: Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
        assert_eq!((last["event"].as_str(), last.get("ledger_id")), (Some("DB_WRITE_FAILED"), None), "mirrored without a ledger id");
        *AUDIT_LEDGER.lock().unwrap() = None;
    }

    #[test]
    fn operational_events_stay_out_of_the_channel_ledger() {
        let _guard = GLOBAL_LEDGER_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let database = Database::open_in_memory().unwrap();
        set_audit_ledger(database.clone());

        for _ in 0..3 {
            record_event(
                "SYNC_MESSAGE_FAILED",
                serde_json::json!({"user_channel_id": "busy", "stage": "send", "error": "RouteNotFound"}),
            )
            .unwrap();
        }
        record_event("SYNC_RETRY_EXHAUSTED", serde_json::json!({"user_channel_id": "busy", "attempts": 10})).unwrap();
        let types: Vec<String> = database
            .list_ledger_events(&crate::ledger::LedgerQuery {
                identifier: Some("busy".to_owned()),
                limit: 10,
                ..Default::default()
            })
            .unwrap()
            .events
            .into_iter()
            .map(|event| event.event_type)
            .collect();
        assert_eq!(types, ["SYNC_RETRY_EXHAUSTED"]);

        *AUDIT_LEDGER.lock().unwrap() = None;
    }

    #[cfg(unix)]
    #[test]
    fn the_audit_log_is_created_private_and_never_readable_by_other_users_through_rotation() {
        use std::os::unix::fs::PermissionsExt;
        let mode_of = |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let directory = tempfile::tempdir().unwrap();
        let log_dir = directory.path().join("wallet");
        let path = log_dir.join("audit_log.txt");

        mirror_event_at(&path, "CHANNEL_READY", serde_json::json!({"n": 1}), Some(1)).unwrap();
        assert_eq!(mode_of(&log_dir), 0o700);
        assert_eq!(mode_of(&path), 0o600);

        // A log left open from before loses other users' access and group write on the next write; group read stays.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o664)).unwrap();
        mirror_event_at(&path, "CHANNEL_READY", serde_json::json!({"n": 2}), Some(2)).unwrap();
        assert_eq!(mode_of(&path), 0o640);

        // Rotating a file that was opened up again: both the rotated and the new file follow the rule.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_len(AUDIT_LOG_MAX_BYTES).unwrap();
        mirror_event_at(&path, "CHANNEL_READY", serde_json::json!({"n": 3}), Some(3)).unwrap();
        let rotated = log_dir.join("audit_log.txt.1");
        assert!(rotated.exists());
        assert_eq!(mode_of(&rotated), 0o640);
        assert_eq!(mode_of(&path), 0o640);
        let line: Value = serde_json::from_str(std::fs::read_to_string(&path).unwrap().trim()).unwrap();
        assert_eq!(line["data"]["n"], 3);

        // An operator's chmod 600 survives rotation too.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_len(AUDIT_LOG_MAX_BYTES).unwrap();
        mirror_event_at(&path, "CHANNEL_READY", serde_json::json!({"n": 4}), Some(4)).unwrap();
        assert_eq!(mode_of(&rotated), 0o600);
        assert_eq!(mode_of(&path), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn logs_an_older_version_left_open_are_narrowed_at_startup_but_a_symlink_is_left_alone() {
        use std::os::unix::fs::PermissionsExt;
        let mode_of = |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("audit_log.txt");
        let rotated = directory.path().join("audit_log.txt.1");
        for log in [&path, &rotated] {
            std::fs::write(log, b"{}\n").unwrap();
            std::fs::set_permissions(log, std::fs::Permissions::from_mode(0o664)).unwrap();
        }

        restrict_existing_logs(&path);

        assert_eq!(mode_of(&path), 0o640);
        assert_eq!(mode_of(&rotated), 0o640);

        let elsewhere = directory.path().join("elsewhere");
        std::fs::write(&elsewhere, b"not the log").unwrap();
        std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o664)).unwrap();
        std::fs::remove_file(&rotated).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &rotated).unwrap();

        restrict_existing_logs(&path);

        assert_eq!(mode_of(&elsewhere), 0o664);
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_at_the_rotated_log_path_does_not_block_startup() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("audit_log.txt");
        let rotated = directory.path().join("audit_log.txt.1");
        assert!(std::process::Command::new("mkfifo").arg(&rotated).status().unwrap().success());

        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            restrict_existing_logs(&path);
            let _ = done.send(());
        });

        assert!(finished.recv_timeout(std::time::Duration::from_secs(5)).is_ok(), "startup blocked on a FIFO");
    }
}
