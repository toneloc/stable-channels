use eframe::egui;

use crate::app::LspServerApp;
use crate::ui::widgets;

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

	let raw = app.state.ldk_log.as_ref().map(|r| r.content.clone()).unwrap_or_default();
	let (filter, wrap, follow) = crate::ui::log_view::controls(ui, "ldk_log", &raw);

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
			let display: String = if filter.is_empty() {
				resp.content.clone()
			} else {
				resp.content.lines().filter(|line| line.contains(&filter)).collect::<Vec<_>>().join("\n")
			};
			crate::ui::log_view::text_area(ui, &display, wrap, follow);
		},
		None => {
			widgets::empty_state(ui, "📜", "No log loaded", "Click Refresh to load");
		},
	}
}
