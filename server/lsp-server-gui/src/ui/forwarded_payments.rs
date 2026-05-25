use eframe::egui;

use crate::app::LspServerApp;
use crate::ui::format_msat;

pub fn render(ui: &mut egui::Ui, app: &mut LspServerApp) {
	ui.heading("Forwarded Payments");
	ui.add_space(5.0);

	if ui.button("Refresh").clicked() {
		app.fetch_forwarded_payments();
	}

	ui.add_space(10.0);

	match &app.state.forwarded_payments {
		Some(resp) => {
			if resp.forwarded_payments.is_empty() {
				ui.label("No forwarded payments.");
			} else {
				egui::ScrollArea::both().max_height(400.0).show(ui, |ui| {
					egui::Grid::new("forwarded_payments_table")
						.striped(true)
						.min_col_width(80.0)
						.show(ui, |ui| {
							ui.label(egui::RichText::new("Total Fee").strong());
							ui.label(egui::RichText::new("Amount Forwarded").strong());
							ui.end_row();

							for fp in &resp.forwarded_payments {
								ui.label(format_msat(fp.total_fee_earned_msat.unwrap_or(0)));
								ui.label(format_msat(
									fp.outbound_amount_forwarded_msat.unwrap_or(0),
								));
								ui.end_row();
							}
						});
				});
			}
		},
		None => {
			ui.label("Not loaded yet. Click Refresh.");
		},
	}
}
