//! Daily limits on ledger records that peers' own activity creates: records of channels that never became usable, and connection changes.

use std::collections::HashMap;

use serde_json::{json, Value};

/// Records a day per peer of channels that never became usable, or of a peer without a usable channel.
pub const NEVER_USABLE_RECORDS_PER_PEER: u32 = 5;
/// Such records a day across all peers; past it they are only counted.
pub const NEVER_USABLE_RECORDS_PER_DAY: u32 = 1_000;
/// Connection changes recorded a day for a peer with a usable channel; the 30 s poll can see up to 2,880.
pub const CONNECTION_RECORDS_PER_PEER: u32 = 200;
const DAY_SECS: i64 = 86_400;

/// The kinds of record limited separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordKind {
    /// About a channel that never became usable, or a peer without a usable channel.
    NeverUsable,
    /// A connection change of a peer with a usable channel.
    Connection,
}

impl RecordKind {
    fn per_peer(self) -> u32 {
        match self {
            Self::NeverUsable => NEVER_USABLE_RECORDS_PER_PEER,
            Self::Connection => CONNECTION_RECORDS_PER_PEER,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::NeverUsable => "never_usable",
            Self::Connection => "connection",
        }
    }
}

/// Records written and held back in one day.
#[derive(Debug, Default)]
struct DayCount {
    written: u32,
    not_recorded: u32,
}

impl DayCount {
    /// Count a held-back record; true for the first of the day.
    fn hold_back(&mut self) -> bool {
        self.not_recorded = self.not_recorded.saturating_add(1);
        self.not_recorded == 1
    }
}

/// A day on which records went over a limit, for one peer or, with no peer, across all peers.
#[derive(Debug, PartialEq)]
pub struct LimitedDay {
    pub peer: Option<String>,
    pub kind: RecordKind,
    pub day: i64,
    /// Records held back that day, so far while the day is not over.
    pub not_recorded: u32,
    /// False for the report made as soon as the all-peers limit is reached, true for the one after the day ends.
    pub day_over: bool,
}

/// Record counts per peer and across all peers, kept for each UTC day until that day is reported.
#[derive(Debug, Default)]
pub struct RecordLimits {
    // An entry is only made for a written record, so never-usable entries stop at the all-peers limit.
    per_peer: HashMap<(String, RecordKind, i64), DayCount>,
    never_usable: HashMap<i64, DayCount>,
    /// Days whose all-peers limit was reached and not yet reported; that limit holds back every peer's records, not only one's own.
    reached: Vec<i64>,
}

impl RecordLimits {
    /// Whether a record of `kind` for `peer` may be written at `now` (unix seconds); past a limit it is only counted.
    pub fn allow(&mut self, peer: &str, kind: RecordKind, now: i64) -> bool {
        let day = now.div_euclid(DAY_SECS);
        let key = (peer.to_ascii_lowercase(), kind, day);
        if let Some(count) = self.per_peer.get_mut(&key).filter(|count| count.written >= kind.per_peer()) {
            if count.hold_back() {
                tracing::warn!("[records] {} reached today's limit of {} {} records; the rest are counted", key.0, kind.per_peer(), kind.name());
            }
            return false;
        }
        if kind == RecordKind::NeverUsable {
            let all = self.never_usable.entry(day).or_default();
            if all.written >= NEVER_USABLE_RECORDS_PER_DAY {
                if all.hold_back() {
                    tracing::warn!("[records] today's limit of {} never-usable records across all peers is reached; the rest are counted", NEVER_USABLE_RECORDS_PER_DAY);
                    self.reached.push(day);
                }
                return false;
            }
            all.written += 1;
        }
        self.per_peer.entry(key).or_default().written += 1;
        true
    }

    /// Take the reports due: today's all-peers limit once it is reached, and every ended day that held records back, whose counts are dropped.
    pub fn take_reports(&mut self, now: i64) -> Vec<LimitedDay> {
        let today = now.div_euclid(DAY_SECS);
        // A day that has ended since its limit was reached gets only the report with its full count.
        let mut reached: Vec<LimitedDay> = std::mem::take(&mut self.reached)
            .into_iter()
            .filter(|day| *day == today)
            .map(|day| LimitedDay {
                peer: None,
                kind: RecordKind::NeverUsable,
                day,
                not_recorded: self.never_usable.get(&day).map_or(0, |count| count.not_recorded),
                day_over: false,
            })
            .collect();
        let mut ended = Vec::new();
        self.per_peer.retain(|(peer, kind, day), count| {
            if *day == today {
                return true;
            }
            if count.not_recorded > 0 {
                ended.push(LimitedDay { peer: Some(peer.clone()), kind: *kind, day: *day, not_recorded: count.not_recorded, day_over: true });
            }
            false
        });
        self.never_usable.retain(|day, count| {
            if *day == today {
                return true;
            }
            if count.not_recorded > 0 {
                ended.push(LimitedDay { peer: None, kind: RecordKind::NeverUsable, day: *day, not_recorded: count.not_recorded, day_over: true });
            }
            false
        });
        ended.sort_by(|a, b| (a.day, &a.peer, a.kind.name()).cmp(&(b.day, &b.peer, b.kind.name())));
        reached.append(&mut ended);
        reached
    }
}

/// The ledger record of a day on which records went over a limit.
pub fn limited_day_record(day: &LimitedDay) -> Value {
    let (scope, limit) = match &day.peer {
        Some(peer) => (peer.as_str(), day.kind.per_peer()),
        None => ("all", NEVER_USABLE_RECORDS_PER_DAY),
    };
    let reached = if day.day_over { "" } else { ":reached" };
    let mut record = json!({
        "records": day.kind.name(),
        "day_started_at": day.day * DAY_SECS,
        "limit": limit,
        "not_recorded": day.not_recorded,
        "day_over": day.day_over,
        "dedup_key": format!("lsp:peer-records-limited:{scope}:{}:{}{reached}", day.kind.name(), day.day),
    });
    if let Some(peer) = &day.peer {
        record["counterparty_node_id"] = json!(peer);
    }
    record
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEER: &str = "02465ed5be53d04fde66c9418ff14a5f2267723810176c9212b722e542dc1afb1b";
    const OTHER: &str = "0380ba01cf1f8232298198ce1c6da12a7bd159656e79cbc22a5dfbea62ab1aef4b";
    // Noon UTC on day 20,389.
    const NOON: i64 = 1_761_652_800;

    /// A distinct node id for each `n`.
    fn node(n: u32) -> String {
        format!("02{:064x}", u64::from(n) * 7_919 + 0xa11ce)
    }

    #[test]
    fn a_peer_gets_its_daily_records_then_the_rest_are_only_counted() {
        let mut limits = RecordLimits::default();
        for _ in 0..NEVER_USABLE_RECORDS_PER_PEER {
            assert!(limits.allow(PEER, RecordKind::NeverUsable, NOON));
        }
        assert!(!limits.allow(PEER, RecordKind::NeverUsable, NOON + 60));
        assert!(!limits.allow(&PEER.to_ascii_uppercase(), RecordKind::NeverUsable, NOON), "the same node in upper case");
        assert!(limits.allow(OTHER, RecordKind::NeverUsable, NOON), "another peer has its own limit");
        assert!(limits.allow(PEER, RecordKind::Connection, NOON), "and so does each kind of record");
        assert!(limits.allow(PEER, RecordKind::NeverUsable, NOON + 43_200), "a new UTC day starts a new limit");
        assert!(limits.allow(PEER, RecordKind::NeverUsable, NOON - 43_201), "so does an earlier one, after a clock step back");
    }

    #[test]
    fn connection_changes_of_a_peer_with_a_usable_channel_have_a_larger_limit() {
        let mut limits = RecordLimits::default();
        for _ in 0..CONNECTION_RECORDS_PER_PEER {
            assert!(limits.allow(PEER, RecordKind::Connection, NOON));
        }
        assert!(!limits.allow(PEER, RecordKind::Connection, NOON));
    }

    #[test]
    fn never_usable_records_stop_at_the_all_peers_limit_however_many_node_ids() {
        let mut limits = RecordLimits::default();
        for n in 0..NEVER_USABLE_RECORDS_PER_DAY {
            assert!(limits.allow(&node(n), RecordKind::NeverUsable, NOON));
        }
        assert!(!limits.allow(&node(NEVER_USABLE_RECORDS_PER_DAY), RecordKind::NeverUsable, NOON), "a new node id");
        assert!(!limits.allow(&node(0), RecordKind::NeverUsable, NOON), "a node still under its own limit");
        assert!(limits.allow(&node(0), RecordKind::Connection, NOON), "connection changes of usable peers aren't part of it");
        for n in 0..10_000 {
            limits.allow(&node(NEVER_USABLE_RECORDS_PER_DAY + n), RecordKind::NeverUsable, NOON);
        }
        assert_eq!(limits.per_peer.len(), NEVER_USABLE_RECORDS_PER_DAY as usize + 1, "held-back node ids are not kept");
        assert!(limits.allow(&node(NEVER_USABLE_RECORDS_PER_DAY), RecordKind::NeverUsable, NOON + 43_200), "the next day starts again");
    }

    #[test]
    fn a_day_that_held_records_back_is_reported_once_after_it_ends() {
        let mut limits = RecordLimits::default();
        for _ in 0..NEVER_USABLE_RECORDS_PER_PEER + 3 {
            limits.allow(PEER, RecordKind::NeverUsable, NOON);
        }
        limits.allow(OTHER, RecordKind::NeverUsable, NOON);
        assert!(limits.take_reports(NOON + 3_600).is_empty(), "not before the day is over");
        let ended = limits.take_reports(NOON + 43_200);
        assert_eq!(ended, [LimitedDay { peer: Some(PEER.to_string()), kind: RecordKind::NeverUsable, day: 20_389, not_recorded: 3, day_over: true }]);
        assert!(limits.take_reports(NOON + 43_200).is_empty(), "reported once");
        assert!(limits.per_peer.is_empty() && limits.never_usable.is_empty(), "an ended day's counts are dropped, also ones that held nothing back");
    }

    #[test]
    fn reaching_the_all_peers_limit_is_reported_at_once_and_again_after_the_day_ends() {
        let mut limits = RecordLimits::default();
        for n in 0..NEVER_USABLE_RECORDS_PER_DAY + 2 {
            limits.allow(&node(n), RecordKind::NeverUsable, NOON);
        }
        let reached = LimitedDay { peer: None, kind: RecordKind::NeverUsable, day: 20_389, not_recorded: 2, day_over: false };
        assert_eq!(limits.take_reports(NOON + 30), [reached], "the next poll reports it while the day goes on");
        for n in 0..2 {
            limits.allow(&node(NEVER_USABLE_RECORDS_PER_DAY + 2 + n), RecordKind::NeverUsable, NOON + 60);
        }
        assert!(limits.take_reports(NOON + 90).is_empty(), "reported once a day");
        let ended = limits.take_reports(NOON + 43_200);
        assert_eq!(ended, [LimitedDay { peer: None, kind: RecordKind::NeverUsable, day: 20_389, not_recorded: 4, day_over: true }]);
    }

    #[test]
    fn a_day_that_ends_before_its_reached_limit_is_reported_gets_only_its_full_count() {
        let mut limits = RecordLimits::default();
        for n in 0..NEVER_USABLE_RECORDS_PER_DAY + 4 {
            limits.allow(&node(n), RecordKind::NeverUsable, NOON);
        }
        let ended = limits.take_reports(NOON + 43_200);
        assert_eq!(ended, [LimitedDay { peer: None, kind: RecordKind::NeverUsable, day: 20_389, not_recorded: 4, day_over: true }]);
    }

    #[test]
    fn the_report_names_the_scope_the_kind_the_day_and_the_count() {
        let peer = LimitedDay { peer: Some(PEER.to_string()), kind: RecordKind::Connection, day: 20_389, not_recorded: 7, day_over: true };
        assert_eq!(limited_day_record(&peer), serde_json::json!({
            "counterparty_node_id": PEER,
            "records": "connection",
            "day_started_at": 1_761_609_600,
            "limit": CONNECTION_RECORDS_PER_PEER,
            "not_recorded": 7,
            "day_over": true,
            "dedup_key": format!("lsp:peer-records-limited:{PEER}:connection:20389"),
        }));
        let all = LimitedDay { peer: None, kind: RecordKind::NeverUsable, day: 20_389, not_recorded: 9, day_over: true };
        assert_eq!(limited_day_record(&all), serde_json::json!({
            "records": "never_usable",
            "day_started_at": 1_761_609_600,
            "limit": NEVER_USABLE_RECORDS_PER_DAY,
            "not_recorded": 9,
            "day_over": true,
            "dedup_key": "lsp:peer-records-limited:all:never_usable:20389",
        }));
        let reached = LimitedDay { day_over: false, not_recorded: 1, ..all };
        assert_eq!(limited_day_record(&reached), serde_json::json!({
            "records": "never_usable",
            "day_started_at": 1_761_609_600,
            "limit": NEVER_USABLE_RECORDS_PER_DAY,
            "not_recorded": 1,
            "day_over": false,
            "dedup_key": "lsp:peer-records-limited:all:never_usable:20389:reached",
        }));
    }
}
