use eframe::egui;

use crate::app::LspServerApp;

pub fn render(ui: &mut egui::Ui, app: &mut LspServerApp) {
	ui.heading("LDK Server Logs");
	ui.add_space(5.0);

	ui.horizontal(|ui| {
		ui.label("Lines:");
		ui.add(
			egui::TextEdit::singleline(&mut app.state.forms.ldk_log.max_lines)
				.desired_width(80.0),
		);
		let loading = app.state.tasks.ldk_log.is_some();
		if ui.add_enabled(!loading, egui::Button::new("Refresh")).clicked() {
			app.fetch_ldk_log();
		}
		if loading {
			ui.spinner();
		}
	});

	ui.add_space(10.0);

	match &app.state.ldk_log {
		Some(resp) if resp.content.is_empty() => {
			ui.label("Empty response.");
			ui.label(
				egui::RichText::new(
					"LDK Server may not have `[log] file = \"...\"` set in its config. \
					 Uncomment that line and restart LDK Server, then refresh.",
				)
				.italics()
				.weak(),
			);
		},
		Some(resp) => {
			egui::ScrollArea::both().auto_shrink([false, false]).stick_to_bottom(true).show(
				ui,
				|ui| {
					ui.add(
						egui::TextEdit::multiline(&mut resp.content.as_str())
							.font(egui::TextStyle::Monospace)
							.desired_width(f32::INFINITY)
							.desired_rows(30),
					);
				},
			);
		},
		None => {
			ui.label("Click Refresh to load.");
		},
	}
}
