use eframe::egui;
use egui::{RichText, ScrollArea};

use crate::app::LspServerApp;
use crate::ui::{format_sats, truncate_id};

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

	// Stable channels table
	match &app.state.stable_channels {
		Some(resp) if !resp.channels.is_empty() => {
			ScrollArea::both().max_height(300.0).show(ui, |ui| {
				egui::Grid::new("stable_channels_table").striped(true).min_col_width(60.0).show(
					ui,
					|ui| {
						// Headers
						for h in [
							"Channel ID",
							"Counterparty",
							"Stable $",
							"Backing Sats",
							"Price",
							"Role",
							"Note",
						] {
							ui.label(RichText::new(h).strong());
						}
						ui.end_row();

						// Rows
						for ch in &resp.channels {
							// Channel ID (truncated + copy)
							ui.horizontal(|ui| {
								ui.label(
									RichText::new(truncate_id(&ch.channel_id, 8, 4)).monospace(),
								);
								if ui.small_button("Copy").clicked() {
									ui.output_mut(|o| o.copied_text = ch.channel_id.clone());
								}
							});

							// Counterparty (truncated)
							ui.label(
								RichText::new(truncate_id(&ch.counterparty, 8, 4)).monospace(),
							);

							// Expected USD - green highlight
							ui.label(
								RichText::new(format!("${:.2}", ch.expected_usd))
									.color(egui::Color32::from_rgb(34, 139, 34))
									.strong(),
							);

							// Backing sats
							ui.label(format_sats(ch.expected_msats / 1000));

							// Latest price
							ui.label(format!("${:.0}", ch.latest_price));

							// Role
							let role = if ch.is_stable_receiver { "Receiver" } else { "Provider" };
							ui.label(role);

							// Note
							ui.label(if ch.note.is_empty() {
								"---".to_string()
							} else {
								ch.note.clone()
							});

							ui.end_row();
						}
					},
				);
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

	// Edit form
	ui.heading("Edit Stable Channel");
	ui.add_space(5.0);

	let form = &mut app.state.forms.edit_stable_channel;

	ui.horizontal(|ui| {
		ui.label("Channel ID:");
		ui.text_edit_singleline(&mut form.channel_id);
	});

	ui.horizontal(|ui| {
		ui.label("Target USD:");
		ui.text_edit_singleline(&mut form.expected_usd);
	});

	ui.horizontal(|ui| {
		ui.label("Note:");
		ui.text_edit_singleline(&mut form.note);
	});

	ui.add_space(5.0);

	let is_loading = app.state.tasks.edit_stable_channel.is_some();
	ui.add_enabled_ui(!is_loading, |ui| {
		if ui.button("Submit").clicked() {
			app.edit_stable_channel();
		}
	});
}
