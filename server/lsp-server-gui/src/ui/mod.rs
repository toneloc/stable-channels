pub mod audit_log;
pub mod balances;
pub mod channel_ledger;
pub mod channels;
pub mod confirm;
pub mod forwarded_payments;
pub mod ldk_log;
pub mod lightning;
pub mod log_view;
pub mod logs;
pub mod network_graph;
pub mod node_info;
pub mod onchain;
pub mod overview;
pub mod payments;
pub mod peers;
pub mod revenue;
pub mod settings;
pub mod stable_channels;
pub mod tools;
pub mod widgets;

use dioxus::prelude::*;

use crate::actions;
use crate::format::now_secs;
use crate::platform;
use crate::state::{
	ActiveTab, AppCtx, Connection, ConnectionStatus, Dialog, DisplayUnit, Drawer, Forms, Nav, Theme,
};
use widgets::{Icon, TooltipLayer};

const CSS: &str = include_str!("../../assets/app.css");

/// Initial connection settings, form state and whether to auto-connect on launch.
fn startup() -> (Connection, Forms, bool) {
	let conn = Connection::new();
	let forms = Forms::default();

	#[cfg(not(target_arch = "wasm32"))]
	if let Some(gui_config) = crate::config::find_and_load_config() {
		let mut conn = conn;
		let mut forms = forms;
		conn.server_url = gui_config.server_url;
		conn.api_key = gui_config.api_key;
		conn.tls_cert_path = gui_config.tls_cert_path;
		conn.network = gui_config.network;
		forms.chain_source = crate::state::ChainSourceForm::from_config(&gui_config.chain_source);
		conn.chain_source = gui_config.chain_source;
		return (conn, forms, true);
	}

	(conn, forms, false)
}

#[component]
pub fn App() -> Element {
	let ctx = use_context_provider(|| {
		let (conn, forms, auto_connect) = startup();
		// First launch with no connection to auto-attempt: land on Settings so connecting is obvious.
		let nav = Nav {
			active_tab: if auto_connect { ActiveTab::Overview } else { ActiveTab::Settings },
			..Default::default()
		};
		AppCtx::new(conn, forms, nav, platform::load_prefs(), auto_connect)
	});

	// Auto-connect once if a daemon config was found at startup.
	use_hook(move || {
		if *ctx.auto_connect.peek() {
			actions::connect(ctx);
		}
	});

	// 1s clock for relative timestamps; every 5s the price poll doubles as a heartbeat.
	use_future(move || async move {
		let mut now = ctx.now;
		let mut tick: u64 = 0;
		loop {
			platform::sleep_ms(1000).await;
			tick += 1;
			now.set(now_secs());
			if ctx.client().is_none() {
				continue;
			}
			if tick % 5 == 0 {
				actions::fetch_price(ctx);
			}
			// Keep the visible page current without the operator pressing Refresh.
			if ctx.prefs.peek().auto_refresh && ctx.is_connected_peek() {
				if tick % 30 == 0 {
					actions::refresh_visible(ctx);
				}
				if tick % 10 == 0 {
					actions::refresh_followed_log(ctx);
				}
			}
		}
	});

	// Web: the container publishes the API key at /setup/key.txt (same origin, behind its auth), so connect with zero input.
	#[cfg(target_arch = "wasm32")]
	use_future(move || async move {
		let mut delay_ms = 500;
		loop {
			if let Some(key) = platform::fetch_setup_key().await {
				let mut conn = ctx.conn;
				let idle = !matches!(conn.peek().status, ConnectionStatus::Connected);
				if idle && conn.peek().api_key.trim().is_empty() {
					conn.write().api_key = key;
					actions::connect(ctx);
					// Leave the Settings landing page once the zero-input connect succeeded.
					let mut nav = ctx.nav;
					if ctx.is_connected_peek() && nav.peek().active_tab == ActiveTab::Settings {
						nav.write().active_tab = ActiveTab::Overview;
					}
				}
				break;
			}
			platform::sleep_ms(delay_ms).await;
			delay_ms = (delay_ms * 2).min(5_000);
		}
	});

	// Persist preferences whenever they change.
	use_effect(move || {
		let prefs = *ctx.prefs.read();
		platform::save_prefs(&prefs);
	});

	let theme = match ctx.prefs.read().theme {
		Theme::System => None,
		Theme::Light => Some("light"),
		Theme::Dark => Some("dark"),
	};

	rsx! {
		document::Style { {CSS} }
		div { class: "app", "data-theme": theme,
			Sidebar {}
			div { class: "main",
				TopBar {}
				main { class: "content",
					div { class: "page", CurrentTab {} }
				}
			}
			Drawers {}
			Dialogs {}
			Toast {}
			TooltipLayer {}
		}
	}
}

#[component]
fn CurrentTab() -> Element {
	let ctx = use_context::<AppCtx>();
	let tab = ctx.nav.read().active_tab;
	match tab {
		ActiveTab::Overview => rsx! { overview::Overview {} },
		ActiveTab::NodeInfo => rsx! { node_info::NodeInfo {} },
		ActiveTab::Balances => rsx! { balances::Balances {} },
		ActiveTab::Revenue => rsx! { revenue::Revenue {} },
		ActiveTab::Channels => rsx! { channels::Channels {} },
		ActiveTab::Peers => rsx! { peers::Peers {} },
		ActiveTab::Payments => rsx! { payments::Payments {} },
		ActiveTab::ForwardedPayments => rsx! { forwarded_payments::ForwardedPayments {} },
		ActiveTab::Lightning => rsx! { lightning::Lightning {} },
		ActiveTab::Onchain => rsx! { onchain::Onchain {} },
		ActiveTab::StableChannels => rsx! { stable_channels::StableChannels {} },
		ActiveTab::Tools => rsx! { tools::Tools {} },
		ActiveTab::NetworkGraph => rsx! { network_graph::NetworkGraph {} },
		ActiveTab::Logs => rsx! { logs::Logs {} },
		ActiveTab::Settings => rsx! { settings::Settings {} },
	}
}

const DOC_LINKS: [(&str, &str); 4] = [
	("LDK Server", "https://github.com/lightningdevkit/ldk-server"),
	("LDK Node", "https://docs.rs/ldk-node/latest/ldk_node/"),
	("Rust Lightning", "https://docs.rs/lightning/latest/lightning/"),
	("BDK", "https://docs.rs/bdk_wallet/latest/bdk_wallet/"),
];

#[component]
fn Sidebar() -> Element {
	let ctx = use_context::<AppCtx>();
	let active = ctx.nav.read().active_tab;
	rsx! {
		nav { class: "sidebar", "aria-label": "Sections",
			div { class: "brand",
				widgets::BrandMark {}
				div {
					div { class: "brand-name", "Stable Channels" }
					div { class: "brand-sub", "LSP Server" }
				}
			}
			for (group, tabs) in ActiveTab::GROUPS {
				div { class: "nav-group", key: "{group}",
					div { class: "nav-group-title", "{group}" }
					for tab in tabs.iter().copied() {
						button {
							key: "{tab:?}",
							class: if tab == active { "nav-item active" } else { "nav-item" },
							"aria-current": if tab == active { "page" } else { "false" },
							onclick: move |_| {
								let mut nav = ctx.nav;
								nav.write().active_tab = tab;
							},
							Icon { name: tab.icon(), size: 18 }
							span { "{tab.label()}" }
						}
					}
				}
			}
			div { class: "sidebar-footer",
				span { class: "nav-group-title", style: "padding: 0 0 2px;", "Documentation" }
				for (label, href) in DOC_LINKS {
					a { key: "{label}", href: "{href}", target: "_blank", rel: "noopener noreferrer",
						"{label}"
						Icon { name: "external", size: 12 }
					}
				}
			}
		}
	}
}

#[component]
fn TopBar() -> Element {
	let ctx = use_context::<AppCtx>();
	let tab = ctx.nav.read().active_tab;
	let conn = ctx.conn.read();
	let (tone, label) = match &conn.status {
		ConnectionStatus::Disconnected => ("", "Disconnected".to_string()),
		ConnectionStatus::Connected => ("success", "Connected".to_string()),
		ConnectionStatus::Error(_) => ("danger", "Error".to_string()),
	};
	let error = match &conn.status {
		ConnectionStatus::Error(e) => Some(e.clone()),
		_ => None,
	};
	let connected = conn.status == ConnectionStatus::Connected;
	let server_url = conn.server_url.clone();
	drop(conn);
	let price = ctx.price.read().as_ref().map(|p| p.price).filter(|p| *p > 0.0);
	let unit = ctx.unit();
	let set_unit = move |unit: DisplayUnit| {
		let mut prefs = ctx.prefs;
		prefs.write().display_unit = unit;
	};
	rsx! {
		header { class: "topbar",
			div { class: "page-title",
				h1 { "{tab.title()}" }
				p { "{tab.subtitle()}" }
			}
			div { class: "topbar-right",
				span { class: "price-chip", title: "BTC/USD",
					span { class: "coin", "₿" }
					match price {
						Some(p) => rsx! { "{crate::format::format_usd(p)}" },
						None => rsx! { span { class: "muted", "price --" } },
					}
				}
				div { class: "seg", role: "group", "aria-label": "Display unit",
					widgets::SegBtn { active: unit == DisplayUnit::Usd, onclick: move |_| set_unit(DisplayUnit::Usd), "USD" }
					widgets::SegBtn { active: unit == DisplayUnit::Btc, onclick: move |_| set_unit(DisplayUnit::Btc), "BTC" }
					widgets::SegBtn { active: unit == DisplayUnit::Sats, onclick: move |_| set_unit(DisplayUnit::Sats), "Sats" }
				}
				match error {
					Some(e) => rsx! {
						widgets::Hover { tip: e,
							span { class: "conn-pill", span { class: "dot {tone}" } "{label}" }
						}
					},
					None => rsx! {
						span { class: "conn-pill",
							span { class: "dot {tone}" }
							"{label}"
							if connected {
								span { class: "url", "· {server_url}" }
							}
						}
					},
				}
			}
		}
	}
}

/// Status message pill: successes fade after a few seconds, errors stay until dismissed.
#[component]
fn Toast() -> Element {
	let ctx = use_context::<AppCtx>();
	use_effect(move || {
		let Some(message) = ctx.status.read().clone() else { return };
		if message.is_error {
			return;
		}
		spawn(async move {
			platform::sleep_ms(6_000).await;
			let mut status = ctx.status;
			if status.peek().as_ref().map(|m| m.id) == Some(message.id) {
				status.set(None);
			}
		});
	});
	let Some(message) = ctx.status.read().clone() else { return rsx! {} };
	rsx! {
		div {
			key: "{message.id}",
			class: if message.is_error { "toast error" } else { "toast" },
			role: "status",
			"aria-live": "polite",
			if message.is_error {
				span { style: "color: var(--red-text); display: inline-flex;", Icon { name: "alert", size: 16 } }
			} else {
				span { style: "color: var(--green-text); display: inline-flex;", Icon { name: "check", size: 16 } }
			}
			span { class: "text", title: "{message.text}", "{message.text}" }
			button {
				class: "icon-btn",
				"aria-label": "Dismiss",
				onclick: move |_| {
					let mut status = ctx.status;
					status.set(None);
				},
				Icon { name: "x", size: 14 }
			}
		}
	}
}

#[component]
fn Dialogs() -> Element {
	let ctx = use_context::<AppCtx>();
	let dialog = *ctx.dialog.read();
	match dialog {
		Some(Dialog::OpenChannel) => rsx! { channels::OpenChannelDialog {} },
		Some(Dialog::CloseChannel) => rsx! { channels::CloseChannelDialog {} },
		Some(Dialog::SpliceIn) => rsx! { channels::SpliceInDialog {} },
		Some(Dialog::SpliceOut) => rsx! { channels::SpliceOutDialog {} },
		Some(Dialog::UpdateChannelConfig) => rsx! { channels::UpdateConfigDialog {} },
		Some(Dialog::ConnectPeer) => rsx! { peers::ConnectPeerDialog {} },
		Some(Dialog::LoadConfig) => rsx! { settings::LoadConfigDialog {} },
		Some(Dialog::ConfirmSend(kind)) => rsx! { confirm::ConfirmSendDialog { kind } },
		Some(Dialog::RefundTradeFee) => rsx! { revenue::RefundTradeFeeDialog {} },
		None => rsx! {},
	}
}

#[component]
fn Drawers() -> Element {
	let ctx = use_context::<AppCtx>();
	let drawer = ctx.drawer.read().clone();
	match drawer {
		Some(Drawer::Payment(id)) => rsx! { payments::PaymentDrawer { key: "{id}", payment_id: id.clone() } },
		Some(Drawer::Channel(id)) => rsx! { channels::ChannelDrawer { key: "{id}", user_channel_id: id.clone() } },
		None => rsx! {},
	}
}

/// Close the side panel.
pub fn close_drawer(ctx: AppCtx) {
	let mut drawer = ctx.drawer;
	drawer.set(None);
}

/// Open the side panel for a table row.
pub fn open_drawer(ctx: AppCtx, which: Drawer) {
	let mut drawer = ctx.drawer;
	drawer.set(Some(which));
}

/// Close whatever dialog is open.
pub fn close_dialog(ctx: AppCtx) {
	let mut dialog = ctx.dialog;
	dialog.set(None);
}

/// Open a dialog.
pub fn open_dialog(ctx: AppCtx, which: Dialog) {
	let mut dialog = ctx.dialog;
	dialog.set(Some(which));
}

#[cfg(test)]
mod tests {
	use std::cell::RefCell;
	use std::collections::BTreeSet;

	use dioxus::prelude::*;
	use sc_rest_client::ldk_server_grpc::api::{ListChannelsResponse, ListPaymentsResponse};
	use sc_rest_client::ldk_server_grpc::types::{Channel, Payment};
	use sc_rest_client::sc_protos::stable::ListChannelLedgerEventsResponse;

	use super::*;
	use crate::state::{Op, Prefs, SettlementKind};

	/// Which fixture state the harness should seed before rendering a tab.
	#[derive(Clone, Copy, PartialEq)]
	enum Seed {
		Disconnected,
		Unreachable,
		ConnectedEmpty,
		ConnectedWithData,
	}

	#[component]
	fn Harness(tab: ActiveTab, seed: Seed) -> Element {
		let ctx = use_context_provider(|| {
			let mut conn = Connection::new();
			conn.status = match seed {
				Seed::Disconnected => ConnectionStatus::Disconnected,
				Seed::Unreachable => ConnectionStatus::Error("LSP unreachable: connection refused".into()),
				_ => ConnectionStatus::Connected,
			};
			AppCtx::new(conn, Forms::default(), Nav { active_tab: tab, ..Default::default() }, Prefs::default(), false)
		});
		use_hook(move || {
			if seed == Seed::ConnectedWithData {
				let mut data = ctx.data;
				let mut d = data.write();
				d.channels = Some(ListChannelsResponse {
					channels: vec![Channel {
						channel_id: "c0ffee".repeat(10),
						user_channel_id: "4242".into(),
						counterparty_node_id: "02ab".repeat(16),
						channel_value_sats: 1_000_000,
						outbound_capacity_msat: 600_000_000,
						inbound_capacity_msat: 400_000_000,
						is_channel_ready: true,
						is_usable: true,
						..Default::default()
					}],
				});
				d.payments = Some(ListPaymentsResponse {
					payments: vec![Payment { payment_id: "pay-1".into(), amount_msat: Some(5_000_000), direction: 1, status: 1, ..Default::default() }],
					next_page_token: None,
				});
				d.settlement_kinds = Some([("pay-1".to_string(), SettlementKind::Stability)].into_iter().collect());
			}
		});
		rsx! { CurrentTab {} }
	}

	fn render(tab: ActiveTab, seed: Seed) -> String {
		let mut dom = VirtualDom::new_with_props(Harness, HarnessProps { tab, seed });
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	#[test]
	fn data_tabs_gate_while_disconnected() {
		for tab in [ActiveTab::Balances, ActiveTab::Channels, ActiveTab::Payments, ActiveTab::Peers, ActiveTab::Logs] {
			let html = render(tab, Seed::Disconnected);
			assert!(html.contains("Not connected to an LSP"), "{tab:?} should gate");
			assert!(html.contains("Open Settings"));
			assert!(!html.contains("Retry"));
		}
	}

	#[test]
	fn unreachable_gate_offers_retry() {
		let html = render(ActiveTab::Channels, Seed::Unreachable);
		assert!(html.contains("Can&#39;t reach the LSP at localhost:3002") || html.contains("Can't reach the LSP at localhost:3002"));
		assert!(html.contains("connection refused"));
		assert!(html.contains("Retry"));
	}

	#[test]
	fn node_info_shows_chain_source_even_when_disconnected() {
		let html = render(ActiveTab::NodeInfo, Seed::Disconnected);
		assert!(html.contains("Chain Source"));
		assert!(html.contains("No chain source configured."));
		assert!(html.contains("Not connected to an LSP"));
	}

	#[test]
	fn empty_states_invite_a_refresh() {
		assert!(render(ActiveTab::Channels, Seed::ConnectedEmpty).contains("No channel data available"));
		assert!(render(ActiveTab::Payments, Seed::ConnectedEmpty).contains("No payment data available"));
		assert!(render(ActiveTab::Balances, Seed::ConnectedEmpty).contains("No balance data"));
	}

	#[test]
	fn loaded_tables_render_rows_and_settlement_labels() {
		let channels = render(ActiveTab::Channels, Seed::ConnectedWithData);
		assert!(channels.contains("1 of 1 channel(s)"));
		assert!(channels.contains("Active"));
		assert!(channels.contains("sats"));
		let payments = render(ActiveTab::Payments, Seed::ConnectedWithData);
		assert!(payments.contains("1/1 payment(s)"));
		assert!(payments.contains("Stability"));
		assert!(payments.contains("Succeeded"));
	}

	#[test]
	fn overview_reports_all_clear_or_problems() {
		let calm = render(ActiveTab::Overview, Seed::ConnectedWithData);
		assert!(calm.contains("Needs attention"));
		assert!(calm.contains("All clear"));
		assert!(!calm.contains(">In progress<"), "an empty In progress card is hidden");
		assert!(render(ActiveTab::Overview, Seed::Disconnected).contains("Not connected to an LSP"));
	}

	#[component]
	fn DashboardHarness(feed: bool) -> Element {
		let ctx = use_context_provider(|| {
			let mut conn = Connection::new();
			conn.status = ConnectionStatus::Connected;
			AppCtx::new(conn, Forms::default(), Nav { active_tab: ActiveTab::Overview, ..Default::default() }, Prefs::default(), false)
		});
		use_hook(move || {
			use sc_rest_client::ldk_server_grpc::api::{GetBalancesResponse, GetNodeInfoResponse, ListPeersResponse};
			use sc_rest_client::ldk_server_grpc::types::{BestBlock, Peer};
			use sc_rest_client::sc_protos::revenue::{GetRevenueResponse, RevenueLine};
			use sc_rest_client::sc_protos::stable::{ChannelLedgerEvent, LedgerRef, ListStableChannelsResponse, StableChannelInfo};
			let now = crate::format::now_secs();
			let ms = now as i64 * 1_000;
			let mut opening = Channel {
				user_channel_id: "7".into(),
				counterparty_node_id: "02aa".into(),
				channel_value_sats: 1_000_000,
				is_outbound: true,
				confirmations: Some(1),
				confirmations_required: Some(3),
				..Default::default()
			};
			opening.channel_id = "c7".into();
			let mut closing = Channel { user_channel_id: "8".into(), channel_id: "c8".into(), counterparty_node_id: "02bb".into(), channel_value_sats: 3_000_000, is_channel_ready: true, is_usable: true, ..Default::default() };
			closing.channel_shutdown_state = Some(3);
			let ev = |id: i64, event_type: &str, uid: &str, secs_ago: i64, detail: serde_json::Value| ChannelLedgerEvent {
				id,
				event_type: event_type.into(),
				occurred_at_ms: ms - secs_ago * 1_000,
				status: "completed".into(),
				detail_json: detail.to_string(),
				refs: vec![LedgerRef { role: "user_channel_id".into(), value: uid.into() }],
				..Default::default()
			};
			let mut data = ctx.data;
			let mut d = data.write();
			d.channels = Some(ListChannelsResponse { channels: vec![opening, closing] });
			d.balances = Some(GetBalancesResponse { spendable_onchain_balance_sats: 2_500_000, ..Default::default() });
			d.node_info = Some(GetNodeInfoResponse { current_best_block: Some(BestBlock { block_hash: "h".into(), height: 900_000 }), ..Default::default() });
			d.peers = Some(ListPeersResponse { peers: vec![Peer { node_id: "02aa".into(), is_connected: true, is_persisted: true, ..Default::default() }] });
			d.stable_channels = Some(ListStableChannelsResponse {
				channels: vec![
					StableChannelInfo { user_channel_id: "7".into(), counterparty: "02aa".into(), expected_usd: 50.0, expected_msats: 50_000_000, latest_price: 100_000.0, created_at: now as i64 - 86_400, ..Default::default() },
					StableChannelInfo { user_channel_id: "8".into(), counterparty: "02bb".into(), expected_usd: 25.0, expected_msats: 25_000_000, latest_price: 100_000.0, ..Default::default() },
				],
			});
			d.revenue_week = Some(GetRevenueResponse {
				lines: vec![
					RevenueLine { category: "trade_fee".into(), direction: "in".into(), count: 3, total_msat: 9_000_000 },
					RevenueLine { category: "jit_open_fee".into(), direction: "out".into(), count: 1, total_msat: 2_000_000 },
				],
				partial: !feed,
				untracked: if feed { Vec::new() } else { vec!["close_fee".into()] },
				..Default::default()
			});
			if feed {
				d.activity = Some(ListChannelLedgerEventsResponse {
					events: vec![
						ev(1, "TRADE_APPLIED", "7", 600, serde_json::json!({"new_expected_usd": 50.0, "trade_id": "t1"})),
						ev(2, "SYNC_MESSAGE_SENT", "7", 500, serde_json::json!({"expected_usd": 50.0, "payment_id": "s1"})),
						ev(3, "CHANNEL_SHUTDOWN_STATE_CHANGED", "8", 7_200, serde_json::json!({"shutdown_state": "RESOLVING_HTLCS"})),
					],
					..Default::default()
				});
				d.activity_pages = 1;
			} else {
				d.activity_error = Some("An exact ledger identifier is required".into());
			}
		});
		rsx! { CurrentTab {} }
	}

	fn render_dashboard(feed: bool) -> String {
		let mut dom = VirtualDom::new_with_props(DashboardHarness, DashboardHarnessProps { feed });
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	#[test]
	fn the_dashboard_shows_business_tiles_work_underway_and_activity() {
		let html = render_dashboard(true);
		for text in [
			"Stable users", ">2<", "1 online · +1 new this week",
			"Stabilized", "$75.00", "all at peg",
			"Net this week", "7,000 sats", "earned 9,000 sats · spent 2,000 sats",
			"Room to grow", "~2 more channels", "at your average JIT size of 1,000,000 sats",
			"In progress", "Channel with 02aa opening", "1/3 confirmations", "Closing the channel with 02bb", "resolving HTLCs",
			"Needs attention", "Close with 02bb stuck at resolving HTLCs for 2 h",
			"Recent activity", "Trade applied: stable target $50.00",
		] {
			assert!(html.contains(text), "missing {text}");
		}
		assert!(!html.contains("LSP published balances"), "successful syncs are routine");
		assert!(!html.contains("is opening"), "opening is work underway, not a problem");
	}

	#[test]
	fn an_older_daemon_only_loses_the_activity_card() {
		let html = render_dashboard(false);
		assert!(html.contains("Activity needs the updated daemon"));
		assert!(html.contains("An exact ledger identifier is required"));
		assert!(html.contains("~2 more channels"), "tiles still render");
		assert!(html.contains("Channel with 02aa opening"), "In progress still renders from the channel list");
		assert!(!html.contains("stuck at"), "with no feed the close's age is unknown");
		assert!(html.contains("spent \u{2265} 2,000 sats · some fees not tracked · partial history"), "the week tile names both gaps");
	}

	#[component]
	fn ChannelPanelHarness(new_fields: bool) -> Element {
		let ctx = use_context_provider(|| {
			let mut conn = Connection::new();
			conn.status = ConnectionStatus::Connected;
			AppCtx::new(conn, Forms::default(), Nav::default(), Prefs::default(), false)
		});
		use_hook(move || {
			let mut ch = Channel { user_channel_id: "7".into(), channel_id: "c7".into(), counterparty_node_id: "02aa".into(), channel_value_sats: 1_000_000, is_channel_ready: true, is_usable: true, ..Default::default() };
			if new_fields {
				ch.short_channel_id = Some((900_000u64 << 40) | (12 << 16) | 1);
				ch.outbound_scid_alias = Some(123_456);
				ch.inbound_htlc_maximum_msat = Some(400_000_000);
				ch.inbound_htlc_minimum_msat = 1_000;
				ch.reserve_type = Some(2);
				ch.channel_shutdown_state = Some(4);
			}
			let mut data = ctx.data;
			data.write().channels = Some(ListChannelsResponse { channels: vec![ch] });
		});
		rsx! { crate::ui::channels::ChannelDrawer { user_channel_id: "7".to_string() } }
	}

	#[test]
	fn the_channel_panel_shows_ldk_fields_only_when_reported() {
		let mut dom = VirtualDom::new_with_props(ChannelPanelHarness, ChannelPanelHarnessProps { new_fields: true });
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		for text in ["Short channel ID", "900000x12x1", "SCID aliases", "out 123456", "in —", "Largest payment it can receive", "Smallest payment it accepts", "Reserve type", "No reserve (trusted peer)", "Close stage", "negotiating fee", "Closing · negotiating fee"] {
			assert!(html.contains(text), "missing {text}");
		}
		let mut dom = VirtualDom::new_with_props(ChannelPanelHarness, ChannelPanelHarnessProps { new_fields: false });
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		for text in ["Short channel ID", "SCID aliases", "Largest payment", "Smallest payment", "Reserve type", "Close stage"] {
			assert!(!html.contains(text), "{text} must not render as 0 or unknown");
		}
		assert!(html.contains("Active"));
	}

	#[component]
	fn OnchainPaymentHarness() -> Element {
		let ctx = use_context_provider(|| {
			let mut conn = Connection::new();
			conn.status = ConnectionStatus::Connected;
			AppCtx::new(conn, Forms::default(), Nav { active_tab: ActiveTab::Payments, ..Default::default() }, Prefs::default(), false)
		});
		use_hook(move || {
			use sc_rest_client::ldk_server_grpc::types::{payment_kind, transaction_type, CooperativeClose, Onchain, PaymentKind, TransactionType};
			let onchain = |id: &str, kind: Option<transaction_type::Kind>| Payment {
				payment_id: id.into(),
				kind: Some(PaymentKind { kind: Some(payment_kind::Kind::Onchain(Onchain { txid: "t".repeat(64), status: None, tx_type: kind.map(|kind| TransactionType { kind: Some(kind) }) })) }),
				amount_msat: Some(5_000_000),
				direction: 1,
				status: 1,
				..Default::default()
			};
			let mut data = ctx.data;
			data.write().payments = Some(ListPaymentsResponse {
				payments: vec![
					onchain("close", Some(transaction_type::Kind::CooperativeClose(CooperativeClose { channel_id: "c".into(), counterparty_node_id: "02aa".into() }))),
					onchain("plain", None),
				],
				next_page_token: None,
			});
		});
		rsx! { CurrentTab {} }
	}

	#[test]
	fn onchain_payments_are_labelled_by_transaction_type() {
		let mut dom = VirtualDom::new(OnchainPaymentHarness);
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		assert!(html.contains(">Co-op close<"));
		assert!(html.contains(">On-chain<"), "an unclassified transaction keeps the plain label");
		assert!(html.contains("option value=\"Force close\""), "the type filter offers the new labels");
	}

	#[test]
	fn lightning_offers_every_payment_flow() {
		let html = render(ActiveTab::Lightning, Seed::ConnectedEmpty);
		for label in ["BOLT11 Send", "BOLT11 Receive", "BOLT12 Send", "BOLT12 Receive", "Keysend", "Pay Invoice"] {
			assert!(html.contains(label), "missing {label}");
		}
	}

	#[component]
	fn HistoryHarness() -> Element {
		let ctx = use_context_provider(|| {
			let mut conn = Connection::new();
			conn.status = ConnectionStatus::Connected;
			let nav = Nav { active_tab: ActiveTab::Logs, logs_tab: crate::state::LogsTab::ChannelLedger, ..Default::default() };
			AppCtx::new(conn, Forms::default(), nav, Prefs::default(), false)
		});
		use_hook(move || {
			let mut forms = ctx.forms;
			forms.write().channel_ledger.identifier = "uid-1".into();
			let ev = |id: i64, event_type: &str, detail: &str| sc_rest_client::sc_protos::stable::ChannelLedgerEvent {
				id,
				event_type: event_type.into(),
				occurred_at_ms: 1_790_000_000_000 + id,
				detail_json: detail.into(),
				..Default::default()
			};
			let mut data = ctx.data;
			data.write().channel_ledger = Some(ListChannelLedgerEventsResponse {
				events: vec![ev(1, "CHANNEL_PENDING", "{}"), ev(2, "SYNC_MESSAGE_SENT", r#"{"expected_usd":43.63}"#)],
				// Older rows exist beyond the loaded page.
				overview: Some(sc_rest_client::sc_protos::stable::ChannelLedgerOverview {
					oldest_occurred_at_ms: Some(1_780_000_000_000),
					..Default::default()
				}),
				..Default::default()
			});
		});
		rsx! { CurrentTab {} }
	}

	#[test]
	fn channel_history_reads_as_sentences() {
		let mut dom = VirtualDom::new(HistoryHarness);
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		assert!(html.contains("Channel History"));
		assert!(html.contains("LSP published balances: target $43.63"));
		assert!(html.contains("Show technical events"));
		assert!(!html.contains("Raw JSON"), "technical cards stay hidden by default");
		assert!(html.contains(&crate::history::day_label(1_780_000_000_000)), "history start comes from the whole ledger, not the loaded page");
	}

	#[component]
	fn LocationHarness(connected: Option<bool>, last_seen_at: i64) -> Element {
		let ctx = use_context_provider(|| {
			let mut conn = Connection::new();
			conn.status = ConnectionStatus::Connected;
			AppCtx::new(conn, Forms::default(), Nav { active_tab: ActiveTab::StableChannels, ..Default::default() }, Prefs::default(), false)
		});
		use_hook(move || {
			use sc_rest_client::sc_protos::stable::{ListStableChannelsResponse, PeerLocation, StableChannelInfo};
			let mut data = ctx.data;
			data.write().stable_channels = Some(ListStableChannelsResponse {
				channels: vec![StableChannelInfo {
					user_channel_id: "u".into(),
					counterparty: "02ab".into(),
					expected_usd: 10.0,
					expected_msats: 10_000_000,
					recent_locations: vec![PeerLocation {
						ip: "118.95.161.42".into(),
						country_code: "IN".into(),
						country_name: "India".into(),
						first_seen_at: 1_790_000_000,
						last_seen_at,
					}],
					..Default::default()
				}],
			});
			if let Some(is_connected) = connected {
				use sc_rest_client::ldk_server_grpc::{api::ListPeersResponse, types::Peer};
				let peer = Peer { node_id: "02ab".into(), is_connected, ..Default::default() };
				data.write().peers = Some(ListPeersResponse { peers: vec![peer], ..Default::default() });
			}
		});
		rsx! { CurrentTab {} }
	}

	#[test]
	fn stable_tab_shows_where_each_user_connects_from() {
		let html = render_location(None, 1_790_000_000);
		assert!(html.contains("Location"));
		assert!(html.contains("IN · India"));
		assert!(html.contains("118.95.161.42"));
		assert!(html.contains("seen "), "peer list not loaded: the cell shows when it was seen");
	}

	fn render_location(connected: Option<bool>, last_seen_at: i64) -> String {
		let mut dom = VirtualDom::new_with_props(LocationHarness, LocationHarnessProps { connected, last_seen_at });
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	#[test]
	fn a_connected_wallet_hides_seen_until_its_sighting_goes_stale() {
		let now = chrono::Utc::now().timestamp();
		assert!(!render_location(Some(true), now).contains("seen "));
		assert!(render_location(Some(true), now - 3_600).contains("seen 1 hour ago"));
	}

	thread_local! {
		static OBSERVED: RefCell<Option<(u64, bool, bool, bool)>> = const { RefCell::new(None) };
	}

	#[component]
	fn InvalidationProbe() -> Element {
		let ctx = use_context_provider(|| {
			AppCtx::new(Connection::new(), Forms::default(), Nav::default(), Prefs::default(), false)
		});
		use_hook(move || {
			let mut data = ctx.data;
			data.write().channel_ledger = Some(ListChannelLedgerEventsResponse::default());
			data.write().channel_ledger_cursor = Some("50".into());
			let mut pending = ctx.pending;
			pending.write().insert(Op::ChannelLedger);
			pending.write().insert(Op::ChannelLedgerExport);
			crate::actions::invalidate_channel_ledger(ctx);
			let d = ctx.data.peek();
			let p = ctx.pending.peek();
			OBSERVED.with(|o| {
				*o.borrow_mut() = Some((
					*ctx.ledger_gen.peek(),
					d.channel_ledger.is_none() && d.channel_ledger_cursor.is_none(),
					!p.has(Op::ChannelLedger),
					!p.has(Op::ChannelLedgerExport),
				))
			});
		});
		rsx! {}
	}

	#[test]
	fn ledger_invalidation_drops_data_and_frees_pending_slots() {
		let mut dom = VirtualDom::new(InvalidationProbe);
		dom.rebuild_in_place();
		assert_eq!(OBSERVED.with(|o| *o.borrow()), Some((1, true, true, true)));
	}

	/// Custom properties declared inside the first `{ ... }` block that follows `selector`.
	fn tokens_after(css: &str, selector: &str) -> BTreeSet<String> {
		let start = css.find(selector).expect("selector present");
		let open = start + css[start..].find('{').unwrap();
		let close = open + css[open..].find('}').unwrap();
		css[open..close]
			.lines()
			.filter_map(|line| line.trim().strip_prefix("--"))
			.filter_map(|decl| decl.split(':').next())
			.map(str::to_owned)
			.collect()
	}

	#[test]
	fn key_value_labels_keep_their_gap_from_the_value() {
		let shared = CSS.split(".kv > .k, .kv > .v {").nth(1).expect("shared key/value rule").split('}').next().unwrap();
		assert!(!shared.contains("padding:"), "a padding shorthand here resets the label's right padding to zero");
		let label = CSS.split(".kv > .k {").nth(1).expect("label rule").split('}').next().unwrap();
		assert!(label.contains("padding-right: 24px"));
	}

	#[test]
	fn every_theme_defines_the_same_tokens() {
		let dark = tokens_after(CSS, ".app[data-theme=\"dark\"] {");
		let system_light = tokens_after(CSS, ".app:not([data-theme=\"dark\"]) {");
		let forced_light = tokens_after(CSS, ".app[data-theme=\"light\"] {");
		assert!(dark.len() > 20);
		assert_eq!(dark, system_light);
		assert_eq!(dark, forced_light);
	}

	#[test]
	fn text_roles_and_accent_are_distinct() {
		let value = |block: &str, token: &str| {
			let start = CSS.find(block).unwrap();
			let body = &CSS[start..start + CSS[start..].find('}').unwrap()];
			let line = body.lines().find(|l| l.trim().starts_with(&format!("--{token}:"))).unwrap();
			line.split(':').nth(1).unwrap().trim().trim_end_matches(';').to_owned()
		};
		for block in [".app[data-theme=\"dark\"] {", ".app[data-theme=\"light\"] {"] {
			let (text, secondary, accent) = (value(block, "text"), value(block, "text-2"), value(block, "orange"));
			assert_ne!(text, secondary);
			assert_ne!(secondary, accent);
			assert_ne!(text, accent);
		}
	}

	#[component]
	fn RevenueHarness(loaded: bool) -> Element {
		let ctx = use_context_provider(|| {
			let mut conn = Connection::new();
			conn.status = ConnectionStatus::Connected;
			AppCtx::new(conn, Forms::default(), Nav { active_tab: ActiveTab::Revenue, ..Default::default() }, Prefs::default(), false)
		});
		use_hook(move || {
			use sc_rest_client::sc_protos::revenue::{GetRevenueResponse, RevenueItem, RevenueLine};
			if !loaded {
				return;
			}
			let line = |category: &str, count: u64, total_msat: u64| RevenueLine { category: category.into(), direction: String::new(), count, total_msat };
			let items = vec![RevenueItem {
				key: "t1".into(),
				occurred_at: 1_790_000_000,
				category: "trade_fee".into(),
				direction: "in".into(),
				amount_msat: 2_000_000,
				node_id: "02ab".into(),
				payment_id: "t1".into(),
				trade_rejected: true,
				..Default::default()
			}];
			let mut data = ctx.data;
			let mut d = data.write();
			d.revenue_items = items.clone();
			d.revenue = Some(GetRevenueResponse {
				lines: vec![line("trade_fee", 1, 2_000_000), line("trade_fee_rejected", 1, 2_000_000), line("channel_funding_fee", 1, 3_000_000), line("jit_open_fee", 2, 574_000), line("stability_out", 1, 50_000_000), line("close_fee", 1, 1_000_000), line("claim_sweep_fee", 2, 400_000)],
				items,
				next_cursor: None,
				snapshot_at: crate::format::now_secs() as i64,
				partial: false,
				untracked: vec!["onchain_fee".into(), "close_fee_bump".into()],
				item_count: 1,
			});
		});
		rsx! { CurrentTab {} }
	}

	fn render_revenue(loaded: bool) -> String {
		let mut dom = VirtualDom::new_with_props(RevenueHarness, RevenueHarnessProps { loaded });
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	#[test]
	fn revenue_tab_shows_earned_spent_net_and_the_peg_apart() {
		let html = render_revenue(true);
		assert!(html.contains("1 rejected · show</button>"), "the rejected count filters the list to those trades");
		assert!(html.contains(">Rejected trades</button>"), "a chip for rejected trades");
		for text in ["Earned", "Spent", "Net", "Stability net", "Peg settlements, not revenue", "JIT channel opens", "574 sats", "2 opens · 287 sats each", "not tracked", "Trade fee", "Activity"] {
			assert!(html.contains(text), "missing {text}");
		}
		assert!(!html.contains("free opening"), "the always-zero JIT fee line is gone");
		assert!(html.contains("Trade and routing fees") && !html.contains("JIT fees"), "no JIT earnings while opens are free");
		assert!(html.contains("\u{2264} \u{2212}2,974 sats"), "net is a ceiling: 2,000 earned − 4,974 known spent, on-chain fees untracked");
		assert!(html.contains("\u{2265} 4,974 sats"), "spent is a floor");
		assert!(html.contains("At most: some fees are not tracked"));
		for text in ["Channel closes", "Close fee bumps", "Claims &", ">Channel close</button>", ">Claim or sweep</button>"] {
			assert!(html.contains(text), "missing {text}");
		}
		assert_eq!(html.matches("not tracked on this node").count(), 2, "on-chain fees and close fee bumps are unknown, never 0");
		assert!(html.contains("\u{2212}50,000 sats"), "stability net is negative");
		assert!(html.contains(">Refund<"), "a rejected trade fee row offers Refund");
		assert!(html.contains("not tracked on this node"), "an untracked category never reads as zero");
	}

	#[test]
	fn revenue_tab_without_data_offers_refresh() {
		let html = render_revenue(false);
		assert!(html.contains("Refresh"));
	}

	#[component]
	fn RefundDialogHarness() -> Element {
		let ctx = use_context_provider(|| AppCtx::new(Connection::new(), Forms::default(), Nav::default(), Prefs::default(), false));
		use_hook(move || {
			let mut forms = ctx.forms;
			forms.write().refund_trade_fee = crate::state::RefundTradeFeeForm { trade_payment_id: "t1".into(), amount_msat: 2_000_000, node_id: "02ab".into() };
		});
		rsx! { crate::ui::revenue::RefundTradeFeeDialog {} }
	}

	#[test]
	fn the_refund_dialog_names_the_amount_and_the_user() {
		let mut dom = VirtualDom::new(RefundDialogHarness);
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		assert!(html.contains("Send 2,000 sats back to"));
		assert!(html.contains("Send refund"));
	}
}
