//! Every daemon operation: validation, the request, and how its result updates state.

use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;

use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use sc_rest_client::client::LspRestClient;
use sc_rest_client::ldk_server_grpc::api::{
	Bolt11ReceiveRequest, Bolt11SendRequest, Bolt12ReceiveRequest, Bolt12SendRequest,
	CloseChannelRequest, ConnectPeerRequest, DisconnectPeerRequest, ExportPathfindingScoresRequest,
	ForceCloseChannelRequest, GetBalancesRequest, GetNodeInfoRequest, GetPaymentDetailsRequest,
	GraphGetChannelRequest, GraphGetNodeRequest, GraphListChannelsRequest, GraphListNodesRequest,
	ListChannelsRequest, ListForwardedPaymentsRequest, ListPaymentsRequest, ListPaymentsResponse,
	ListPeersRequest, OnchainReceiveRequest, OnchainSendRequest, OpenChannelRequest,
	SignMessageRequest, SpliceInRequest, SpliceOutRequest, SpontaneousSendRequest,
	UpdateChannelConfigRequest, VerifySignatureRequest, AllFunds, onchain_send_request, open_channel_request,
	splice_in_request,
};
use sc_rest_client::ldk_server_grpc::types::{
	bolt11_invoice_description, Bolt11InvoiceDescription, ChannelConfig,
};
use sc_rest_client::sc_protos::revenue::{GetRevenueRequest, RefundTradeFeeRequest};
use sc_rest_client::sc_protos::stable::{
	EditStableChannelRequest, GetPriceRequest, ListChannelLedgerEventsRequest,
	ListChannelLedgerEventsResponse, ListSettlementPaymentsRequest, ListStableChannelsRequest,
	LogRequest,
};

use crate::ledger::{checked_next_cursor, merge_ledger_events};
use crate::state::{
	ActiveTab, AppCtx, ChannelLedgerForm, ChannelLedgerRequestKey, ConnectionStatus, Dialog,
	Drawer, LogsTab, Op, SendKind, SettlementKind,
};

/// Run `fut` for `op` unless it is already running; `on_ok` applies the result and errors toast.
fn run<T, E, F, H>(ctx: AppCtx, op: Op, fut: F, on_ok: H)
where
	T: 'static,
	E: std::fmt::Display + 'static,
	F: Future<Output = Result<T, E>> + 'static,
	H: FnOnce(AppCtx, T) + 'static,
{
	run_with(ctx, op, fut, on_ok, |ctx, e| ctx.error(e));
}

/// `run` with a custom error path, for background fetches whose failure is shown in place.
fn run_with<T, E, F, H, G>(ctx: AppCtx, op: Op, fut: F, on_ok: H, on_err: G)
where
	T: 'static,
	E: std::fmt::Display + 'static,
	F: Future<Output = Result<T, E>> + 'static,
	H: FnOnce(AppCtx, T) + 'static,
	G: FnOnce(AppCtx, String) + 'static,
{
	let mut pending = ctx.pending;
	if pending.peek().has(op) {
		return;
	}
	pending.write().insert(op);
	// Not scope-bound: the request finishes even if its tab or dialog unmounts.
	spawn_forever(async move {
		let result = fut.await;
		let mut pending = ctx.pending;
		pending.write().remove(op);
		match result {
			Ok(value) => {
				mark_fetched(ctx, op);
				on_ok(ctx, value)
			},
			Err(e) => on_err(ctx, e.to_string()),
		}
	});
}

/// Remember when `op` last succeeded (drives the "updated … ago" labels).
fn mark_fetched(ctx: AppCtx, op: Op) {
	let mut data = ctx.data;
	data.write().fetched_at.insert(op, crate::format::now_secs());
}

fn busy(ctx: AppCtx, op: Op) -> bool {
	ctx.pending.peek().has(op)
}

fn client(ctx: AppCtx) -> Option<Arc<LspRestClient>> {
	ctx.client()
}

pub fn connect(ctx: AppCtx) {
	let mut conn = ctx.conn;
	let (url, api_key) = {
		let c = conn.peek();
		(c.server_url.trim().to_string(), c.api_key.clone())
	};

	#[cfg(not(target_arch = "wasm32"))]
	let built = {
		let cert_path = conn.peek().tls_cert_path.trim().to_string();
		if url.is_empty() || api_key.is_empty() || cert_path.is_empty() {
			ctx.error("Please fill in all connection fields");
			return;
		}
		let cert_data = match std::fs::read(&cert_path) {
			Ok(data) => data,
			Err(e) => {
				ctx.error(format!("Failed to read TLS cert: {}", e));
				return;
			},
		};
		LspRestClient::new(url, api_key, &cert_data)
	};

	#[cfg(target_arch = "wasm32")]
	let built = {
		if url.is_empty() || api_key.is_empty() {
			ctx.error("Please fill in server URL and API key");
			return;
		}
		// On WASM, the browser handles TLS - no certificate needed
		LspRestClient::new(url, api_key, &[])
	};

	match built {
		Ok(client) => {
			{
				let mut c = conn.write();
				c.client = Some(Arc::new(client));
				c.status = ConnectionStatus::Connected;
			}
			ctx.success("Connected");
			{
				// A new daemon may support what the last one refused.
				let mut data = ctx.data;
				let mut d = data.write();
				d.activity_unsupported = false;
				d.revenue_week_unsupported = false;
			}
			fetch_node_info(ctx);
			fetch_balances(ctx);
			fetch_channels(ctx);
			fetch_price(ctx);
			fetch_stable_channels(ctx);
			fetch_payments(ctx, false);
			fetch_forwarded_payments(ctx);
			fetch_peers(ctx);
			fetch_activity_feed(ctx, false);
			fetch_revenue_week(ctx);
		},
		Err(e) => {
			conn.write().status = ConnectionStatus::Error(e.clone());
			ctx.error(e);
		},
	}
}

pub fn disconnect(ctx: AppCtx) {
	let mut conn = ctx.conn;
	{
		let mut c = conn.write();
		c.client = None;
		c.status = ConnectionStatus::Disconnected;
	}
	let mut data = ctx.data;
	{
		let mut d = data.write();
		d.node_info = None;
		d.balances = None;
		d.channels = None;
		d.payments = None;
	}
	ctx.error("Disconnected");
}

pub fn fetch_node_info(ctx: AppCtx) {
	let Some(client) = client(ctx) else { return };
	run(ctx, Op::NodeInfo, async move { client.get_node_info(GetNodeInfoRequest {}).await }, |ctx, v| {
		let mut data = ctx.data;
		data.write().node_info = Some(v);
	});
}

pub fn fetch_balances(ctx: AppCtx) {
	let Some(client) = client(ctx) else { return };
	run(ctx, Op::Balances, async move { client.get_balances(GetBalancesRequest {}).await }, |ctx, v| {
		let mut data = ctx.data;
		data.write().balances = Some(v);
	});
}

pub fn fetch_channels(ctx: AppCtx) {
	let Some(client) = client(ctx) else { return };
	run(ctx, Op::Channels, async move { client.list_channels(ListChannelsRequest {}).await }, |ctx, v| {
		let peers: Vec<String> = v.channels.iter().map(|c| c.counterparty_node_id.clone()).collect();
		let mut data = ctx.data;
		data.write().channels = Some(v);
		resolve_aliases(ctx, peers);
	});
}

/// Apply one payments page: "Load More" appends, anything else replaces.
pub fn apply_payments_page(
	payments: &mut Option<ListPaymentsResponse>, page_token: &mut Option<String>,
	page: ListPaymentsResponse, appending: bool,
) {
	*page_token = page.next_page_token.clone();
	match payments.as_mut() {
		Some(existing) if appending => {
			existing.payments.extend(page.payments);
			existing.next_page_token = page.next_page_token;
		},
		_ => *payments = Some(page),
	}
}

/// Apply a page to `Data`, keeping the loaded-page count in step.
fn apply_page_to_data(ctx: AppCtx, page: ListPaymentsResponse, appending: bool) {
	let mut data = ctx.data;
	let mut d = data.write();
	let d = &mut *d;
	let had_pages = d.payments.is_some();
	apply_payments_page(&mut d.payments, &mut d.payments_page_token, page, appending);
	d.payments_pages = if appending && had_pages { d.payments_pages + 1 } else { 1 };
}

/// Upper bound on pages fetched by "Load all", so a misbehaving cursor cannot loop forever.
const MAX_PAYMENT_PAGES: usize = 5_000;

/// Follow empty payment pages until a visible page or the end of the history.
///
/// The REST server filters some upstream payment rows, so a page can be empty while still
/// carrying a cursor. A single Load More click should not make the user click through those
/// pages one at a time. Keep this separate from `load_all_payments`: Load All has its own
/// progress and pagination behavior.
async fn fetch_page_until_visible<F, Fut>(
	mut page_token: Option<String>,
	mut fetch: F,
) -> Result<ListPaymentsResponse, String>
where
	F: FnMut(Option<String>) -> Fut,
	Fut: Future<Output = Result<ListPaymentsResponse, String>>,
{
	let mut seen = HashSet::new();
	loop {
		if let Some(token) = &page_token {
			if !seen.insert(token.clone()) {
				return Err("Stopped loading payments: the server repeated a page token".to_string());
			}
		}

		let page = fetch(page_token.clone()).await?;
		if !page.payments.is_empty() || page.next_page_token.is_none() {
			return Ok(page);
		}
		page_token = page.next_page_token;
	}
}

/// Fetch every remaining payments page so search and sorting cover the full history.
pub fn load_all_payments(ctx: AppCtx) {
	if busy(ctx, Op::Payments) || busy(ctx, Op::PaymentsAll) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let mut pending = ctx.pending;
	pending.write().insert(Op::PaymentsAll);
	spawn_forever(async move {
		let mut data = ctx.data;
		let mut seen = HashSet::new();
		let mut fetched = 0usize;
		let error = loop {
			let token = data.peek().payments_page_token.clone();
			let Some(token) = token else { break None };
			if fetched >= MAX_PAYMENT_PAGES {
				break Some(format!("Stopped loading payments after {MAX_PAYMENT_PAGES} pages"));
			}
			if !seen.insert(token.clone()) {
				break Some("Stopped loading payments: the server repeated a page token".to_string());
			}
			match client.list_payments(ListPaymentsRequest { page_token: Some(token) }).await {
				Ok(page) => {
					apply_page_to_data(ctx, page, true);
					fetched += 1;
					data.write().payments_load_all_progress = Some(fetched);
				},
				Err(e) => break Some(e.to_string()),
			}
		};
		data.write().payments_load_all_progress = None;
		let mut pending = ctx.pending;
		pending.write().remove(Op::PaymentsAll);
		match error {
			Some(e) => ctx.error(e),
			None => {
				mark_fetched(ctx, Op::Payments);
				let total = data.peek().payments.as_ref().map(|p| p.payments.len()).unwrap_or(0);
				ctx.success(format!("Loaded all {total} payments"));
			},
		}
	});
}

/// Fetch the first payments page, or the next one when `appending` ("Load More").
pub fn fetch_payments(ctx: AppCtx, appending: bool) {
	if !busy(ctx, Op::Payments) && !busy(ctx, Op::PaymentsAll) {
		if let Some(client) = client(ctx) {
			let page_token = if appending { ctx.data.peek().payments_page_token.clone() } else { None };
			if appending {
				run(
					ctx,
					Op::Payments,
					async move {
						fetch_page_until_visible(page_token, |page_token| {
							let client = Arc::clone(&client);
							async move {
								client
									.list_payments(ListPaymentsRequest { page_token })
									.await
									.map_err(|e| e.to_string())
							}
						})
						.await
					},
					move |ctx, v| apply_page_to_data(ctx, v, true),
				);
			} else {
				run(
					ctx,
					Op::Payments,
					async move { client.list_payments(ListPaymentsRequest { page_token }).await },
					move |ctx, v| apply_page_to_data(ctx, v, false),
				);
			}
		}
	}
	fetch_settlement_payments(ctx);
}

pub fn fetch_peers(ctx: AppCtx) {
	let Some(client) = client(ctx) else { return };
	run(ctx, Op::Peers, async move { client.list_peers(ListPeersRequest {}).await }, |ctx, v| {
		let peers: Vec<String> = v.peers.iter().map(|p| p.node_id.clone()).collect();
		let mut data = ctx.data;
		data.write().peers = Some(v);
		resolve_aliases(ctx, peers);
	});
}

pub fn fetch_forwarded_payments(ctx: AppCtx) {
	let Some(client) = client(ctx) else { return };
	run(
		ctx,
		Op::ForwardedPayments,
		async move {
			client.list_forwarded_payments(ListForwardedPaymentsRequest { page_token: None }).await
		},
		|ctx, v| {
			// Peer names for both legs of every forward.
			let nodes: Vec<String> = v
				.forwarded_payments
				.iter()
				.flat_map(|fp| [fp.prev_node_id.clone(), fp.next_node_id.clone()])
				.flatten()
				.collect();
			let mut data = ctx.data;
			{
				let mut d = data.write();
				d.forwarded_payments_page_token = v.next_page_token.clone();
				d.forwarded_payments = Some(v);
			}
			resolve_aliases(ctx, nodes);
		},
	);
}

fn log_lines(input: &str) -> u32 {
	input.parse::<u32>().unwrap_or(200)
}

pub fn fetch_ldk_log(ctx: AppCtx) {
	let Some(client) = client(ctx) else { return };
	let max_lines = log_lines(&ctx.forms.peek().ldk_log.max_lines);
	run(
		ctx,
		Op::LdkLog,
		async move { client.ldk_log(LogRequest { max_lines, filter: String::new(), full: false }).await },
		|ctx, v| {
			let mut data = ctx.data;
			data.write().ldk_log = Some(v);
		},
	);
}

pub fn fetch_audit_log(ctx: AppCtx) {
	let Some(client) = client(ctx) else { return };
	let max_lines = log_lines(&ctx.forms.peek().audit_log.max_lines);
	run(
		ctx,
		Op::AuditLog,
		async move { client.audit_log(LogRequest { max_lines, filter: String::new(), full: false }).await },
		|ctx, v| {
			let mut data = ctx.data;
			data.write().audit_log = Some(v);
		},
	);
}

/// Drop loaded ledger data and make every in-flight ledger request stale.
pub fn invalidate_channel_ledger(ctx: AppCtx) {
	let mut gen = ctx.ledger_gen;
	let next = *gen.peek() + 1;
	gen.set(next);
	let mut pending = ctx.pending;
	{
		let mut p = pending.write();
		p.remove(Op::ChannelLedger);
		p.remove(Op::ChannelLedgerExport);
	}
	let mut data = ctx.data;
	let mut d = data.write();
	d.channel_ledger = None;
	d.channel_ledger_cursor = None;
}

/// Like [`run`], but the result is dropped (and pending left alone) once the ledger generation moved on.
fn run_ledger<T, F, H>(ctx: AppCtx, op: Op, fut: F, on_ok: H)
where
	T: 'static,
	F: Future<Output = Result<T, String>> + 'static,
	H: FnOnce(AppCtx, T) + 'static,
{
	let mut pending = ctx.pending;
	if pending.peek().has(op) {
		return;
	}
	pending.write().insert(op);
	let gen = *ctx.ledger_gen.peek();
	spawn_forever(async move {
		let result = fut.await;
		if *ctx.ledger_gen.peek() != gen {
			return;
		}
		let mut pending = ctx.pending;
		pending.write().remove(op);
		match result {
			Ok(value) => on_ok(ctx, value),
			Err(e) => ctx.error(e),
		}
	});
}

/// Query one newest-selected ledger page (or the next older one when `appending`); each page is chronological.
/// Channel history asks for linked, state-only rows; the technical view's filters apply only while it is shown.
pub(crate) fn ledger_request(form: &ChannelLedgerForm, cursor: String, page_size: u32) -> ListChannelLedgerEventsRequest {
	let technical = form.show_technical;
	let filter = |value: &String| if technical { value.clone() } else { String::new() };
	ListChannelLedgerEventsRequest {
		identifier: form.identifier.trim().to_owned(),
		category: filter(&form.category),
		status: filter(&form.status),
		completeness: filter(&form.completeness),
		cursor,
		page_size,
		include_linked: true,
		state_changes_only: !technical,
		all_channels: false,
	}
}

/// The Overview feed: newest state events across every channel, or the next older page when `more`.
/// An older daemon rejects the request; that is shown on the card, never toasted.
pub fn fetch_activity_feed(ctx: AppCtx, more: bool) {
	let Some(client) = client(ctx) else { return };
	if ctx.data.peek().activity_unsupported {
		return;
	}
	let cursor = if more { ctx.data.peek().activity_cursor.clone() } else { None };
	if more && cursor.is_none() {
		return;
	}
	let request = ListChannelLedgerEventsRequest {
		all_channels: true,
		cursor: cursor.unwrap_or_default(),
		page_size: 100,
		..Default::default()
	};
	run_with(
		ctx,
		Op::ActivityFeed,
		async move { client.list_channel_ledger_events(request).await },
		move |ctx, v: ListChannelLedgerEventsResponse| {
			let mut data = ctx.data;
			let mut d = data.write();
			d.activity_error = None;
			d.activity_cursor = v.next_cursor.clone();
			if more {
				let mut combined = d.activity.take().unwrap_or_default();
				merge_ledger_events(&mut combined.events, v.events);
				combined.next_cursor = v.next_cursor;
				d.activity = Some(combined);
				d.activity_pages += 1;
			} else {
				d.activity = Some(v);
				d.activity_pages = 1;
			}
		},
		|ctx, e| {
			let mut data = ctx.data;
			let mut d = data.write();
			d.activity_unsupported = crate::dashboard::needs_newer_daemon(&e);
			d.activity_error = Some(e);
		},
	);
}

/// Revenue lines for the last 7 days (the "Net this week" tile); no activity rows are needed.
pub fn fetch_revenue_week(ctx: AppCtx) {
	let Some(client) = client(ctx) else { return };
	if ctx.data.peek().revenue_week_unsupported {
		return;
	}
	let since = crate::format::now_secs() as i64 - 7 * 86_400;
	let request = GetRevenueRequest { since, categories: Vec::new(), cursor: None, limit: 1 };
	run_with(
		ctx,
		Op::RevenueWeek,
		async move { client.get_revenue(request).await },
		|ctx, v| {
			let mut data = ctx.data;
			let mut d = data.write();
			d.revenue_week_error = None;
			d.revenue_week = Some(v);
		},
		|ctx, e| {
			let mut data = ctx.data;
			let mut d = data.write();
			d.revenue_week_unsupported = crate::dashboard::needs_newer_daemon(&e);
			d.revenue_week_error = Some(e);
		},
	);
}

pub fn fetch_channel_ledger(ctx: AppCtx, appending: bool) {
	let Some(client) = client(ctx) else { return };
	let form = ctx.forms.peek().channel_ledger.clone();
	let request = ChannelLedgerRequestKey::from(&form);
	let cursor =
		if appending { ctx.data.peek().channel_ledger_cursor.clone().unwrap_or_default() } else { String::new() };
	run_ledger(
		ctx,
		Op::ChannelLedger,
		async move {
			client
				.list_channel_ledger_events(ledger_request(&form, cursor, 50))
				.await
				.map_err(|e| e.to_string())
		},
		move |ctx, v: ListChannelLedgerEventsResponse| {
			if request != ChannelLedgerRequestKey::from(&ctx.forms.peek().channel_ledger) {
				return;
			}
			let mut data = ctx.data;
			let mut d = data.write();
			d.channel_ledger_cursor = v.next_cursor.clone();
			if appending {
				let mut combined = d.channel_ledger.take().unwrap_or_default();
				merge_ledger_events(&mut combined.events, v.events);
				combined.next_cursor = v.next_cursor;
				combined.overview = v.overview.or(combined.overview);
				d.channel_ledger = Some(combined);
			} else {
				d.channel_ledger = Some(v);
			}
		},
	);
}

/// Fetch every page matching the current exact filters, then save it as JSONL.
/// Export never depends on which pages happen to be loaded.
pub fn export_channel_ledger(ctx: AppCtx) {
	let Some(client) = client(ctx) else { return };
	let form = ctx.forms.peek().channel_ledger.clone();
	let request = ChannelLedgerRequestKey::from(&form);
	run_ledger(
		ctx,
		Op::ChannelLedgerExport,
		async move {
			let mut cursor = String::new();
			let mut seen_cursors = HashSet::new();
			let mut events = Vec::new();
			let mut overview = None;
			loop {
				let page = client
					.list_channel_ledger_events(ledger_request(&form, cursor.clone(), 200))
					.await
					.map_err(|error| error.to_string())?;
				if overview.is_none() {
					overview = page.overview.clone();
				}
				merge_ledger_events(&mut events, page.events);
				match checked_next_cursor(&cursor, page.next_cursor, &mut seen_cursors)? {
					Some(next) => cursor = next,
					None => break,
				}
			}
			Ok(ListChannelLedgerEventsResponse { events, next_cursor: None, overview })
		},
		move |ctx, history| {
			if request == ChannelLedgerRequestKey::from(&ctx.forms.peek().channel_ledger) {
				save_ledger_jsonl(ctx, history);
			}
		},
	);
}

fn save_ledger_jsonl(ctx: AppCtx, history: ListChannelLedgerEventsResponse) {
	let content = crate::ledger::history_jsonl(&history);
	let count = history.events.len();
	save_export(ctx, "channel-ledger.jsonl", "application/x-ndjson", content, format!("{count} matching events"));
}

/// Save an export: native save dialog, or a browser download on the web.
pub fn save_export(ctx: AppCtx, file_name: &'static str, mime: &'static str, content: String, what: String) {
	#[cfg(not(target_arch = "wasm32"))]
	{
		let _ = mime;
		let mut pending = ctx.pending;
		if pending.peek().has(Op::Export) {
			return;
		}
		pending.write().insert(Op::Export);
		spawn_forever(async move {
			if let Some(path) = crate::platform::save_file(&[], file_name, None).await {
				match std::fs::write(&path, content) {
					Ok(()) => ctx.success(format!("Exported {} to {}", what, path.display())),
					Err(error) => ctx.error(format!("Export failed: {error}")),
				}
			}
			let mut pending = ctx.pending;
			pending.write().remove(Op::Export);
		});
	}

	#[cfg(target_arch = "wasm32")]
	match crate::platform::download_text(file_name, mime, &content) {
		Ok(()) => ctx.success(format!("Exported {} to {}", what, file_name)),
		Err(error) => ctx.error(format!("Export failed: {error}")),
	}
}

/// Open the ledger from an operator-facing stable-channel row using its
/// splice-stable user_channel_id and a clean filter set.
pub fn open_channel_ledger(ctx: AppCtx, user_channel_id: String) {
	invalidate_channel_ledger(ctx);
	let mut forms = ctx.forms;
	forms.write().channel_ledger = ChannelLedgerForm { identifier: user_channel_id, ..Default::default() };
	let mut nav = ctx.nav;
	{
		let mut n = nav.write();
		n.logs_tab = LogsTab::ChannelLedger;
		n.active_tab = ActiveTab::Logs;
	}
	fetch_channel_ledger(ctx, false);
}

pub fn open_payment_details(ctx: AppCtx, payment_id: String) {
	let mut data = ctx.data;
	data.write().payment_details = None;
	let mut drawer = ctx.drawer;
	drawer.set(Some(Drawer::Payment(payment_id.clone())));
	let Some(client) = client(ctx) else { return };
	let id = payment_id.clone();
	run(
		ctx,
		Op::PaymentDetails,
		async move { client.get_payment_details(GetPaymentDetailsRequest { payment_id }).await },
		move |ctx, v| {
			// A slow reply for a row the user has since left must not land under the newer panel.
			if ctx.drawer.peek().as_ref() != Some(&Drawer::Payment(id)) {
				return;
			}
			let mut data = ctx.data;
			data.write().payment_details = Some(v);
		},
	);
}

pub fn generate_onchain_address(ctx: AppCtx) {
	let Some(client) = client(ctx) else { return };
	run(ctx, Op::OnchainReceive, async move { client.onchain_receive(OnchainReceiveRequest {}).await }, |ctx, v| {
		let mut results = ctx.results;
		results.write().onchain_address = Some(v.address);
		ctx.success("Address generated");
	});
}

/// Validate the on-chain send form into a request (errors go to the toast).
pub fn prepare_onchain(ctx: AppCtx) -> Option<OnchainSendRequest> {
	let form = ctx.forms.peek().onchain_send.clone();
	let address = form.address.trim().to_string();
	let amount = onchain_send_amount(ctx.parse_amount_sats(&form.amount_sats), form.send_all);
	let fee_rate_sat_per_vb = form.fee_rate_sat_per_vb.trim().parse::<u64>().ok();
	if address.is_empty() {
		ctx.error("Address is required");
		return None;
	}
	if amount.is_none() {
		ctx.error("Invalid amount");
		return None;
	}
	Some(OnchainSendRequest { address, amount, fee_rate_sat_per_vb })
}

// "Send entire balance" wins over a typed amount; LDK Server rejects a request with neither.
fn onchain_send_amount(amount_sats: Option<u64>, send_all: bool) -> Option<onchain_send_request::Amount> {
	if send_all {
		Some(onchain_send_request::Amount::AllFunds(AllFunds {}))
	} else {
		amount_sats.map(onchain_send_request::Amount::AmountSats)
	}
}

pub fn send_onchain(ctx: AppCtx) {
	if busy(ctx, Op::OnchainSend) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let Some(request) = prepare_onchain(ctx) else { return };
	run(ctx, Op::OnchainSend, async move { client.onchain_send(request).await }, |ctx, v| {
		let mut results = ctx.results;
		results.write().last_txid = Some(v.txid.clone());
		ctx.success(format!("Sent! TXID: {}", v.txid));
		let mut forms = ctx.forms;
		forms.write().onchain_send = Default::default();
		let mut view = ctx.view;
		view.write().send_all_confirm = false;
		close_dialog(ctx);
		fetch_balances(ctx);
	});
}

pub fn generate_bolt11_invoice(ctx: AppCtx) {
	if busy(ctx, Op::Bolt11Receive) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let form = ctx.forms.peek().bolt11_receive.clone();
	let amount_msat = ctx.parse_amount_msat(&form.amount_msat);
	let description = form.description.trim().to_string();
	let expiry_secs = form.expiry_secs.trim().parse::<u32>().unwrap_or(86400);
	let description = if !description.is_empty() {
		Some(Bolt11InvoiceDescription { kind: Some(bolt11_invoice_description::Kind::Direct(description)) })
	} else {
		None
	};
	run(
		ctx,
		Op::Bolt11Receive,
		async move { client.bolt11_receive(Bolt11ReceiveRequest { amount_msat, description, expiry_secs }).await },
		|ctx, v| {
			let mut results = ctx.results;
			results.write().generated_invoice = Some(v.invoice);
			ctx.success("Invoice generated");
		},
	);
}

pub fn prepare_bolt11(ctx: AppCtx) -> Option<Bolt11SendRequest> {
	let form = ctx.forms.peek().bolt11_send.clone();
	let invoice = form.invoice.trim().to_string();
	let amount_msat = ctx.parse_amount_msat(&form.amount_msat);
	if invoice.is_empty() {
		ctx.error("Invoice is required");
		return None;
	}
	Some(Bolt11SendRequest { invoice, amount_msat, route_parameters: None })
}

pub fn send_bolt11(ctx: AppCtx) {
	if busy(ctx, Op::Bolt11Send) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let Some(request) = prepare_bolt11(ctx) else { return };
	run(ctx, Op::Bolt11Send, async move { client.bolt11_send(request).await }, |ctx, v| {
		let mut results = ctx.results;
		results.write().last_payment_id = Some(v.payment_id.clone());
		ctx.success(format!("Payment sent! ID: {}", v.payment_id));
		let mut forms = ctx.forms;
		forms.write().bolt11_send = Default::default();
		close_dialog(ctx);
	});
}

pub fn generate_bolt12_offer(ctx: AppCtx) {
	if busy(ctx, Op::Bolt12Receive) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let form = ctx.forms.peek().bolt12_receive.clone();
	let description = form.description.trim().to_string();
	let amount_msat = ctx.parse_amount_msat(&form.amount_msat);
	let expiry_secs = form.expiry_secs.trim().parse::<u32>().ok();
	let quantity = form.quantity.trim().parse::<u64>().ok();
	if description.is_empty() {
		ctx.error("Description is required");
		return;
	}
	run(
		ctx,
		Op::Bolt12Receive,
		async move {
			client.bolt12_receive(Bolt12ReceiveRequest { description, amount_msat, expiry_secs, quantity }).await
		},
		|ctx, v| {
			let mut results = ctx.results;
			results.write().generated_offer = Some(v.offer);
			ctx.success("Offer generated");
		},
	);
}

pub fn prepare_bolt12(ctx: AppCtx) -> Option<Bolt12SendRequest> {
	let form = ctx.forms.peek().bolt12_send.clone();
	let offer = form.offer.trim().to_string();
	let amount_msat = ctx.parse_amount_msat(&form.amount_msat);
	let quantity = form.quantity.trim().parse::<u64>().ok();
	let payer_note =
		if form.payer_note.trim().is_empty() { None } else { Some(form.payer_note.trim().to_string()) };
	if offer.is_empty() {
		ctx.error("Offer is required");
		return None;
	}
	Some(Bolt12SendRequest { offer, amount_msat, quantity, payer_note, route_parameters: None })
}

pub fn send_bolt12(ctx: AppCtx) {
	if busy(ctx, Op::Bolt12Send) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let Some(request) = prepare_bolt12(ctx) else { return };
	run(ctx, Op::Bolt12Send, async move { client.bolt12_send(request).await }, |ctx, v| {
		let mut results = ctx.results;
		results.write().last_payment_id = Some(v.payment_id.clone());
		ctx.success(format!("Payment sent! ID: {}", v.payment_id));
		let mut forms = ctx.forms;
		forms.write().bolt12_send = Default::default();
		close_dialog(ctx);
	});
}

pub fn build_channel_config(fee_prop: &str, fee_base: &str, cltv: &str) -> Option<ChannelConfig> {
	let fee_prop = fee_prop.parse::<u32>().ok();
	let fee_base = fee_base.parse::<u32>().ok();
	let cltv = cltv.parse::<u32>().ok();

	if fee_prop.is_none() && fee_base.is_none() && cltv.is_none() {
		return None;
	}

	Some(ChannelConfig {
		forwarding_fee_proportional_millionths: fee_prop,
		forwarding_fee_base_msat: fee_base,
		cltv_expiry_delta: cltv,
		force_close_avoidance_max_fee_satoshis: None,
		accept_underpaying_htlcs: None,
		max_dust_htlc_exposure: None,
	})
}

fn close_dialog(ctx: AppCtx) {
	let mut dialog = ctx.dialog;
	dialog.set(None);
}

pub fn open_channel(ctx: AppCtx) {
	if busy(ctx, Op::OpenChannel) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let form = ctx.forms.peek().open_channel.clone();
	let node_pubkey = form.node_pubkey.trim().to_string();
	let address = form.address.trim().to_string();
	let Some(channel_amount_sats) = ctx.parse_amount_sats(&form.channel_amount_sats) else {
		ctx.error("Invalid channel amount");
		return;
	};
	let push_to_counterparty_msat = ctx.parse_amount_msat(&form.push_to_counterparty_msat);
	let announce_channel = form.announce_channel;
	let channel_config = build_channel_config(
		form.forwarding_fee_proportional_millionths.trim(),
		form.forwarding_fee_base_msat.trim(),
		form.cltv_expiry_delta.trim(),
	);
	if node_pubkey.is_empty() || address.is_empty() {
		ctx.error("Node pubkey and address are required");
		return;
	}
	run(
		ctx,
		Op::OpenChannel,
		async move {
			client
				.open_channel(OpenChannelRequest {
					node_pubkey,
					address,
					amount: Some(open_channel_request::Amount::ChannelAmountSats(channel_amount_sats)),
					push_to_counterparty_msat,
					channel_config,
					announce_channel,
					disable_counterparty_reserve: false,
				})
				.await
		},
		|ctx, v| {
			ctx.success(format!("Channel opened! ID: {}", v.user_channel_id));
			let mut forms = ctx.forms;
			forms.write().open_channel = Default::default();
			close_dialog(ctx);
			fetch_channels(ctx);
		},
	);
}

fn channel_ids(user_channel_id: &str, counterparty_node_id: &str) -> Option<(String, String)> {
	let user_channel_id = user_channel_id.trim().to_string();
	let counterparty_node_id = counterparty_node_id.trim().to_string();
	(!user_channel_id.is_empty() && !counterparty_node_id.is_empty()).then_some((user_channel_id, counterparty_node_id))
}

const CHANNEL_IDS_REQUIRED: &str = "Channel ID and counterparty node ID are required";

pub fn close_channel(ctx: AppCtx) {
	if busy(ctx, Op::CloseChannel) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let form = ctx.forms.peek().close_channel.clone();
	let Some((user_channel_id, counterparty_node_id)) =
		channel_ids(&form.user_channel_id, &form.counterparty_node_id)
	else {
		ctx.error(CHANNEL_IDS_REQUIRED);
		return;
	};
	run(
		ctx,
		Op::CloseChannel,
		async move { client.close_channel(CloseChannelRequest { user_channel_id, counterparty_node_id }).await },
		|ctx, _v| {
			ctx.success("Channel close initiated");
			let mut forms = ctx.forms;
			forms.write().close_channel = Default::default();
			close_dialog(ctx);
			fetch_channels(ctx);
		},
	);
}

pub fn force_close_channel(ctx: AppCtx) {
	if busy(ctx, Op::ForceCloseChannel) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let form = ctx.forms.peek().close_channel.clone();
	let force_close_reason = if form.force_close_reason.trim().is_empty() {
		None
	} else {
		Some(form.force_close_reason.trim().to_string())
	};
	let Some((user_channel_id, counterparty_node_id)) =
		channel_ids(&form.user_channel_id, &form.counterparty_node_id)
	else {
		ctx.error(CHANNEL_IDS_REQUIRED);
		return;
	};
	run(
		ctx,
		Op::ForceCloseChannel,
		async move {
			client
				.force_close_channel(ForceCloseChannelRequest {
					user_channel_id,
					counterparty_node_id,
					force_close_reason,
				})
				.await
		},
		|ctx, _v| {
			ctx.success("Force close initiated");
			let mut forms = ctx.forms;
			forms.write().close_channel = Default::default();
			close_dialog(ctx);
			fetch_channels(ctx);
		},
	);
}

pub fn splice_in(ctx: AppCtx) {
	if busy(ctx, Op::SpliceIn) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let form = ctx.forms.peek().splice_in.clone();
	let Some(splice_amount_sats) = ctx.parse_amount_sats(&form.splice_amount_sats) else {
		ctx.error("Invalid splice amount");
		return;
	};
	let Some((user_channel_id, counterparty_node_id)) =
		channel_ids(&form.user_channel_id, &form.counterparty_node_id)
	else {
		ctx.error(CHANNEL_IDS_REQUIRED);
		return;
	};
	run(
		ctx,
		Op::SpliceIn,
		async move {
			client
				.splice_in(SpliceInRequest {
					user_channel_id,
					counterparty_node_id,
					amount: Some(splice_in_request::Amount::SpliceAmountSats(splice_amount_sats)),
				})
				.await
		},
		|ctx, _v| {
			ctx.success("Splice-in initiated");
			let mut forms = ctx.forms;
			forms.write().splice_in = Default::default();
			close_dialog(ctx);
			fetch_channels(ctx);
		},
	);
}

pub fn splice_out(ctx: AppCtx) {
	if busy(ctx, Op::SpliceOut) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let form = ctx.forms.peek().splice_out.clone();
	let Some(splice_amount_sats) = ctx.parse_amount_sats(&form.splice_amount_sats) else {
		ctx.error("Invalid splice amount");
		return;
	};
	let address = if form.address.trim().is_empty() { None } else { Some(form.address.trim().to_string()) };
	let Some((user_channel_id, counterparty_node_id)) =
		channel_ids(&form.user_channel_id, &form.counterparty_node_id)
	else {
		ctx.error(CHANNEL_IDS_REQUIRED);
		return;
	};
	run(
		ctx,
		Op::SpliceOut,
		async move {
			client
				.splice_out(SpliceOutRequest { user_channel_id, counterparty_node_id, address, splice_amount_sats })
				.await
		},
		|ctx, v| {
			ctx.success(format!("Splice-out initiated to {}", v.address));
			let mut forms = ctx.forms;
			forms.write().splice_out = Default::default();
			close_dialog(ctx);
			fetch_channels(ctx);
		},
	);
}

pub fn update_channel_config(ctx: AppCtx) {
	if busy(ctx, Op::UpdateChannelConfig) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let form = ctx.forms.peek().update_channel_config.clone();
	let channel_config = ChannelConfig {
		forwarding_fee_proportional_millionths: form.forwarding_fee_proportional_millionths.trim().parse().ok(),
		forwarding_fee_base_msat: form.forwarding_fee_base_msat.trim().parse().ok(),
		cltv_expiry_delta: form.cltv_expiry_delta.trim().parse().ok(),
		force_close_avoidance_max_fee_satoshis: None,
		accept_underpaying_htlcs: None,
		max_dust_htlc_exposure: None,
	};
	let Some((user_channel_id, counterparty_node_id)) =
		channel_ids(&form.user_channel_id, &form.counterparty_node_id)
	else {
		ctx.error(CHANNEL_IDS_REQUIRED);
		return;
	};
	run(
		ctx,
		Op::UpdateChannelConfig,
		async move {
			client
				.update_channel_config(UpdateChannelConfigRequest {
					user_channel_id,
					counterparty_node_id,
					channel_config: Some(channel_config),
				})
				.await
		},
		|ctx, _v| {
			ctx.success("Channel config updated");
			let mut forms = ctx.forms;
			forms.write().update_channel_config = Default::default();
			close_dialog(ctx);
			fetch_channels(ctx);
		},
	);
}

pub fn connect_peer(ctx: AppCtx) {
	if busy(ctx, Op::ConnectPeer) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let form = ctx.forms.peek().connect_peer.clone();
	let node_pubkey = form.node_pubkey.trim().to_string();
	let address = form.address.trim().to_string();
	let persist = form.persist;
	if node_pubkey.is_empty() || address.is_empty() {
		ctx.error("Node pubkey and address are required");
		return;
	}
	run(
		ctx,
		Op::ConnectPeer,
		async move { client.connect_peer(ConnectPeerRequest { node_pubkey, address, persist }).await },
		|ctx, _v| {
			ctx.success("Peer connected successfully");
			let mut forms = ctx.forms;
			forms.write().connect_peer = Default::default();
			close_dialog(ctx);
			fetch_peers(ctx);
		},
	);
}

/// The 5s price poll doubles as a connectivity heartbeat: a successful GetPrice means the
/// LSP is reachable, an error means it is not — so the status badge reflects reality, not
/// just whether the client object was built. Recovers automatically when the LSP returns.
pub fn fetch_price(ctx: AppCtx) {
	let Some(client) = client(ctx) else { return };
	let mut pending = ctx.pending;
	if pending.peek().has(Op::GetPrice) {
		return;
	}
	pending.write().insert(Op::GetPrice);
	spawn_forever(async move {
		let result = client.get_price(GetPriceRequest {}).await;
		let mut pending = ctx.pending;
		pending.write().remove(Op::GetPrice);
		let mut conn = ctx.conn;
		// A disconnect while the request was in flight wins.
		if conn.peek().client.is_none() {
			return;
		}
		match result {
			Ok(v) => {
				let mut price = ctx.price;
				price.set(Some(v));
				if conn.peek().status != ConnectionStatus::Connected {
					conn.write().status = ConnectionStatus::Connected;
				}
			},
			Err(e) => {
				conn.write().status = ConnectionStatus::Error(format!("LSP unreachable: {}", e));
			},
		}
	});
}

pub fn fetch_stable_channels(ctx: AppCtx) {
	let Some(client) = client(ctx) else { return };
	run(
		ctx,
		Op::ListStableChannels,
		async move { client.list_stable_channels(ListStableChannelsRequest {}).await },
		|ctx, v| {
			let peers: Vec<String> = v.channels.iter().map(|c| c.counterparty.clone()).collect();
			let mut data = ctx.data;
			data.write().stable_channels = Some(v);
			resolve_aliases(ctx, peers);
		},
	);
}

/// Loads the revenue summary and the first activity page, or the next page when `more`.
pub fn fetch_revenue(ctx: AppCtx, more: bool) {
	let Some(client) = client(ctx) else { return };
	let (window, categories) = {
		let view = ctx.view.peek();
		(view.revenue_window, view.revenue_categories.clone())
	};
	let cursor = if more { ctx.data.peek().revenue_cursor.clone() } else { None };
	if more && cursor.is_none() {
		return;
	}
	let since = crate::ui::revenue::window_since(window, crate::format::now_secs() as i64, crate::ui::revenue::local_midnight());
	let request = GetRevenueRequest { since, categories: categories.clone(), cursor, limit: 50 };
	run(ctx, Op::GetRevenue, async move { client.get_revenue(request).await }, move |ctx, v| {
		// A window or filter click dropped while this was in flight: fetch the current selection instead.
		if crate::ui::revenue::selection_moved(window, &categories, &ctx.view.peek()) {
			fetch_revenue(ctx, false);
			return;
		}
		let peers: Vec<String> = v.items.iter().map(|i| i.node_id.clone()).filter(|n| !n.is_empty()).collect();
		let mut data = ctx.data;
		{
			let mut d = data.write();
			if more {
				d.revenue_items.extend(v.items.iter().cloned());
				d.revenue_pages += 1;
			} else {
				d.revenue_items = v.items.clone();
				d.revenue_pages = 1;
			}
			d.revenue_cursor = v.next_cursor.clone();
			d.revenue = Some(v);
		}
		resolve_aliases(ctx, peers);
	});
}

/// Sends the refund the dialog describes; the daemon refuses a second refund of the same fee.
pub fn refund_trade_fee(ctx: AppCtx) {
	if busy(ctx, Op::RefundTradeFee) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let trade_payment_id = ctx.forms.peek().refund_trade_fee.trade_payment_id.clone();
	if trade_payment_id.is_empty() {
		return;
	}
	run(
		ctx,
		Op::RefundTradeFee,
		async move { client.refund_trade_fee(RefundTradeFeeRequest { trade_payment_id }).await },
		|ctx, v| {
			ctx.success(format!("Refund of {} sent", ctx.fmt_msat(v.amount_msat)));
			let mut forms = ctx.forms;
			forms.write().refund_trade_fee = Default::default();
			close_dialog(ctx);
			fetch_revenue(ctx, false);
		},
	);
}

pub fn edit_stable_channel(ctx: AppCtx) {
	if busy(ctx, Op::EditStableChannel) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let form = ctx.forms.peek().edit_stable_channel.clone();
	let channel_id = form.channel_id.trim().to_string();
	let expected_usd = form.expected_usd.trim().parse::<f64>().ok();
	let note = if form.note.trim().is_empty() { None } else { Some(form.note.trim().to_string()) };
	if channel_id.is_empty() {
		ctx.error("Channel ID is required");
		return;
	}
	if !form.expected_usd.trim().is_empty() && expected_usd.is_none() {
		ctx.error("Target USD must be a number");
		return;
	}
	run(
		ctx,
		Op::EditStableChannel,
		async move { client.edit_stable_channel(EditStableChannelRequest { channel_id, expected_usd, note }).await },
		|ctx, v| {
			if v.ok {
				ctx.success(v.status);
				let mut forms = ctx.forms;
				forms.write().edit_stable_channel = Default::default();
				// Refresh the table so the new target shows immediately.
				fetch_stable_channels(ctx);
			} else {
				ctx.error(v.status);
			}
		},
	);
}

pub fn fetch_settlement_payments(ctx: AppCtx) {
	let Some(client) = client(ctx) else { return };
	run(
		ctx,
		Op::ListSettlementPayments,
		async move { client.list_settlement_payments(ListSettlementPaymentsRequest {}).await },
		|ctx, v| {
			let mut data = ctx.data;
			data.write().settlement_kinds = Some(
				v.settlements
					.into_iter()
					.filter_map(|p| SettlementKind::parse(&p.kind).map(|k| (p.payment_id, k)))
					.collect(),
			);
		},
	);
}

pub fn disconnect_peer(ctx: AppCtx, node_pubkey: String) {
	let Some(client) = client(ctx) else { return };
	run(
		ctx,
		Op::DisconnectPeer,
		async move { client.disconnect_peer(DisconnectPeerRequest { node_pubkey }).await },
		|ctx, _v| {
			ctx.success("Peer disconnected");
			fetch_peers(ctx);
		},
	);
}

pub fn prepare_keysend(ctx: AppCtx) -> Option<SpontaneousSendRequest> {
	let form = ctx.forms.peek().spontaneous_send.clone();
	let Some(amount_msat) = ctx.parse_amount_msat(&form.amount_msat) else {
		ctx.error("Invalid amount");
		return None;
	};
	let node_id = form.node_id.trim().to_string();
	if node_id.is_empty() {
		ctx.error("Node ID is required");
		return None;
	}
	Some(SpontaneousSendRequest { amount_msat, node_id, route_parameters: None, custom_tlvs: vec![], preimage: None })
}

pub fn spontaneous_send(ctx: AppCtx) {
	if busy(ctx, Op::SpontaneousSend) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let Some(request) = prepare_keysend(ctx) else { return };
	run(ctx, Op::SpontaneousSend, async move { client.spontaneous_send(request).await }, |ctx, v| {
		let mut results = ctx.results;
		results.write().last_payment_id = Some(v.payment_id.clone());
		ctx.success(format!("Keysend sent! ID: {}", v.payment_id));
		let mut forms = ctx.forms;
		forms.write().spontaneous_send = Default::default();
		close_dialog(ctx);
	});
}

/// Validate a send form and, if it is complete, open the review step.
pub fn review_send(ctx: AppCtx, kind: SendKind) {
	let ready = match kind {
		SendKind::Bolt11 => prepare_bolt11(ctx).is_some(),
		SendKind::Bolt12 => prepare_bolt12(ctx).is_some(),
		SendKind::Keysend => prepare_keysend(ctx).is_some(),
		SendKind::Onchain => prepare_onchain(ctx).is_some(),
	};
	if ready {
		if kind == SendKind::Keysend {
			let node_id = ctx.forms.peek().spontaneous_send.node_id.trim().to_string();
			resolve_aliases(ctx, [node_id]);
		}
		let mut dialog = ctx.dialog;
		dialog.set(Some(Dialog::ConfirmSend(kind)));
	}
}

/// Send the payment the review step is showing.
pub fn confirm_send(ctx: AppCtx, kind: SendKind) {
	match kind {
		SendKind::Bolt11 => send_bolt11(ctx),
		SendKind::Bolt12 => send_bolt12(ctx),
		SendKind::Keysend => spontaneous_send(ctx),
		SendKind::Onchain => send_onchain(ctx),
	}
}

/// Look up gossip aliases for `node_ids` in the background (each node once per session).
pub fn resolve_aliases(ctx: AppCtx, node_ids: impl IntoIterator<Item = String>) {
	let mut data = ctx.data;
	{
		let mut d = data.write();
		for id in node_ids {
			if !id.is_empty() && !d.aliases.contains_key(&id) && !d.alias_queue.contains(&id) {
				d.alias_queue.push(id);
			}
		}
		if d.alias_queue.is_empty() {
			return;
		}
	}
	let mut pending = ctx.pending;
	if pending.peek().has(Op::Aliases) {
		return;
	}
	if client(ctx).is_none() {
		return;
	}
	pending.write().insert(Op::Aliases);
	spawn_forever(async move {
		let mut data = ctx.data;
		loop {
			// Re-read the client each time so a reconnect never resolves against the old daemon.
			let Some(client) = client(ctx) else { break };
			let next = data.write().alias_queue.pop();
			let Some(node_id) = next else { break };
			// Private or unknown nodes simply have no alias; a failed lookup is retried on the next fetch.
			let alias = match client.graph_get_node(GraphGetNodeRequest { node_id: node_id.clone() }).await {
				Ok(r) => r
					.node
					.and_then(|n| n.announcement_info)
					.map(|a| a.alias.trim().to_string())
					.filter(|a| !a.is_empty()),
				Err(_) => {
					data.write().alias_queue.clear();
					break;
				},
			};
			data.write().aliases.insert(node_id, alias);
		}
		let mut pending = ctx.pending;
		pending.write().remove(Op::Aliases);
	});
}

/// Background refresh of whatever the visible page shows.
pub fn refresh_visible(ctx: AppCtx) {
	let nav = *ctx.nav.peek();
	match nav.active_tab {
		ActiveTab::Overview => {
			fetch_node_info(ctx);
			fetch_balances(ctx);
			fetch_channels(ctx);
			fetch_stable_channels(ctx);
			fetch_peers(ctx);
			refresh_first_payments_page(ctx);
			fetch_revenue_week(ctx);
			// Like payments, the feed is only reloaded while "Show more" has not extended it.
			if ctx.data.peek().activity_pages <= 1 {
				fetch_activity_feed(ctx, false);
			}
		},
		ActiveTab::NodeInfo => fetch_node_info(ctx),
		ActiveTab::Balances | ActiveTab::Onchain => fetch_balances(ctx),
		ActiveTab::Channels => {
			fetch_channels(ctx);
			fetch_stable_channels(ctx);
			// The "Splicing" pill reads the feed.
			if ctx.data.peek().activity_pages <= 1 {
				fetch_activity_feed(ctx, false);
			}
		},
		ActiveTab::StableChannels => {
			fetch_stable_channels(ctx);
			fetch_channels(ctx);
			fetch_peers(ctx);
		},
		ActiveTab::Revenue if ctx.data.peek().revenue_pages <= 1 => fetch_revenue(ctx, false),
		ActiveTab::Peers => fetch_peers(ctx),
		ActiveTab::Payments => refresh_first_payments_page(ctx),
		ActiveTab::ForwardedPayments => {
			fetch_forwarded_payments(ctx);
			fetch_channels(ctx);
		},
		_ => {},
	}
}

/// Refresh payments only while a single page is loaded, so paging and "Load all" are never undone.
pub fn refresh_first_payments_page(ctx: AppCtx) {
	if ctx.data.peek().payments_pages <= 1 {
		fetch_payments(ctx, false);
	}
}

/// Re-read the log being followed on the Logs page.
pub fn refresh_followed_log(ctx: AppCtx) {
	let nav = *ctx.nav.peek();
	if nav.active_tab != ActiveTab::Logs {
		return;
	}
	let view = ctx.view.peek().clone();
	let data = ctx.data.peek();
	match nav.logs_tab {
		LogsTab::Audit if view.audit_view.follow && data.audit_log.is_some() => {
			drop(data);
			fetch_audit_log(ctx);
		},
		LogsTab::Ldk if view.ldk_view.follow && data.ldk_log.is_some() => {
			drop(data);
			fetch_ldk_log(ctx);
		},
		_ => {},
	}
}

pub fn sign_message(ctx: AppCtx) {
	if busy(ctx, Op::SignMessage) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let message = ctx.forms.peek().sign_message.message.trim().to_string();
	if message.is_empty() {
		ctx.error("Message is required");
		return;
	}
	let message = bytes::Bytes::from(message.into_bytes());
	run(ctx, Op::SignMessage, async move { client.sign_message(SignMessageRequest { message }).await }, |ctx, v| {
		let mut results = ctx.results;
		results.write().sign_result = Some(v.signature.clone());
		ctx.success("Message signed");
	});
}

pub fn verify_signature(ctx: AppCtx) {
	if busy(ctx, Op::VerifySignature) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let form = ctx.forms.peek().verify_signature.clone();
	let message = form.message.trim().to_string();
	let signature = form.signature.trim().to_string();
	let public_key = form.public_key.trim().to_string();
	if message.is_empty() || signature.is_empty() || public_key.is_empty() {
		ctx.error("All fields are required");
		return;
	}
	let message = bytes::Bytes::from(message.into_bytes());
	run(
		ctx,
		Op::VerifySignature,
		async move { client.verify_signature(VerifySignatureRequest { message, signature, public_key }).await },
		|ctx, v| {
			let mut results = ctx.results;
			results.write().verify_result = Some(v.valid);
			if v.valid {
				ctx.success("Signature is valid");
			} else {
				ctx.error("Signature is INVALID");
			}
		},
	);
}

pub fn fetch_graph_channels(ctx: AppCtx) {
	let Some(client) = client(ctx) else { return };
	run(
		ctx,
		Op::GraphListChannels,
		async move { client.graph_list_channels(GraphListChannelsRequest {}).await },
		|ctx, v| {
			let mut data = ctx.data;
			data.write().graph_channels = Some(v);
		},
	);
}

pub fn fetch_graph_channel(ctx: AppCtx) {
	if busy(ctx, Op::GraphGetChannel) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let Ok(short_channel_id) = ctx.forms.peek().graph_get_channel.short_channel_id.trim().parse::<u64>() else {
		ctx.error("Invalid short channel ID");
		return;
	};
	run(
		ctx,
		Op::GraphGetChannel,
		async move { client.graph_get_channel(GraphGetChannelRequest { short_channel_id }).await },
		|ctx, v| {
			let mut data = ctx.data;
			data.write().graph_channel_detail = Some(v);
		},
	);
}

pub fn fetch_graph_nodes(ctx: AppCtx) {
	let Some(client) = client(ctx) else { return };
	run(ctx, Op::GraphListNodes, async move { client.graph_list_nodes(GraphListNodesRequest {}).await }, |ctx, v| {
		let mut data = ctx.data;
		data.write().graph_nodes = Some(v);
	});
}

pub fn fetch_graph_node(ctx: AppCtx) {
	if busy(ctx, Op::GraphGetNode) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let node_id = ctx.forms.peek().graph_get_node.node_id.trim().to_string();
	if node_id.is_empty() {
		ctx.error("Node ID is required");
		return;
	}
	run(
		ctx,
		Op::GraphGetNode,
		async move { client.graph_get_node(GraphGetNodeRequest { node_id }).await },
		|ctx, v| {
			let mut data = ctx.data;
			data.write().graph_node_detail = Some(v);
		},
	);
}

pub fn export_pathfinding_scores(ctx: AppCtx) {
	let Some(client) = client(ctx) else { return };
	run(
		ctx,
		Op::ExportPathfindingScores,
		async move { client.export_pathfinding_scores(ExportPathfindingScoresRequest {}).await },
		|ctx, v| {
			let size = v.scores.len();
			let mut results = ctx.results;
			results.write().export_scores_result = Some(v);
			ctx.success(format!("Exported pathfinding scores ({} bytes)", size));
		},
	);
}

/// Copy `text` and confirm in the status toast.
pub fn copy(ctx: AppCtx, text: &str) {
	#[cfg(not(target_arch = "wasm32"))]
	report_copy(ctx, crate::platform::copy_text(text));
	#[cfg(target_arch = "wasm32")]
	{
		let text = text.to_string();
		spawn_forever(async move { report_copy(ctx, crate::platform::copy_text(&text).await) });
	}
}

fn report_copy(ctx: AppCtx, ok: bool) {
	if ok {
		ctx.success("Copied to clipboard");
	} else {
		ctx.error("Clipboard is not available");
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use sc_rest_client::ldk_server_grpc::types::Payment;

	fn page(ids: &[&str], next: Option<i64>) -> ListPaymentsResponse {
		ListPaymentsResponse {
			payments: ids.iter().map(|id| Payment { payment_id: id.to_string(), ..Default::default() }).collect(),
			next_page_token: next.map(|index| format!("t{index}")),
		}
	}

	#[test]
	fn onchain_send_all_takes_precedence_over_a_typed_amount() {
		assert!(matches!(onchain_send_amount(Some(50_000), true), Some(onchain_send_request::Amount::AllFunds(_))));
		assert!(matches!(onchain_send_amount(None, true), Some(onchain_send_request::Amount::AllFunds(_))));
	}

	#[test]
	fn onchain_send_uses_the_typed_amount_or_nothing() {
		assert!(matches!(onchain_send_amount(Some(50_000), false), Some(onchain_send_request::Amount::AmountSats(50_000))));
		assert!(onchain_send_amount(None, false).is_none());
	}

	fn ids(payments: &Option<ListPaymentsResponse>) -> Vec<String> {
		payments.as_ref().map(|p| p.payments.iter().map(|p| p.payment_id.clone()).collect()).unwrap_or_default()
	}

	#[test]
	fn load_more_appends_and_refresh_replaces() {
		let mut payments = None;
		let mut token = None;
		apply_payments_page(&mut payments, &mut token, page(&["a", "b"], Some(2)), false);
		assert_eq!(ids(&payments), vec!["a", "b"]);
		assert_eq!(token.as_deref(), Some("t2"));

		apply_payments_page(&mut payments, &mut token, page(&["c"], None), true);
		assert_eq!(ids(&payments), vec!["a", "b", "c"]);
		assert!(token.is_none());
		assert!(payments.as_ref().unwrap().next_page_token.is_none());

		apply_payments_page(&mut payments, &mut token, page(&["z"], Some(1)), false);
		assert_eq!(ids(&payments), vec!["z"]);
	}

	#[test]
	fn appending_without_a_first_page_just_stores_it() {
		let mut payments = None;
		let mut token = None;
		apply_payments_page(&mut payments, &mut token, page(&["a"], None), true);
		assert_eq!(ids(&payments), vec!["a"]);
	}

	#[tokio::test]
	async fn load_more_skips_empty_pages_and_advances_cursor() {
		let mut pages = std::collections::HashMap::new();
		pages.insert(Some("t1".to_string()), page(&[], Some(2)));
		pages.insert(Some("t2".to_string()), page(&["visible"], Some(3)));
		let mut requested = Vec::new();

		let result = fetch_page_until_visible(Some("t1".to_string()), |token| {
			requested.push(token.clone());
			std::future::ready(Ok::<_, String>(pages.get(&token).cloned().unwrap()))
		})
		.await
		.unwrap();

		assert_eq!(ids(&Some(result)), vec!["visible"]);
		assert_eq!(requested, vec![Some("t1".to_string()), Some("t2".to_string())]);
	}

	#[tokio::test]
	async fn load_more_stops_after_empty_pages_reach_the_end() {
		let mut pages = std::collections::HashMap::new();
		pages.insert(Some("t1".to_string()), page(&[], Some(2)));
		pages.insert(Some("t2".to_string()), page(&[], None));
		let mut requested = Vec::new();

		let result = fetch_page_until_visible(Some("t1".to_string()), |token| {
			requested.push(token.clone());
			std::future::ready(Ok::<_, String>(pages.get(&token).cloned().unwrap()))
		})
		.await
		.unwrap();

		assert!(result.payments.is_empty());
		assert!(result.next_page_token.is_none());
		assert_eq!(requested, vec![Some("t1".to_string()), Some("t2".to_string())]);
	}

	#[tokio::test]
	async fn load_more_rejects_a_repeated_cursor_while_skipping_empty_pages() {
		let mut pages = std::collections::HashMap::new();
		pages.insert(Some("t1".to_string()), page(&[], Some(2)));
		pages.insert(Some("t2".to_string()), page(&[], Some(1)));
		let mut requested = Vec::new();

		let error = fetch_page_until_visible(Some("t1".to_string()), |token| {
			requested.push(token.clone());
			std::future::ready(Ok::<_, String>(pages.get(&token).cloned().unwrap()))
		})
		.await
		.unwrap_err();

		assert_eq!(error, "Stopped loading payments: the server repeated a page token");
		assert_eq!(requested, vec![Some("t1".to_string()), Some("t2".to_string())]);
	}

	#[test]
	fn channel_config_is_omitted_when_every_field_is_blank() {
		assert!(build_channel_config("", "", "").is_none());
		let config = build_channel_config("100", "x", "").unwrap();
		assert_eq!(config.forwarding_fee_proportional_millionths, Some(100));
		assert_eq!(config.forwarding_fee_base_msat, None);
	}

	#[test]
	fn channel_ids_are_trimmed_and_required() {
		assert_eq!(channel_ids(" a ", " b "), Some(("a".to_string(), "b".to_string())));
		assert_eq!(channel_ids("a", "  "), None);
	}
}
