use eframe::egui;
use egui::{Color32, RichText};
use egui_extras::{Column, TableBuilder};

use crate::app::LspServerApp;
use crate::ui::layout::{card, page_scrolled, FORM_WIDTH};
use crate::ui::widgets;

pub fn render(ui: &mut egui::Ui, app: &mut LspServerApp) {
	ui.heading("Stable Channels");
	ui.add_space(5.0);

	// Price display
	ui.horizontal(|ui| {
		if let Some(price_resp) = &app.state.price {
			if price_resp.price > 0.0 {
				ui.label(
					RichText::new(format!("BTC/USD: ${:.2}", price_resp.price)).strong().size(16.0),
				);
			} else {
				ui.label("BTC/USD: fetching...");
			}
		} else {
			ui.label("BTC/USD: --");
		}

		if ui.button("Refresh").clicked() {
			app.fetch_price();
			app.fetch_stable_channels();
		}
	});

	ui.add_space(10.0);

	// Pre-extract row data so &app.state.stable_channels borrow is released
	// before we call app.fmt_sats and widgets::id_with_copy below.
	struct StableRow {
		channel_id: String,
		user_channel_id: String,
		counterparty: String,
		expected_usd: f64,
		backing_sats: u64,
		is_stable_receiver: bool,
		note: String,
	}

	let rows: Option<Vec<StableRow>> = app.state.stable_channels.as_ref().map(|resp| {
		resp.channels
			.iter()
			.map(|ch| StableRow {
				channel_id: ch.channel_id.clone(),
				user_channel_id: ch.user_channel_id.clone(),
				counterparty: ch.counterparty.clone(),
				expected_usd: ch.expected_usd,
				backing_sats: ch.expected_msats / 1000,
				is_stable_receiver: ch.is_stable_receiver,
				note: ch.note.clone(),
			})
			.collect()
	});

	// Table, separator, and Edit form share one outer scroll so the form stays reachable below the table.
	page_scrolled(ui, |ui| {
		match rows {
			Some(ref rows) if !rows.is_empty() => {
				crate::ui::layout::h_scroll(ui, 900.0, |ui| {
					TableBuilder::new(ui)
						.striped(true)
						.resizable(false)
						.vscroll(false)
						.cell_layout(egui::Layout::left_to_right(egui::Align::Center))
						.auto_shrink([false, true])
						.column(Column::remainder().at_least(64.0).clip(true)) // Channel ID
						.column(Column::remainder().at_least(64.0).clip(true)) // User Channel ID
						.column(Column::remainder().at_least(64.0).clip(true)) // Counterparty
						.column(Column::auto()) // Stable $
						.column(Column::auto()) // Backing
						.column(Column::auto()) // Role
						.column(Column::remainder().at_least(64.0).clip(true)) // Note
						.column(Column::auto()) // Edit
						.header(22.0, |mut header| {
							for t in ["Channel ID", "User Channel ID", "Counterparty", "Stable $", "Backing", "Role", "Note", ""] {
								header.col(|ui| { ui.strong(t); });
							}
						})
						.body(|mut body| {
							for row in rows {
								body.row(26.0, |mut r| {
									// Channel ID — monospace truncated + copy-to-clipboard
									r.col(|ui| { widgets::id_with_copy(ui, &row.channel_id, &mut app.state.status_message); });
									// User Channel ID — monospace truncated + copy-to-clipboard
									r.col(|ui| { widgets::id_with_copy(ui, &row.user_channel_id, &mut app.state.status_message); });
									// Counterparty — truncated + copy
									r.col(|ui| { widgets::id_with_copy(ui, &row.counterparty, &mut app.state.status_message); });
									// Expected USD — green, always dollars (USD target)
									r.col(|ui| {
										ui.label(
											RichText::new(format!("${:.2}", row.expected_usd))
												.color(Color32::from_rgb(34, 139, 34))
												.strong(),
										);
									});
									// Backing sats — unit-aware via app.fmt_sats
									r.col(|ui| { ui.label(app.fmt_sats(row.backing_sats)); });
									// Role — status pill
									r.col(|ui| {
										let (role_text, role_color) = if row.is_stable_receiver {
											("Receiver", Color32::from_rgb(0xF7, 0x93, 0x1A))
										} else {
											("Provider", Color32::from_rgb(0x5B, 0x9B, 0xD5))
										};
										widgets::status_pill(ui, role_text, role_color);
									});
									// Note
									r.col(|ui| { ui.label(if row.note.is_empty() { "---" } else { &row.note }); });
									// Prefill the edit form from this row
									r.col(|ui| {
										if ui.button("Edit").on_hover_text("Edit this channel's stable target").clicked() {
											let form = &mut app.state.forms.edit_stable_channel;
											form.channel_id = row.channel_id.clone();
											form.expected_usd = format!("{:.2}", row.expected_usd);
											form.note = row.note.clone();
										}
									});
								});
							}
						});
				});
			},
			Some(_) => {
				ui.label("No stable channels.");
			},
			None => {
				ui.label("Not loaded yet. Click Refresh.");
			},
		}

		ui.add_space(15.0);
		ui.separator();
		ui.add_space(5.0);

		// Edit a channel's stable target. "Edit" on a row prefills; submitting sets
		// expected_usd (and note) via EditStableChannel on the daemon.
		let form_w = ui.available_width().min(FORM_WIDTH);
		ui.vertical(|ui| {
			ui.set_width(form_w);
			card(ui, "Edit Stable Channel", |ui| {
				let form = &mut app.state.forms.edit_stable_channel;
				ui.horizontal(|ui| {
					ui.label("Channel ID:");
					ui.add(egui::TextEdit::singleline(&mut form.channel_id).desired_width(420.0));
				});
				ui.horizontal(|ui| {
					ui.label("Target USD:");
					ui.add(egui::TextEdit::singleline(&mut form.expected_usd).desired_width(120.0));
					ui.label(RichText::new("0 = stop stabilizing").weak());
				});
				ui.horizontal(|ui| {
					ui.label("Note:");
					ui.add(egui::TextEdit::singleline(&mut form.note).desired_width(300.0));
				});

				ui.add_space(5.0);
				let is_loading = app.state.tasks.edit_stable_channel.is_some();
				ui.add_enabled_ui(!is_loading, |ui| {
					if ui.button(if is_loading { "Submitting..." } else { "Submit" }).clicked() {
						app.edit_stable_channel();
					}
				});
			});
		});
	});
}
