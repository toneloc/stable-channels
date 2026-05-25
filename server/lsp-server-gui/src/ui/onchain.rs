use egui::{ScrollArea, Ui};
#[cfg(target_arch = "wasm32")]
use web_sys::js_sys;

use crate::app::LspServerApp;
use crate::state::{ConnectionStatus, OnchainTab};
use crate::ui::{format_sats, truncate_id};

pub fn render(ui: &mut Ui, app: &mut LspServerApp) {
	ui.heading("On-chain Transactions");
	ui.add_space(10.0);

	if !matches!(app.state.connection_status, ConnectionStatus::Connected) {
		ui.label("Connect to a server to use on-chain transactions.");
		return;
	}

	ui.horizontal(|ui| {
		if ui.selectable_label(app.state.onchain_tab == OnchainTab::Send, "Send").clicked() {
			app.state.onchain_tab = OnchainTab::Send;
		}
		if ui.selectable_label(app.state.onchain_tab == OnchainTab::Receive, "Receive").clicked() {
			app.state.onchain_tab = OnchainTab::Receive;
		}
		if ui.selectable_label(app.state.onchain_tab == OnchainTab::History, "History").clicked() {
			app.state.onchain_tab = OnchainTab::History;
			// Fetch payments if not already loaded
			if app.state.payments.is_none() {
				app.fetch_payments();
			}
		}
	});

	ui.separator();
	ui.add_space(10.0);

	match app.state.onchain_tab {
		OnchainTab::Send => render_send(ui, app),
		OnchainTab::Receive => render_receive(ui, app),
		OnchainTab::History => render_history(ui, app),
	}
}

fn render_send(ui: &mut Ui, app: &mut LspServerApp) {
	ui.group(|ui| {
		ui.heading("Send On-chain");
		ui.add_space(5.0);

		let form = &mut app.state.forms.onchain_send;

		egui::Grid::new("onchain_send_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
			ui.label("Address:");
			ui.text_edit_singleline(&mut form.address);
			ui.end_row();

			ui.label("Amount (sats):");
			ui.add_enabled(!form.send_all, egui::TextEdit::singleline(&mut form.amount_sats));
			ui.end_row();

			ui.label("Send All:");
			ui.checkbox(&mut form.send_all, "Send entire balance");
			ui.end_row();

			ui.label("Fee Rate (sat/vB, optional):");
			ui.text_edit_singleline(&mut form.fee_rate_sat_per_vb);
			ui.end_row();
		});

		ui.add_space(10.0);

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.onchain_send.is_some();
			if is_pending {
				ui.spinner();
				ui.label("Sending...");
			} else if ui.button("Send").clicked() {
				app.send_onchain();
			}
		});

		if let Some(txid) = &app.state.last_txid {
			ui.add_space(10.0);
			ui.separator();
			ui.horizontal(|ui| {
				ui.label("Last TXID:");
				ui.monospace(crate::ui::truncate_id(txid, 12, 12));
				if ui.small_button("Copy").clicked() {
					ui.output_mut(|o| o.copied_text = txid.clone());
				}
			});
		}
	});
}

fn render_receive(ui: &mut Ui, app: &mut LspServerApp) {
	ui.group(|ui| {
		ui.heading("Receive On-chain");
		ui.add_space(5.0);

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.onchain_receive.is_some();
			if is_pending {
				ui.spinner();
				ui.label("Generating...");
			} else if ui.button("Generate Address").clicked() {
				app.generate_onchain_address();
			}
		});

		if let Some(address) = &app.state.onchain_address {
			ui.add_space(10.0);
			ui.separator();
			ui.label("Address:");
			ui.add(
				egui::TextEdit::singleline(&mut address.as_str())
					.desired_width(f32::INFINITY)
					.interactive(false),
			);
			if ui.button("Copy Address").clicked() {
				ui.output_mut(|o| o.copied_text = address.clone());
			}
		}
	});
}

fn render_history(ui: &mut Ui, app: &mut LspServerApp) {
	ui.heading("On-chain History");
	ui.add_space(10.0);

	// Show balances summary
	if let Some(balances) = &app.state.balances {
		ui.group(|ui| {
			ui.label("Wallet Summary");
			ui.add_space(5.0);
			egui::Grid::new("onchain_summary_grid").num_columns(2).spacing([10.0, 4.0]).show(
				ui,
				|ui| {
					ui.label("Total Balance:");
					ui.label(format!("{} sats", format_sats(balances.total_onchain_balance_sats)));
					ui.end_row();

					ui.label("Spendable:");
					ui.label(format!(
						"{} sats",
						format_sats(balances.spendable_onchain_balance_sats)
					));
					ui.end_row();

					if balances.total_anchor_channels_reserve_sats > 0 {
						ui.label("Anchor Reserve:");
						ui.label(format!(
							"{} sats",
							format_sats(balances.total_anchor_channels_reserve_sats)
						));
						ui.end_row();
					}
				},
			);
		});

		ui.add_space(10.0);

		// Show pending sweeps if any
		if !balances.pending_balances_from_channel_closures.is_empty() {
			ui.group(|ui| {
				ui.label("Pending Sweeps");
				ui.add_space(5.0);
				for sweep in &balances.pending_balances_from_channel_closures {
					if let Some(balance_type) = &sweep.balance_type {
						render_pending_sweep(ui, balance_type);
						ui.add_space(3.0);
					}
				}
			});
			ui.add_space(10.0);
		}
	}

	// Note about transaction history
	ui.group(|ui| {
		ui.label(egui::RichText::new("Transaction History").strong());
		ui.add_space(5.0);
		ui.label("Full on-chain transaction history is not yet available.");
		ui.label(
			egui::RichText::new(
				"ldk-node does not currently expose BDK wallet transaction history.",
			)
			.small()
			.color(egui::Color32::GRAY),
		);

		if let Some(txid) = &app.state.last_txid {
			ui.add_space(10.0);
			ui.separator();
			ui.horizontal(|ui| {
				ui.label("Last Sent TXID:");
				ui.monospace(truncate_id(txid, 8, 8));
				if ui.small_button("Copy").clicked() {
					ui.output_mut(|o| o.copied_text = txid.clone());
				}
			});
		}
	});
}

fn render_pending_sweep(
	ui: &mut Ui,
	balance_type: &sc_rest_client::ldk_server_grpc::types::pending_sweep_balance::BalanceType,
) {
	use sc_rest_client::ldk_server_grpc::types::pending_sweep_balance::BalanceType;

	match balance_type {
		BalanceType::PendingBroadcast(b) => {
			ui.horizontal(|ui| {
				ui.colored_label(egui::Color32::YELLOW, "Pending Broadcast");
				ui.label(format!("{} sats", format_sats(b.amount_satoshis)));
			});
		},
		BalanceType::BroadcastAwaitingConfirmation(b) => {
			ui.horizontal(|ui| {
				ui.colored_label(egui::Color32::YELLOW, "Awaiting Confirmation");
				ui.label(format!("{} sats", format_sats(b.amount_satoshis)));
			});
			ui.horizontal(|ui| {
				ui.label("TXID:");
				ui.monospace(truncate_id(&b.latest_spending_txid, 8, 8));
				if ui.small_button("Copy").clicked() {
					ui.output_mut(|o| o.copied_text = b.latest_spending_txid.clone());
				}
			});
		},
		BalanceType::AwaitingThresholdConfirmations(b) => {
			ui.horizontal(|ui| {
				ui.colored_label(egui::Color32::GREEN, "Awaiting Threshold");
				ui.label(format!(
					"{} sats (height {})",
					format_sats(b.amount_satoshis),
					b.confirmation_height
				));
			});
		},
	}
}

#[allow(dead_code)]
fn render_history_table(ui: &mut Ui, app: &mut LspServerApp) {
	// This function is kept for future use when ldk-node exposes transaction history
	ui.horizontal(|ui| {
		ui.heading("Transaction History");
		if app.state.tasks.payments.is_some() {
			ui.spinner();
		} else if ui.button("Refresh").clicked() {
			app.state.payments_page_token = None;
			app.fetch_payments();
		}
	});

	ui.add_space(10.0);

	if let Some(payments_response) = &app.state.payments {
		use sc_rest_client::ldk_server_grpc::types::payment_kind::Kind;

		// Filter to only onchain payments
		let onchain_payments: Vec<_> = payments_response
			.payments
			.iter()
			.filter(|p| {
				p.kind
					.as_ref()
					.and_then(|k| k.kind.as_ref())
					.map(|k| matches!(k, Kind::Onchain(_)))
					.unwrap_or(false)
			})
			.collect();

		if onchain_payments.is_empty() {
			ui.label("No on-chain transactions found in payment history.");
		} else {
			ui.label(format!("{} on-chain transaction(s)", onchain_payments.len()));
			ui.add_space(5.0);

			ScrollArea::both().max_height(400.0).show(ui, |ui| {
				egui::Grid::new("onchain_history_grid").striped(true).spacing([12.0, 6.0]).show(
					ui,
					|ui| {
						// Header
						ui.strong("Payment ID");
						ui.strong("TXID");
						ui.strong("Amount");
						ui.strong("Direction");
						ui.strong("Status");
						ui.strong("Time");
						ui.end_row();

						for payment in onchain_payments {
							// Payment ID
							ui.horizontal(|ui| {
								ui.monospace(truncate_id(&payment.id, 5, 4));
								if ui.small_button("Copy").clicked() {
									ui.output_mut(|o| o.copied_text = payment.id.clone());
								}
							});

							// TXID (from onchain kind)
							if let Some(kind) = &payment.kind {
								if let Some(Kind::Onchain(onchain)) = &kind.kind {
									ui.horizontal(|ui| {
										ui.monospace(truncate_id(&onchain.txid, 5, 4));
										if ui.small_button("Copy").clicked() {
											ui.output_mut(|o| o.copied_text = onchain.txid.clone());
										}
									});
								} else {
									ui.label("-");
								}
							} else {
								ui.label("-");
							}

							// Amount
							if let Some(amount) = payment.amount_msat {
								ui.label(format!("{} sats", format_sats(amount / 1000)));
							} else {
								ui.label("-");
							}

							// Direction
							let direction = match payment.direction {
								0 => "Receive",
								1 => "Send",
								_ => "Unknown",
							};
							ui.label(direction);

							// Status
							match payment.status {
								0 => ui.colored_label(egui::Color32::YELLOW, "Pending"),
								1 => ui.colored_label(egui::Color32::GREEN, "Confirmed"),
								2 => ui.colored_label(egui::Color32::RED, "Failed"),
								_ => ui.label("Unknown"),
							};

							// Time
							ui.label(format_timestamp(payment.latest_update_timestamp));

							ui.end_row();
						}
					},
				);
			});
		}
	} else {
		ui.label("Loading transaction history...");
		if app.state.tasks.payments.is_none() {
			app.fetch_payments();
		}
	}
}

fn format_timestamp(ts: u64) -> String {
	#[cfg(target_arch = "wasm32")]
	let now_secs = (js_sys::Date::now() / 1000.0) as u64;

	#[cfg(not(target_arch = "wasm32"))]
	let now_secs = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.as_secs())
		.unwrap_or(0);

	if now_secs >= ts {
		let secs = now_secs - ts;
		if secs < 60 {
			format!("{}s ago", secs)
		} else if secs < 3600 {
			format!("{}m ago", secs / 60)
		} else if secs < 86400 {
			format!("{}h ago", secs / 3600)
		} else {
			format!("{}d ago", secs / 86400)
		}
	} else {
		format!("{}", ts)
	}
}
