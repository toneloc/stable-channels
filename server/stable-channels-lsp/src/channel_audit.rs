//! Maps ldk-server channel lifecycle data (pending open, splice rounds, channel snapshot fields) into audit-log JSON.

use ldk_server_client::ldk_server_grpc::events::{SpliceNegotiated, SpliceNegotiationFailed};
use ldk_server_client::ldk_server_grpc::types::{Channel, ChannelShutdownState, ReserveType};
use serde_json::{json, Map, Value};

use crate::channel_close::short;

/// Build the audit `data` for a channel whose funding transaction was created but is not ready yet.
pub fn pending_audit_data(
    channel_id: &str,
    user_channel_id: &str,
    counterparty_node_id: Option<&str>,
    funding_txo: Option<&str>,
    former_temporary_channel_id: Option<&str>,
) -> Value {
    let mut data = json!({
        "channel_id": channel_id,
        "user_channel_id": user_channel_id,
    });
    if let Some(cp) = counterparty_node_id {
        data["counterparty_node_id"] = json!(cp);
    }
    if let Some(txo) = funding_txo {
        data["funding_txo"] = json!(txo);
    }
    if let Some(temporary) = former_temporary_channel_id {
        data["former_temporary_channel_id"] = json!(temporary);
    }
    data
}

/// Build the audit `data` for a negotiated splice; `funding_txo` matches the key CHANNEL_READY_SPLICE uses for the same outpoint.
pub fn splice_negotiated_audit_data(e: &SpliceNegotiated) -> Value {
    json!({
        "channel_id": e.channel_id,
        "user_channel_id": e.user_channel_id,
        "counterparty_node_id": e.counterparty_node_id,
        "funding_txo": e.new_funding_txo,
        "dedup_key": format!("lsp:splice-negotiated:{}:{}", e.user_channel_id, e.new_funding_txo),
    })
}

/// Build the audit `data` for a failed splice negotiation round.
pub fn splice_failed_audit_data(e: &SpliceNegotiationFailed) -> Value {
    json!({
        "channel_id": e.channel_id,
        "user_channel_id": e.user_channel_id,
        "counterparty_node_id": e.counterparty_node_id,
    })
}

/// Decode a ChannelShutdownState value, keeping the raw number for values this build does not know.
pub fn shutdown_state_name(state: i32) -> String {
    ChannelShutdownState::from_i32(state)
        .map(|s| short(s.as_str_name(), "CHANNEL_SHUTDOWN_STATE_"))
        .unwrap_or_else(|| format!("UNKNOWN({})", state))
}

/// Decode a ReserveType value, keeping the raw number for values this build does not know.
pub fn reserve_type_name(reserve: i32) -> String {
    ReserveType::from_i32(reserve)
        .map(|r| short(r.as_str_name(), "RESERVE_TYPE_"))
        .unwrap_or_else(|| format!("UNKNOWN({})", reserve))
}

/// Routing ids, HTLC bounds, reserve type and shutdown state of a live channel; absent fields are omitted.
pub fn channel_snapshot_fields(c: &Channel) -> Map<String, Value> {
    let mut fields = Map::new();
    if let Some(scid) = c.short_channel_id {
        fields.insert("short_channel_id".into(), json!(scid));
    }
    if let Some(alias) = c.outbound_scid_alias {
        fields.insert("outbound_scid_alias".into(), json!(alias));
    }
    if let Some(alias) = c.inbound_scid_alias {
        fields.insert("inbound_scid_alias".into(), json!(alias));
    }
    // An LDK Server older than a56d5a9 leaves this at the proto default 0, so 0 is treated as absent.
    if c.inbound_htlc_minimum_msat > 0 {
        fields.insert("inbound_htlc_minimum_msat".into(), json!(c.inbound_htlc_minimum_msat));
    }
    if let Some(maximum) = c.inbound_htlc_maximum_msat {
        fields.insert("inbound_htlc_maximum_msat".into(), json!(maximum));
    }
    if let Some(reserve) = c.reserve_type {
        fields.insert("reserve_type".into(), json!(reserve_type_name(reserve)));
    }
    if let Some(state) = c.channel_shutdown_state {
        fields.insert("channel_shutdown_state".into(), json!(shutdown_state_name(state)));
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    const UCID: &str = "189476124653200987495269098788434301048";
    const CHANNEL: &str = "f9634c603646c60b0df9f07c3011708652125915c80300a9bb8fb37c9c0de05b";
    const PEER: &str = "02465ed5be53d04fde66c9418ff14a5f2267723810176c9212b722e542dc1afb1b";
    const TXO: &str = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b:1";

    #[test]
    fn pending_records_funding_outpoint_and_former_temporary_id() {
        let d = pending_audit_data(CHANNEL, UCID, Some(PEER), Some(TXO), Some("7e3a"));
        assert_eq!(d["channel_id"], CHANNEL);
        assert_eq!(d["user_channel_id"], UCID);
        assert_eq!(d["counterparty_node_id"], PEER);
        assert_eq!(d["funding_txo"], TXO);
        assert_eq!(d["former_temporary_channel_id"], "7e3a");
    }

    #[test]
    fn pending_omits_fields_an_older_ldk_server_does_not_send() {
        let d = pending_audit_data(CHANNEL, UCID, None, None, None);
        assert_eq!(d["user_channel_id"], UCID);
        assert!(d.get("counterparty_node_id").is_none());
        assert!(d.get("funding_txo").is_none());
        assert!(d.get("former_temporary_channel_id").is_none());
    }

    #[test]
    fn splice_negotiated_keys_the_new_outpoint_like_the_splice_ready_row() {
        let d = splice_negotiated_audit_data(&SpliceNegotiated {
            channel_id: CHANNEL.into(),
            user_channel_id: UCID.into(),
            counterparty_node_id: PEER.into(),
            new_funding_txo: TXO.into(),
        });
        assert_eq!(d["channel_id"], CHANNEL);
        assert_eq!(d["user_channel_id"], UCID);
        assert_eq!(d["counterparty_node_id"], PEER);
        assert_eq!(d["funding_txo"], TXO);
        assert_eq!(d["dedup_key"], format!("lsp:splice-negotiated:{UCID}:{TXO}"));
    }

    #[test]
    fn splice_negotiation_failed_carries_both_ids_and_no_outpoint() {
        let d = splice_failed_audit_data(&SpliceNegotiationFailed {
            channel_id: CHANNEL.into(),
            user_channel_id: UCID.into(),
            counterparty_node_id: PEER.into(),
        });
        assert_eq!(d["channel_id"], CHANNEL);
        assert_eq!(d["user_channel_id"], UCID);
        assert_eq!(d["counterparty_node_id"], PEER);
        assert!(d.get("funding_txo").is_none());
        assert!(d.get("dedup_key").is_none(), "failed rounds carry no unique id to deduplicate on");
    }

    #[test]
    fn snapshot_fields_carry_routing_ids_htlc_bounds_and_decoded_states() {
        let channel = Channel {
            short_channel_id: Some(934_190_049_236_975_617),
            outbound_scid_alias: Some(17_592_186_044_417),
            inbound_scid_alias: Some(17_592_186_044_418),
            inbound_htlc_minimum_msat: 1_000,
            inbound_htlc_maximum_msat: Some(99_000_000),
            channel_shutdown_state: Some(ChannelShutdownState::ResolvingHtlcs as i32),
            reserve_type: Some(ReserveType::TrustedPeersNoReserve as i32),
            ..Default::default()
        };
        let f = channel_snapshot_fields(&channel);
        assert_eq!(f["short_channel_id"], 934_190_049_236_975_617u64);
        assert_eq!(f["outbound_scid_alias"], 17_592_186_044_417u64);
        assert_eq!(f["inbound_scid_alias"], 17_592_186_044_418u64);
        assert_eq!(f["inbound_htlc_minimum_msat"], 1_000u64);
        assert_eq!(f["inbound_htlc_maximum_msat"], 99_000_000u64);
        assert_eq!(f["channel_shutdown_state"], "RESOLVING_HTLCS");
        assert_eq!(f["reserve_type"], "TRUSTED_PEERS_NO_RESERVE");
    }

    #[test]
    fn snapshot_fields_are_empty_for_an_older_ldk_server() {
        assert!(channel_snapshot_fields(&Channel::default()).is_empty());
    }

    #[test]
    fn unknown_enum_values_keep_the_raw_number() {
        assert_eq!(shutdown_state_name(42), "UNKNOWN(42)");
        assert_eq!(reserve_type_name(42), "UNKNOWN(42)");
        assert_eq!(shutdown_state_name(ChannelShutdownState::NotShuttingDown as i32), "NOT_SHUTTING_DOWN");
    }
}
