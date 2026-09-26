//! Maps ldk-server channel lifecycle data (pending open, splice rounds, channel snapshot fields) into audit-log JSON.

use ldk_server_client::ldk_server_grpc::events::{SpliceNegotiated, SpliceNegotiationFailed};
use ldk_server_client::ldk_server_grpc::types::{
    confirmation_status, payment_kind, transaction_type, Channel, ChannelShutdownState, Payment,
    PaymentDirection, PaymentStatus, ReserveType,
};
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

/// Ledger `data` for an on-chain transaction LDK classified against channels (funding, close, claim, sweep); None for anything else.
pub fn onchain_channel_tx_audit_data(
    payment: &Payment,
    user_channel_id_for: &dyn Fn(&str) -> Option<String>,
) -> Option<Value> {
    let Some(payment_kind::Kind::Onchain(onchain)) = payment.kind.as_ref().and_then(|k| k.kind.as_ref())
    else {
        return None;
    };
    let pairs = |channels: &[ldk_server_client::ldk_server_grpc::types::TransactionChannel]| {
        channels.iter().map(|c| (c.channel_id.clone(), c.counterparty_node_id.clone())).collect::<Vec<_>>()
    };
    let (tx_type, channels) = match onchain.tx_type.as_ref()?.kind.as_ref()? {
        transaction_type::Kind::Funding(x) => ("FUNDING", pairs(&x.channels)),
        transaction_type::Kind::InteractiveFunding(x) => ("INTERACTIVE_FUNDING", pairs(&x.channels)),
        transaction_type::Kind::Sweep(x) => ("SWEEP", pairs(&x.channels)),
        transaction_type::Kind::CooperativeClose(x) => ("COOPERATIVE_CLOSE", vec![(x.channel_id.clone(), x.counterparty_node_id.clone())]),
        transaction_type::Kind::UnilateralClose(x) => ("UNILATERAL_CLOSE", vec![(x.channel_id.clone(), x.counterparty_node_id.clone())]),
        transaction_type::Kind::AnchorBump(x) => ("ANCHOR_BUMP", vec![(x.channel_id.clone(), x.counterparty_node_id.clone())]),
        transaction_type::Kind::Claim(x) => ("CLAIM", vec![(x.channel_id.clone(), x.counterparty_node_id.clone())]),
    };
    let mut channel_ids: Vec<String> = Vec::new();
    let mut node_ids: Vec<String> = Vec::new();
    for (channel_id, node_id) in channels {
        if !channel_id.is_empty() && !channel_ids.contains(&channel_id) {
            channel_ids.push(channel_id);
        }
        if !node_id.is_empty() && !node_ids.contains(&node_id) {
            node_ids.push(node_id);
        }
    }
    if channel_ids.is_empty() {
        return None;
    }
    let mut user_channel_ids: Vec<String> = Vec::new();
    for uid in channel_ids.iter().filter_map(|id| user_channel_id_for(id)) {
        if !user_channel_ids.contains(&uid) {
            user_channel_ids.push(uid);
        }
    }
    let (confirmed_height, block_time) = match onchain.status.as_ref().and_then(|s| s.status.as_ref()) {
        Some(confirmation_status::Status::Confirmed(c)) => (Some(c.height), c.timestamp),
        _ => (None, 0),
    };
    let confirmation = if confirmed_height.is_some() { "confirmed" } else { "unconfirmed" };
    let (state, status) = if payment.status == PaymentStatus::Failed as i32 {
        ("failed", "failed")
    } else if confirmed_height.is_some() {
        ("confirmed", "completed")
    } else {
        ("unconfirmed", "pending")
    };
    let mut data = json!({
        "payment_id": payment.payment_id,
        "txid": onchain.txid,
        "tx_type": tx_type,
        "confirmation": confirmation,
        "channel_ids": channel_ids,
        "user_channel_ids": user_channel_ids,
        "node_ids": node_ids,
        "amount_msat": payment.amount_msat,
        "fee_paid_msat": payment.fee_paid_msat,
        "direction": if payment.direction == PaymentDirection::Outbound as i32 { "outbound" } else { "inbound" },
        "status": status,
        "dedup_key": format!("lsp:channel-onchain-tx:{}:{}", onchain.txid, state),
    });
    if let Some(height) = confirmed_height {
        data["confirmation_height"] = json!(height);
    }
    // Date the row when it happened on-chain, not when the LSP noticed it (a first sync can surface months-old transactions).
    let happened_secs = if confirmed_height.is_some() && block_time > 0 { block_time } else { payment.latest_update_timestamp };
    if happened_secs > 0 {
        data["occurred_at_ms"] = json!(happened_secs.saturating_mul(1000) as i64);
    }
    Some(data)
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

    mod onchain {
        use super::*;
        use ldk_server_client::ldk_server_grpc::types::{
            confirmation_status, payment_kind, transaction_type, ConfirmationStatus, Confirmed,
            CooperativeClose, Funding, Onchain, Payment, PaymentDirection, PaymentKind,
            PaymentStatus, Sweep, TransactionChannel, TransactionType, Unconfirmed,
        };

        const TXID: &str = "b6f6991d03df0e2e04dafffcd6bc418aac66049e2cd74b80f14ac86db1e3f0da";

        fn onchain(tx_type: Option<transaction_type::Kind>, confirmed_at: Option<u32>, status: PaymentStatus) -> Payment {
            let confirmation = match confirmed_at {
                Some(height) => confirmation_status::Status::Confirmed(Confirmed { block_hash: "00ab".into(), height, timestamp: 1_758_000_000 }),
                None => confirmation_status::Status::Unconfirmed(Unconfirmed {}),
            };
            Payment {
                payment_id: "b6f6991d".into(),
                kind: Some(PaymentKind {
                    kind: Some(payment_kind::Kind::Onchain(Onchain {
                        txid: TXID.into(),
                        status: Some(ConfirmationStatus { status: Some(confirmation) }),
                        tx_type: tx_type.map(|kind| TransactionType { kind: Some(kind) }),
                    })),
                }),
                amount_msat: Some(250_000_000),
                fee_paid_msat: Some(1_410_000),
                direction: PaymentDirection::Outbound as i32,
                status: status as i32,
                ..Default::default()
            }
        }

        fn known(channel_id: &str) -> Option<String> {
            (channel_id == CHANNEL).then(|| UCID.to_owned())
        }

        fn funding() -> transaction_type::Kind {
            transaction_type::Kind::Funding(Funding {
                channels: vec![TransactionChannel { counterparty_node_id: PEER.into(), channel_id: CHANNEL.into() }],
            })
        }

        #[test]
        fn confirmed_funding_links_the_transaction_to_its_channel() {
            let d = onchain_channel_tx_audit_data(&onchain(Some(funding()), Some(861_204), PaymentStatus::Succeeded), &known).unwrap();
            assert_eq!(d["txid"], TXID);
            assert_eq!(d["tx_type"], "FUNDING");
            assert_eq!(d["confirmation"], "confirmed");
            assert_eq!(d["confirmation_height"], 861_204);
            assert_eq!(d["channel_ids"], serde_json::json!([CHANNEL]));
            assert_eq!(d["user_channel_ids"], serde_json::json!([UCID]));
            assert_eq!(d["node_ids"], serde_json::json!([PEER]));
            assert_eq!(d["amount_msat"], 250_000_000u64);
            assert_eq!(d["fee_paid_msat"], 1_410_000u64);
            assert_eq!(d["direction"], "outbound");
            assert_eq!(d["status"], "completed");
            assert_eq!(d["dedup_key"], format!("lsp:channel-onchain-tx:{TXID}:confirmed"));
            assert_eq!(d["occurred_at_ms"], 1_758_000_000_000i64, "a confirmed row is dated at its block time");
        }

        #[test]
        fn an_unconfirmed_row_is_dated_at_ldks_last_update_or_left_to_now() {
            let mut payment = onchain(Some(funding()), None, PaymentStatus::Pending);
            payment.latest_update_timestamp = 1_757_990_000;
            let d = onchain_channel_tx_audit_data(&payment, &known).unwrap();
            assert_eq!(d["occurred_at_ms"], 1_757_990_000_000i64);
            payment.latest_update_timestamp = 0;
            assert!(onchain_channel_tx_audit_data(&payment, &known).unwrap().get("occurred_at_ms").is_none());
        }

        #[test]
        fn an_unconfirmed_close_is_pending_and_keyed_separately_from_its_confirmation() {
            let close = transaction_type::Kind::CooperativeClose(CooperativeClose { counterparty_node_id: PEER.into(), channel_id: CHANNEL.into() });
            let d = onchain_channel_tx_audit_data(&onchain(Some(close), None, PaymentStatus::Pending), &known).unwrap();
            assert_eq!(d["tx_type"], "COOPERATIVE_CLOSE");
            assert_eq!(d["confirmation"], "unconfirmed");
            assert!(d.get("confirmation_height").is_none());
            assert_eq!(d["status"], "pending");
            assert_eq!(d["dedup_key"], format!("lsp:channel-onchain-tx:{TXID}:unconfirmed"));
        }

        #[test]
        fn a_failed_transaction_is_recorded_as_failed() {
            let d = onchain_channel_tx_audit_data(&onchain(Some(funding()), None, PaymentStatus::Failed), &known).unwrap();
            assert_eq!(d["status"], "failed");
            assert_eq!(d["dedup_key"], format!("lsp:channel-onchain-tx:{TXID}:failed"));
        }

        #[test]
        fn a_channel_the_lsp_cannot_resolve_keeps_its_channel_id_only() {
            let d = onchain_channel_tx_audit_data(&onchain(Some(funding()), None, PaymentStatus::Pending), &|_| None).unwrap();
            assert_eq!(d["channel_ids"], serde_json::json!([CHANNEL]));
            assert_eq!(d["user_channel_ids"], serde_json::json!([]));
        }

        #[test]
        fn transactions_without_a_channel_produce_no_row() {
            let unclassified = onchain(None, Some(861_204), PaymentStatus::Succeeded);
            assert!(onchain_channel_tx_audit_data(&unclassified, &known).is_none(), "plain on-chain sends are not channel activity");
            let anonymous_sweep = onchain(Some(transaction_type::Kind::Sweep(Sweep { channels: vec![] })), None, PaymentStatus::Pending);
            assert!(onchain_channel_tx_audit_data(&anonymous_sweep, &known).is_none());
            let lightning = Payment { payment_id: "ln".into(), ..Default::default() };
            assert!(onchain_channel_tx_audit_data(&lightning, &known).is_none());
        }
    }
}
