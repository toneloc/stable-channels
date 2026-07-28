//! Reconcile-from-truth: on (re)connect, backfill audit records for forwards missed during the gap.

use ldk_server_client::ldk_server_grpc::api::{
    GetBalancesRequest, GetPaymentDetailsRequest, ListChannelsRequest,
    ListForwardedPaymentsRequest, ListPaymentsRequest, ListPeersRequest,
};
use ldk_server_client::ldk_server_grpc::types::{
    pending_sweep_balance, PageToken, PaymentStatus, PendingSweepBalance,
};
use stable_channels::db::{forward_fingerprint, Database};
use stable_channels::ledger::LedgerEventDraft;

use crate::stable_manager::LdkServerCalls;

/// LDK Server pages payments newest-created first, but currently exposes no modified-since query.
/// Keep reconnect work bounded; reaching this limit is recorded as a ledger gap, never as complete.
const MAX_RECONSTRUCTION_PAGES: usize = 10;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ForwardBackfillResult {
    pub emitted: usize,
    pub failure: Option<String>,
    pub incomplete: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct ReconstructedCounts {
    pub channels: usize,
    pub payments: usize,
    pub forwards: usize,
    pub peers: usize,
    pub sweeps: usize,
    pub failed_scopes: usize,
    pub incomplete_scopes: usize,
    pub settlement_outcomes_safe: bool,
}

/// Snapshot all queryable LDK state after a stream reconnect. Payment rows are
/// deliberately payment-keyed: LDK does not expose a reliable channel
/// association for MPP/general payment history.
pub async fn reconcile_event_history(
    ldk: &dyn LdkServerCalls,
    db: &Database,
) -> ReconstructedCounts {
    let mut counts = ReconstructedCounts::default();

    counts.settlement_outcomes_safe = match reconcile_pending_settlement_outcomes(ldk, db).await {
        Ok(()) => true,
        Err(error) => {
            scope_failed("settlement_outcomes", &error, &mut counts);
            false
        },
    };

    match ldk.list_channels(ListChannelsRequest {}).await {
        Ok(response) => {
            for channel in response.channels {
                let identity = if channel.user_channel_id.is_empty() {
                    channel.channel_id.clone()
                } else {
                    channel.user_channel_id.clone()
                };
                let detail = serde_json::json!({
                    "source": "reconnect_reconciliation",
                    "channel_id": channel.channel_id,
                    "user_channel_id": channel.user_channel_id,
                    "counterparty_node_id": channel.counterparty_node_id,
                    "funding_txo": channel.funding_txo.as_ref().map(|outpoint| outpoint.txid.clone()),
                    "channel_value_sats": channel.channel_value_sats,
                    "outbound_capacity_msat": channel.outbound_capacity_msat,
                    "inbound_capacity_msat": channel.inbound_capacity_msat,
                    "is_channel_ready": channel.is_channel_ready,
                    "is_usable": channel.is_usable,
                });
                match append_reconstructed_if_changed(
                    db,
                    "channel",
                    &identity,
                    "CHANNEL_RECONSTRUCTED",
                    detail,
                ) {
                    Ok(true) => counts.channels += 1,
                    Ok(false) => {},
                    Err(error) => {
                        scope_failed("channels", &error.to_string(), &mut counts);
                        break;
                    },
                }
            }
        },
        Err(error) => scope_failed("channels", &error.to_string(), &mut counts),
    }

    let mut page_token = None;
    let mut payment_pages = 0usize;
    let mut seen_payment_cursors = std::collections::HashSet::new();
    loop {
        match ldk.list_payments(ListPaymentsRequest { page_token }).await {
            Ok(response) => {
                payment_pages += 1;
                for payment in response.payments {
                    let (status, ldk_status) = payment_status(payment.status);
                    stable_channels::audit::audit_event(
                        "PAYMENT_RECONSTRUCTED",
                        serde_json::json!({
                            "source": "reconnect_reconciliation",
                            "dedup_key": format!("lsp:reconstructed-payment:{}:{}:{}", payment.id, payment.status, payment.latest_update_timestamp),
                            "payment_id": payment.id,
                            "amount_msat": payment.amount_msat,
                            "fee_paid_msat": payment.fee_paid_msat,
                            "direction": payment.direction,
                            "status": status,
                            "ldk_status": ldk_status,
                            "latest_update_timestamp": payment.latest_update_timestamp,
                            "channel_association": "unavailable_from_ldk",
                        }),
                    );
                    counts.payments += 1;
                }
                match response.next_page_token {
                    Some(_token) if payment_pages >= MAX_RECONSTRUCTION_PAGES => {
                        scope_incomplete(
                            "payments",
                            &format!(
                                "reconstruction stopped after {MAX_RECONSTRUCTION_PAGES} pages because LDK Server does not expose a modified-since cursor"
                            ),
                            &mut counts,
                        );
                        break;
                    },
                    Some(token) => {
                        let cursor = page_cursor_key(&token);
                        if !seen_payment_cursors.insert(cursor) {
                            scope_incomplete(
                                "payments",
                                "LDK Server repeated a payment page cursor",
                                &mut counts,
                            );
                            break;
                        }
                        page_token = Some(token);
                    },
                    None => break,
                }
            },
            Err(error) => {
                scope_failed("payments", &error.to_string(), &mut counts);
                break;
            },
        }
    }

    let forwards = backfill_forwards(ldk, db).await;
    counts.forwards = forwards.emitted;
    if let Some(error) = forwards.failure {
        scope_failed("forwards", &error, &mut counts);
    }
    if let Some(reason) = forwards.incomplete {
        scope_incomplete("forwards", &reason, &mut counts);
    }

    match ldk.list_peers(ListPeersRequest {}).await {
        Ok(response) => {
            for peer in response.peers {
                let identity = peer.node_id.clone();
                let detail = serde_json::json!({
                    "source": "reconnect_reconciliation",
                    "node_id": peer.node_id,
                    "address": peer.address,
                    "is_connected": peer.is_connected,
                });
                match append_reconstructed_if_changed(
                    db,
                    "peer",
                    &identity,
                    "PEER_RECONSTRUCTED",
                    detail,
                ) {
                    Ok(true) => counts.peers += 1,
                    Ok(false) => {},
                    Err(error) => {
                        scope_failed("peers", &error.to_string(), &mut counts);
                        break;
                    },
                }
            }
        },
        Err(error) => scope_failed("peers", &error.to_string(), &mut counts),
    }

    match ldk.get_balances(GetBalancesRequest {}).await {
        Ok(response) => {
            let mut identity_occurrences = std::collections::HashMap::new();
            for sweep in response.pending_balances_from_channel_closures {
                let base_identity = sweep_identity(&sweep);
                let occurrence = identity_occurrences
                    .entry(base_identity.clone())
                    .or_insert(0usize);
                let identity = format!("{base_identity}#{occurrence}");
                *occurrence += 1;
                let raw = serde_json::to_value(&sweep)
                    .unwrap_or_else(|_| serde_json::json!({"debug": format!("{sweep:?}")}));
                let detail = serde_json::json!({
                    "source": "reconnect_reconciliation",
                    "sweep_identity": identity,
                    "sweep": raw,
                });
                match append_reconstructed_if_changed(
                    db,
                    "sweep",
                    &identity,
                    "SWEEP_RECONSTRUCTED",
                    detail,
                ) {
                    Ok(true) => counts.sweeps += 1,
                    Ok(false) => {},
                    Err(error) => {
                        scope_failed("sweeps", &error.to_string(), &mut counts);
                        break;
                    },
                }
            }
        },
        Err(error) => scope_failed("sweeps", &error.to_string(), &mut counts),
    }

    counts
}

/// Retry terminal outcomes for protocol payments from LDK's durable payment store before live
/// dispatch resumes. This closes the fire-and-forget stream gap when SQLite rejected the original
/// PaymentSuccessful write: a successful payment cannot remain reversible in our database.
async fn reconcile_pending_settlement_outcomes(
    ldk: &dyn LdkServerCalls,
    db: &Database,
) -> Result<(), String> {
    let pending = db
        .list_pending_settlements()
        .map_err(|error| format!("failed to list pending settlements: {error}"))?;
    for (payment_id, _) in pending {
        let response = ldk
            .get_payment_details(GetPaymentDetailsRequest {
                payment_id: payment_id.clone(),
            })
            .await
            .map_err(|error| {
                format!("failed to query pending settlement {payment_id}: {error}")
            })?;
        let Some(payment) = response.payment else {
            continue;
        };
        if payment.status != PaymentStatus::Succeeded as i32 {
            continue;
        }
        let direction = if payment.direction == 1 {
            "outbound"
        } else {
            "inbound"
        };
        db.mark_settlement_succeeded(
            &payment_id,
            payment.amount_msat,
            payment.fee_paid_msat,
            Some(direction),
        )
        .map_err(|error| {
            format!("failed to persist successful settlement {payment_id}: {error}")
        })?;
    }
    Ok(())
}

fn append_reconstructed_if_changed(
    db: &Database,
    scope: &str,
    identity: &str,
    event_type: &str,
    detail: serde_json::Value,
) -> rusqlite::Result<bool> {
    let fingerprint = serde_json::to_string(&detail)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    let draft = LedgerEventDraft::from_audit_event(event_type, detail);
    db.append_reconstructed_event_if_changed(scope, identity, &fingerprint, &draft)
}

fn page_cursor_key(token: &PageToken) -> String {
    format!("{}:{}", token.token, token.index)
}

/// LDK Server does not expose the swept output's outpoint in every state. Once a spending
/// transaction exists its txid is stable across confirmation states; before that, channel and
/// amount are the strongest available identity. The occurrence suffix handles indistinguishable
/// same-channel/same-amount pending outputs without coupling unrelated sweeps to list positions.
fn sweep_identity(sweep: &PendingSweepBalance) -> String {
    match sweep.balance_type.as_ref() {
        Some(pending_sweep_balance::BalanceType::PendingBroadcast(balance)) => format!(
            "pending:{}:{}",
            balance.channel_id.as_deref().unwrap_or("unknown-channel"),
            balance.amount_satoshis,
        ),
        Some(pending_sweep_balance::BalanceType::BroadcastAwaitingConfirmation(balance)) => {
            if balance.latest_spending_txid.is_empty() {
                format!(
                    "broadcast:{}:{}",
                    balance.channel_id.as_deref().unwrap_or("unknown-channel"),
                    balance.amount_satoshis,
                )
            } else {
                format!("tx:{}", balance.latest_spending_txid)
            }
        },
        Some(pending_sweep_balance::BalanceType::AwaitingThresholdConfirmations(balance)) => {
            if balance.latest_spending_txid.is_empty() {
                format!(
                    "confirmed:{}:{}",
                    balance.channel_id.as_deref().unwrap_or("unknown-channel"),
                    balance.amount_satoshis,
                )
            } else {
                format!("tx:{}", balance.latest_spending_txid)
            }
        },
        None => "unknown-sweep".to_owned(),
    }
}

fn payment_status(value: i32) -> (&'static str, &'static str) {
    match value {
        value if value == PaymentStatus::Pending as i32 => ("pending", "PENDING"),
        value if value == PaymentStatus::Succeeded as i32 => ("completed", "SUCCEEDED"),
        value if value == PaymentStatus::Failed as i32 => ("failed", "FAILED"),
        _ => ("observed", "UNKNOWN"),
    }
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

fn scope_incomplete(scope: &str, reason: &str, counts: &mut ReconstructedCounts) {
    counts.incomplete_scopes += 1;
    stable_channels::audit::audit_event(
        "RECONCILIATION_GAP_DETECTED",
        serde_json::json!({
            "source": "reconnect_reconciliation",
            "scope": scope,
            "reason": reason,
            "status": "partial",
        }),
    );
}

/// Page ListForwardedPayments and emit PAYMENT_FORWARDED_BACKFILL for each unseen forward.
/// Audit-only: does NOT touch the peg. Pagination failures and safety stops are returned so the
/// enclosing reconciliation is marked partial instead of silently claiming complete coverage.
pub async fn backfill_forwards(
    ldk: &dyn LdkServerCalls,
    db: &Database,
) -> ForwardBackfillResult {
    let mut result = ForwardBackfillResult::default();
    let mut page_token = None;
    let mut pages = 0usize;
    let mut seen_cursors = std::collections::HashSet::new();
    'pages: loop {
        let resp = match ldk
            .list_forwarded_payments(ListForwardedPaymentsRequest { page_token })
            .await
        {
            Ok(r) => r,
            Err(e) => {
                result.failure = Some(e.to_string());
                break;
            }
        };
        pages += 1;
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
            let detail = serde_json::json!({
                "prev_channel_id": prev_channel_id,
                "next_channel_id": next_channel_id,
                "prev_user_channel_id": prev.and_then(|h| h.user_channel_id.clone()),
                "next_user_channel_id": next.and_then(|h| h.user_channel_id.clone()),
                "prev_node_id": prev.and_then(|h| h.node_id.clone()),
                "next_node_id": next.and_then(|h| h.node_id.clone()),
                "outbound_amount_msat": fp.outbound_amount_forwarded_msat,
                "total_fee_msat": fp.total_fee_earned_msat,
            });
            let draft = LedgerEventDraft::from_audit_event("PAYMENT_FORWARDED_BACKFILL", detail);
            match db.append_forwarded_event_if_unseen(&key, &draft) {
                Ok(true) => result.emitted += 1,
                Ok(false) => {},
                Err(error) => {
                    result.failure = Some(format!(
                        "failed to persist forwarded-payment reconstruction: {error}"
                    ));
                    break 'pages;
                },
            }
        }
        match resp.next_page_token {
            Some(_) if pages >= MAX_RECONSTRUCTION_PAGES => {
                result.incomplete = Some(format!(
                    "reconstruction stopped after {MAX_RECONSTRUCTION_PAGES} pages"
                ));
                break;
            },
            Some(token) => {
                let cursor = page_cursor_key(&token);
                if !seen_cursors.insert(cursor) {
                    result.incomplete =
                        Some("LDK Server repeated a forwarded-payment page cursor".to_owned());
                    break;
                }
                page_token = Some(token);
            },
            None => break,
        }
    }
    result
}
