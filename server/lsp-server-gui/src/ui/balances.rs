use egui::Ui;
use egui_extras::{Column, TableBuilder};

use crate::app::LspServerApp;
use crate::ui::layout::{card, kv_grid_custom, page_scrolled};
use crate::ui::widgets;

pub fn render(ui: &mut Ui, app: &mut LspServerApp) {
	ui.heading("Balances");
	ui.add_space(10.0);

	if app.render_disconnected_gate(ui) {
		return;
	}

	ui.horizontal(|ui| {
		if app.state.tasks.balances.is_some() {
			widgets::loading_row(ui, "Loading balances...");
		} else if ui.button("Refresh").clicked() {
			app.fetch_balances();
		}
	});

	ui.add_space(10.0);

	if app.state.balances.is_none() {
		widgets::empty_state(ui, "💰", "No balance data", "Click Refresh to load");
		return;
	}

	// Extract headline totals into locals before borrowing state for iteration.
	let total_onchain = app.state.balances.as_ref().map(|b| b.total_onchain_balance_sats).unwrap_or(0);
	let spendable = app.state.balances.as_ref().map(|b| b.spendable_onchain_balance_sats).unwrap_or(0);
	let reserve = app.state.balances.as_ref().map(|b| b.total_anchor_channels_reserve_sats).unwrap_or(0);
	let total_lightning = app.state.balances.as_ref().map(|b| b.total_lightning_balance_sats).unwrap_or(0);

	// Build a per-amount formatter once, before we borrow state.
	let fmt = |sats: u64| app.fmt_sats(sats);

	let onchain_val = fmt(spendable);
	let onchain_sec = format!("reserve {} | total {}", fmt(reserve), fmt(total_onchain));
	let ln_val = fmt(total_lightning);
	let onchain_total_str = fmt(total_onchain);
	let onchain_spendable_str = fmt(spendable);
	let onchain_reserve_str = fmt(reserve);
	let lightning_total_str = fmt(total_lightning);

	// Snapshot lightning-balance rows into owned strings before the scrolled closure.
	let lightning_rows: Vec<[String; 4]> = app
		.state
		.balances
		.as_ref()
		.map(|b| {
			b.lightning_balances
				.iter()
				.filter_map(|balance| balance.balance_type.as_ref().map(|bt| lightning_balance_row(bt, &fmt)))
				.collect()
		})
		.unwrap_or_default();

	let pending_sweeps: Vec<(usize, String)> = app
		.state
		.balances
		.as_ref()
		.map(|b| {
			b.pending_balances_from_channel_closures
				.iter()
				.enumerate()
				.filter_map(|(i, sweep)| {
					sweep.balance_type.as_ref().map(|bt| (i, pending_sweep_text(bt, &fmt)))
				})
				.collect()
		})
		.unwrap_or_default();

	page_scrolled(ui, |ui| {
		ui.columns(2, |cols| {
			widgets::stat_card(&mut cols[0], "On-chain Spendable", &onchain_val, &onchain_sec);
			widgets::stat_card(&mut cols[1], "Lightning Total", &ln_val, "");
		});

		ui.add_space(10.0);

		card(ui, "On-chain Balance", |ui| {
			let rows: crate::ui::layout::KvRows = vec![
				(
					"Total",
					Box::new(|ui: &mut egui::Ui| {
						ui.label(&onchain_total_str).on_hover_text(format!("{} sats", crate::ui::format_sats(total_onchain)));
					}),
				),
				(
					"Spendable",
					Box::new(|ui: &mut egui::Ui| {
						ui.label(&onchain_spendable_str).on_hover_text(format!("{} sats", crate::ui::format_sats(spendable)));
					}),
				),
				(
					"Anchor Reserve",
					Box::new(|ui: &mut egui::Ui| {
						ui.label(&onchain_reserve_str).on_hover_text(format!("{} sats", crate::ui::format_sats(reserve)));
					}),
				),
			];
			kv_grid_custom(ui, "onchain_balance", rows);
		});

		ui.add_space(10.0);

		card(ui, "Lightning Balance", |ui| {
			let rows: crate::ui::layout::KvRows = vec![(
				"Total",
				Box::new(|ui: &mut egui::Ui| {
					ui.label(&lightning_total_str).on_hover_text(format!("{} sats", crate::ui::format_sats(total_lightning)));
				}),
			)];
			kv_grid_custom(ui, "lightning_balance", rows);

			if !lightning_rows.is_empty() {
				ui.add_space(8.0);
				ui.label(format!("Details ({} items)", lightning_rows.len()));
				ui.add_space(4.0);
				crate::ui::layout::h_scroll(ui, 600.0, |ui| {
					TableBuilder::new(ui)
						.striped(true)
						.resizable(false)
						.vscroll(false)
						.cell_layout(egui::Layout::left_to_right(egui::Align::Center))
						.auto_shrink([false, true])
						.column(Column::remainder().at_least(64.0).clip(true)) // Type
						.column(Column::remainder().at_least(64.0).clip(true)) // Channel
						.column(Column::auto()) // Amount
						.column(Column::remainder().at_least(64.0).clip(true)) // Extra
						.header(22.0, |mut h| {
							h.col(|ui| { ui.strong("Type"); });
							h.col(|ui| { ui.strong("Channel"); });
							h.col(|ui| { ui.strong("Amount"); });
							h.col(|ui| { ui.strong("Extra"); });
						})
						.body(|mut body| {
							for row in &lightning_rows {
								body.row(24.0, |mut r| {
									r.col(|ui| { ui.label(&row[0]); });
									r.col(|ui| { ui.monospace(&row[1]); });
									r.col(|ui| { ui.monospace(&row[2]); });
									r.col(|ui| { ui.label(&row[3]); });
								});
							}
						});
				});
			}
		});

		ui.add_space(10.0);

		if !pending_sweeps.is_empty() {
			card(ui, "Pending Sweep Balances", |ui| {
				for (i, text) in &pending_sweeps {
					ui.group(|ui| {
						ui.label(format!("Sweep #{}", i + 1));
						ui.label(text);
					});
				}
			});
		}
	});
}

fn lightning_balance_row(
	balance: &sc_rest_client::ldk_server_grpc::types::lightning_balance::BalanceType,
	fmt: &dyn Fn(u64) -> String,
) -> [String; 4] {
	use sc_rest_client::ldk_server_grpc::types::lightning_balance::BalanceType;

	match balance {
		BalanceType::ClaimableOnChannelClose(b) => [
			"Claimable on Channel Close".to_string(),
			crate::ui::truncate_id(&b.channel_id, 8, 8),
			fmt(b.amount_satoshis),
			String::new(),
		],
		BalanceType::ClaimableAwaitingConfirmations(b) => [
			"Awaiting Confirmations".to_string(),
			crate::ui::truncate_id(&b.channel_id, 8, 8),
			fmt(b.amount_satoshis),
			format!("Confirmation Height: {}", b.confirmation_height),
		],
		BalanceType::ContentiousClaimable(b) => [
			"Contentious Claimable".to_string(),
			crate::ui::truncate_id(&b.channel_id, 8, 8),
			fmt(b.amount_satoshis),
			format!("Timeout Height: {}", b.timeout_height),
		],
		BalanceType::MaybeTimeoutClaimableHtlc(b) => [
			"Maybe Timeout Claimable HTLC".to_string(),
			crate::ui::truncate_id(&b.channel_id, 8, 8),
			fmt(b.amount_satoshis),
			format!("Claimable Height: {}", b.claimable_height),
		],
		BalanceType::MaybePreimageClaimableHtlc(b) => [
			"Maybe Preimage Claimable HTLC".to_string(),
			crate::ui::truncate_id(&b.channel_id, 8, 8),
			fmt(b.amount_satoshis),
			format!("Expiry Height: {}", b.expiry_height),
		],
		BalanceType::CounterpartyRevokedOutputClaimable(b) => [
			"Counterparty Revoked Output".to_string(),
			crate::ui::truncate_id(&b.channel_id, 8, 8),
			fmt(b.amount_satoshis),
			String::new(),
		],
	}
}

fn pending_sweep_text(
	balance: &sc_rest_client::ldk_server_grpc::types::pending_sweep_balance::BalanceType,
	fmt: &dyn Fn(u64) -> String,
) -> String {
	use sc_rest_client::ldk_server_grpc::types::pending_sweep_balance::BalanceType;

	match balance {
		BalanceType::PendingBroadcast(b) => {
			let ch_line = b.channel_id.as_ref().map(|c| format!("Channel: {}\n", crate::ui::truncate_id(c, 8, 8))).unwrap_or_default();
			format!("Type: Pending Broadcast\n{}Amount: {}", ch_line, fmt(b.amount_satoshis))
		},
		BalanceType::BroadcastAwaitingConfirmation(b) => {
			let ch_line = b.channel_id.as_ref().map(|c| format!("Channel: {}\n", crate::ui::truncate_id(c, 8, 8))).unwrap_or_default();
			format!(
				"Type: Broadcast Awaiting Confirmation\n{}Amount: {}\nTXID: {}",
				ch_line,
				fmt(b.amount_satoshis),
				crate::ui::truncate_id(&b.latest_spending_txid, 8, 8)
			)
		},
		BalanceType::AwaitingThresholdConfirmations(b) => {
			let ch_line = b.channel_id.as_ref().map(|c| format!("Channel: {}\n", crate::ui::truncate_id(c, 8, 8))).unwrap_or_default();
			format!(
				"Type: Awaiting Threshold Confirmations\n{}Amount: {}\nConfirmed at height: {}",
				ch_line,
				fmt(b.amount_satoshis),
				b.confirmation_height
			)
		},
	}
}
