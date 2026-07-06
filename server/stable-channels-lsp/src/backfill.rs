//! Reconcile-from-truth: on (re)connect, backfill audit records for forwards missed during the gap.

use ldk_server_client::ldk_server_grpc::api::ListForwardedPaymentsRequest;
use stable_channels::db::{forward_fingerprint, Database};

use crate::stable_manager::LdkServerCalls;

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
            let key = forward_fingerprint(
                &fp.prev_channel_id,
                &fp.next_channel_id,
                fp.outbound_amount_forwarded_msat,
                fp.total_fee_earned_msat,
            );
            let is_new = db.record_forwarded_seen(&key).unwrap_or(false);
            if is_new {
                stable_channels::audit::audit_event(
                    "PAYMENT_FORWARDED_BACKFILL",
                    serde_json::json!({
                        "prev_channel_id": fp.prev_channel_id,
                        "next_channel_id": fp.next_channel_id,
                        "prev_user_channel_id": fp.prev_user_channel_id,
                        "next_user_channel_id": fp.next_user_channel_id,
                        "prev_node_id": fp.prev_node_id,
                        "next_node_id": fp.next_node_id,
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
