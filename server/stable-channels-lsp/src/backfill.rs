//! Reconcile-from-truth: on (re)connect, backfill audit records for forwards missed during the gap.

use ldk_server_client::ldk_server_grpc::api::{
    GetBalancesRequest, ListChannelsRequest, ListForwardedPaymentsRequest, ListPaymentsRequest,
    ListPeersRequest,
};
use stable_channels::db::{forward_fingerprint, Database};

use crate::stable_manager::LdkServerCalls;

#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct ReconstructedCounts {
    pub channels: usize,
    pub payments: usize,
    pub forwards: usize,
    pub peers: usize,
    pub sweeps: usize,
    pub failed_scopes: usize,
}

/// Snapshot all queryable LDK state after a stream reconnect. Payment rows are
/// deliberately payment-keyed: LDK does not expose a reliable channel
/// association for MPP/general payment history.
pub async fn reconcile_event_history(
    ldk: &dyn LdkServerCalls,
    db: &Database,
) -> ReconstructedCounts {
    let mut counts = ReconstructedCounts::default();

    match ldk.list_channels(ListChannelsRequest {}).await {
        Ok(response) => {
            for channel in response.channels {
                stable_channels::audit::audit_event(
                    "CHANNEL_RECONSTRUCTED",
                    serde_json::json!({
                        "source": "reconnect_reconciliation",
                        "dedup_key": format!("reconstructed-channel:{}:{}:{}:{}", channel.channel_id, channel.user_channel_id, channel.outbound_capacity_msat, channel.inbound_capacity_msat),
                        "channel_id": channel.channel_id,
                        "user_channel_id": channel.user_channel_id,
                        "counterparty_node_id": channel.counterparty_node_id,
                        "funding_txo": channel.funding_txo.as_ref().map(|outpoint| outpoint.txid.clone()),
                        "channel_value_sats": channel.channel_value_sats,
                        "outbound_capacity_msat": channel.outbound_capacity_msat,
                        "inbound_capacity_msat": channel.inbound_capacity_msat,
                        "is_channel_ready": channel.is_channel_ready,
                        "is_usable": channel.is_usable,
                    }),
                );
                counts.channels += 1;
            }
        },
        Err(error) => scope_failed("channels", &error.to_string(), &mut counts),
    }

    let mut page_token = None;
    loop {
        match ldk.list_payments(ListPaymentsRequest { page_token }).await {
            Ok(response) => {
                for payment in response.payments {
                    stable_channels::audit::audit_event(
                        "PAYMENT_RECONSTRUCTED",
                        serde_json::json!({
                            "source": "reconnect_reconciliation",
                            "dedup_key": format!("reconstructed-payment:{}:{}:{}", payment.id, payment.status, payment.latest_update_timestamp),
                            "payment_id": payment.id,
                            "amount_msat": payment.amount_msat,
                            "fee_paid_msat": payment.fee_paid_msat,
                            "direction": payment.direction,
                            "status": payment.status,
                            "latest_update_timestamp": payment.latest_update_timestamp,
                            "channel_association": "unavailable_from_ldk",
                        }),
                    );
                    counts.payments += 1;
                }
                match response.next_page_token {
                    Some(token) => page_token = Some(token),
                    None => break,
                }
            },
            Err(error) => {
                scope_failed("payments", &error.to_string(), &mut counts);
                break;
            },
        }
    }

    counts.forwards = backfill_forwards(ldk, db).await;

    match ldk.list_peers(ListPeersRequest {}).await {
        Ok(response) => {
            for peer in response.peers {
                stable_channels::audit::audit_event(
                    "PEER_RECONSTRUCTED",
                    serde_json::json!({
                        "source": "reconnect_reconciliation",
                        "dedup_key": format!("reconstructed-peer:{}:{}:{}", peer.node_id, peer.address, peer.is_connected),
                        "node_id": peer.node_id,
                        "address": peer.address,
                        "is_connected": peer.is_connected,
                    }),
                );
                counts.peers += 1;
            }
        },
        Err(error) => scope_failed("peers", &error.to_string(), &mut counts),
    }

    match ldk.get_balances(GetBalancesRequest {}).await {
        Ok(response) => {
            for (index, sweep) in response.pending_balances_from_channel_closures.into_iter().enumerate() {
                let raw = serde_json::to_value(&sweep)
                    .unwrap_or_else(|_| serde_json::json!({"debug": format!("{sweep:?}")}));
                stable_channels::audit::audit_event(
                    "SWEEP_RECONSTRUCTED",
                    serde_json::json!({
                        "source": "reconnect_reconciliation",
                        "dedup_key": format!("reconstructed-sweep:{}:{}", index, raw),
                        "sweep_index": index,
                        "sweep": raw,
                    }),
                );
                counts.sweeps += 1;
            }
        },
        Err(error) => scope_failed("sweeps", &error.to_string(), &mut counts),
    }

    counts
}

fn scope_failed(scope: &str, error: &str, counts: &mut ReconstructedCounts) {
    counts.failed_scopes += 1;
    stable_channels::audit::audit_event(
        "RECONCILIATION_SCOPE_FAILED",
        serde_json::json!({
            "source": "reconnect_reconciliation",
            "scope": scope,
            "error": error,
        }),
    );
}

/// Page ListForwardedPayments and emit PAYMENT_FORWARDED_BACKFILL for each forward not already seen.
/// Audit-only: does NOT touch the peg. Returns the number of backfill events emitted.
pub async fn backfill_forwards(ldk: &dyn LdkServerCalls, db: &Database) -> usize {
    let mut emitted = 0usize;
    let mut page_token = None;
    loop {
        let resp = match ldk
            .list_forwarded_payments(ListForwardedPaymentsRequest { page_token })
            .await
        {
            Ok(r) => r,
            Err(e) => {
                stable_channels::audit::audit_event(
                    "LDK_CALL_FAILED",
                    serde_json::json!({ "op": "list_forwarded_payments", "context": "backfill", "error": e.to_string() }),
                );
                break;
            }
        };
        for fp in &resp.forwarded_payments {
            // ForwardedPayment now carries per-HTLC locators; take the first of each list as the representative channel/node.
            let prev = fp.prev_htlcs.first();
            let next = fp.next_htlcs.first();
            let prev_channel_id = prev.map(|h| h.channel_id.clone()).unwrap_or_default();
            let next_channel_id = next.map(|h| h.channel_id.clone()).unwrap_or_default();
            let key = forward_fingerprint(
                &prev_channel_id,
                &next_channel_id,
                fp.outbound_amount_forwarded_msat,
                fp.total_fee_earned_msat,
            );
            let is_new = db.record_forwarded_seen(&key).unwrap_or(false);
            if is_new {
                stable_channels::audit::audit_event(
                    "PAYMENT_FORWARDED_BACKFILL",
                    serde_json::json!({
                        "prev_channel_id": prev_channel_id,
                        "next_channel_id": next_channel_id,
                        "prev_user_channel_id": prev.and_then(|h| h.user_channel_id.clone()),
                        "next_user_channel_id": next.and_then(|h| h.user_channel_id.clone()),
                        "prev_node_id": prev.and_then(|h| h.node_id.clone()),
                        "next_node_id": next.and_then(|h| h.node_id.clone()),
                        "outbound_amount_msat": fp.outbound_amount_forwarded_msat,
                        "total_fee_msat": fp.total_fee_earned_msat,
                    }),
                );
                emitted += 1;
            }
        }
        match resp.next_page_token {
            Some(t) => page_token = Some(t),
            None => break,
        }
    }
    emitted
}
