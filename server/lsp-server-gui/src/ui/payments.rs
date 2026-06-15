use egui::{Context, ScrollArea, Ui};
use hex::DisplayHex;
#[cfg(target_arch = "wasm32")]
use web_sys::js_sys;

use crate::app::LspServerApp;
use crate::ui::truncate_id;
use crate::ui::widgets;

// Per-row snapshot extracted from state.payments so the state borrow is released
// before app.fmt_msat / status_pill run in the grid body (see channels.rs).
struct PaymentRow {
	id: String,
	hash: String,
	type_label: String,
	amount_msat: Option<u64>,
	fee_paid_msat: Option<u64>,
	direction: i32,
	status: i32,
	timestamp: u64,
}

pub fn render(ui: &mut Ui, app: &mut LspServerApp) {
	ui.heading("Payments");
	ui.add_space(10.0);

	if app.render_disconnected_gate(ui) {
		return;
	}

	ui.horizontal(|ui| {
		if app.state.tasks.payments.is_some() {
			ui.spinner();
			ui.label("Loading...");
		} else {
			if ui.button("Refresh").clicked() {
				app.state.payments_page_token = None;
				app.state.payments_appending = false;
				app.fetch_payments();
			}
			if app.state.payments_page_token.is_some() && ui.button("Load More").clicked() {
				app.state.payments_appending = true;
				app.fetch_payments();
			}
		}
	});

	ui.add_space(10.0);

	let loading = app.state.tasks.payments.is_some();

	// Track which payment details button was clicked
	let mut clicked_payment_id: Option<String> = None;

	// Pre-extract per-row data into locals so the &app.state.payments borrow is
	// released before app.fmt_msat / status_pill below; never mutate the underlying list.
	let rows: Option<(Vec<PaymentRow>, bool)> = app.state.payments.as_ref().map(|resp| {
		let rows = resp
			.payments
			.iter()
			.map(|p| PaymentRow {
				id: p.id.clone(),
				hash: p.kind.as_ref().map(payment_hash).unwrap_or_default(),
				type_label: p
					.kind
					.as_ref()
					.map(|k| format_payment_kind(k))
					.unwrap_or_else(|| "Unknown".to_string()),
				amount_msat: p.amount_msat,
				fee_paid_msat: p.fee_paid_msat,
				direction: p.direction,
				status: p.status,
				timestamp: p.latest_update_timestamp,
			})
			.collect();
		(rows, resp.next_page_token.is_some())
	});

	if let Some((rows, more_available)) = rows {
		if rows.is_empty() {
			if !loading {
				ui.label("No payments found.");
			}
		} else {
			let total = rows.len();

			// Filter + sort controls live in egui temp memory (not persisted).
			let filter_id = ui.id().with("pay_filter");
			let status_id = ui.id().with("pay_status");
			let dir_id = ui.id().with("pay_dir");
			let sort_id = ui.id().with("pay_sort");

			let mut filter =
				ui.memory_mut(|m| m.data.get_temp::<String>(filter_id).unwrap_or_default());
			// status filter: -1 = all, else 0/1/2 like payment.status
			let mut status_filter =
				ui.memory_mut(|m| m.data.get_temp::<i32>(status_id).unwrap_or(-1));
			// direction filter: -1 = all, else 0/1 like payment.direction
			let mut dir_filter = ui.memory_mut(|m| m.data.get_temp::<i32>(dir_id).unwrap_or(-1));
			// sort key: (column, descending) where 0 = Amount, 1 = Date
			let mut sort =
				ui.memory_mut(|m| m.data.get_temp::<(u8, bool)>(sort_id).unwrap_or((1, true)));

			ui.horizontal(|ui| {
				ui.label("Filter:");
				ui.add(egui::TextEdit::singleline(&mut filter).hint_text("id or hash"));
				egui::ComboBox::from_id_salt(status_id)
					.selected_text(status_label(status_filter))
					.show_ui(ui, |ui| {
						ui.selectable_value(&mut status_filter, -1, "All statuses");
						ui.selectable_value(&mut status_filter, 0, "Pending");
						ui.selectable_value(&mut status_filter, 1, "Succeeded");
						ui.selectable_value(&mut status_filter, 2, "Failed");
					});
				egui::ComboBox::from_id_salt(dir_id)
					.selected_text(direction_label(dir_filter))
					.show_ui(ui, |ui| {
						ui.selectable_value(&mut dir_filter, -1, "All directions");
						ui.selectable_value(&mut dir_filter, 0, "Inbound");
						ui.selectable_value(&mut dir_filter, 1, "Outbound");
					});
			});

			// Build the rendered view by filtering indices into the loaded rows.
			let needle = filter.trim().to_lowercase();
			let mut view: Vec<usize> = (0..rows.len())
				.filter(|&i| {
					let r = &rows[i];
					let matches_text = needle.is_empty()
						|| r.id.to_lowercase().contains(&needle)
						|| r.hash.to_lowercase().contains(&needle);
					let matches_status = status_filter < 0 || r.status == status_filter;
					let matches_dir = dir_filter < 0 || r.direction == dir_filter;
					matches_text && matches_status && matches_dir
				})
				.collect();

			// Sort the view (purely a display ordering; underlying list untouched).
			view.sort_by(|&a, &b| {
				let (ra, rb) = (&rows[a], &rows[b]);
				let ord = match sort.0 {
					0 => ra.amount_msat.unwrap_or(0).cmp(&rb.amount_msat.unwrap_or(0)),
					_ => ra.timestamp.cmp(&rb.timestamp),
				};
				if sort.1 {
					ord.reverse()
				} else {
					ord
				}
			});

			ui.label(format!("{}/{} payment(s)", view.len(), total));
			ui.add_space(5.0);

			ScrollArea::both().max_height(500.0).show(ui, |ui| {
				egui::Grid::new("payments_grid").striped(true).min_col_width(80.0).show(ui, |ui| {
					// Header (Amount/Date are clickable sort toggles)
					ui.strong("Payment ID");
					ui.strong("Type");
					if ui.button(sort_header("Amount", &sort, 0)).clicked() {
						sort = (0, if sort.0 == 0 { !sort.1 } else { true });
					}
					ui.strong("Fee");
					ui.strong("Direction");
					ui.strong("Status");
					if ui.button(sort_header("Timestamp", &sort, 1)).clicked() {
						sort = (1, if sort.0 == 1 { !sort.1 } else { true });
					}
					ui.strong(""); // Details column
					ui.end_row();

					for &i in &view {
						let row = &rows[i];

						// Payment ID
						ui.horizontal(|ui| {
							ui.monospace(truncate_id(&row.id, 5, 4));
							if ui.small_button("Copy").clicked() {
								ui.output_mut(|o| o.copied_text = row.id.clone());
							}
						});

						// Type
						ui.label(&row.type_label);

						// Amount (unit-aware, right-aligned)
						ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
							match row.amount_msat {
								Some(amount) => ui.monospace(app.fmt_msat(amount)),
								None => ui.monospace("-"),
							}
						});

						// Fee (unit-aware, right-aligned)
						ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
							match row.fee_paid_msat {
								Some(fee) => ui.monospace(app.fmt_msat(fee)),
								None => ui.monospace("-"),
							}
						});

						// Direction badge (0 = Inbound, 1 = Outbound)
						match row.direction {
							0 => widgets::status_pill(ui, "⬇ In", egui::Color32::LIGHT_BLUE),
							1 => widgets::status_pill(ui, "⬆ Out", egui::Color32::GOLD),
							_ => widgets::status_pill(ui, "Unknown", egui::Color32::GRAY),
						};

						// Status pill (0 = Pending, 1 = Succeeded, 2 = Failed)
						let (status_text, status_color) = status_style(row.status);
						widgets::status_pill(ui, status_text, status_color);

						// Timestamp (relative text, exact epoch on hover)
						ui.label(format_timestamp(row.timestamp))
							.on_hover_text(format!("unix: {}", row.timestamp));

						// Details button - track click without modifying app state yet
						if ui.small_button("Details").clicked() {
							clicked_payment_id = Some(row.id.clone());
						}

						ui.end_row();
					}
				});
			});

			if more_available {
				ui.add_space(5.0);
				ui.label("More payments available. Click 'Load More' to fetch.");
			}

			// Persist the control state back into temp memory.
			ui.memory_mut(|m| {
				m.data.insert_temp(filter_id, filter);
				m.data.insert_temp(status_id, status_filter);
				m.data.insert_temp(dir_id, dir_filter);
				m.data.insert_temp(sort_id, sort);
			});
		}
	} else {
		if !loading {
			ui.label("No payment data available. Click Refresh to fetch.");
		}
	}

	// Handle the Details button click outside the borrow
	if let Some(payment_id) = clicked_payment_id {
		app.state.payment_details_id = payment_id.clone();
		app.state.payment_details = None;
		app.state.show_payment_details_dialog = true;
		app.fetch_payment_details(payment_id);
	}
}

// Color/text mapping for the status pill (same colors as the old colored_label).
fn status_style(status: i32) -> (&'static str, egui::Color32) {
	match status {
		0 => ("Pending", egui::Color32::YELLOW),
		1 => ("Succeeded", egui::Color32::GREEN),
		2 => ("Failed", egui::Color32::RED),
		_ => ("Unknown", egui::Color32::GRAY),
	}
}

fn status_label(filter: i32) -> &'static str {
	match filter {
		0 => "Pending",
		1 => "Succeeded",
		2 => "Failed",
		_ => "All statuses",
	}
}

fn direction_label(filter: i32) -> &'static str {
	match filter {
		0 => "Inbound",
		1 => "Outbound",
		_ => "All directions",
	}
}

// Header label with a sort-direction arrow when this column is the active sort key.
fn sort_header(label: &str, sort: &(u8, bool), col: u8) -> String {
	if sort.0 == col {
		format!("{} {}", label, if sort.1 { "⬇" } else { "⬆" })
	} else {
		label.to_string()
	}
}

// Best-effort payment hash string for substring filtering (empty if none).
fn payment_hash(kind: &sc_rest_client::ldk_server_grpc::types::PaymentKind) -> String {
	use sc_rest_client::ldk_server_grpc::types::payment_kind::Kind;

	match &kind.kind {
		Some(Kind::Onchain(o)) => o.txid.clone(),
		Some(Kind::Bolt11(b)) => b.hash.clone(),
		Some(Kind::Bolt11Jit(j)) => j.hash.clone(),
		Some(Kind::Bolt12Offer(o)) => o.hash.clone().unwrap_or_default(),
		Some(Kind::Bolt12Refund(r)) => r.hash.clone().unwrap_or_default(),
		Some(Kind::Spontaneous(s)) => s.hash.clone(),
		None => String::new(),
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
			// Pre-format unit-aware amounts so the &app.state.payment_details borrow
			// below doesn't conflict with app.fmt_msat (which borrows &app.state).
			let amount_str = app
				.state
				.payment_details
				.as_ref()
				.and_then(|r| r.payment.as_ref())
				.and_then(|p| p.amount_msat)
				.map(|a| app.fmt_msat(a))
				.unwrap_or_else(|| "-".to_string());
			let fee_str = app
				.state
				.payment_details
				.as_ref()
				.and_then(|r| r.payment.as_ref())
				.and_then(|p| p.fee_paid_msat)
				.map(|f| app.fmt_msat(f))
				.unwrap_or_else(|| "-".to_string());
			let lsp_fee_str = app
				.state
				.payment_details
				.as_ref()
				.and_then(|r| r.payment.as_ref())
				.and_then(|p| p.kind.as_ref())
				.and_then(jit_max_total_fee_msat)
				.map(|m| app.fmt_msat(m));

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

								// Amount (unit-aware, pre-formatted above)
								ui.strong("Amount:");
								ui.label(&amount_str);
								ui.end_row();

								// Fee (unit-aware, pre-formatted above)
								ui.strong("Fee Paid:");
								ui.label(&fee_str);
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

								// Status pill (same colors as the table)
								ui.strong("Status:");
								let (status_text, status_color) = status_style(payment.status);
								widgets::status_pill(ui, status_text, status_color);
								ui.end_row();

								// Timestamp (relative text, exact epoch on hover)
								ui.strong("Last Updated:");
								ui.label(format_timestamp(payment.latest_update_timestamp))
									.on_hover_text(format!(
										"unix: {}",
										payment.latest_update_timestamp
									));
								ui.end_row();

								// Kind-specific details
								if let Some(kind) = &payment.kind {
									render_payment_kind_details(ui, kind, &lsp_fee_str);
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

// Extract the BOLT11-JIT LSP max-total opening fee (msat) for unit-aware formatting.
fn jit_max_total_fee_msat(
	kind: &sc_rest_client::ldk_server_grpc::types::PaymentKind,
) -> Option<u64> {
	use sc_rest_client::ldk_server_grpc::types::payment_kind::Kind;
	match &kind.kind {
		Some(Kind::Bolt11Jit(jit)) => {
			jit.lsp_fee_limits.as_ref().and_then(|f| f.max_total_opening_fee_msat)
		},
		_ => None,
	}
}

fn render_payment_kind_details(
	ui: &mut egui::Ui, kind: &sc_rest_client::ldk_server_grpc::types::PaymentKind,
	lsp_fee_str: &Option<String>,
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
				if let Some(fee_str) = lsp_fee_str {
					ui.strong("LSP Max Total Fee:");
					ui.label(fee_str);
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
