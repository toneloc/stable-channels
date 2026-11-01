//! Maps ldk-server channel state-change events into audit-log JSON.

use ldk_server_client::ldk_server_grpc::events::{
    channel_state_change_reason::Details, ChannelClosureInitiator, ChannelState,
    ChannelStateChangeReason, ChannelStateChangeReasonKind,
};
use serde_json::{json, Value};

/// Longest close-reason text kept in a record, in characters.
pub(crate) const MAX_REASON_CHARS: usize = 256;

/// Close-reason text cut to MAX_REASON_CHARS, noting the original length in bytes when cut.
fn bounded(text: &str) -> String {
    match text.char_indices().nth(MAX_REASON_CHARS) {
        Some((end, _)) => format!("{}… [{} bytes]", &text[..end], text.len()),
        None => text.to_string(),
    }
}

/// Strip a prost enum's screaming prefix for readability.
pub(crate) fn short(name: &str, prefix: &str) -> String {
    name.strip_prefix(prefix).unwrap_or(name).to_string()
}

/// Build the audit `data` object for a channel close/open-fail from the ldk-server event fields.
pub fn close_audit_data(
    channel_id: &str,
    user_channel_id: &str,
    counterparty_node_id: Option<&str>,
    funding_txo: Option<&str>,
    closure_initiator: i32,
    reason: Option<&ChannelStateChangeReason>,
) -> Value {
    let initiator = ChannelClosureInitiator::from_i32(closure_initiator)
        .map(|i| short(i.as_str_name(), "CHANNEL_CLOSURE_INITIATOR_"))
        .unwrap_or_else(|| format!("UNKNOWN({})", closure_initiator));

    let mut data = json!({
        "channel_id": channel_id,
        "user_channel_id": user_channel_id,
        "closure_initiator": initiator,
    });
    if let Some(cp) = counterparty_node_id {
        data["counterparty_node_id"] = json!(cp);
    }
    if let Some(txo) = funding_txo {
        data["funding_txo"] = json!(txo);
    }
    if let Some(r) = reason {
        let kind = ChannelStateChangeReasonKind::from_i32(r.kind)
            .map(|k| short(k.as_str_name(), "CHANNEL_STATE_CHANGE_REASON_KIND_"))
            .unwrap_or_else(|| format!("UNKNOWN({})", r.kind));
        data["reason_kind"] = json!(kind);
        if !r.message.is_empty() {
            data["reason_message"] = json!(bounded(&r.message));
        }
        if let Some(d) = &r.details {
            data["details"] = details_value(d);
        }
    }
    data
}

/// Build the audit `data` for an unrecognized channel-state transition (defense against enum drift).
pub fn unknown_state_audit_data(
    channel_id: &str,
    user_channel_id: &str,
    counterparty_node_id: Option<&str>,
    state: i32,
) -> Value {
    let state_name = ChannelState::from_i32(state)
        .map(|s| short(s.as_str_name(), "CHANNEL_STATE_"))
        .unwrap_or_else(|| format!("UNKNOWN({})", state));
    let mut data = json!({
        "channel_id": channel_id,
        "user_channel_id": user_channel_id,
        "state": state_name,
    });
    if let Some(cp) = counterparty_node_id {
        data["counterparty_node_id"] = json!(cp);
    }
    data
}

// Serialize the variant-specific closure details into JSON.
fn details_value(d: &Details) -> Value {
    match d {
        Details::CounterpartyForceClosed(x) => json!({ "peer_msg": bounded(&x.peer_msg) }),
        Details::HolderForceClosed(x) => json!({
            "broadcasted_latest_txn": x.broadcasted_latest_txn,
            "message": bounded(&x.message),
        }),
        Details::ProcessingError(x) => json!({ "err": bounded(&x.err) }),
        Details::HtlcsTimedOut(x) => json!({ "payment_hash": x.payment_hash }),
        Details::PeerFeerateTooLow(x) => json!({
            "peer_feerate_sat_per_kw": x.peer_feerate_sat_per_kw,
            "required_feerate_sat_per_kw": x.required_feerate_sat_per_kw,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ldk_server_client::ldk_server_grpc::events::{
        CounterpartyForceClosedDetails, HolderForceClosedDetails, ProcessingErrorDetails,
    };

    fn reason(message: &str, details: Details) -> ChannelStateChangeReason {
        ChannelStateChangeReason {
            kind: ChannelStateChangeReasonKind::CounterpartyForceClosed as i32,
            message: message.to_string(),
            details: Some(details),
        }
    }

    #[test]
    fn long_reason_text_is_cut_to_a_bounded_length() {
        let long = "é".repeat(MAX_REASON_CHARS + 1);
        let cut = format!("{}… [{} bytes]", "é".repeat(MAX_REASON_CHARS), long.len());
        let peer = Details::CounterpartyForceClosed(CounterpartyForceClosedDetails { peer_msg: long.clone() });
        let v = close_audit_data("abcd", "1842", None, None, 0, Some(&reason(&long, peer)));
        assert_eq!((&v["reason_message"], &v["details"]["peer_msg"]), (&json!(cut), &json!(cut)));
        let holder = Details::HolderForceClosed(HolderForceClosedDetails { broadcasted_latest_txn: Some(true), message: long.clone() });
        assert_eq!(close_audit_data("abcd", "1842", None, None, 0, Some(&reason("", holder)))["details"]["message"], cut);
        let error = Details::ProcessingError(ProcessingErrorDetails { err: long.clone() });
        assert_eq!(close_audit_data("abcd", "1842", None, None, 0, Some(&reason("", error)))["details"]["err"], cut);
    }

    #[test]
    fn reason_text_up_to_the_limit_is_kept_whole() {
        let whole = "é".repeat(MAX_REASON_CHARS);
        let peer = Details::CounterpartyForceClosed(CounterpartyForceClosedDetails { peer_msg: whole.clone() });
        let v = close_audit_data("abcd", "1842", None, None, 0, Some(&reason(&whole, peer)));
        assert_eq!((&v["reason_message"], &v["details"]["peer_msg"]), (&json!(whole), &json!(whole)));
    }

    #[test]
    fn force_close_maps_full_reason() {
        let reason = ChannelStateChangeReason {
            kind: ChannelStateChangeReasonKind::CounterpartyForceClosed as i32,
            message: "peer broadcast commitment".to_string(),
            details: Some(Details::CounterpartyForceClosed(CounterpartyForceClosedDetails {
                peer_msg: "commitment tx confirmed".to_string(),
            })),
        };
        let v = close_audit_data(
            "abcd",
            "1842",
            Some("02deadbeef"),
            Some("txid:0"),
            ChannelClosureInitiator::Remote as i32,
            Some(&reason),
        );
        assert_eq!(v["channel_id"], "abcd");
        assert_eq!(v["user_channel_id"], "1842");
        assert_eq!(v["counterparty_node_id"], "02deadbeef");
        assert_eq!(v["funding_txo"], "txid:0");
        assert_eq!(v["closure_initiator"], "REMOTE");
        assert_eq!(v["reason_kind"], "COUNTERPARTY_FORCE_CLOSED");
        assert_eq!(v["reason_message"], "peer broadcast commitment");
        assert_eq!(v["details"]["peer_msg"], "commitment tx confirmed");
    }

    #[test]
    fn no_reason_omits_reason_fields() {
        let v = close_audit_data("abcd", "1842", None, None, 0, None);
        assert_eq!(v["channel_id"], "abcd");
        assert_eq!(v["closure_initiator"], "UNSPECIFIED");
        assert!(v.get("reason_kind").is_none());
        assert!(v.get("counterparty_node_id").is_none());
        assert!(v.get("funding_txo").is_none());
    }

    #[test]
    fn unknown_kind_preserves_raw_value() {
        let reason = ChannelStateChangeReason {
            kind: 999,
            message: String::new(),
            details: None,
        };
        let v = close_audit_data("abcd", "1842", None, None, 0, Some(&reason));
        assert_eq!(v["reason_kind"], "UNKNOWN(999)");
    }

    #[test]
    fn unknown_state_decodes_unspecified() {
        let v = unknown_state_audit_data("5a9c", "42", Some("03aa"), 0);
        assert_eq!(v["state"], "UNSPECIFIED");
        assert_eq!(v["channel_id"], "5a9c");
        assert_eq!(v["user_channel_id"], "42");
        assert_eq!(v["counterparty_node_id"], "03aa");
    }

    #[test]
    fn unknown_state_out_of_range_falls_back() {
        let v = unknown_state_audit_data("5a9c", "42", None, 99);
        assert_eq!(v["state"], "UNKNOWN(99)");
        assert!(v.get("counterparty_node_id").is_none());
    }

    #[test]
    fn unknown_state_decodes_known_state_name() {
        // A known state (READY = 2) never reaches the else in production, but the decoder still names it.
        let v = unknown_state_audit_data("5a9c", "42", None, 2);
        assert_eq!(v["state"], "READY");
    }
}
