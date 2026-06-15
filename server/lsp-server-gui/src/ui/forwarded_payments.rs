use eframe::egui;

use crate::app::LspServerApp;
use crate::ui::widgets;

// Per-row snapshot extracted from state.forwarded_payments so the state borrow
// is released before app.fmt_msat runs in the grid body.
struct ForwardRow {
	fee_msat: u64,
	amount_msat: u64,
}

pub fn render(ui: &mut egui::Ui, app: &mut LspServerApp) {
	ui.heading("Forwarded Payments");
	ui.add_space(5.0);

	if ui.button("Refresh").clicked() {
		app.state.forwarded_payments_page_token = None;
		app.fetch_forwarded_payments();
	}

	let loading = app.state.tasks.forwarded_payments.is_some();
	if loading {
		ui.horizontal(|ui| {
			ui.spinner();
			ui.label("Loading...");
		});
	}

	ui.add_space(10.0);

	// Pre-extract rows and compute sums so the &app.state.forwarded_payments borrow
	// ends before app.fmt_msat (borrows &app.state via &self) is called below.
	let data: Option<(Vec<ForwardRow>, u64, u64)> =
		app.state.forwarded_payments.as_ref().map(|resp| {
			let mut total_fee: u64 = 0;
			let mut total_forwarded: u64 = 0;
			let rows: Vec<ForwardRow> = resp
				.forwarded_payments
				.iter()
				.map(|fp| {
					let fee = fp.total_fee_earned_msat.unwrap_or(0);
					let amt = fp.outbound_amount_forwarded_msat.unwrap_or(0);
					total_fee += fee;
					total_forwarded += amt;
					ForwardRow { fee_msat: fee, amount_msat: amt }
				})
				.collect();
			(rows, total_fee, total_forwarded)
		});

	match data {
		Some((rows, total_fee, total_forwarded)) => {
			if rows.is_empty() {
				if !loading {
					widgets::empty_state(
						ui,
						"↪",
						"No forwarded payments yet",
						"Click Refresh to load",
					);
				}
			} else {
				// Summary header: count + operator revenue (fee) + total forwarded
				ui.horizontal(|ui| {
					ui.label(format!("{} forwards", rows.len()));
					ui.separator();
					ui.label(
						egui::RichText::new(format!(
							"Revenue: {}",
							app.fmt_msat(total_fee)
						))
						.strong()
						.color(egui::Color32::from_rgb(0xF7, 0x93, 0x1A)),
					);
					ui.separator();
					ui.label(format!("Forwarded: {}", app.fmt_msat(total_forwarded)));
				});
				ui.add_space(5.0);

				egui::ScrollArea::both().max_height(400.0).show(ui, |ui| {
					egui::Grid::new("forwarded_payments_table")
						.striped(true)
						.min_col_width(80.0)
						.show(ui, |ui| {
							ui.label(egui::RichText::new("Total Fee").strong());
							ui.label(egui::RichText::new("Amount Forwarded").strong());
							ui.end_row();

							for row in &rows {
								// Right-align both msat columns
								ui.with_layout(
									egui::Layout::right_to_left(egui::Align::Center),
									|ui| {
										ui.monospace(app.fmt_msat(row.fee_msat));
									},
								);
								ui.with_layout(
									egui::Layout::right_to_left(egui::Align::Center),
									|ui| {
										ui.monospace(app.fmt_msat(row.amount_msat));
									},
								);
								ui.end_row();
							}
						});
				});
			}
		},
		None => {
			if !loading {
				widgets::empty_state(ui, "↪", "No forwarded payments yet", "Click Refresh to load");
			}
		},
	}
}
