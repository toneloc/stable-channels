use eframe::egui;

use crate::app::LspServerApp;
use crate::ui::audit_log::format_audit_line;
use crate::ui::widgets;

pub fn render(ui: &mut egui::Ui, app: &mut LspServerApp) {
	ui.heading("Channel History");
	ui.add_space(5.0);
	ui.label("Complete audit history for one id, oldest-first.");
	ui.add_space(8.0);

	ui.horizontal(|ui| {
		ui.label("Channel id:");
		ui.add(
			egui::TextEdit::singleline(&mut app.state.forms.channel_history.filter)
				.desired_width(360.0)
				.hint_text("user_channel_id / channel_id / payment_id"),
		);
		let has_id = !app.state.forms.channel_history.filter.trim().is_empty();
		let loading = app.state.tasks.channel_history.is_some();
		if ui.add_enabled(!loading && has_id, egui::Button::new("Load")).clicked() {
			app.fetch_channel_history();
		}
		if loading {
			ui.spinner();
		}
	});

	let formatted: String = app
		.state
		.channel_history
		.as_ref()
		.map(|r| r.content.lines().map(format_audit_line).collect::<Vec<_>>().join("\n"))
		.unwrap_or_default();

	let (filter, wrap, follow) = crate::ui::log_view::controls(ui, "channel_history", &formatted);

	ui.add_space(10.0);

	match &app.state.channel_history {
		Some(resp) if resp.content.is_empty() => {
			ui.label("No events for that id.");
		},
		Some(_) => {
			let display: String = if filter.is_empty() {
				formatted.clone()
			} else {
				formatted.lines().filter(|line| line.contains(&filter)).collect::<Vec<_>>().join("\n")
			};
			crate::ui::log_view::text_area(ui, &display, wrap, follow);
		},
		None => {
			widgets::empty_state(ui, "🧵", "No channel loaded", "Enter an id and click Load");
		},
	}
}
