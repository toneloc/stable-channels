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
	ListPeersRequest, OnchainReceiveRequest, OnchainSendRequest, OnchainSendResponse, OpenChannelRequest,
	SignMessageRequest, SpliceOutRequest, SpliceOutResponse, SpontaneousSendRequest,
	UpdateChannelConfigRequest, VerifySignatureRequest, AllFunds, onchain_send_request, open_channel_request,
};
use sc_rest_client::ldk_server_grpc::endpoints::{
	GET_BALANCES_PATH, GET_NODE_INFO_PATH, GET_PAYMENT_DETAILS_PATH,
	GRAPH_GET_CHANNEL_PATH, GRAPH_GET_NODE_PATH, GRAPH_LIST_CHANNELS_PATH, GRAPH_LIST_NODES_PATH,
	LIST_CHANNELS_PATH, LIST_FORWARDED_PAYMENTS_PATH, LIST_PAYMENTS_PATH, LIST_PEERS_PATH,
};
use sc_rest_client::ldk_server_grpc::types::{
	bolt11_invoice_description, Bolt11InvoiceDescription, ChannelConfig,
};
use sc_rest_client::error::LspRestError;
use sc_rest_client::error::LspRestErrorCode::{AuthError, InternalError};
use sc_rest_client::sc_protos::access::{
	CreateTokenRequest, CreateTokenResponse, ListTokensRequest, RevokeTokenRequest, CREATE_TOKEN_PATH, LIST_TOKENS_PATH,
	REVOKE_TOKEN_PATH,
};
use sc_rest_client::sc_protos::revenue::{GetRevenueRequest, RefundTradeFeeRequest, GET_REVENUE_PATH};
use sc_rest_client::sc_protos::stable::{
	CancelQueuedSpliceRequest, CancelQueuedSpliceResponse, EditStableChannelRequest, GetPriceRequest,
	ListChannelLedgerEventsRequest, ListChannelLedgerEventsResponse, ListQueuedSplicesRequest,
	ListQueuedSplicesResponse, ListSettlementPaymentsRequest, ListStableChannelsRequest, LogRequest,
	QueueSpliceInRequest, ReleaseStabilityPaymentRequest, ReleaseStabilityPaymentResponse, AUDIT_LOG_PATH,
	GET_PRICE_PATH, LDK_LOG_PATH, LIST_CHANNEL_LEDGER_EVENTS_PATH, LIST_QUEUED_SPLICES_PATH,
	LIST_SETTLEMENT_PAYMENTS_PATH, LIST_STABLE_CHANNELS_PATH,
};

use crate::ledger::{checked_next_cursor, merge_ledger_events};
use crate::state::{
	AccessForm, ActiveTab, AppCtx, ChannelLedgerForm, ChannelLedgerRequestKey, ConnectionStatus, CreatedToken, Data, Dialog,
	Drawer, LogsTab, Op, QueuedRelease, RevokeTokenForm, SendKind, SettlementKind,
};

/// A failed request as the operator sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
	/// Show this; the session stays.
	Show(String),
	/// The daemon will not take this token (refused, revoked, or too old for logins): back to Settings with this reason.
	SignedOut(String),
}

impl From<LspRestError> for Failure {
	fn from(e: LspRestError) -> Self {
		explain(&e, false)
	}
}

impl From<String> for Failure {
	fn from(text: String) -> Self {
		Failure::Show(text)
	}
}

/// What the daemon said when Login refused the token itself, rather than a passing limit.
const TOKEN_REFUSALS: [&str; 2] =
	["LDK Server refused this token (wrong, revoked or expired)", "Tokens limited to one method can't log into the GUI"];

/// How a daemon from before token logins refuses /Login: its api-key check answers every request before routing.
const OLD_DAEMON_REFUSAL: &str = "Invalid X-Auth header";

/// How the operator sees a daemon error; `at_login` is true for the Login call, where any refusal signs out.
pub fn explain(e: &LspRestError, at_login: bool) -> Failure {
	let message = e.message.as_str();
	// A daemon without /Login refuses it at its api-key check, or answers an empty-body 404.
	let not_found = e.error_code == InternalError && message.starts_with("HTTP 404");
	let old_daemon = e.error_code == AuthError && message == OLD_DAEMON_REFUSAL;
	if at_login && (not_found || old_daemon) {
		return Failure::SignedOut("This LSP is too old for token logins. Update it".to_string());
	}
	if e.error_code == AuthError && message.starts_with("request too old") {
		return Failure::Show(
			"The LSP refused the request as too old. Check that this computer's clock is within a minute of the LSP's".to_string(),
		);
	}
	let refused = e.error_code == AuthError
		&& (at_login || message == "token revoked or expired" || TOKEN_REFUSALS.contains(&message));
	if refused {
		return Failure::SignedOut(message.to_string());
	}
	// A login that could not reach the daemon or LDK Server says so plainly; the token was not judged.
	if at_login {
		return Failure::Show(message.to_string());
	}
	Failure::Show(e.to_string())
}

/// True while `client` is still the logged-in client, so a reply from an ended session is dropped.
fn same_session(ctx: AppCtx, client: &Option<Arc<LspRestClient>>) -> bool {
	match (ctx.client(), client) {
		(Some(now), Some(then)) => Arc::ptr_eq(&now, then),
		_ => false,
	}
}

/// Forget everything loaded for the last session, so another token never sees it.
fn clear_data(ctx: AppCtx) {
	invalidate_channel_ledger(ctx);
	// A token shown once must not outlive its session.
	let mut forms = ctx.forms;
	forms.write().access = AccessForm::default();
	let mut data = ctx.data;
	*data.write() = Data::default();
	// A Release ticked for one session's call must not carry over to the next.
	let mut view = ctx.view;
	view.write().close_release();
}

/// The daemon refused the token: drop the session and land on Settings with its reason.
fn end_session(ctx: AppCtx, reason: String) {
	let mut conn = ctx.conn;
	{
		let mut c = conn.write();
		c.client = None;
		c.status = ConnectionStatus::Error(reason.clone());
	}
	clear_data(ctx);
	// An open dialog, a confirm step above all, closes with its session; its form survives.
	close_dialog(ctx);
	let mut nav = ctx.nav;
	nav.write().active_tab = ActiveTab::Settings;
	ctx.error(reason);
}

/// Run `fut` for `op` unless it is already running; `on_ok` applies the result and errors toast.
fn run<T, E, F, H>(ctx: AppCtx, op: Op, fut: F, on_ok: H)
where
	T: 'static,
	E: Into<Failure> + 'static,
	F: Future<Output = Result<T, E>> + 'static,
	H: FnOnce(AppCtx, T) + 'static,
{
	run_with(ctx, op, fut, move |ctx, value, _| on_ok(ctx, value), |ctx, e| ctx.error(e));
}

/// `run` for an action whose `on_ok` must know whether the session that sent it is still the logged-in one.
fn run_action<T, E, F, H>(ctx: AppCtx, op: Op, fut: F, on_ok: H)
where
	T: 'static,
	E: Into<Failure> + 'static,
	F: Future<Output = Result<T, E>> + 'static,
	H: FnOnce(AppCtx, T, bool) + 'static,
{
	run_with(ctx, op, fut, on_ok, |ctx, e| ctx.error(e));
}

/// `run` with a custom error path; a refused token ends the session, and only a [`Op::must_report`] reply outlives it, with `on_ok` told so.
fn run_with<T, E, F, H, G>(ctx: AppCtx, op: Op, fut: F, on_ok: H, on_err: G)
where
	T: 'static,
	E: Into<Failure> + 'static,
	F: Future<Output = Result<T, E>> + 'static,
	H: FnOnce(AppCtx, T, bool) + 'static,
	G: FnOnce(AppCtx, String) + 'static,
{
	let mut pending = ctx.pending;
	if pending.peek().has(op) {
		return;
	}
	pending.write().insert(op);
	let started = ctx.client();
	// Not scope-bound: the request finishes even if its tab or dialog unmounts.
	spawn_forever(async move {
		let result = fut.await;
		let mut pending = ctx.pending;
		pending.write().remove(op);
		let current = same_session(ctx, &started);
		// An action's outcome is always shown, or the operator would send it again; any other reply from an ended session is dropped.
		if !current && !op.must_report() {
			return;
		}
		match result {
			Ok(value) => {
				if current {
					mark_fetched(ctx, op);
				}
				on_ok(ctx, value, current)
			},
			Err(e) => match e.into() {
				Failure::SignedOut(reason) if current => end_session(ctx, reason),
				// The session already ended or was replaced: report the refusal without ending the login that took over.
				Failure::SignedOut(text) | Failure::Show(text) => on_err(ctx, text),
			},
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

/// The logged-in client when its token may call `route`, so fetches skip what the token can't read.
fn client_for(ctx: AppCtx, route: &str) -> Option<Arc<LspRestClient>> {
	if ctx.may_call(route) {
		client(ctx)
	} else {
		None
	}
}

/// Builds a client from the Settings fields; a missing field or unreadable certificate is reported here.
fn build_client(ctx: AppCtx) -> Option<LspRestClient> {
	let mut conn = ctx.conn;
	let (url, token) = {
		let c = conn.peek();
		(c.server_url.trim().to_string(), c.token.trim().to_string())
	};

	#[cfg(not(target_arch = "wasm32"))]
	let built = {
		let cert_path = conn.peek().tls_cert_path.trim().to_string();
		if url.is_empty() || token.is_empty() || cert_path.is_empty() {
			ctx.error("Fill in all connection fields");
			return None;
		}
		let cert_data = match std::fs::read(&cert_path) {
			Ok(data) => data,
			Err(e) => {
				ctx.error(format!("Couldn't read the TLS certificate: {}", e));
				return None;
			},
		};
		LspRestClient::new(url, token, &cert_data)
	};

	#[cfg(target_arch = "wasm32")]
	let built = {
		if url.is_empty() || token.is_empty() {
			ctx.error("Fill in the server URL and token");
			return None;
		}
		// On WASM, the browser handles TLS - no certificate needed
		LspRestClient::new(url, token, &[])
	};

	match built {
		Ok(client) => Some(client),
		Err(e) => {
			conn.write().status = ConnectionStatus::Error(e.clone());
			ctx.error(e);
			None
		},
	}
}

/// Logs in with the token in Settings in the background; a bad token is reported at once.
pub fn connect(ctx: AppCtx) {
	spawn_forever(async move {
		connect_now(ctx).await;
	});
}

/// Builds the client and logs in; on success the session goes live and the pages load. Returns whether it connected.
pub async fn connect_now(ctx: AppCtx) -> bool {
	if busy(ctx, Op::Login) {
		return false;
	}
	let Some(client) = build_client(ctx) else { return false };
	let client = Arc::new(client);
	let mut pending = ctx.pending;
	pending.write().insert(Op::Login);
	let login = client.login().await;
	pending.write().remove(Op::Login);
	if let Err(e) = login {
		match explain(&e, true) {
			Failure::SignedOut(reason) => end_session(ctx, reason),
			Failure::Show(reason) => {
				// Not a refusal of the token (the daemon or LDK Server is down): stay put so the page offers Retry.
				let mut conn = ctx.conn;
				{
					let mut c = conn.write();
					c.client = None;
					c.status = ConnectionStatus::Error(reason.clone());
				}
				clear_data(ctx);
				ctx.error(reason);
			},
		}
		return false;
	}
	clear_data(ctx);
	{
		let mut conn = ctx.conn;
		let mut c = conn.write();
		c.client = Some(Arc::clone(&client));
		c.status = ConnectionStatus::Connected;
	}
	ctx.success(format!("Connected as {}", client.token_name()));
	fetch_node_info(ctx);
	fetch_balances(ctx);
	fetch_channels(ctx);
	fetch_price(ctx);
	fetch_stable_channels(ctx);
	fetch_queued_splices(ctx);
	fetch_payments(ctx, false);
	fetch_forwarded_payments(ctx);
	fetch_peers(ctx);
	fetch_activity_feed(ctx, false);
	fetch_revenue_week(ctx);
	true
}

/// Ends the session here and on the daemon, and forgets what it loaded.
pub fn disconnect(ctx: AppCtx) {
	let mut conn = ctx.conn;
	let client = {
		let mut c = conn.write();
		c.status = ConnectionStatus::Disconnected;
		c.client.take()
	};
	if let Some(client) = client {
		// Frees the daemon's session slot; nothing waits on the answer.
		spawn_forever(async move {
			let _ = client.logout().await;
		});
	}
	clear_data(ctx);
	ctx.error("Disconnected");
}

pub fn fetch_node_info(ctx: AppCtx) {
	let Some(client) = client_for(ctx, GET_NODE_INFO_PATH) else { return };
	run(ctx, Op::NodeInfo, async move { client.get_node_info(GetNodeInfoRequest {}).await }, |ctx, v| {
		let mut data = ctx.data;
		data.write().node_info = Some(v);
	});
}

pub fn fetch_balances(ctx: AppCtx) {
	let Some(client) = client_for(ctx, GET_BALANCES_PATH) else { return };
	run(ctx, Op::Balances, async move { client.get_balances(GetBalancesRequest {}).await }, |ctx, v| {
		let mut data = ctx.data;
		data.write().balances = Some(v);
	});
}

pub fn fetch_channels(ctx: AppCtx) {
	let Some(client) = client_for(ctx, LIST_CHANNELS_PATH) else { return };
	run(ctx, Op::Channels, async move { client.list_channels(ListChannelsRequest {}).await }, |ctx, v| {
		let peers: Vec<String> = v.channels.iter().map(|c| c.counterparty_node_id.clone()).collect();
		let mut data = ctx.data;
		data.write().channels = Some(v);
		resolve_aliases(ctx, peers);
	});
}

/// Apply one payments page: "Load more" appends, anything else replaces.
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
				return Err("Stopped loading payments. The LSP sent the same page twice".to_string());
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
	let Some(client) = client_for(ctx, LIST_PAYMENTS_PATH) else { return };
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
				break Some("Stopped loading payments. The LSP sent the same page twice".to_string());
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

/// Fetch the first payments page, or the next one when `appending` ("Load more").
pub fn fetch_payments(ctx: AppCtx, appending: bool) {
	if !busy(ctx, Op::Payments) && !busy(ctx, Op::PaymentsAll) {
		if let Some(client) = client_for(ctx, LIST_PAYMENTS_PATH) {
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
	let Some(client) = client_for(ctx, LIST_PEERS_PATH) else { return };
	run(ctx, Op::Peers, async move { client.list_peers(ListPeersRequest {}).await }, |ctx, v| {
		let peers: Vec<String> = v.peers.iter().map(|p| p.node_id.clone()).collect();
		let mut data = ctx.data;
		data.write().peers = Some(v);
		resolve_aliases(ctx, peers);
	});
}

pub fn fetch_forwarded_payments(ctx: AppCtx) {
	let Some(client) = client_for(ctx, LIST_FORWARDED_PAYMENTS_PATH) else { return };
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
	let Some(client) = client_for(ctx, LDK_LOG_PATH) else { return };
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
	let Some(client) = client_for(ctx, AUDIT_LOG_PATH) else { return };
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
	let Some(client) = client_for(ctx, LIST_CHANNEL_LEDGER_EVENTS_PATH) else { return };
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
		move |ctx, v: ListChannelLedgerEventsResponse, _| {
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
	let Some(client) = client_for(ctx, GET_REVENUE_PATH) else { return };
	if ctx.data.peek().revenue_week_unsupported {
		return;
	}
	let since = crate::format::now_secs() as i64 - 7 * 86_400;
	let request = GetRevenueRequest { since, categories: Vec::new(), cursor: None, limit: 1 };
	run_with(
		ctx,
		Op::RevenueWeek,
		async move { client.get_revenue(request).await },
		|ctx, v, _| {
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
	let Some(client) = client_for(ctx, LIST_CHANNEL_LEDGER_EVENTS_PATH) else { return };
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
	let Some(client) = client_for(ctx, LIST_CHANNEL_LEDGER_EVENTS_PATH) else { return };
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
					Err(error) => ctx.error(format!("Couldn't export: {error}")),
				}
			}
			let mut pending = ctx.pending;
			pending.write().remove(Op::Export);
		});
	}

	#[cfg(target_arch = "wasm32")]
	match crate::platform::download_text(file_name, mime, &content) {
		Ok(()) => ctx.success(format!("Exported {} to {}", what, file_name)),
		Err(error) => ctx.error(format!("Couldn't export: {error}")),
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
	let Some(client) = client_for(ctx, GET_PAYMENT_DETAILS_PATH) else { return };
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
	run(ctx, Op::OnchainSend, async move { client.onchain_send(request).await }, onchain_sent);
}

fn onchain_sent(ctx: AppCtx, v: OnchainSendResponse) {
	let mut results = ctx.results;
	results.write().last_txid = Some(v.txid.clone());
	ctx.success("Transaction sent");
	let mut forms = ctx.forms;
	forms.write().onchain_send = Default::default();
	let mut view = ctx.view;
	view.write().send_all_confirm = false;
	close_dialog(ctx);
	fetch_balances(ctx);
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
		ctx.success("Payment sent");
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
		ctx.success("Payment sent");
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
		ctx.error("Node ID and address are required");
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
		|ctx, _v| {
			ctx.success("Channel open started");
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

const CHANNEL_IDS_REQUIRED: &str = "User channel ID and peer node ID are required";

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
			ctx.success("Channel close started");
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
			ctx.success("Force close started");
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
	// The daemon queues the splice, wakes the phone by push and runs it once the channel is usable.
	run_with(
		ctx,
		Op::SpliceIn,
		async move {
			client
				.queue_splice_in(QueueSpliceInRequest {
					user_channel_id,
					counterparty_node_id,
					amount_sats: splice_amount_sats,
				})
				.await
		},
		|ctx, v, _| {
			if v.ok {
				ctx.success(v.status);
				let mut forms = ctx.forms;
				forms.write().splice_in = Default::default();
				close_dialog(ctx);
			} else {
				ctx.error(v.status);
			}
			fetch_queued_splices(ctx);
			fetch_channels(ctx);
		},
		|ctx, e| ctx.error(splice_in_error(e)),
	);
}

/// Every channel's queued splice-in. An older daemon without the route leaves the list as it was; any other failure is reported like the other fetches.
pub fn fetch_queued_splices(ctx: AppCtx) {
	let Some(client) = client_for(ctx, LIST_QUEUED_SPLICES_PATH) else { return };
	run_with(
		ctx,
		Op::ListQueuedSplices,
		async move { client.list_queued_splices(ListQueuedSplicesRequest {}).await },
		|ctx, list, _| queued_splices_listed(ctx, list),
		queued_splices_failed,
	);
}

/// A new queue list. An open Release confirmation closes once its call is no longer pending on the node, so its tick cannot carry over to a later call.
pub fn queued_splices_listed(ctx: AppCtx, list: ListQueuedSplicesResponse) {
	let mut view = ctx.view;
	if view.peek().release_outlived(&list.splices) {
		view.write().close_release();
	}
	let mut data = ctx.data;
	data.write().queued_splices = Some(list);
}

/// A failed queue list: reported like the other fetches, except from an older daemon that has no such route.
pub fn queued_splices_failed(ctx: AppCtx, error: String) {
	if !crate::dashboard::needs_newer_daemon(&error) {
		ctx.error(error);
	}
}

/// What Splice+ shows when the daemon refused the request outright. An older daemon has no queue route, and an unknown route answers 404 with no body.
fn splice_in_error(error: String) -> String {
	if error.contains("HTTP 404") {
		"Splice+ needs the updated daemon: this one has no splice-in queue, so nothing was spliced".to_owned()
	} else {
		error
	}
}

/// What Release says when the daemon no longer lists the checked call as pending on the node.
const RELEASE_CALL_CHANGED: &str =
	"Not released: the call you checked is no longer the one pending on the node. Check the node again before releasing.";

/// Cancel a channel's queued splice-in. A call the node has not answered stays pending, so no new request goes out until it answers or is released.
pub fn cancel_queued_splice(ctx: AppCtx, user_channel_id: String) {
	if busy(ctx, Op::CancelQueuedSplice) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	run(
		ctx,
		Op::CancelQueuedSplice,
		async move { client.cancel_queued_splice(CancelQueuedSpliceRequest { user_channel_id, force: false, ..Default::default() }).await.map(Some) },
		cancel_answered,
	);
}

/// Release the call a ticked confirmation is for: the forced cancel, which drops the queue's guard against a second splice on the channel. Right before sending, the daemon's list is read again, and nothing is sent unless that same call is still pending on the node.
pub fn release_queued_splice(ctx: AppCtx) {
	if busy(ctx, Op::CancelQueuedSplice) {
		return;
	}
	let mut view = ctx.view;
	let Some(release) = view.peek().queued_release.clone().filter(|release| release.confirmed) else { return };
	view.write().close_release();
	let Some(client) = client(ctx) else { return };
	run(
		ctx,
		Op::CancelQueuedSplice,
		async move {
			let listed = client.list_queued_splices(ListQueuedSplicesRequest {}).await?;
			let Some(request) = queued_release_request(&release, &listed) else { return Ok(None) };
			client.cancel_queued_splice(request).await.map(Some)
		},
		cancel_answered,
	);
}

/// The forced cancel for a ticked confirmation, naming the call that was checked, or None when the daemon's fresh list no longer has that call pending on the node.
pub fn queued_release_request(release: &QueuedRelease, listed: &ListQueuedSplicesResponse) -> Option<CancelQueuedSpliceRequest> {
	listed
		.splices
		.iter()
		.any(|row| release.is_for(row))
		.then(|| CancelQueuedSpliceRequest {
			user_channel_id: release.user_channel_id.clone(),
			force: true,
			// The daemon releases only this call, so one that changes after this list is left pending.
			created_at: release.created_at,
			last_attempt_at: release.last_attempt_at,
			funding_txo: release.funding_txo.clone(),
		})
}

/// The daemon's answer to a Cancel or Release; None when Release found the checked call no longer pending.
fn cancel_answered(ctx: AppCtx, answer: Option<CancelQueuedSpliceResponse>) {
	match answer {
		None => ctx.error(RELEASE_CALL_CHANGED),
		Some(v) if v.ok => ctx.success(v.status),
		Some(v) => ctx.error(v.status),
	}
	fetch_queued_splices(ctx);
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
		splice_out_started,
	);
}

fn splice_out_started(ctx: AppCtx, v: SpliceOutResponse) {
	ctx.success(format!("Splice-out to {} started", v.address));
	let mut forms = ctx.forms;
	forms.write().splice_out = Default::default();
	close_dialog(ctx);
	fetch_channels(ctx);
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
		ctx.error("Node ID and address are required");
		return;
	}
	run(
		ctx,
		Op::ConnectPeer,
		async move { client.connect_peer(ConnectPeerRequest { node_pubkey, address, persist }).await },
		|ctx, _v| {
			ctx.success("Peer connected");
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
	// A token without node:read beats with WhoAmI instead, so a revoked token is still noticed.
	let read_price = ctx.may_call(GET_PRICE_PATH);
	pending.write().insert(Op::GetPrice);
	let started = Some(Arc::clone(&client));
	spawn_forever(async move {
		let result = if read_price {
			client.get_price(GetPriceRequest {}).await.map(Some)
		} else {
			client.who_am_i().await.map(|_| None)
		};
		let mut pending = ctx.pending;
		pending.write().remove(Op::GetPrice);
		let mut conn = ctx.conn;
		// A disconnect or a new login while the request was in flight wins.
		if !same_session(ctx, &started) {
			return;
		}
		match result {
			Ok(v) => {
				if let Some(v) = v {
					let mut price = ctx.price;
					price.set(Some(v));
				}
				if conn.peek().status != ConnectionStatus::Connected {
					conn.write().status = ConnectionStatus::Connected;
				}
			},
			Err(e) => {
				let unreachable = e.error_code == InternalError;
				match explain(&e, false) {
					Failure::SignedOut(reason) => end_session(ctx, reason),
					Failure::Show(text) if unreachable => {
						conn.write().status = ConnectionStatus::Error(format!("LSP unreachable: {text}"))
					},
					Failure::Show(text) => conn.write().status = ConnectionStatus::Error(text),
				}
			},
		}
	});
}

pub fn fetch_stable_channels(ctx: AppCtx) {
	let Some(client) = client_for(ctx, LIST_STABLE_CHANNELS_PATH) else { return };
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
	let Some(client) = client_for(ctx, GET_REVENUE_PATH) else { return };
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

pub fn fetch_tokens(ctx: AppCtx) {
	let Some(client) = client_for(ctx, LIST_TOKENS_PATH) else { return };
	run(ctx, Op::ListTokens, async move { client.list_tokens(ListTokensRequest {}).await }, |ctx, v| {
		let mut data = ctx.data;
		data.write().tokens = Some(v);
	});
}

/// Creates the token the Access form describes; its hex is kept only for the one-time box.
pub fn create_token(ctx: AppCtx) {
	if busy(ctx, Op::CreateToken) {
		return;
	}
	let Some(client) = client_for(ctx, CREATE_TOKEN_PATH) else { return };
	let form = ctx.forms.peek().access.clone();
	let name = form.name.trim().to_string();
	if name.is_empty() {
		ctx.error("Token name is required");
		return;
	}
	if form.permissions.is_empty() {
		ctx.error("Pick a preset or at least one permission");
		return;
	}
	let expires_at = match crate::ui::access::expiry_secs(&form.expiry, crate::format::now_secs()) {
		Ok(secs) => secs,
		Err(e) => {
			ctx.error(e);
			return;
		},
	};
	let request = CreateTokenRequest { name, permissions: form.permissions, expires_at };
	run_action(ctx, Op::CreateToken, async move { client.create_token(request).await }, token_created);
}

/// Shows the new token once; after its session ended it is reported without the hex, which must never reach another session.
fn token_created(ctx: AppCtx, v: CreateTokenResponse, current: bool) {
	let name = v.token.map(|t| t.name).unwrap_or_default();
	if current {
		ctx.success(format!("Token {name} created"));
		let mut forms = ctx.forms;
		forms.write().access = AccessForm { created: Some(CreatedToken { name, hex: v.token_hex }), ..Default::default() };
	} else {
		ctx.error(format!("Token {name} was created, but its session ended before it was shown. Revoke it"));
	}
	fetch_tokens(ctx);
}

/// Revokes the token the dialog names; revoking your own ends your session.
pub fn revoke_token(ctx: AppCtx) {
	if busy(ctx, Op::RevokeToken) {
		return;
	}
	let Some(client) = client_for(ctx, REVOKE_TOKEN_PATH) else { return };
	let chosen = ctx.forms.peek().access.revoke.clone();
	if chosen.id.is_empty() {
		return;
	}
	let request = RevokeTokenRequest { id: chosen.id.clone() };
	run_action(ctx, Op::RevokeToken, async move { client.revoke_token(request).await }, move |ctx, _, current| {
		token_revoked(ctx, &chosen, current)
	});
}

/// Reports a revoke; revoking your own token ends your session, unless it already ended or another login took its place.
fn token_revoked(ctx: AppCtx, chosen: &RevokeTokenForm, current: bool) {
	let mut forms = ctx.forms;
	forms.write().access.revoke = Default::default();
	close_dialog(ctx);
	if chosen.is_you {
		if current {
			// The daemon already ended this session; just return to the logged-out state.
			let mut conn = ctx.conn;
			{
				let mut c = conn.write();
				c.client = None;
				c.status = ConnectionStatus::Disconnected;
			}
			clear_data(ctx);
			let mut nav = ctx.nav;
			nav.write().active_tab = ActiveTab::Settings;
		}
		ctx.error("You revoked your own token. Log in with another one");
	} else {
		ctx.success(format!("Token {} revoked", chosen.name));
		fetch_tokens(ctx);
	}
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

/// The request for an answer: the first one asks the daemon for the channel's figures, and only a confirmation of that same answer is sent as acknowledged.
pub(crate) fn release_request(form: &crate::state::ReleaseStabilityPaymentForm, arrived: bool) -> ReleaseStabilityPaymentRequest {
	ReleaseStabilityPaymentRequest {
		payment_id: form.payment_id.clone(),
		decision: if arrived { "arrived" } else { "not_arrived" }.to_owned(),
		acknowledged: form.confirming == Some(arrived),
	}
}

/// Show the daemon's figures for an answer, unless the dialog has moved on to another payment since they were asked for.
pub(crate) fn show_release_preview(
	form: &mut crate::state::ReleaseStabilityPaymentForm, requested_payment_id: &str, arrived: bool, figures: String,
) -> bool {
	if form.payment_id != requested_payment_id {
		return false;
	}
	form.confirming = Some(arrived);
	form.notice = figures;
	true
}

/// Reset the dialog after a release went through, unless it has moved on to another payment since; returns whether it should close.
pub(crate) fn finish_release(form: &mut crate::state::ReleaseStabilityPaymentForm, requested_payment_id: &str) -> bool {
	if form.payment_id != requested_payment_id {
		return false;
	}
	*form = Default::default();
	true
}

/// Sends the operator's answer about the payment the dialog describes; the daemon first returns the figures to confirm, and releases only a payment the node will not report on.
pub fn release_stability_payment(ctx: AppCtx, arrived: bool) {
	if busy(ctx, Op::ReleaseStabilityPayment) {
		return;
	}
	let Some(client) = client(ctx) else { return };
	let form = ctx.forms.peek().release_stability_payment.clone();
	if form.payment_id.is_empty() {
		return;
	}
	let request = release_request(&form, arrived);
	let requested_payment_id = form.payment_id;
	run_action(
		ctx,
		Op::ReleaseStabilityPayment,
		async move { client.release_stability_payment(request).await },
		move |ctx, v, current| release_answered(ctx, v, &requested_payment_id, arrived, current),
	);
}

/// Applies the daemon's answer about `requested_payment_id`: figures to confirm, the release, or a refusal.
fn release_answered(ctx: AppCtx, v: ReleaseStabilityPaymentResponse, requested_payment_id: &str, arrived: bool, current: bool) {
	let mut forms = ctx.forms;
	if v.needs_acknowledgement {
		// Nothing moved yet, and an ended session's figures must not be confirmed in the next one.
		if current {
			show_release_preview(&mut forms.write().release_stability_payment, requested_payment_id, arrived, v.status);
		}
	} else if v.ok {
		ctx.success(v.status);
		// The dialog may by now be open for another payment, which this answer must not close.
		if finish_release(&mut forms.write().release_stability_payment, requested_payment_id) {
			close_dialog(ctx);
		}
		// Reload the history so the released payment shows its outcome.
		invalidate_channel_ledger(ctx);
		fetch_channel_ledger(ctx, false);
		fetch_stable_channels(ctx);
	} else {
		ctx.error(v.status);
	}
}

pub fn fetch_settlement_payments(ctx: AppCtx) {
	let Some(client) = client_for(ctx, LIST_SETTLEMENT_PAYMENTS_PATH) else { return };
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
		ctx.success("Keysend sent");
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
	if client_for(ctx, GRAPH_GET_NODE_PATH).is_none() {
		return;
	}
	pending.write().insert(Op::Aliases);
	spawn_forever(async move {
		let mut data = ctx.data;
		loop {
			// Re-read the client each time so a reconnect never resolves against the old daemon or token.
			let Some(client) = client_for(ctx, GRAPH_GET_NODE_PATH) else { break };
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
	// On every page: the sidebar lists Queued splice-ins only while one is live, and the Channels list dots its channel.
	fetch_queued_splices(ctx);
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
				ctx.error("Signature is not valid");
			}
		},
	);
}

pub fn fetch_graph_channels(ctx: AppCtx) {
	let Some(client) = client_for(ctx, GRAPH_LIST_CHANNELS_PATH) else { return };
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
	let Some(client) = client_for(ctx, GRAPH_GET_CHANNEL_PATH) else { return };
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
	let Some(client) = client_for(ctx, GRAPH_LIST_NODES_PATH) else { return };
	run(ctx, Op::GraphListNodes, async move { client.graph_list_nodes(GraphListNodesRequest {}).await }, |ctx, v| {
		let mut data = ctx.data;
		data.write().graph_nodes = Some(v);
	});
}

pub fn fetch_graph_node(ctx: AppCtx) {
	if busy(ctx, Op::GraphGetNode) {
		return;
	}
	let Some(client) = client_for(ctx, GRAPH_GET_NODE_PATH) else { return };
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
	fn splice_in_against_an_older_daemon_says_to_upgrade_and_other_errors_pass_through() {
		assert!(splice_in_error("HTTP 404 Not Found".into()).starts_with("Splice+ needs the updated daemon"));
		assert_eq!(splice_in_error("connection refused".into()), "connection refused");
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

		assert_eq!(error, "Stopped loading payments. The LSP sent the same page twice");
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

	use sc_rest_client::error::LspRestErrorCode::InternalServerError;

	#[test]
	fn the_clock_error_tells_the_operator_to_check_the_clock() {
		let stale = LspRestError::new(AuthError, "request too old: check this computer's clock");
		let Failure::Show(text) = explain(&stale, false) else { panic!("a clock error keeps the session") };
		assert!(text.contains("Check that this computer's clock") && text.contains("minute"), "{text}");
		assert!(!text.contains("AuthError"), "the hint replaces the raw error: {text}");
	}

	#[test]
	fn a_daemon_without_login_is_reported_as_too_old() {
		let too_old = Failure::SignedOut("This LSP is too old for token logins. Update it".into());
		// What a daemon from before token logins really answers: its api-key check runs before routing.
		let old_daemon = LspRestError::new(AuthError, "Invalid X-Auth header");
		assert_eq!(explain(&old_daemon, true), too_old);
		let missing = LspRestError::new(InternalError, "HTTP 404 Not Found");
		assert_eq!(explain(&missing, true), too_old);
		assert!(matches!(explain(&missing, false), Failure::Show(_)), "a 404 on an ordinary request is not about logins");
	}

	#[test]
	fn a_refused_or_revoked_token_ends_the_session_in_the_daemons_words() {
		for message in [
			"token revoked or expired",
			"LDK Server refused this token (wrong, revoked or expired)",
			"Tokens limited to one method can't log into the GUI",
		] {
			assert_eq!(explain(&LspRestError::new(AuthError, message), false), Failure::SignedOut(message.into()));
		}
		// At login every refusal lands on Settings in the daemon's words; an outage does not call the token bad.
		assert_eq!(explain(&LspRestError::new(AuthError, "too many sessions"), true), Failure::SignedOut("too many sessions".into()));
		let down = LspRestError::new(InternalServerError, "Can't reach LDK Server");
		assert_eq!(explain(&down, true), Failure::Show("Can't reach LDK Server".into()));
	}

	#[test]
	fn a_missing_permission_or_a_passing_outage_is_shown_as_is() {
		for error in [
			LspRestError::new(AuthError, "this token lacks payments:send"),
			LspRestError::new(AuthError, "too many logins for this token, wait a minute"),
			LspRestError::new(InternalServerError, "Can't reach LDK Server"),
		] {
			assert_eq!(explain(&error, false), Failure::Show(error.to_string()));
		}
	}

	/// A self-signed certificate so tests can build a real client; no request is ever polled.
	const TEST_CERT: &str = "-----BEGIN CERTIFICATE-----
MIIBgDCCASWgAwIBAgIUH8bXjc3DNbImV0bNNHlTEtA6IlIwCgYIKoZIzj0EAwIw
FDESMBAGA1UEAwwJbG9jYWxob3N0MCAXDTI2MTExODE5NDUyNVoYDzIxMjYxMDI1
MTk0NTI1WjAUMRIwEAYDVQQDDAlsb2NhbGhvc3QwWTATBgcqhkjOPQIBBggqhkjO
PQMBBwNCAAQC2YhpNQPaPLcWR/4j4juAGiqc7y6kkyPLiUKTGAbTCW1aDBomsocf
AOydQXSh7omWK0pB1LGj68APuh4xiq4bo1MwUTAdBgNVHQ4EFgQU9g7U37w89Z6J
rcnN5+ipfbVPZx4wHwYDVR0jBBgwFoAU9g7U37w89Z6JrcnN5+ipfbVPZx4wDwYD
VR0TAQH/BAUwAwEB/zAKBggqhkjOPQQDAgNJADBGAiEA95rmpRrugkaBRmQiVPDk
rryFE9twO1qdq/pwPZpPekQCIQDhC0K9qQAKvtBD7jWw/nhCIH9FuqDNL7sU2gTt
NHhGIw==
-----END CERTIFICATE-----
";

	thread_local! {
		static QUEUED: std::cell::RefCell<Vec<Op>> = const { std::cell::RefCell::new(Vec::new()) };
	}

	/// Runs one refresh of the Overview for a session with these permissions and records which fetches it started.
	#[component]
	fn RefreshProbe(permissions: Vec<&'static str>) -> Element {
		use crate::state::{Connection, Forms, Nav, Prefs};
		let ctx = use_context_provider(|| {
			let mut conn = Connection::signed_in("t", &permissions);
			conn.client = Some(Arc::new(LspRestClient::new("127.0.0.1:9".into(), "00".into(), TEST_CERT.as_bytes()).unwrap()));
			AppCtx::new(conn, Forms::default(), Nav::default(), Prefs::default(), false)
		});
		use_hook(move || {
			refresh_visible(ctx);
			fetch_price(ctx);
			let all = [
				Op::NodeInfo, Op::Balances, Op::Channels, Op::ListStableChannels, Op::Peers, Op::Payments,
				Op::ListSettlementPayments, Op::RevenueWeek, Op::ActivityFeed, Op::GetPrice,
			];
			let started: Vec<Op> = all.into_iter().filter(|op| ctx.pending.peek().has(*op)).collect();
			QUEUED.with(|q| *q.borrow_mut() = started);
		});
		rsx! {}
	}

	fn refreshed(permissions: &[&'static str]) -> Vec<Op> {
		let mut dom = VirtualDom::new_with_props(RefreshProbe, RefreshProbeProps { permissions: permissions.to_vec() });
		dom.rebuild_in_place();
		QUEUED.with(|q| q.borrow().clone())
	}

	#[test]
	fn auto_refresh_skips_what_the_token_cannot_fetch() {
		assert_eq!(refreshed(&["node:read"]), vec![Op::NodeInfo, Op::Balances, Op::GetPrice]);
		assert_eq!(
			refreshed(&["admin"]),
			vec![
				Op::NodeInfo, Op::Balances, Op::Channels, Op::ListStableChannels, Op::Peers, Op::Payments,
				Op::ListSettlementPayments, Op::RevenueWeek, Op::ActivityFeed, Op::GetPrice,
			],
		);
	}

	thread_local! {
		static SESSION: std::cell::Cell<Option<AppCtx>> = const { std::cell::Cell::new(None) };
	}

	fn test_client() -> Arc<LspRestClient> {
		Arc::new(LspRestClient::new("127.0.0.1:9".into(), "00".into(), TEST_CERT.as_bytes()).unwrap())
	}

	/// A signed-in admin session whose context the test drives.
	#[component]
	fn SessionProbe() -> Element {
		use crate::state::{Connection, Forms, Nav, Prefs};
		let ctx = use_context_provider(|| {
			let mut conn = Connection::signed_in("ops", &["admin"]);
			conn.client = Some(test_client());
			AppCtx::new(conn, Forms::default(), Nav::default(), Prefs::default(), false)
		});
		SESSION.with(|s| s.set(Some(ctx)));
		rsx! {}
	}

	/// How the session moves on while a request is in flight.
	#[derive(Clone, Copy, Debug)]
	enum Moved {
		/// A background fetch was refused, so the session ended.
		Ended,
		/// Another login took its place.
		Replaced,
	}

	type Reply<T> = tokio::sync::oneshot::Receiver<Result<T, LspRestError>>;

	/// Starts a request whose reply the test holds, moves the session on, then delivers `reply` and lets the request finish.
	fn finish_after<T: 'static>(how: Moved, reply: Result<T, LspRestError>, start: impl FnOnce(AppCtx, Reply<T>)) -> (VirtualDom, AppCtx) {
		let mut dom = VirtualDom::new(SessionProbe);
		dom.rebuild_in_place();
		let ctx = SESSION.with(|s| s.get()).unwrap();
		let (send, receive) = tokio::sync::oneshot::channel();
		dom.in_scope(ScopeId::ROOT, || start(ctx, receive));
		dom.process_events();
		dom.in_scope(ScopeId::ROOT, || match how {
			Moved::Ended => end_session(ctx, "token revoked or expired".into()),
			Moved::Replaced => {
				let mut conn = ctx.conn;
				let mut c = conn.write();
				c.client = Some(test_client());
				// The new login can read nothing, so no refresh it would start ever reaches the network.
				c.test_session = Some(("other".into(), Vec::new()));
			},
		});
		assert!(send.send(reply).is_ok(), "the request is still waiting for its reply");
		dom.process_events();
		(dom, ctx)
	}

	fn shown(ctx: AppCtx) -> Option<(String, bool)> {
		ctx.status.peek().as_ref().map(|s| (s.text.clone(), s.is_error))
	}

	/// The login that took over is untouched by the old session's reply.
	fn new_session_kept(ctx: AppCtx, how: Moved) {
		if let Moved::Replaced = how {
			let conn = ctx.conn.peek();
			assert_eq!(conn.token_name(), "other");
			assert!(conn.client.is_some() && conn.status == ConnectionStatus::Connected, "the new login stays signed in");
		}
	}

	#[test]
	fn a_send_that_succeeds_after_the_session_ended_still_reports_and_clears_its_form() {
		for how in [Moved::Ended, Moved::Replaced] {
			let sent = OnchainSendResponse { txid: "5e9b".into(), ..Default::default() };
			let (_dom, ctx) = finish_after(how, Ok(sent), |ctx, reply| {
				let mut forms = ctx.forms;
				forms.write().onchain_send.address = "bcrt1qrecipient".into();
				forms.write().onchain_send.amount_sats = "50000".into();
				let mut dialog = ctx.dialog;
				dialog.set(Some(Dialog::ConfirmSend(SendKind::Onchain)));
				run(ctx, Op::OnchainSend, async move { reply.await.unwrap() }, onchain_sent);
			});
			assert!(!ctx.pending.peek().has(Op::OnchainSend), "{how:?}: the reply was handled");
			assert_eq!(shown(ctx), Some(("Transaction sent".into(), false)), "{how:?}: the operator learns the money left");
			assert_eq!(*ctx.dialog.peek(), None, "{how:?}: the confirm step cannot send it again");
			assert_eq!(ctx.forms.peek().onchain_send.address, "", "{how:?}: the form cannot send it again");
			assert!(!ctx.data.peek().fetched_at.contains_key(&Op::OnchainSend), "{how:?}: nothing lands in the next session's data");
			new_session_kept(ctx, how);
		}
	}

	#[test]
	fn a_splice_out_that_succeeds_after_the_session_ended_still_reports_and_clears_its_form() {
		for how in [Moved::Ended, Moved::Replaced] {
			let started = SpliceOutResponse { address: "bcrt1qsweep".into(), ..Default::default() };
			let (_dom, ctx) = finish_after(how, Ok(started), |ctx, reply| {
				let mut forms = ctx.forms;
				forms.write().splice_out.user_channel_id = "7".into();
				forms.write().splice_out.splice_amount_sats = "20000".into();
				let mut dialog = ctx.dialog;
				dialog.set(Some(Dialog::SpliceOut));
				run(ctx, Op::SpliceOut, async move { reply.await.unwrap() }, splice_out_started);
			});
			assert_eq!(shown(ctx), Some(("Splice-out to bcrt1qsweep started".into(), false)), "{how:?}");
			assert_eq!(*ctx.dialog.peek(), None, "{how:?}");
			assert_eq!(ctx.forms.peek().splice_out.user_channel_id, "", "{how:?}");
			new_session_kept(ctx, how);
		}
	}

	#[test]
	fn a_fetch_that_returns_after_the_session_changed_writes_nothing() {
		use sc_rest_client::ldk_server_grpc::api::GetBalancesResponse;
		for how in [Moved::Ended, Moved::Replaced] {
			let balances = GetBalancesResponse { total_onchain_balance_sats: 7, ..Default::default() };
			let (_dom, ctx) = finish_after(how, Ok(balances), |ctx, reply| {
				run(ctx, Op::Balances, async move { reply.await.unwrap() }, |ctx, v| {
					let mut data = ctx.data;
					data.write().balances = Some(v);
				});
			});
			assert!(!ctx.pending.peek().has(Op::Balances), "{how:?}: the reply was handled");
			let data = ctx.data.peek();
			assert!(data.balances.is_none() && !data.fetched_at.contains_key(&Op::Balances), "{how:?}: an old session's figures never show");
			new_session_kept(ctx, how);
		}
	}

	#[test]
	fn a_send_that_fails_after_the_session_changed_still_shows_the_error() {
		use sc_rest_client::ldk_server_grpc::api::SpontaneousSendResponse;
		let failed = LspRestError::new(InternalServerError, "no route to the recipient");
		let (_dom, ctx) = finish_after(Moved::Ended, Err(failed.clone()), |ctx, reply| {
			run(ctx, Op::SpontaneousSend, async move { reply.await.unwrap() }, |_, _: SpontaneousSendResponse| panic!("it failed"));
		});
		assert_eq!(shown(ctx), Some((failed.to_string(), true)));
		// The old token's refusal is shown without ending the login that replaced it.
		let refused = LspRestError::new(AuthError, "token revoked or expired");
		let (_dom, ctx) = finish_after(Moved::Replaced, Err(refused), |ctx, reply| {
			run(ctx, Op::SpontaneousSend, async move { reply.await.unwrap() }, |_, _: SpontaneousSendResponse| panic!("it failed"));
		});
		assert_eq!(shown(ctx), Some(("token revoked or expired".into(), true)));
		new_session_kept(ctx, Moved::Replaced);
	}

	#[test]
	fn end_session_closes_an_open_dialog() {
		let mut dom = VirtualDom::new(SessionProbe);
		dom.rebuild_in_place();
		let ctx = SESSION.with(|s| s.get()).unwrap();
		dom.in_scope(ScopeId::ROOT, || {
			let mut dialog = ctx.dialog;
			dialog.set(Some(Dialog::ConfirmSend(SendKind::Keysend)));
			end_session(ctx, "token revoked or expired".into());
		});
		assert_eq!(*ctx.dialog.peek(), None, "a confirm step must not outlive its session");
		assert!(ctx.client().is_none());
	}

	#[test]
	fn money_channel_and_token_actions_always_report_and_reads_do_not() {
		let actions = [
			Op::OnchainSend, Op::Bolt11Send, Op::Bolt12Send, Op::SpontaneousSend, Op::OpenChannel, Op::CloseChannel,
			Op::ForceCloseChannel, Op::SpliceIn, Op::SpliceOut, Op::UpdateChannelConfig, Op::EditStableChannel,
			Op::ReleaseStabilityPayment, Op::RefundTradeFee, Op::CreateToken, Op::RevokeToken,
		];
		for op in actions {
			assert!(op.must_report(), "{op:?} moves money or changes a channel or a token");
		}
		let dropped = [
			Op::NodeInfo, Op::Balances, Op::Channels, Op::Payments, Op::ListStableChannels, Op::ListTokens, Op::GetPrice,
			Op::ActivityFeed, Op::RevenueWeek, Op::PaymentDetails, Op::Bolt11Receive, Op::ConnectPeer, Op::SignMessage,
		];
		for op in dropped {
			assert!(!op.must_report(), "{op:?} is dropped with its session");
		}
	}

	#[test]
	fn a_token_created_after_the_session_changed_is_reported_without_its_secret() {
		use sc_rest_client::sc_protos::access::{CreateTokenResponse, TokenInfo};
		for how in [Moved::Ended, Moved::Replaced] {
			let token = TokenInfo { name: "ops-2".into(), ..Default::default() };
			let created = CreateTokenResponse { token: Some(token), token_hex: "deadbeef00".into() };
			let (_dom, ctx) = finish_after(how, Ok(created), |ctx, reply| {
				run_action(ctx, Op::CreateToken, async move { reply.await.unwrap() }, token_created);
			});
			let (text, is_error) = shown(ctx).unwrap();
			assert!(is_error && text.contains("ops-2") && text.contains("Revoke it"), "{how:?}: {text}");
			assert!(ctx.forms.peek().access.created.is_none(), "{how:?}: a token shown once never reaches another session");
			new_session_kept(ctx, how);
		}
	}

	#[test]
	fn revoking_your_own_token_after_the_session_changed_keeps_the_new_login() {
		use sc_rest_client::sc_protos::access::RevokeTokenResponse;
		for how in [Moved::Ended, Moved::Replaced] {
			let (_dom, ctx) = finish_after(how, Ok(RevokeTokenResponse {}), |ctx, reply| {
				let chosen = RevokeTokenForm { id: "t1".into(), name: "ops".into(), is_you: true, is_gui_token: true };
				let mut dialog = ctx.dialog;
				dialog.set(Some(Dialog::RevokeToken));
				run_action(ctx, Op::RevokeToken, async move { reply.await.unwrap() }, move |ctx, _, current| {
					token_revoked(ctx, &chosen, current)
				});
			});
			assert_eq!(shown(ctx), Some(("You revoked your own token. Log in with another one".into(), true)), "{how:?}");
			assert_eq!(*ctx.dialog.peek(), None, "{how:?}");
			new_session_kept(ctx, how);
		}
	}

	#[test]
	fn a_release_answer_after_the_session_changed_reports_a_release_but_never_offers_a_preview() {
		use crate::state::ReleaseStabilityPaymentForm;
		let form = || ReleaseStabilityPaymentForm { payment_id: "p1".into(), summary: "top-up".into(), ..Default::default() };
		for how in [Moved::Ended, Moved::Replaced] {
			// The first step only returns figures: nothing moved, and an old session's figures are never offered to confirm.
			let preview = ReleaseStabilityPaymentResponse { needs_acknowledgement: true, status: "figures".into(), ..Default::default() };
			let (_dom, ctx) = finish_after(how, Ok(preview), |ctx, reply| {
				let mut forms = ctx.forms;
				forms.write().release_stability_payment = form();
				run_action(ctx, Op::ReleaseStabilityPayment, async move { reply.await.unwrap() }, |ctx, v, current| {
					release_answered(ctx, v, "p1", true, current)
				});
			});
			let after = ctx.forms.peek().release_stability_payment.clone();
			assert!(after.confirming.is_none() && after.notice.is_empty(), "{how:?}: no stale figures to confirm");
			let released = ReleaseStabilityPaymentResponse { ok: true, status: "Released".into(), ..Default::default() };
			let (_dom, ctx) = finish_after(how, Ok(released), |ctx, reply| {
				let mut forms = ctx.forms;
				forms.write().release_stability_payment = ReleaseStabilityPaymentForm { confirming: Some(true), ..form() };
				let mut dialog = ctx.dialog;
				dialog.set(Some(Dialog::ReleaseStabilityPayment));
				run_action(ctx, Op::ReleaseStabilityPayment, async move { reply.await.unwrap() }, |ctx, v, current| {
					release_answered(ctx, v, "p1", true, current)
				});
			});
			assert_eq!(shown(ctx), Some(("Released".into(), false)), "{how:?}");
			assert_eq!(*ctx.dialog.peek(), None, "{how:?}");
			assert!(ctx.forms.peek().release_stability_payment.payment_id.is_empty(), "{how:?}");
			new_session_kept(ctx, how);
		}
	}
}
