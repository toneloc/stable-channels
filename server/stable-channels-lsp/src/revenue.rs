//! Operator revenue: what the LSP earned, spent and settled, rebuilt off the request path from payments, labels and the ledger.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use ldk_server_client::error::LdkServerErrorCode;
use ldk_server_client::ldk_server_grpc::api::{
    GetPaymentDetailsRequest, ListChannelsRequest, ListPaymentsRequest, SpontaneousSendRequest,
};
use ldk_server_client::ldk_server_grpc::types::{
    payment_kind, transaction_type, Channel, Payment, PaymentDirection, PaymentStatus,
};
use sc_protos::revenue::{RevenueItem, RevenueLine};
use stable_channels::db::{Database, RevenueLedgerRow, SettlementLabel, TradeDecisionSummary, TradeFeeRefund};
use tracing::warn;

use crate::stable_manager::LdkServerCalls;
use crate::payment_filter::is_failed_protocol_message;
use crate::state::AppState;

pub const TRADE_FEE: &str = "trade_fee";
pub const TRADE_FEE_REJECTED: &str = "trade_fee_rejected";
pub const ROUTING_FEE: &str = "routing_fee";
pub const JIT_FEE: &str = "jit_fee";
pub const PROTOCOL_MESSAGE: &str = "protocol_message";
pub const TRADE_FEE_REFUND: &str = "trade_fee_refund";
pub const CHANNEL_FUNDING_FEE: &str = "channel_funding_fee";
/// On-chain fee the LSP paid to open a private channel for a user (JIT opens are free for them).
pub const JIT_OPEN_FEE: &str = "jit_open_fee";
pub const ONCHAIN_FEE: &str = "onchain_fee";
/// On-chain fee of a cooperative or force close transaction.
pub const CLOSE_FEE: &str = "close_fee";
/// On-chain fee of an anchor bump that sped up a force close.
pub const CLOSE_FEE_BUMP: &str = "close_fee_bump";
/// On-chain fee of a claim or sweep bringing closed-channel funds back to the wallet.
pub const CLAIM_SWEEP_FEE: &str = "claim_sweep_fee";
pub const LIGHTNING_SEND_FEE: &str = "lightning_send_fee";
pub const STABILITY_IN: &str = "stability_in";
pub const STABILITY_OUT: &str = "stability_out";

pub const DEFAULT_PAGE_LIMIT: usize = 50;
pub const MAX_PAGE_LIMIT: usize = 500;

/// Forward fees and funding txids parsed from the ledger, kept across rebuilds so each reads only new rows.
#[derive(Default)]
pub struct LedgerFacts {
    pub after_id: i64,
    pub forwards: Vec<RevenueItem>,
    pub funding_txids: HashSet<String>,
    /// First funding txid seen per user_channel_id: the channel's open.
    pub first_funding: HashMap<String, String>,
    /// Recorded "the LSP opened it privately" flag per user_channel_id, kept for channels that later close.
    pub private_outbound: HashMap<String, bool>,
}

impl LedgerFacts {
    /// Folds new ledger rows in and advances the high-water id past every row, readable or not.
    pub fn absorb(&mut self, rows: &[RevenueLedgerRow]) {
        for row in rows {
            self.after_id = self.after_id.max(row.id);
            let Ok(detail) = serde_json::from_str::<serde_json::Value>(&row.detail_json) else { continue };
            let number = |key: &str| detail.get(key).and_then(|v| v.as_u64());
            let text = |key: &str| detail.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_owned();
            match row.event_type.as_str() {
                "PAYMENT_FORWARDED" | "PAYMENT_FORWARDED_BACKFILL" => {
                    let fee = number("fee_msat").or_else(|| number("total_fee_msat")).unwrap_or(0);
                    let skim = number("skimmed_fee_msat").unwrap_or(0).min(fee);
                    let base = RevenueItem {
                        occurred_at: row.occurred_at_ms / 1000,
                        direction: "in".into(),
                        node_id: text("prev_node_id"),
                        user_channel_id: text("prev_user_channel_id"),
                        approximate_time: row.event_type == "PAYMENT_FORWARDED_BACKFILL",
                        ..Default::default()
                    };
                    if skim > 0 {
                        self.forwards.push(RevenueItem { key: format!("jit:{}", row.id), category: JIT_FEE.into(), amount_msat: skim, ..base.clone() });
                    }
                    self.forwards.push(RevenueItem { key: format!("fwd:{}", row.id), category: ROUTING_FEE.into(), amount_msat: fee - skim, ..base });
                },
                _ => {
                    let txo = text("funding_txo");
                    let txid = txo.split(':').next().unwrap_or_default();
                    if !txid.is_empty() {
                        self.funding_txids.insert(txid.to_owned());
                    }
                    let uid = text("user_channel_id");
                    if uid.is_empty() {
                        continue;
                    }
                    if !txid.is_empty() {
                        self.first_funding.entry(uid.clone()).or_insert_with(|| txid.to_owned());
                    }
                    let flag = |key: &str| detail.get(key).and_then(|v| v.as_bool());
                    if let (Some(outbound), Some(announced)) = (flag("is_outbound"), flag("is_announced")) {
                        self.private_outbound.insert(uid, outbound && !announced);
                    }
                },
            }
        }
    }
}

/// Everything one rebuild read; classification is a pure function of it.
pub struct Sources<'a> {
    pub payments: &'a [Payment],
    pub labels: &'a [SettlementLabel],
    pub decisions: &'a [TradeDecisionSummary],
    pub refunds: &'a [TradeFeeRefund],
    pub channels: &'a [Channel],
    pub ledger: &'a LedgerFacts,
}

/// Txid and LDK's classification of an on-chain payment (unset on older LDK Servers and plain sends).
fn onchain_tx(payment: &Payment) -> Option<(&str, Option<&transaction_type::Kind>)> {
    match payment.kind.as_ref()?.kind.as_ref()? {
        payment_kind::Kind::Onchain(onchain) => {
            Some((onchain.txid.as_str(), onchain.tx_type.as_ref().and_then(|t| t.kind.as_ref())))
        },
        _ => None,
    }
}

fn onchain_txid(payment: &Payment) -> Option<&str> {
    onchain_tx(payment).map(|(txid, _)| txid)
}

fn refund_status(refund: &TradeFeeRefund, status_of: &HashMap<&str, i32>) -> &'static str {
    match refund.refund_payment_id.as_deref() {
        None => "unknown",
        Some(id) => match status_of.get(id).copied() {
            Some(status) if status == PaymentStatus::Succeeded as i32 => "succeeded",
            Some(status) if status == PaymentStatus::Failed as i32 => "failed",
            _ => "pending",
        },
    }
}

/// Turns one rebuild's sources into revenue items, newest first.
pub fn classify(src: &Sources) -> Vec<RevenueItem> {
    let labels: HashMap<&str, &SettlementLabel> = src.labels.iter().map(|l| (l.payment_id.as_str(), l)).collect();
    let decisions: HashMap<&str, &TradeDecisionSummary> =
        src.decisions.iter().map(|d| (d.inbound_payment_id.as_str(), d)).collect();
    let replies: HashSet<&str> = src.decisions.iter().filter_map(|d| d.response_payment_id.as_deref()).collect();
    let refund_by_trade: HashMap<&str, &TradeFeeRefund> =
        src.refunds.iter().map(|r| (r.trade_payment_id.as_str(), r)).collect();
    let refund_by_payment: HashMap<&str, &TradeFeeRefund> =
        src.refunds.iter().filter_map(|r| r.refund_payment_id.as_deref().map(|id| (id, r))).collect();
    let node_of: HashMap<&str, &str> =
        src.channels.iter().map(|c| (c.user_channel_id.as_str(), c.counterparty_node_id.as_str())).collect();
    let mut funding: HashSet<&str> = src.ledger.funding_txids.iter().map(String::as_str).collect();
    funding.extend(src.channels.iter().filter_map(|c| c.funding_txo.as_ref().map(|o| o.txid.as_str())));
    let status_of: HashMap<&str, i32> = src.payments.iter().map(|p| (p.payment_id.as_str(), p.status)).collect();
    let opened_channel: HashMap<&str, &str> =
        src.ledger.first_funding.iter().map(|(uid, txid)| (txid.as_str(), uid.as_str())).collect();
    // LSPS2 opens JIT channels outbound and unannounced; a closed channel falls back to its recorded flags.
    let private_outbound = |uid: &str| {
        src.channels
            .iter()
            .find(|c| c.user_channel_id == uid)
            .map(|c| c.is_outbound && !c.is_announced)
            .or_else(|| src.ledger.private_outbound.get(uid).copied())
            .unwrap_or(false)
    };

    let mut items = src.ledger.forwards.clone();
    for payment in src.payments {
        if payment.status != PaymentStatus::Succeeded as i32 {
            continue;
        }
        let inbound = payment.direction == PaymentDirection::Inbound as i32;
        let amount = payment.amount_msat.unwrap_or(0);
        let fee = payment.fee_paid_msat.unwrap_or(0);
        let id = payment.payment_id.as_str();
        let label = labels.get(id);
        let uid = label.and_then(|l| l.user_channel_id.clone()).unwrap_or_default();
        let mut item = RevenueItem {
            key: payment.payment_id.clone(),
            occurred_at: payment.latest_update_timestamp as i64,
            direction: if inbound { "in" } else { "out" }.into(),
            payment_id: payment.payment_id.clone(),
            node_id: node_of.get(uid.as_str()).map(|n| n.to_string()).unwrap_or_default(),
            user_channel_id: uid,
            ..Default::default()
        };
        use transaction_type::Kind;
        let (category, value) = match (inbound, label.map(|l| l.kind.as_str()), onchain_tx(payment)) {
            // LDK records closes, claims and sweeps as inbound (the funds come back to the wallet); the fee is what
            // matters, whichever way the transaction went. BDK reports 0 when it cannot price a transaction whose
            // inputs it does not own, so a 0 fee is unknown, never free.
            (_, _, Some((txid, Some(kind @ (Kind::CooperativeClose(_) | Kind::UnilateralClose(_) | Kind::AnchorBump(_) | Kind::Claim(_) | Kind::Sweep(_)))))) => {
                if fee == 0 {
                    continue;
                }
                item.txid = txid.to_owned();
                item.direction = "out".into();
                let category = match kind {
                    Kind::AnchorBump(_) => CLOSE_FEE_BUMP,
                    Kind::Claim(_) | Kind::Sweep(_) => CLAIM_SWEEP_FEE,
                    _ => CLOSE_FEE,
                };
                (category, fee)
            },
            (true, Some("trade"), _) => {
                if let Some(decision) = decisions.get(id) {
                    item.node_id = decision.counterparty.clone();
                    item.user_channel_id = decision.user_channel_id.clone();
                    item.trade_rejected = decision.outcome == "rejected";
                }
                if let Some(refund) = refund_by_trade.get(id) {
                    item.refund_status = refund_status(refund, &status_of).into();
                }
                (TRADE_FEE, amount)
            },
            (true, Some("stability"), _) => (STABILITY_IN, amount),
            (false, Some("stability"), _) => (STABILITY_OUT, amount),
            (false, Some("sync" | "trade"), _) => (PROTOCOL_MESSAGE, amount),
            (false, _, _) if replies.contains(id) => (PROTOCOL_MESSAGE, amount),
            (false, _, _) if refund_by_payment.contains_key(id) => {
                item.node_id = refund_by_payment[id].counterparty.clone();
                (TRADE_FEE_REFUND, amount)
            },
            (false, _, Some((txid, tx_type))) => {
                item.txid = txid.to_owned();
                let opened = opened_channel.get(txid).filter(|uid| private_outbound(uid));
                match (tx_type, opened) {
                    // A first private outbound funding is a JIT open whatever LDK calls it.
                    (Some(Kind::Funding(_)) | None, Some(uid)) => {
                        item.user_channel_id = uid.to_string();
                        item.node_id = node_of.get(uid).map(|n| n.to_string()).unwrap_or_default();
                        (JIT_OPEN_FEE, fee)
                    },
                    (Some(_), _) => (CHANNEL_FUNDING_FEE, fee),
                    (None, None) if funding.contains(txid) => (CHANNEL_FUNDING_FEE, fee),
                    (None, None) => (ONCHAIN_FEE, fee),
                }
            },
            (false, _, None) => (LIGHTNING_SEND_FEE, fee),
            (true, _, _) => continue,
        };
        item.category = category.into();
        item.amount_msat = value;
        items.push(item);
    }
    items.sort_by(|a, b| b.occurred_at.cmp(&a.occurred_at).then_with(|| b.key.cmp(&a.key)));
    items
}

/// Per-category totals for items at or after `since`, plus the rejected subset of trade fees.
pub fn summarize(items: &[RevenueItem], since: i64) -> Vec<RevenueLine> {
    let mut lines: Vec<RevenueLine> = Vec::new();
    let mut add = |category: &str, direction: &str, amount: u64| match lines.iter_mut().find(|l| l.category == category) {
        Some(line) => {
            line.count += 1;
            line.total_msat = line.total_msat.saturating_add(amount);
        },
        None => lines.push(RevenueLine { category: category.into(), direction: direction.into(), count: 1, total_msat: amount }),
    };
    for item in items.iter().filter(|i| i.occurred_at >= since) {
        add(&item.category, &item.direction, item.amount_msat);
        if item.category == TRADE_FEE && item.trade_rejected {
            add(TRADE_FEE_REJECTED, &item.direction, item.amount_msat);
        }
    }
    lines.sort_by(|a, b| a.category.cmp(&b.category));
    lines
}

/// A filter entry is a category, or `trade_fee_rejected` for trade fees whose trade was rejected.
fn matches_filter(item: &RevenueItem, categories: &[String]) -> bool {
    categories.iter().any(|c| *c == item.category || (c == TRADE_FEE_REJECTED && item.category == TRADE_FEE && item.trade_rejected))
}

fn parse_cursor(cursor: &str) -> Option<(i64, String)> {
    let (at, key) = cursor.split_once(':')?;
    Some((at.parse().ok()?, key.to_owned()))
}

/// One page of items, newest first, strictly after the cursor and inside the window and filter.
pub fn page(
    items: &[RevenueItem],
    since: i64,
    categories: &[String],
    cursor: Option<&str>,
    limit: usize,
) -> (Vec<RevenueItem>, Option<String>) {
    let cursor = cursor.and_then(parse_cursor);
    let mut matching = items
        .iter()
        .filter(|i| i.occurred_at >= since)
        .filter(|i| categories.is_empty() || matches_filter(i, categories))
        .filter(|i| {
            cursor.as_ref().is_none_or(|(at, key)| i.occurred_at < *at || (i.occurred_at == *at && i.key < *key))
        });
    let page: Vec<RevenueItem> = matching.by_ref().take(limit.clamp(1, MAX_PAGE_LIMIT)).cloned().collect();
    let next = if matching.next().is_some() { page.last().map(|i| format!("{}:{}", i.occurred_at, i.key)) } else { None };
    (page, next)
}

const REBUILD_INTERVAL_SECS: u64 = 60;
// A cold start walks the whole history; this only guards a server whose pages never end.
const PAYMENT_PAGE_CAP: usize = 20_000;
// Ledger ids scanned per query, so the shared DB lock is held briefly.
const LEDGER_CHUNK_IDS: i64 = 5_000;

/// The latest classified items and when they were built.
pub struct Snapshot {
    pub items: Vec<RevenueItem>,
    pub built_at: i64,
    pub partial: bool,
    /// Categories this node's LDK Server cannot report, so their totals are not facts.
    pub untracked: Vec<String>,
}

/// What rebuilds carry forward: ledger facts, listed payments and funding transactions already found in the wallet.
#[derive(Default)]
struct Cache {
    ledger: LedgerFacts,
    labels: Vec<SettlementLabel>,
    labels_after: i64,
    onchain: HashMap<String, Payment>,
    payments: PaymentBook,
}

/// Payments read from ListPayments, kept across rebuilds so a warm rebuild only fetches new pages.
#[derive(Default)]
struct PaymentBook {
    /// Everything revenue can use; failed 1-msat protocol attempts are dropped.
    by_id: HashMap<String, Payment>,
    /// Hashes of every payment id ever listed, dropped ones included.
    seen: HashSet<u64>,
    /// Set once a scan reached the end of the history.
    complete: bool,
}

impl PaymentBook {
    /// Keeps the payment's latest state; true when it had not been listed before.
    fn absorb(&mut self, payment: Payment) -> bool {
        let new = self.seen.insert(id_hash(&payment.payment_id));
        if is_failed_protocol_message(&payment) {
            self.by_id.remove(&payment.payment_id);
        } else {
            self.by_id.insert(payment.payment_id.clone(), payment);
        }
        new
    }
}

fn id_hash(id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hasher);
    hasher.finish()
}

/// Walks ListPayments newest first. After one full walk, a page with nothing new ends it; true when the cap cut it short.
async fn scan_payments(book: &mut PaymentBook, ldk: &dyn LdkServerCalls) -> Result<bool, String> {
    let mut page_token = None;
    for _ in 0..PAYMENT_PAGE_CAP {
        let response = ldk.list_payments(ListPaymentsRequest { page_token }).await.map_err(|e| e.to_string())?;
        let mut unseen = response.payments.is_empty();
        for payment in response.payments {
            unseen |= book.absorb(payment);
        }
        match response.next_page_token {
            Some(_) if book.complete && !unseen => return Ok(false),
            Some(token) => page_token = Some(token),
            None => {
                book.complete = true;
                return Ok(false);
            },
        }
    }
    Ok(true)
}

/// Pages are in creation order, so a pending payment below where the scan stopped is re-read directly.
async fn refresh_pending(book: &mut PaymentBook, ldk: &dyn LdkServerCalls) {
    let pending: Vec<String> =
        book.by_id.values().filter(|p| p.status == PaymentStatus::Pending as i32).map(|p| p.payment_id.clone()).collect();
    for payment_id in pending {
        match ldk.get_payment_details(GetPaymentDetailsRequest { payment_id: payment_id.clone() }).await {
            Ok(response) => {
                if let Some(payment) = response.payment {
                    book.absorb(payment);
                }
            },
            Err(error) => warn!("[revenue] pending payment {} lookup failed: {}", payment_id, error),
        }
    }
}

/// Shared by the rebuild task and the routes.
#[derive(Default)]
pub struct RevenueStore {
    snapshot: RwLock<Option<Arc<Snapshot>>>,
    cache: tokio::sync::Mutex<Cache>,
    wake: tokio::sync::Notify,
}

impl RevenueStore {
    pub fn snapshot(&self) -> Option<Arc<Snapshot>> {
        self.snapshot.read().unwrap().clone()
    }

    /// Asks the rebuild task to run now instead of at the next tick.
    pub fn request_rebuild(&self) {
        self.wake.notify_one();
    }
}

/// ldk-node keys an on-chain payment by the txid's internal byte order, the reverse of its displayed hex.
pub fn onchain_payment_id(txid: &str) -> Option<String> {
    let mut bytes = hex::decode(txid).ok().filter(|b| b.len() == 32)?;
    bytes.reverse();
    Some(hex::encode(bytes))
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Rebuilds the snapshot; on any error the previous snapshot stays.
pub async fn rebuild(store: &RevenueStore, ldk: &dyn LdkServerCalls, db: &Arc<Database>, now: i64) -> Result<(), String> {
    let mut cache = store.cache.lock().await;
    let partial = scan_payments(&mut cache.payments, ldk).await?;
    refresh_pending(&mut cache.payments, ldk).await;
    let mut payments: Vec<Payment> = cache.payments.by_id.values().cloned().collect();
    let channels = ldk.list_channels(ListChannelsRequest {}).await.map_err(|e| e.to_string())?.channels;
    let listed_onchain = payments.iter().any(|p| onchain_txid(p).is_some());
    let reader = Arc::clone(db);
    let (after, labels_after) = (cache.ledger.after_id, cache.labels_after);
    let (rows, max_id, new_labels, decisions, refunds) = tokio::task::spawn_blocking(move || -> rusqlite::Result<_> {
        let max_id = reader.max_ledger_event_id()?;
        let mut rows = Vec::new();
        let mut cursor = after;
        while cursor < max_id {
            let up_to = (cursor + LEDGER_CHUNK_IDS).min(max_id);
            rows.extend(reader.revenue_ledger_rows_between(cursor, up_to)?);
            cursor = up_to;
        }
        let labels = reader.list_settlement_labels_after(labels_after)?;
        Ok((rows, max_id, labels, reader.list_trade_decision_summaries()?, reader.list_trade_fee_refunds()?))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    cache.ledger.absorb(&rows);
    cache.ledger.after_id = cache.ledger.after_id.max(max_id);
    cache.labels.extend(new_labels.0);
    cache.labels_after = new_labels.1;
    // LDK Server's list can lack on-chain payments; find funding transactions in the wallet instead.
    let listed: HashSet<String> = payments.iter().map(|p| p.payment_id.clone()).collect();
    let mut funding: HashSet<String> = cache.ledger.funding_txids.clone();
    funding.extend(channels.iter().filter_map(|c| c.funding_txo.as_ref().map(|o| o.txid.clone())));
    for txid in funding {
        if cache.onchain.contains_key(&txid) {
            continue;
        }
        let Some(payment_id) = onchain_payment_id(&txid).filter(|id| !listed.contains(id)) else { continue };
        match ldk.get_payment_details(GetPaymentDetailsRequest { payment_id }).await {
            Ok(response) => {
                if let Some(payment) = response.payment {
                    if payment.status == PaymentStatus::Succeeded as i32 {
                        cache.onchain.insert(txid, payment);
                    } else {
                        payments.push(payment);
                    }
                }
            },
            Err(error) => warn!("[revenue] wallet lookup for funding tx {} failed: {}", txid, error),
        }
    }
    payments.extend(cache.onchain.values().filter(|p| !listed.contains(&p.payment_id)).cloned());
    let items = classify(&Sources {
        payments: &payments,
        labels: &cache.labels,
        decisions: &decisions,
        refunds: &refunds,
        channels: &channels,
        ledger: &cache.ledger,
    });
    drop(cache);
    if partial {
        warn!("[revenue] payment scan stopped after {} pages; totals are partial", PAYMENT_PAGE_CAP);
    }
    let mut untracked = if listed_onchain { Vec::new() } else { vec![ONCHAIN_FEE.to_string()] };
    // A fee category with no item yet reads as "not tracked" in the GUI, never as a zero cost.
    for category in [CLOSE_FEE, CLOSE_FEE_BUMP, CLAIM_SWEEP_FEE] {
        if !items.iter().any(|i| i.category == category) {
            untracked.push(category.to_string());
        }
    }
    *store.snapshot.write().unwrap() = Some(Arc::new(Snapshot { items, built_at: now, partial, untracked }));
    Ok(())
}

/// Rebuilds every minute, or sooner when a refund asks for it.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        loop {
            let ldk: &dyn LdkServerCalls = state.ldk_server.as_ref();
            if let Err(error) = rebuild(&state.revenue, ldk, &state.db, now_secs()).await {
                warn!("[revenue] rebuild failed; keeping the previous snapshot: {}", error);
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(REBUILD_INTERVAL_SECS)) => {},
                _ = state.revenue.wake.notified() => {},
            }
        }
    });
}

/// Why a refund was not sent.
#[derive(Debug, PartialEq)]
pub enum RefundError {
    NotRejectedTradeFee,
    AlreadyRefunded,
    RefundPending,
    OutcomeUnknown,
    Send(String),
    Db(String),
}

impl RefundError {
    pub fn message(&self) -> String {
        match self {
            RefundError::NotRejectedTradeFee => "Only a received fee of a rejected trade can be refunded".into(),
            RefundError::AlreadyRefunded => "This trade fee was already refunded".into(),
            RefundError::RefundPending => "A refund for this trade fee is still in flight".into(),
            RefundError::OutcomeUnknown => "A refund was started but its outcome is unknown; check Payments before retrying".into(),
            RefundError::Send(error) => format!("Refund keysend failed: {error}"),
            RefundError::Db(error) => format!("Refund bookkeeping failed: {error}"),
        }
    }
}

/// Refunds a rejected trade fee exactly once: claim, keysend the received amount back, record the payment id.
pub async fn refund_trade_fee(
    db: &Arc<Database>,
    ldk: &dyn LdkServerCalls,
    trade_payment_id: &str,
    now: i64,
) -> Result<(String, u64), RefundError> {
    let db_error = |e: rusqlite::Error| RefundError::Db(e.to_string());
    let decision = db.trade_decision_summary(trade_payment_id).map_err(db_error)?;
    let Some(decision) = decision.filter(|d| d.outcome == "rejected") else {
        return Err(RefundError::NotRejectedTradeFee);
    };
    let details = |payment_id: String| ldk.get_payment_details(GetPaymentDetailsRequest { payment_id });
    let received = details(trade_payment_id.to_owned()).await.map_err(|e| RefundError::Send(e.to_string()))?.payment;
    let amount_msat = received
        .filter(|p| p.direction == PaymentDirection::Inbound as i32 && p.status == PaymentStatus::Succeeded as i32)
        .and_then(|p| p.amount_msat)
        .filter(|amount| *amount > 0)
        .ok_or(RefundError::NotRejectedTradeFee)?;
    if let Some(existing) = db.claim_trade_fee_refund(trade_payment_id, amount_msat, &decision.counterparty, now).map_err(db_error)? {
        let Some(previous) = existing.refund_payment_id else {
            return Err(RefundError::OutcomeUnknown);
        };
        let status = details(previous.clone()).await.map_err(|e| RefundError::Send(e.to_string()))?.payment.map(|p| p.status);
        match status {
            Some(s) if s == PaymentStatus::Failed as i32 => {
                if !db.retake_failed_trade_fee_refund(trade_payment_id, &previous, now).map_err(db_error)? {
                    return Err(RefundError::RefundPending);
                }
            },
            Some(s) if s == PaymentStatus::Succeeded as i32 => return Err(RefundError::AlreadyRefunded),
            _ => return Err(RefundError::RefundPending),
        }
    }
    let request = SpontaneousSendRequest {
        amount_msat,
        node_id: decision.counterparty.clone(),
        route_parameters: None,
        custom_tlvs: Vec::new(),
        preimage: None,
    };
    let refund_payment_id = match ldk.spontaneous_send(request).await {
        Ok(response) => response.payment_id,
        // Only a server-reported rejection proves nothing left; a transport or internal error may follow a real send.
        Err(error) if matches!(
            error.error_code,
            LdkServerErrorCode::LightningError | LdkServerErrorCode::InvalidRequestError | LdkServerErrorCode::AuthError
        ) => {
            let _ = db.release_trade_fee_refund(trade_payment_id);
            return Err(RefundError::Send(error.to_string()));
        },
        Err(error) => {
            stable_channels::audit::audit_event(
                "TRADE_FEE_REFUND_OUTCOME_UNKNOWN",
                serde_json::json!({ "trade_payment_id": trade_payment_id, "counterparty": decision.counterparty, "error": error.to_string() }),
            );
            return Err(RefundError::OutcomeUnknown);
        },
    };
    db.record_trade_fee_refund_payment(trade_payment_id, &refund_payment_id).map_err(db_error)?;
    stable_channels::audit::audit_event(
        "TRADE_FEE_REFUND_SENT",
        serde_json::json!({
            "trade_payment_id": trade_payment_id,
            "refund_payment_id": refund_payment_id,
            "amount_msat": amount_msat,
            "counterparty": decision.counterparty,
            "user_channel_id": decision.user_channel_id,
        }),
    );
    Ok((refund_payment_id, amount_msat))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ldk_server_client::ldk_server_grpc::types::{Onchain, PaymentKind, Spontaneous};

    pub(crate) fn pay(id: &str, inbound: bool, status: PaymentStatus, amount: Option<u64>, fee: Option<u64>, at: u64, txid: Option<&str>) -> Payment {
        let kind = match txid {
            Some(txid) => payment_kind::Kind::Onchain(Onchain { txid: txid.into(), status: None, tx_type: None }),
            None => payment_kind::Kind::Spontaneous(Spontaneous { hash: "h".into(), preimage: None }),
        };
        Payment {
            payment_id: id.into(),
            kind: Some(PaymentKind { kind: Some(kind) }),
            amount_msat: amount,
            fee_paid_msat: fee,
            direction: if inbound { PaymentDirection::Inbound as i32 } else { PaymentDirection::Outbound as i32 },
            status: status as i32,
            latest_update_timestamp: at,
        }
    }

    /// An outbound on-chain payment carrying LDK's transaction classification.
    fn typed(id: &str, fee: u64, at: u64, txid: &str, kind: transaction_type::Kind) -> Payment {
        let mut payment = pay(id, false, PaymentStatus::Succeeded, Some(1_000_000), Some(fee), at, Some(txid));
        if let Some(payment_kind::Kind::Onchain(onchain)) = payment.kind.as_mut().and_then(|k| k.kind.as_mut()) {
            onchain.tx_type = Some(ldk_server_client::ldk_server_grpc::types::TransactionType { kind: Some(kind) });
        }
        payment
    }

    fn label(id: &str, kind: &str, uid: Option<&str>) -> SettlementLabel {
        SettlementLabel { payment_id: id.into(), kind: kind.into(), user_channel_id: uid.map(Into::into) }
    }

    fn row(id: i64, event: &str, at_ms: i64, detail: serde_json::Value) -> RevenueLedgerRow {
        RevenueLedgerRow { id, event_type: event.into(), occurred_at_ms: at_ms, detail_json: detail.to_string() }
    }

    fn by_key<'a>(items: &'a [RevenueItem], key: &str) -> &'a RevenueItem {
        items.iter().find(|i| i.key == key).unwrap_or_else(|| panic!("no item {key}"))
    }

    #[test]
    fn every_payment_lands_in_its_category() {
        let payments = vec![
            pay("trade-ok", true, PaymentStatus::Succeeded, Some(1_000_000), None, 10, None),
            pay("trade-rej", true, PaymentStatus::Succeeded, Some(2_000_000), None, 11, None),
            pay("stab-in", true, PaymentStatus::Succeeded, Some(40_000_000), None, 12, None),
            pay("stab-out", false, PaymentStatus::Succeeded, Some(90_000_000), Some(0), 13, None),
            pay("sync", false, PaymentStatus::Succeeded, Some(1), Some(0), 14, None),
            pay("reply", false, PaymentStatus::Succeeded, Some(1), Some(0), 15, None),
            pay("refund", false, PaymentStatus::Succeeded, Some(2_000_000), Some(0), 16, None),
            pay("fund", false, PaymentStatus::Succeeded, Some(500_000_000), Some(300_000), 17, Some("aa")),
            pay("spend", false, PaymentStatus::Succeeded, Some(10_000_000), Some(200_000), 18, Some("bb")),
            pay("send", false, PaymentStatus::Succeeded, Some(5_000_000), Some(1_500), 19, None),
            pay("deposit", true, PaymentStatus::Succeeded, Some(9_000_000), Some(0), 20, Some("cc")),
            pay("failed", false, PaymentStatus::Failed, Some(5_000_000), Some(1_500), 21, None),
            pay("pending", true, PaymentStatus::Pending, Some(5_000_000), None, 22, None),
        ];
        let labels = vec![
            label("trade-ok", "trade", None),
            label("trade-rej", "trade", None),
            label("stab-in", "stability", Some("7")),
            label("stab-out", "stability", Some("7")),
            label("sync", "sync", Some("7")),
            label("refund", "refund", None),
        ];
        let decisions = vec![
            TradeDecisionSummary { inbound_payment_id: "trade-ok".into(), outcome: "accepted".into(), counterparty: "02aa".into(), user_channel_id: "7".into(), response_payment_id: Some("reply".into()) },
            TradeDecisionSummary { inbound_payment_id: "trade-rej".into(), outcome: "rejected".into(), counterparty: "02aa".into(), user_channel_id: "7".into(), response_payment_id: None },
        ];
        let refunds = vec![TradeFeeRefund { trade_payment_id: "trade-rej".into(), refund_payment_id: Some("refund".into()), amount_msat: 2_000_000, counterparty: "02aa".into(), created_at: 16 }];
        let channels = vec![Channel { user_channel_id: "7".into(), counterparty_node_id: "02aa".into(), ..Default::default() }];
        let mut ledger = LedgerFacts::default();
        ledger.funding_txids.insert("aa".into());
        let items = classify(&Sources { payments: &payments, labels: &labels, decisions: &decisions, refunds: &refunds, channels: &channels, ledger: &ledger });
        let cat = |key: &str| (by_key(&items, key).category.as_str(), by_key(&items, key).direction.as_str(), by_key(&items, key).amount_msat);
        assert_eq!(cat("trade-ok"), (TRADE_FEE, "in", 1_000_000));
        assert_eq!(cat("trade-rej"), (TRADE_FEE, "in", 2_000_000));
        assert!(by_key(&items, "trade-rej").trade_rejected && !by_key(&items, "trade-ok").trade_rejected);
        assert_eq!(by_key(&items, "trade-rej").refund_status, "succeeded", "its refund payment succeeded");
        assert_eq!(cat("stab-in"), (STABILITY_IN, "in", 40_000_000));
        assert_eq!(cat("stab-out"), (STABILITY_OUT, "out", 90_000_000));
        assert_eq!(by_key(&items, "stab-out").node_id, "02aa", "node comes from the channel list");
        assert_eq!(cat("sync"), (PROTOCOL_MESSAGE, "out", 1));
        assert_eq!(cat("reply"), (PROTOCOL_MESSAGE, "out", 1));
        assert_eq!(cat("refund"), (TRADE_FEE_REFUND, "out", 2_000_000));
        assert_eq!(cat("fund"), (CHANNEL_FUNDING_FEE, "out", 300_000));
        assert_eq!(by_key(&items, "fund").txid, "aa");
        assert_eq!(cat("spend"), (ONCHAIN_FEE, "out", 200_000));
        assert_eq!(cat("send"), (LIGHTNING_SEND_FEE, "out", 1_500));
        for ignored in ["deposit", "failed", "pending"] {
            assert!(items.iter().all(|i| i.key != ignored), "{ignored} must not be counted");
        }
        assert_eq!(items.first().unwrap().key, "send", "newest first");
    }

    #[test]
    fn ldk_transaction_types_pick_the_onchain_fee_category() {
        use ldk_server_client::ldk_server_grpc::types::{
            AnchorBump, Claim, CooperativeClose, Funding, InteractiveFunding, Sweep, TransactionChannel, UnilateralClose,
        };
        let chan = |id: &str| TransactionChannel { channel_id: id.into(), counterparty_node_id: "02aa".into() };
        let close = |id: &str| CooperativeClose { channel_id: id.into(), counterparty_node_id: "02aa".into() };
        let payments = vec![
            typed("jit", 100, 1, "aa", transaction_type::Kind::Funding(Funding { channels: vec![chan("c7")] })),
            typed("open", 110, 2, "bb", transaction_type::Kind::Funding(Funding { channels: vec![chan("c8")] })),
            typed("splice", 120, 3, "cc", transaction_type::Kind::InteractiveFunding(InteractiveFunding { channels: vec![chan("c8")] })),
            // LDK lists a cooperative close as inbound: the channel balance comes back to the wallet.
            { let mut p = typed("coop", 130, 4, "dd", transaction_type::Kind::CooperativeClose(close("c8"))); p.direction = PaymentDirection::Inbound as i32; p },
            { let mut p = typed("sweep0", 0, 4, "55", transaction_type::Kind::Sweep(Sweep { channels: vec![chan("c8")] })); p.direction = PaymentDirection::Inbound as i32; p },
            typed("bump0", 0, 10, "66", transaction_type::Kind::AnchorBump(AnchorBump { channel_id: "c8".into(), counterparty_node_id: "02aa".into() })),
            typed("force", 140, 5, "ee", transaction_type::Kind::UnilateralClose(UnilateralClose { channel_id: "c8".into(), counterparty_node_id: "02aa".into() })),
            typed("bump", 150, 6, "ff", transaction_type::Kind::AnchorBump(AnchorBump { channel_id: "c8".into(), counterparty_node_id: "02aa".into() })),
            typed("claim", 160, 7, "11", transaction_type::Kind::Claim(Claim { channel_id: "c8".into(), counterparty_node_id: "02aa".into() })),
            typed("sweep", 170, 8, "22", transaction_type::Kind::Sweep(Sweep { channels: vec![chan("c8")] })),
            pay("untyped", false, PaymentStatus::Succeeded, Some(1_000_000), Some(180), 9, Some("33")),
        ];
        let channels = vec![
            Channel { user_channel_id: "7".into(), counterparty_node_id: "02aa".into(), is_outbound: true, is_announced: false, ..Default::default() },
            Channel { user_channel_id: "8".into(), counterparty_node_id: "02aa".into(), is_outbound: true, is_announced: true, ..Default::default() },
        ];
        let mut ledger = LedgerFacts::default();
        ledger.first_funding.insert("7".into(), "aa".into());
        ledger.first_funding.insert("8".into(), "bb".into());
        let items = classify(&Sources { payments: &payments, labels: &[], decisions: &[], refunds: &[], channels: &channels, ledger: &ledger });
        let cat = |key: &str| by_key(&items, key).category.as_str();
        assert_eq!(cat("jit"), JIT_OPEN_FEE, "a first private outbound funding stays a JIT open");
        assert_eq!(cat("open"), CHANNEL_FUNDING_FEE);
        assert_eq!(cat("splice"), CHANNEL_FUNDING_FEE);
        assert_eq!(cat("coop"), CLOSE_FEE, "an inbound close still counts its fee");
        assert_eq!(by_key(&items, "coop").direction, "out", "a fee always leaves");
        assert_eq!(cat("force"), CLOSE_FEE);
        for unknown in ["sweep0", "bump0"] {
            assert!(items.iter().all(|i| i.key != unknown), "{unknown}: a 0 fee is unknown, not a free transaction");
        }
        assert_eq!(cat("bump"), CLOSE_FEE_BUMP);
        assert_eq!(cat("claim"), CLAIM_SWEEP_FEE);
        assert_eq!(cat("sweep"), CLAIM_SWEEP_FEE);
        assert_eq!(cat("untyped"), ONCHAIN_FEE, "unset keeps the old behaviour");
        assert_eq!(by_key(&items, "coop").amount_msat, 130, "the fee is the amount");
    }

    #[test]
    fn refund_status_follows_the_refund_payment() {
        let refunds = |id: Option<&str>| vec![TradeFeeRefund { trade_payment_id: "t".into(), refund_payment_id: id.map(Into::into), amount_msat: 1, counterparty: "02aa".into(), created_at: 1 }];
        let decisions = vec![TradeDecisionSummary { inbound_payment_id: "t".into(), outcome: "rejected".into(), counterparty: "02aa".into(), user_channel_id: "7".into(), response_payment_id: None }];
        let labels = vec![label("t", "trade", None)];
        let ledger = LedgerFacts::default();
        let status = |extra: Option<Payment>, id: Option<&str>| {
            let mut payments = vec![pay("t", true, PaymentStatus::Succeeded, Some(1), None, 1, None)];
            payments.extend(extra);
            let r = refunds(id);
            let items = classify(&Sources { payments: &payments, labels: &labels, decisions: &decisions, refunds: &r, channels: &[], ledger: &ledger });
            by_key(&items, "t").refund_status.clone()
        };
        assert_eq!(status(None, None), "unknown");
        assert_eq!(status(Some(pay("r", false, PaymentStatus::Failed, Some(1), None, 2, None)), Some("r")), "failed");
        assert_eq!(status(Some(pay("r", false, PaymentStatus::Succeeded, Some(1), None, 2, None)), Some("r")), "succeeded");
        assert_eq!(status(Some(pay("r", false, PaymentStatus::Pending, Some(1), None, 2, None)), Some("r")), "pending");
    }

    #[test]
    fn unset_amounts_count_as_zero() {
        let payments = vec![pay("spend", false, PaymentStatus::Succeeded, None, None, 5, Some("bb"))];
        let ledger = LedgerFacts::default();
        let items = classify(&Sources { payments: &payments, labels: &[], decisions: &[], refunds: &[], channels: &[], ledger: &ledger });
        assert_eq!((items[0].category.as_str(), items[0].amount_msat), (ONCHAIN_FEE, 0));
    }

    #[test]
    fn forwards_split_routing_and_jit_fees_and_mark_backfill_approximate() {
        let mut ledger = LedgerFacts::default();
        ledger.absorb(&[
            row(3, "PAYMENT_FORWARDED", 9_000, serde_json::json!({"fee_msat": 5_000, "skimmed_fee_msat": 4_000, "prev_node_id": "02aa", "prev_user_channel_id": "7"})),
            row(5, "PAYMENT_FORWARDED_BACKFILL", 12_000, serde_json::json!({"total_fee_msat": 1_000, "prev_node_id": null})),
            row(8, "CHANNEL_READY_SPLICE", 1, serde_json::json!({"funding_txo": "aa:1"})),
            row(9, "CHANNEL_RECONSTRUCTED", 1, serde_json::json!({"funding_txo": "bb"})),
        ]);
        assert_eq!(ledger.after_id, 9);
        assert_eq!(ledger.funding_txids, ["aa".to_string(), "bb".to_string()].into_iter().collect());
        let routing = by_key(&ledger.forwards, "fwd:3");
        assert_eq!((routing.category.as_str(), routing.amount_msat, routing.occurred_at, routing.node_id.as_str()), (ROUTING_FEE, 1_000, 9, "02aa"));
        assert_eq!(by_key(&ledger.forwards, "jit:3").amount_msat, 4_000);
        let backfill = by_key(&ledger.forwards, "fwd:5");
        assert!(backfill.approximate_time && backfill.node_id.is_empty());
        assert!(ledger.forwards.iter().all(|i| i.key != "jit:5"), "no skim, no JIT line");
    }

    #[test]
    fn absorb_skips_malformed_rows_and_still_advances() {
        let mut ledger = LedgerFacts::default();
        ledger.absorb(&[
            RevenueLedgerRow { id: 4, event_type: "PAYMENT_FORWARDED".into(), occurred_at_ms: 1, detail_json: "not json".into() },
            row(6, "PAYMENT_FORWARDED", 2_000, serde_json::json!({})),
        ]);
        assert_eq!(ledger.after_id, 6);
        assert_eq!(by_key(&ledger.forwards, "fwd:6").amount_msat, 0);
        assert_eq!(ledger.forwards.len(), 1);
    }

    fn item(key: &str, at: i64, category: &str, amount: u64) -> RevenueItem {
        RevenueItem { key: key.into(), occurred_at: at, category: category.into(), direction: "in".into(), amount_msat: amount, ..Default::default() }
    }

    #[test]
    fn summaries_cover_the_window_and_count_rejected_trade_fees() {
        let mut rejected = item("b", 200, TRADE_FEE, 3_000);
        rejected.trade_rejected = true;
        let items = vec![item("c", 300, ROUTING_FEE, 7), rejected, item("a", 100, TRADE_FEE, 1_000)];
        let lines = summarize(&items, 150);
        let get = |c: &str| lines.iter().find(|l| l.category == c).map(|l| (l.count, l.total_msat));
        assert_eq!(get(TRADE_FEE), Some((1, 3_000)));
        assert_eq!(get(TRADE_FEE_REJECTED), Some((1, 3_000)));
        assert_eq!(get(ROUTING_FEE), Some((1, 7)));
        assert_eq!(summarize(&items, 0).iter().find(|l| l.category == TRADE_FEE).unwrap().count, 2);
    }

    #[test]
    fn pages_filter_by_category_and_follow_the_cursor() {
        let items = vec![item("d", 400, ROUTING_FEE, 1), item("c", 300, TRADE_FEE, 1), item("b", 300, ROUTING_FEE, 1), item("a", 100, ROUTING_FEE, 1)];
        let (first, cursor) = page(&items, 0, &[ROUTING_FEE.to_string()], None, 2);
        assert_eq!(first.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["d", "b"]);
        let (second, end) = page(&items, 0, &[ROUTING_FEE.to_string()], cursor.as_deref(), 2);
        assert_eq!(second.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["a"]);
        assert_eq!(end, None);
        let (windowed, _) = page(&items, 250, &[], None, 10);
        assert_eq!(windowed.len(), 3);
    }

    #[test]
    fn cursor_pages_stay_stable_when_new_items_arrive() {
        let before = vec![item("c", 300, ROUTING_FEE, 1), item("b", 200, ROUTING_FEE, 1), item("a", 100, ROUTING_FEE, 1)];
        let (first, cursor) = page(&before, 0, &[], None, 2);
        assert_eq!(first.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["c", "b"]);
        let mut after = vec![item("e", 500, ROUTING_FEE, 1), item("d", 400, ROUTING_FEE, 1)];
        after.extend(before);
        let (second, _) = page(&after, 0, &[], cursor.as_deref(), 2);
        assert_eq!(second.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["a"]);
    }

    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;
    use ldk_server_client::error::{LdkServerError, LdkServerErrorCode};
    use ldk_server_client::ldk_server_grpc::api::{
        GetPaymentDetailsRequest, GetPaymentDetailsResponse, ListChannelsRequest, ListChannelsResponse,
        ListPaymentsRequest, ListPaymentsResponse, SignMessageRequest, SignMessageResponse,
        SpontaneousSendRequest, SpontaneousSendResponse, VerifySignatureRequest, VerifySignatureResponse,
    };

    // LDK Server stand-in: serves payments on the first page, can page forever, fail, or record keysends.
    #[derive(Default)]
    pub(crate) struct Fake {
        pub payments: StdMutex<Vec<Payment>>,
        pub wallet: StdMutex<Vec<Payment>>,
        pub channels: Vec<Channel>,
        pub endless: bool,
        pub fail_list: AtomicBool,
        pub send_fails: bool,
        pub ambiguous_send: bool,
        pub sends: StdMutex<Vec<SpontaneousSendRequest>>,
        // Serve `payments` (newest first) in pages of this size, like LDK Server does.
        pub page_size: Option<usize>,
        pub list_calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl LdkServerCalls for Fake {
        async fn list_channels(&self, _req: ListChannelsRequest) -> Result<ListChannelsResponse, LdkServerError> {
            Ok(ListChannelsResponse { channels: self.channels.clone() })
        }
        async fn spontaneous_send(&self, req: SpontaneousSendRequest) -> Result<SpontaneousSendResponse, LdkServerError> {
            if self.send_fails {
                return Err(LdkServerError::new(LdkServerErrorCode::LightningError, "no route".to_string()));
            }
            if self.ambiguous_send {
                return Err(LdkServerError::new(LdkServerErrorCode::InternalError, "connection reset".to_string()));
            }
            let mut sends = self.sends.lock().unwrap();
            sends.push(req);
            Ok(SpontaneousSendResponse { payment_id: format!("refund-{}", sends.len()) })
        }
        async fn sign_message(&self, _req: SignMessageRequest) -> Result<SignMessageResponse, LdkServerError> {
            unreachable!("revenue never signs")
        }
        async fn verify_signature(&self, _req: VerifySignatureRequest) -> Result<VerifySignatureResponse, LdkServerError> {
            unreachable!("revenue never verifies")
        }
        async fn list_payments(&self, req: ListPaymentsRequest) -> Result<ListPaymentsResponse, LdkServerError> {
            if self.fail_list.load(Ordering::SeqCst) {
                return Err(LdkServerError::new(LdkServerErrorCode::InternalServerError, "down".to_string()));
            }
            self.list_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(size) = self.page_size {
                let all = self.payments.lock().unwrap().clone();
                let start: usize = req.page_token.as_deref().map_or(0, |t| t.parse().unwrap());
                let end = (start + size).min(all.len());
                return Ok(ListPaymentsResponse { payments: all[start..end].to_vec(), next_page_token: (end < all.len()).then(|| end.to_string()) });
            }
            let payments = if req.page_token.is_none() { self.payments.lock().unwrap().clone() } else { Vec::new() };
            Ok(ListPaymentsResponse { payments, next_page_token: self.endless.then(|| "t".to_owned()) })
        }
        async fn get_payment_details(&self, req: GetPaymentDetailsRequest) -> Result<GetPaymentDetailsResponse, LdkServerError> {
            let find = |list: &StdMutex<Vec<Payment>>| list.lock().unwrap().iter().find(|p| p.payment_id == req.payment_id).cloned();
            Ok(GetPaymentDetailsResponse { payment: find(&self.payments).or_else(|| find(&self.wallet)) })
        }
    }

    // A real on-disk database in a temp dir, like the daemon's other tests.
    pub(crate) fn temp_db() -> (tempfile::TempDir, std::sync::Arc<stable_channels::db::Database>) {
        let dir = tempfile::tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        (dir, std::sync::Arc::new(db))
    }

    #[tokio::test]
    async fn a_rebuild_reads_payments_labels_and_new_ledger_rows_only() {
        let (_dir, db) = temp_db();
        db.record_settlement("trade-1", "trade").unwrap();
        db.append_ledger_event(&stable_channels::ledger::LedgerEventDraft::from_audit_event(
            "PAYMENT_FORWARDED", serde_json::json!({"fee_msat": 2_000, "occurred_at_ms": 50_000}))).unwrap();
        let fake = Fake::default();
        fake.payments.lock().unwrap().push(pay("trade-1", true, PaymentStatus::Succeeded, Some(1_000_000), None, 60, None));
        let store = RevenueStore::default();
        assert!(store.snapshot().is_none(), "no snapshot before the first build");
        rebuild(&store, &fake, &db, 100).await.unwrap();
        let snap = store.snapshot().unwrap();
        assert_eq!((snap.built_at, snap.partial, snap.items.len()), (100, false, 2));
        rebuild(&store, &fake, &db, 160).await.unwrap();
        assert_eq!(store.snapshot().unwrap().items.len(), 2, "ledger rows are not read twice");
    }

    #[tokio::test]
    async fn a_failed_rebuild_keeps_the_previous_snapshot() {
        let (_dir, db) = temp_db();
        let fake = Fake::default();
        let store = RevenueStore::default();
        rebuild(&store, &fake, &db, 100).await.unwrap();
        fake.fail_list.store(true, Ordering::SeqCst);
        assert!(rebuild(&store, &fake, &db, 160).await.is_err());
        assert_eq!(store.snapshot().unwrap().built_at, 100);
    }

    #[tokio::test]
    async fn an_endless_payment_list_stops_at_the_cap_and_is_marked_partial() {
        let (_dir, db) = temp_db();
        let fake = Fake { endless: true, ..Default::default() };
        let store = RevenueStore::default();
        rebuild(&store, &fake, &db, 100).await.unwrap();
        assert!(store.snapshot().unwrap().partial);
    }

    fn failed_sync(i: usize) -> Payment {
        pay(&format!("sync-{i}"), false, PaymentStatus::Failed, Some(1), None, i as u64, None)
    }

    #[tokio::test]
    async fn a_warm_rebuild_reads_only_pages_it_has_not_seen() {
        let (_dir, db) = temp_db();
        let fake = Fake { page_size: Some(50), ..Default::default() };
        *fake.payments.lock().unwrap() = (0..120).rev().map(failed_sync).collect();
        let store = RevenueStore::default();
        rebuild(&store, &fake, &db, 100).await.unwrap();
        assert_eq!(fake.list_calls.load(Ordering::SeqCst), 3, "a cold start walks the whole history");
        assert!(!store.snapshot().unwrap().partial);
        fake.payments.lock().unwrap().insert(0, pay("send-1", false, PaymentStatus::Succeeded, Some(5_000), Some(7), 200, None));
        rebuild(&store, &fake, &db, 160).await.unwrap();
        assert_eq!(fake.list_calls.load(Ordering::SeqCst), 5, "a warm rebuild stops at the first page it has already seen");
        let snap = store.snapshot().unwrap();
        assert!(!snap.partial, "the rest of the history was read before");
        assert_eq!(by_key(&snap.items, "send-1").category, LIGHTNING_SEND_FEE);
    }

    #[tokio::test]
    async fn a_pending_payment_is_rechecked_after_the_scan_stops_above_it() {
        let (_dir, db) = temp_db();
        let fake = Fake { page_size: Some(50), ..Default::default() };
        let mut history: Vec<Payment> = (0..120).rev().map(failed_sync).collect();
        history.push(pay("slow", false, PaymentStatus::Pending, Some(9_000), Some(11), 1, None));
        *fake.payments.lock().unwrap() = history;
        let store = RevenueStore::default();
        rebuild(&store, &fake, &db, 100).await.unwrap();
        {
            let mut payments = fake.payments.lock().unwrap();
            payments.last_mut().unwrap().status = PaymentStatus::Succeeded as i32;
            payments.insert(0, failed_sync(500));
        }
        rebuild(&store, &fake, &db, 160).await.unwrap();
        assert_eq!(by_key(&store.snapshot().unwrap().items, "slow").category, LIGHTNING_SEND_FEE);
    }

    fn rejected_trade(db: &stable_channels::db::Database, fake: &Fake, id: &str, amount: u64) {
        db.record_settlement(id, "trade").unwrap();
        db.persist_trade_rejection(id, &format!("trade-{id}"), "hash", "chan", "7", "02aa", "quote_expired", 100, "{}").unwrap();
        fake.payments.lock().unwrap().push(pay(id, true, PaymentStatus::Succeeded, Some(amount), None, 100, None));
    }

    #[tokio::test]
    async fn a_rejected_trade_fee_is_refunded_once_with_the_exact_amount() {
        let (_dir, db) = temp_db();
        let fake = Fake::default();
        rejected_trade(&db, &fake, "t1", 1_234_000);
        let (refund_id, amount) = refund_trade_fee(&db, &fake, "t1", 200).await.unwrap();
        assert_eq!((refund_id.as_str(), amount), ("refund-1", 1_234_000));
        let sends = fake.sends.lock().unwrap().clone();
        assert_eq!((sends.len(), sends[0].amount_msat, sends[0].node_id.as_str(), sends[0].custom_tlvs.len()), (1, 1_234_000, "02aa", 0));
        assert_eq!(db.list_trade_fee_refunds().unwrap()[0].refund_payment_id.as_deref(), Some("refund-1"));
        assert_eq!(refund_trade_fee(&db, &fake, "t1", 201).await, Err(RefundError::RefundPending), "the refund payment is not in LDK's list yet");
        fake.payments.lock().unwrap().push(pay("refund-1", false, PaymentStatus::Succeeded, Some(1_234_000), Some(0), 202, None));
        assert_eq!(refund_trade_fee(&db, &fake, "t1", 203).await, Err(RefundError::AlreadyRefunded));
        assert_eq!(fake.sends.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn only_rejected_trade_fees_can_be_refunded() {
        let (_dir, db) = temp_db();
        let fake = Fake::default();
        db.record_settlement("accepted", "trade").unwrap();
        fake.payments.lock().unwrap().push(pay("accepted", true, PaymentStatus::Succeeded, Some(1_000), None, 1, None));
        assert_eq!(refund_trade_fee(&db, &fake, "accepted", 2).await, Err(RefundError::NotRejectedTradeFee));
        assert_eq!(refund_trade_fee(&db, &fake, "missing", 2).await, Err(RefundError::NotRejectedTradeFee));
        assert!(fake.sends.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_failed_refund_can_be_retried_and_a_send_error_releases_the_claim() {
        let (_dir, db) = temp_db();
        let failing = Fake { send_fails: true, ..Default::default() };
        rejected_trade(&db, &failing, "t1", 5_000);
        assert!(matches!(refund_trade_fee(&db, &failing, "t1", 10).await, Err(RefundError::Send(_))));
        assert!(db.list_trade_fee_refunds().unwrap().is_empty(), "the claim is released when nothing left");
        let fake = Fake::default();
        rejected_trade(&db, &fake, "t1", 5_000);
        refund_trade_fee(&db, &fake, "t1", 11).await.unwrap();
        fake.payments.lock().unwrap().push(pay("refund-1", false, PaymentStatus::Failed, Some(5_000), None, 12, None));
        let (again, _) = refund_trade_fee(&db, &fake, "t1", 13).await.unwrap();
        assert_eq!(again, "refund-2");
        assert_eq!(fake.sends.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_claimed_refund_without_a_payment_id_blocks_another_send() {
        let (_dir, db) = temp_db();
        let fake = Fake::default();
        rejected_trade(&db, &fake, "t1", 5_000);
        db.claim_trade_fee_refund("t1", 5_000, "02aa", 10).unwrap();
        assert_eq!(refund_trade_fee(&db, &fake, "t1", 11).await, Err(RefundError::OutcomeUnknown));
        assert!(fake.sends.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_ambiguous_send_error_keeps_the_claim_so_the_fee_is_never_sent_twice() {
        let (_dir, db) = temp_db();
        let fake = Fake { ambiguous_send: true, ..Default::default() };
        rejected_trade(&db, &fake, "t1", 5_000);
        assert_eq!(refund_trade_fee(&db, &fake, "t1", 10).await, Err(RefundError::OutcomeUnknown));
        let claims = db.list_trade_fee_refunds().unwrap();
        assert_eq!((claims.len(), claims[0].refund_payment_id.as_deref()), (1, None), "the claim survives");
        assert_eq!(refund_trade_fee(&db, &fake, "t1", 11).await, Err(RefundError::OutcomeUnknown));
    }

    #[test]
    fn onchain_payment_ids_are_the_txid_bytes_reversed() {
        let txid: String = (1..=32u8).map(|b| format!("{b:02x}")).collect();
        let expected: String = (1..=32u8).rev().map(|b| format!("{b:02x}")).collect();
        assert_eq!(onchain_payment_id(&txid), Some(expected));
        assert_eq!(onchain_payment_id("zz"), None);
    }

    #[tokio::test]
    async fn funding_fees_come_from_the_wallet_when_the_payment_list_has_no_onchain_payments() {
        use ldk_server_client::ldk_server_grpc::types::OutPoint;
        let (_dir, db) = temp_db();
        let txid = format!("4669d6b4924a{}", "00".repeat(26));
        let fake = Fake {
            channels: vec![Channel { funding_txo: Some(OutPoint { txid: txid.clone(), vout: 0 }), ..Default::default() }],
            ..Default::default()
        };
        fake.payments.lock().unwrap().push(pay("keysend", false, PaymentStatus::Succeeded, Some(1), Some(0), 5, None));
        let wallet_id = onchain_payment_id(&txid).unwrap();
        fake.wallet.lock().unwrap().push(pay(&wallet_id, false, PaymentStatus::Succeeded, Some(100_000_000), Some(227_000), 7, Some(&txid)));
        let store = RevenueStore::default();
        rebuild(&store, &fake, &db, 10).await.unwrap();
        let snap = store.snapshot().unwrap();
        let fund = snap.items.iter().find(|i| i.category == CHANNEL_FUNDING_FEE).expect("funding fee found through the wallet");
        assert_eq!((fund.amount_msat, fund.txid.as_str()), (227_000, txid.as_str()));
        assert_eq!(
            snap.untracked,
            [ONCHAIN_FEE, CLOSE_FEE, CLOSE_FEE_BUMP, CLAIM_SWEEP_FEE].map(String::from),
            "other on-chain fees cannot be measured from this list, and no close, bump or sweep fee is known yet"
        );
    }

    #[tokio::test]
    async fn a_rebuild_moves_past_ledger_rows_it_does_not_use_and_keeps_new_labels() {
        let (_dir, db) = temp_db();
        for _ in 0..3 {
            db.append_ledger_event(&stable_channels::ledger::LedgerEventDraft::from_audit_event(
                "TRADE_ACCEPTED", serde_json::json!({"trade_id": "t"}))).unwrap();
        }
        let fake = Fake::default();
        fake.payments.lock().unwrap().push(pay("s1", false, PaymentStatus::Succeeded, Some(1), Some(0), 5, None));
        let store = RevenueStore::default();
        rebuild(&store, &fake, &db, 10).await.unwrap();
        assert_eq!(store.cache.lock().await.ledger.after_id, db.max_ledger_event_id().unwrap(), "unused rows are not rescanned");
        assert_eq!(store.snapshot().unwrap().items[0].category, LIGHTNING_SEND_FEE);
        db.record_settlement_with_channel("s1", "sync", "7").unwrap();
        rebuild(&store, &fake, &db, 20).await.unwrap();
        assert_eq!(store.snapshot().unwrap().items[0].category, PROTOCOL_MESSAGE, "a label written later is picked up");
    }

    #[test]
    fn a_private_channel_the_lsp_opened_counts_its_opening_fee_as_a_jit_open() {
        let mut ledger = LedgerFacts::default();
        ledger.absorb(&[
            row(1, "CHANNEL_READY_TRACKED", 1, serde_json::json!({"user_channel_id": "7", "funding_txo": "aa:0"})),
            row(2, "CHANNEL_READY_SPLICE", 2, serde_json::json!({"user_channel_id": "7", "funding_txo": "bb:0"})),
            row(3, "CHANNEL_RECONSTRUCTED", 3, serde_json::json!({"user_channel_id": "9", "funding_txo": "cc", "is_outbound": true, "is_announced": false})),
            row(4, "CHANNEL_READY_TRACKED", 4, serde_json::json!({"user_channel_id": "11", "funding_txo": "dd:0"})),
        ]);
        let channels = vec![
            Channel { user_channel_id: "7".into(), counterparty_node_id: "02aa".into(), is_outbound: true, is_announced: false, ..Default::default() },
            Channel { user_channel_id: "11".into(), counterparty_node_id: "03bb".into(), is_outbound: true, is_announced: true, ..Default::default() },
        ];
        let payments = vec![
            pay("open-7", false, PaymentStatus::Succeeded, Some(1), Some(287_000), 10, Some("aa")),
            pay("splice-7", false, PaymentStatus::Succeeded, Some(1), Some(100_000), 11, Some("bb")),
            pay("open-9", false, PaymentStatus::Succeeded, Some(1), Some(200_000), 12, Some("cc")),
            pay("open-11", false, PaymentStatus::Succeeded, Some(1), Some(150_000), 13, Some("dd")),
        ];
        let items = classify(&Sources { payments: &payments, labels: &[], decisions: &[], refunds: &[], channels: &channels, ledger: &ledger });
        let cat = |key: &str| (by_key(&items, key).category.as_str(), by_key(&items, key).amount_msat);
        assert_eq!(cat("open-7"), (JIT_OPEN_FEE, 287_000));
        assert_eq!((by_key(&items, "open-7").node_id.as_str(), by_key(&items, "open-7").user_channel_id.as_str()), ("02aa", "7"));
        assert_eq!(cat("splice-7"), (CHANNEL_FUNDING_FEE, 100_000), "a later splice is not the free open");
        assert_eq!(cat("open-9"), (JIT_OPEN_FEE, 200_000), "a closed channel uses its recorded flags");
        assert_eq!(cat("open-11"), (CHANNEL_FUNDING_FEE, 150_000), "a public channel is one the operator opened");
    }

    #[test]
    fn the_rejected_filter_pages_only_rejected_trade_fees() {
        let mut rejected = item("b", 200, TRADE_FEE, 3_000);
        rejected.trade_rejected = true;
        let items = vec![item("c", 300, TRADE_FEE, 1), rejected, item("a", 100, ROUTING_FEE, 1)];
        let (page_items, _) = page(&items, 0, &[TRADE_FEE_REJECTED.to_string()], None, 10);
        assert_eq!(page_items.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["b"]);
        let (both, _) = page(&items, 0, &[TRADE_FEE_REJECTED.to_string(), ROUTING_FEE.to_string()], None, 10);
        assert_eq!(both.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["b", "a"]);
    }
}
