use eframe::egui;
use egui::{Color32, RichText, ScrollArea};

use crate::app::LspServerApp;
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
		counterparty: String,
		expected_usd: f64,
		backing_sats: u64,
		latest_price: f64,
		is_stable_receiver: bool,
		note: String,
	}

	let rows: Option<Vec<StableRow>> = app.state.stable_channels.as_ref().map(|resp| {
		resp.channels
			.iter()
			.map(|ch| StableRow {
				channel_id: ch.channel_id.clone(),
				counterparty: ch.counterparty.clone(),
				expected_usd: ch.expected_usd,
				backing_sats: ch.expected_msats / 1000,
				latest_price: ch.latest_price,
				is_stable_receiver: ch.is_stable_receiver,
				note: ch.note.clone(),
			})
			.collect()
	});

	match rows {
		Some(ref rows) if !rows.is_empty() => {
			ScrollArea::both().max_height(300.0).show(ui, |ui| {
				egui::Grid::new("stable_channels_table")
					.striped(true)
					.min_col_width(60.0)
					.show(ui, |ui| {
						// Headers
						for h in [
							"Channel ID",
							"Counterparty",
							"Stable $",
							"Backing",
							"Price",
							"Role",
							"Note",
						] {
							ui.label(RichText::new(h).strong());
						}
						ui.end_row();

						for row in rows {
							// Channel ID — monospace truncated + copy-to-clipboard
							widgets::id_with_copy(
								ui,
								&row.channel_id,
								&mut app.state.status_message,
							);

							// Counterparty — truncated + copy
							widgets::id_with_copy(
								ui,
								&row.counterparty,
								&mut app.state.status_message,
							);

							// Expected USD — green, always dollars (USD target)
							ui.label(
								RichText::new(format!("${:.2}", row.expected_usd))
									.color(Color32::from_rgb(34, 139, 34))
									.strong(),
							);

							// Backing sats — unit-aware via app.fmt_sats
							ui.label(app.fmt_sats(row.backing_sats));

							// Latest price — always dollars (it is a BTC/USD price)
							ui.label(format!("${:.0}", row.latest_price));

							// Role — status pill
							let (role_text, role_color) = if row.is_stable_receiver {
								("Receiver", Color32::from_rgb(0xF7, 0x93, 0x1A))
							} else {
								("Provider", Color32::from_rgb(0x5B, 0x9B, 0xD5))
							};
							widgets::status_pill(ui, role_text, role_color);

							// Note
							ui.label(if row.note.is_empty() { "---" } else { &row.note });

							ui.end_row();
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

}
