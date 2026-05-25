use egui::{Context, ScrollArea, Ui};
use hex::DisplayHex;
#[cfg(target_arch = "wasm32")]
use web_sys::js_sys;

use crate::app::LspServerApp;
use crate::state::ConnectionStatus;
use crate::ui::{format_msat, truncate_id};

pub fn render(ui: &mut Ui, app: &mut LspServerApp) {
	ui.heading("Payments");
	ui.add_space(10.0);

	if !matches!(app.state.connection_status, ConnectionStatus::Connected) {
		ui.label("Connect to a server to view payments.");
		return;
	}

	ui.horizontal(|ui| {
		if app.state.tasks.payments.is_some() {
			ui.spinner();
			ui.label("Loading...");
		} else {
			if ui.button("Refresh").clicked() {
				app.state.payments_page_token = None;
				app.fetch_payments();
			}
			if app.state.payments_page_token.is_some() && ui.button("Load More").clicked() {
				app.fetch_payments();
			}
		}
	});

	ui.add_space(10.0);

	// Track which payment details button was clicked
	let mut clicked_payment_id: Option<String> = None;

	if let Some(payments_response) = &app.state.payments {
		let payments = &payments_response.payments;
		if payments.is_empty() {
			ui.label("No payments found.");
		} else {
			ui.label(format!("{} payment(s)", payments.len()));
			ui.add_space(5.0);

			ScrollArea::both().max_height(500.0).show(ui, |ui| {
				egui::Grid::new("payments_grid").striped(true).min_col_width(80.0).show(ui, |ui| {
					// Header
					ui.strong("Payment ID");
					ui.strong("Type");
					ui.strong("Amount");
					ui.strong("Fee");
					ui.strong("Direction");
					ui.strong("Status");
					ui.strong("Timestamp");
					ui.strong(""); // Details column
					ui.end_row();

					for payment in payments {
						// Payment ID
						ui.horizontal(|ui| {
							ui.monospace(truncate_id(&payment.id, 5, 4));
							if ui.small_button("Copy").clicked() {
								ui.output_mut(|o| o.copied_text = payment.id.clone());
							}
						});

						// Type
						let payment_type = payment
							.kind
							.as_ref()
							.map(|k| format_payment_kind(k))
							.unwrap_or_else(|| "Unknown".to_string());
						ui.label(payment_type);

						// Amount
						if let Some(amount) = payment.amount_msat {
							ui.label(format_msat(amount));
						} else {
							ui.label("-");
						}

						// Fee
						if let Some(fee) = payment.fee_paid_msat {
							ui.label(format_msat(fee));
						} else {
							ui.label("-");
						}

						// Direction (0 = Inbound, 1 = Outbound)
						let direction = match payment.direction {
							0 => "Inbound",
							1 => "Outbound",
							_ => "Unknown",
						};
						ui.label(direction);

						// Status (0 = Pending, 1 = Succeeded, 2 = Failed)
						match payment.status {
							0 => {
								ui.colored_label(egui::Color32::YELLOW, "Pending");
							},
							1 => {
								ui.colored_label(egui::Color32::GREEN, "Succeeded");
							},
							2 => {
								ui.colored_label(egui::Color32::RED, "Failed");
							},
							_ => {
								ui.label("Unknown");
							},
						};

						// Timestamp
						ui.label(format_timestamp(payment.latest_update_timestamp));

						// Details button - track click without modifying app state yet
						if ui.small_button("Details").clicked() {
							clicked_payment_id = Some(payment.id.clone());
						}

						ui.end_row();
					}
				});
			});

			if payments_response.next_page_token.is_some() {
				ui.add_space(5.0);
				ui.label("More payments available. Click 'Load More' to fetch.");
			}
		}
	} else {
		ui.label("No payment data available. Click Refresh to fetch.");
	}

	// Handle the Details button click outside the borrow
	if let Some(payment_id) = clicked_payment_id {
		app.state.payment_details_id = payment_id.clone();
		app.state.payment_details = None;
		app.state.show_payment_details_dialog = true;
		app.fetch_payment_details(payment_id);
	}
}

fn format_payment_kind(kind: &sc_rest_client::ldk_server_grpc::types::PaymentKind) -> String {
	use sc_rest_client::ldk_server_grpc::types::payment_kind::Kind;

	match &kind.kind {
		Some(Kind::Onchain(_)) => "On-chain".to_string(),
		Some(Kind::Bolt11(_)) => "BOLT11".to_string(),
		Some(Kind::Bolt11Jit(_)) => "BOLT11 JIT".to_string(),
		Some(Kind::Bolt12Offer(_)) => "BOLT12 Offer".to_string(),
		Some(Kind::Bolt12Refund(_)) => "BOLT12 Refund".to_string(),
		Some(Kind::Spontaneous(_)) => "Spontaneous".to_string(),
		None => "Unknown".to_string(),
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

pub fn render_dialogs(ctx: &Context, app: &mut LspServerApp) {
	render_payment_details_dialog(ctx, app);
}

fn render_payment_details_dialog(ctx: &Context, app: &mut LspServerApp) {
	if !app.state.show_payment_details_dialog {
		return;
	}

	egui::Window::new("Payment Details")
		.collapsible(false)
		.resizable(true)
		.default_width(500.0)
		.show(ctx, |ui| {
			if app.state.tasks.payment_details.is_some() {
				ui.horizontal(|ui| {
					ui.spinner();
					ui.label("Loading payment details...");
				});
			} else if let Some(response) = &app.state.payment_details {
				if let Some(payment) = &response.payment {
					egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
						egui::Grid::new("payment_details_grid")
							.num_columns(2)
							.spacing([10.0, 5.0])
							.show(ui, |ui| {
								// Payment ID
								ui.strong("Payment ID:");
								ui.horizontal(|ui| {
									ui.monospace(&payment.id);
									if ui.small_button("Copy").clicked() {
										ui.output_mut(|o| o.copied_text = payment.id.clone());
									}
								});
								ui.end_row();

								// Type
								ui.strong("Type:");
								let payment_type = payment
									.kind
									.as_ref()
									.map(|k| format_payment_kind(k))
									.unwrap_or_else(|| "Unknown".to_string());
								ui.label(payment_type);
								ui.end_row();

								// Amount
								ui.strong("Amount:");
								if let Some(amount) = payment.amount_msat {
									ui.label(format_msat(amount));
								} else {
									ui.label("-");
								}
								ui.end_row();

								// Fee
								ui.strong("Fee Paid:");
								if let Some(fee) = payment.fee_paid_msat {
									ui.label(format_msat(fee));
								} else {
									ui.label("-");
								}
								ui.end_row();

								// Direction
								ui.strong("Direction:");
								let direction = match payment.direction {
									0 => "Inbound",
									1 => "Outbound",
									_ => "Unknown",
								};
								ui.label(direction);
								ui.end_row();

								// Status
								ui.strong("Status:");
								match payment.status {
									0 => ui.colored_label(egui::Color32::YELLOW, "Pending"),
									1 => ui.colored_label(egui::Color32::GREEN, "Succeeded"),
									2 => ui.colored_label(egui::Color32::RED, "Failed"),
									_ => ui.label("Unknown"),
								};
								ui.end_row();

								// Timestamp
								ui.strong("Last Updated:");
								ui.label(format_timestamp(payment.latest_update_timestamp));
								ui.end_row();

								// Kind-specific details
								if let Some(kind) = &payment.kind {
									render_payment_kind_details(ui, kind);
								}
							});
					});
				} else {
					ui.label("Payment not found.");
				}
			} else {
				ui.label("No payment data.");
			}

			ui.add_space(10.0);
			ui.separator();

			if ui.button("Close").clicked() {
				app.state.show_payment_details_dialog = false;
				app.state.payment_details = None;
				app.state.payment_details_id.clear();
			}
		});
}

fn render_payment_kind_details(
	ui: &mut egui::Ui, kind: &sc_rest_client::ldk_server_grpc::types::PaymentKind,
) {
	use sc_rest_client::ldk_server_grpc::types::payment_kind::Kind;

	ui.strong("--- Details ---");
	ui.label("");
	ui.end_row();

	match &kind.kind {
		Some(Kind::Onchain(onchain)) => {
			ui.strong("Txid:");
			if !onchain.txid.is_empty() {
				ui.horizontal(|ui| {
					ui.monospace(truncate_id(&onchain.txid, 8, 8));
					if ui.small_button("Copy").clicked() {
						ui.output_mut(|o| o.copied_text = onchain.txid.clone());
					}
				});
			} else {
				ui.label("-");
			}
			ui.end_row();
		},
		Some(Kind::Bolt11(bolt11)) => {
			ui.strong("Payment Hash:");
			ui.horizontal(|ui| {
				ui.monospace(truncate_id(&bolt11.hash, 8, 8));
				if ui.small_button("Copy").clicked() {
					ui.output_mut(|o| o.copied_text = bolt11.hash.clone());
				}
			});
			ui.end_row();

			if let Some(preimage) = &bolt11.preimage {
				ui.strong("Preimage:");
				ui.horizontal(|ui| {
					ui.monospace(truncate_id(preimage, 8, 8));
					if ui.small_button("Copy").clicked() {
						ui.output_mut(|o| o.copied_text = preimage.clone());
					}
				});
				ui.end_row();
			}

			if let Some(secret) = &bolt11.secret {
				let secret_hex = secret.to_lower_hex_string();
				ui.strong("Secret:");
				ui.horizontal(|ui| {
					ui.monospace(truncate_id(&secret_hex, 8, 8));
					if ui.small_button("Copy").clicked() {
						ui.output_mut(|o| o.copied_text = secret_hex.clone());
					}
				});
				ui.end_row();
			}
		},
		Some(Kind::Bolt11Jit(jit)) => {
			ui.strong("Payment Hash:");
			ui.horizontal(|ui| {
				ui.monospace(truncate_id(&jit.hash, 8, 8));
				if ui.small_button("Copy").clicked() {
					ui.output_mut(|o| o.copied_text = jit.hash.clone());
				}
			});
			ui.end_row();

			if let Some(preimage) = &jit.preimage {
				ui.strong("Preimage:");
				ui.horizontal(|ui| {
					ui.monospace(truncate_id(preimage, 8, 8));
					if ui.small_button("Copy").clicked() {
						ui.output_mut(|o| o.copied_text = preimage.clone());
					}
				});
				ui.end_row();
			}

			if let Some(lsp_fee) = jit.lsp_fee_limits.as_ref() {
				if let Some(max_total) = lsp_fee.max_total_opening_fee_msat {
					ui.strong("LSP Max Total Fee:");
					ui.label(format_msat(max_total));
					ui.end_row();
				}
				if let Some(max_proportional) = lsp_fee.max_proportional_opening_fee_ppm_msat {
					ui.strong("LSP Max Proportional Fee:");
					ui.label(format!("{} ppm", max_proportional));
					ui.end_row();
				}
			}
		},
		Some(Kind::Bolt12Offer(offer)) => {
			if let Some(hash) = &offer.hash {
				ui.strong("Payment Hash:");
				ui.horizontal(|ui| {
					ui.monospace(truncate_id(hash, 8, 8));
					if ui.small_button("Copy").clicked() {
						ui.output_mut(|o| o.copied_text = hash.clone());
					}
				});
				ui.end_row();
			}

			if let Some(preimage) = &offer.preimage {
				ui.strong("Preimage:");
				ui.horizontal(|ui| {
					ui.monospace(truncate_id(preimage, 8, 8));
					if ui.small_button("Copy").clicked() {
						ui.output_mut(|o| o.copied_text = preimage.clone());
					}
				});
				ui.end_row();
			}

			if let Some(secret) = &offer.secret {
				let secret_hex = secret.to_lower_hex_string();
				ui.strong("Secret:");
				ui.horizontal(|ui| {
					ui.monospace(truncate_id(&secret_hex, 8, 8));
					if ui.small_button("Copy").clicked() {
						ui.output_mut(|o| o.copied_text = secret_hex.clone());
					}
				});
				ui.end_row();
			}

			if !offer.offer_id.is_empty() {
				ui.strong("Offer ID:");
				ui.horizontal(|ui| {
					ui.monospace(truncate_id(&offer.offer_id, 8, 8));
					if ui.small_button("Copy").clicked() {
						ui.output_mut(|o| o.copied_text = offer.offer_id.clone());
					}
				});
				ui.end_row();
			}

			if let Some(payer_note) = &offer.payer_note {
				ui.strong("Payer Note:");
				ui.label(payer_note);
				ui.end_row();
			}

			if let Some(quantity) = offer.quantity {
				ui.strong("Quantity:");
				ui.label(format!("{}", quantity));
				ui.end_row();
			}
		},
		Some(Kind::Bolt12Refund(refund)) => {
			if let Some(hash) = &refund.hash {
				ui.strong("Payment Hash:");
				ui.horizontal(|ui| {
					ui.monospace(truncate_id(hash, 8, 8));
					if ui.small_button("Copy").clicked() {
						ui.output_mut(|o| o.copied_text = hash.clone());
					}
				});
				ui.end_row();
			}

			if let Some(preimage) = &refund.preimage {
				ui.strong("Preimage:");
				ui.horizontal(|ui| {
					ui.monospace(truncate_id(preimage, 8, 8));
					if ui.small_button("Copy").clicked() {
						ui.output_mut(|o| o.copied_text = preimage.clone());
					}
				});
				ui.end_row();
			}

			if let Some(secret) = &refund.secret {
				let secret_hex = secret.to_lower_hex_string();
				ui.strong("Secret:");
				ui.horizontal(|ui| {
					ui.monospace(truncate_id(&secret_hex, 8, 8));
					if ui.small_button("Copy").clicked() {
						ui.output_mut(|o| o.copied_text = secret_hex.clone());
					}
				});
				ui.end_row();
			}
		},
		Some(Kind::Spontaneous(spontaneous)) => {
			ui.strong("Payment Hash:");
			ui.horizontal(|ui| {
				ui.monospace(truncate_id(&spontaneous.hash, 8, 8));
				if ui.small_button("Copy").clicked() {
					ui.output_mut(|o| o.copied_text = spontaneous.hash.clone());
				}
			});
			ui.end_row();

			if let Some(preimage) = &spontaneous.preimage {
				ui.strong("Preimage:");
				ui.horizontal(|ui| {
					ui.monospace(truncate_id(preimage, 8, 8));
					if ui.small_button("Copy").clicked() {
						ui.output_mut(|o| o.copied_text = preimage.clone());
					}
				});
				ui.end_row();
			}
		},
		None => {},
	}
}
