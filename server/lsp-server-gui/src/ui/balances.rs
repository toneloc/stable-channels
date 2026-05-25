use egui::Ui;

use crate::app::LspServerApp;
use crate::state::ConnectionStatus;
use crate::ui::format_sats;

pub fn render(ui: &mut Ui, app: &mut LspServerApp) {
	ui.heading("Balances");
	ui.add_space(10.0);

	if !matches!(app.state.connection_status, ConnectionStatus::Connected) {
		ui.label("Connect to a server to view balances.");
		return;
	}

	ui.horizontal(|ui| {
		if app.state.tasks.balances.is_some() {
			ui.spinner();
			ui.label("Loading...");
		} else if ui.button("Refresh").clicked() {
			app.fetch_balances();
		}
	});

	ui.add_space(10.0);

	if let Some(balances) = &app.state.balances {
		ui.group(|ui| {
			ui.heading("On-chain Balance");
			egui::Grid::new("onchain_balance_grid").num_columns(2).spacing([10.0, 5.0]).show(
				ui,
				|ui| {
					ui.label("Total:");
					ui.monospace(format!(
						"{} sats",
						format_sats(balances.total_onchain_balance_sats)
					));
					ui.end_row();

					ui.label("Spendable:");
					ui.monospace(format!(
						"{} sats",
						format_sats(balances.spendable_onchain_balance_sats)
					));
					ui.end_row();

					ui.label("Anchor Reserve:");
					ui.monospace(format!(
						"{} sats",
						format_sats(balances.total_anchor_channels_reserve_sats)
					));
					ui.end_row();
				},
			);
		});

		ui.add_space(10.0);

		ui.group(|ui| {
			ui.heading("Lightning Balance");
			ui.monospace(format!(
				"Total: {} sats",
				format_sats(balances.total_lightning_balance_sats)
			));

			if !balances.lightning_balances.is_empty() {
				ui.add_space(5.0);
				egui::CollapsingHeader::new(format!(
					"Lightning Balance Details ({} items)",
					balances.lightning_balances.len()
				))
				.show(ui, |ui| {
					for (i, balance) in balances.lightning_balances.iter().enumerate() {
						ui.group(|ui| {
							ui.label(format!("Balance #{}", i + 1));
							if let Some(balance_type) = &balance.balance_type {
								render_lightning_balance(ui, balance_type);
							}
						});
					}
				});
			}
		});

		ui.add_space(10.0);

		if !balances.pending_balances_from_channel_closures.is_empty() {
			ui.group(|ui| {
				ui.heading("Pending Sweep Balances");
				egui::CollapsingHeader::new(format!(
					"Pending Sweeps ({} items)",
					balances.pending_balances_from_channel_closures.len()
				))
				.show(ui, |ui| {
					for (i, sweep) in
						balances.pending_balances_from_channel_closures.iter().enumerate()
					{
						ui.group(|ui| {
							ui.label(format!("Sweep #{}", i + 1));
							if let Some(balance_type) = &sweep.balance_type {
								render_pending_sweep(ui, balance_type);
							}
						});
					}
				});
			});
		}
	} else {
		ui.label("No balance data available. Click Refresh to fetch.");
	}
}

fn render_lightning_balance(
	ui: &mut Ui,
	balance: &sc_rest_client::ldk_server_grpc::types::lightning_balance::BalanceType,
) {
	use sc_rest_client::ldk_server_grpc::types::lightning_balance::BalanceType;

	match balance {
		BalanceType::ClaimableOnChannelClose(b) => {
			ui.label("Type: Claimable on Channel Close");
			ui.label(format!("Channel: {}", crate::ui::truncate_id(&b.channel_id, 8, 8)));
			ui.label(format!("Amount: {} sats", format_sats(b.amount_satoshis)));
		},
		BalanceType::ClaimableAwaitingConfirmations(b) => {
			ui.label("Type: Awaiting Confirmations");
			ui.label(format!("Channel: {}", crate::ui::truncate_id(&b.channel_id, 8, 8)));
			ui.label(format!("Amount: {} sats", format_sats(b.amount_satoshis)));
			ui.label(format!("Confirmation Height: {}", b.confirmation_height));
		},
		BalanceType::ContentiousClaimable(b) => {
			ui.label("Type: Contentious Claimable");
			ui.label(format!("Channel: {}", crate::ui::truncate_id(&b.channel_id, 8, 8)));
			ui.label(format!("Amount: {} sats", format_sats(b.amount_satoshis)));
			ui.label(format!("Timeout Height: {}", b.timeout_height));
		},
		BalanceType::MaybeTimeoutClaimableHtlc(b) => {
			ui.label("Type: Maybe Timeout Claimable HTLC");
			ui.label(format!("Channel: {}", crate::ui::truncate_id(&b.channel_id, 8, 8)));
			ui.label(format!("Amount: {} sats", format_sats(b.amount_satoshis)));
			ui.label(format!("Claimable Height: {}", b.claimable_height));
		},
		BalanceType::MaybePreimageClaimableHtlc(b) => {
			ui.label("Type: Maybe Preimage Claimable HTLC");
			ui.label(format!("Channel: {}", crate::ui::truncate_id(&b.channel_id, 8, 8)));
			ui.label(format!("Amount: {} sats", format_sats(b.amount_satoshis)));
			ui.label(format!("Expiry Height: {}", b.expiry_height));
		},
		BalanceType::CounterpartyRevokedOutputClaimable(b) => {
			ui.label("Type: Counterparty Revoked Output");
			ui.label(format!("Channel: {}", crate::ui::truncate_id(&b.channel_id, 8, 8)));
			ui.label(format!("Amount: {} sats", format_sats(b.amount_satoshis)));
		},
	}
}

fn render_pending_sweep(
	ui: &mut Ui,
	balance: &sc_rest_client::ldk_server_grpc::types::pending_sweep_balance::BalanceType,
) {
	use sc_rest_client::ldk_server_grpc::types::pending_sweep_balance::BalanceType;

	match balance {
		BalanceType::PendingBroadcast(b) => {
			ui.label("Type: Pending Broadcast");
			if let Some(ch) = &b.channel_id {
				ui.label(format!("Channel: {}", crate::ui::truncate_id(ch, 8, 8)));
			}
			ui.label(format!("Amount: {} sats", format_sats(b.amount_satoshis)));
		},
		BalanceType::BroadcastAwaitingConfirmation(b) => {
			ui.label("Type: Broadcast Awaiting Confirmation");
			if let Some(ch) = &b.channel_id {
				ui.label(format!("Channel: {}", crate::ui::truncate_id(ch, 8, 8)));
			}
			ui.label(format!("Amount: {} sats", format_sats(b.amount_satoshis)));
			ui.label(format!("TXID: {}", crate::ui::truncate_id(&b.latest_spending_txid, 8, 8)));
		},
		BalanceType::AwaitingThresholdConfirmations(b) => {
			ui.label("Type: Awaiting Threshold Confirmations");
			if let Some(ch) = &b.channel_id {
				ui.label(format!("Channel: {}", crate::ui::truncate_id(ch, 8, 8)));
			}
			ui.label(format!("Amount: {} sats", format_sats(b.amount_satoshis)));
			ui.label(format!("Confirmed at height: {}", b.confirmation_height));
		},
	}
}
