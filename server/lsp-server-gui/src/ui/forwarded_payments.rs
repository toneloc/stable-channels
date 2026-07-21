use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::app::LspServerApp;
use crate::ui::layout::page_scrolled;
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
				page_scrolled(ui, |ui| {
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

					TableBuilder::new(ui)
						.striped(true)
						.resizable(false)
						.auto_shrink([false, true])
						.column(Column::remainder().clip(true))
						.column(Column::remainder().clip(true))
						.header(22.0, |mut h| {
							h.col(|ui| { ui.strong("Total Fee"); });
							h.col(|ui| { ui.strong("Amount Forwarded"); });
						})
						.body(|mut body| {
							for row in &rows {
								body.row(24.0, |mut r| {
									// Left-align both msat columns
									r.col(|ui| {
										ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
											ui.monospace(app.fmt_msat(row.fee_msat));
										});
									});
									r.col(|ui| {
										ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
											ui.monospace(app.fmt_msat(row.amount_msat));
										});
									});
								});
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
