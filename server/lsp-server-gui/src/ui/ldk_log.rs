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

	// Log controls — all stored in egui temp memory, no AppState fields
	let filter_id = ui.id().with("log_filter");
	let wrap_id = ui.id().with("log_wrap");
	let follow_id = ui.id().with("log_follow");

	let mut filter = ui.memory_mut(|m| m.data.get_temp::<String>(filter_id).unwrap_or_default());
	let mut wrap = ui.memory_mut(|m| m.data.get_temp::<bool>(wrap_id).unwrap_or(false));
	let mut follow = ui.memory_mut(|m| m.data.get_temp::<bool>(follow_id).unwrap_or(true));

	ui.horizontal(|ui| {
		ui.label("Filter:");
		ui.text_edit_singleline(&mut filter);

		if ui.button("Copy all").clicked() {
			if let Some(resp) = &app.state.ldk_log {
				let content = resp.content.clone();
				ui.output_mut(|o| o.copied_text = content);
			}
		}

		ui.checkbox(&mut wrap, "Wrap");
		ui.checkbox(&mut follow, "Follow tail");
	});

	ui.memory_mut(|m| m.data.insert_temp(filter_id, filter.clone()));
	ui.memory_mut(|m| m.data.insert_temp(wrap_id, wrap));
	ui.memory_mut(|m| m.data.insert_temp(follow_id, follow));

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
			// Build filtered display string from already-fetched content
			let display: String = if filter.is_empty() {
				resp.content.clone()
			} else {
				resp.content
					.lines()
					.filter(|line| line.contains(&filter))
					.collect::<Vec<_>>()
					.join("\n")
			};

			// Wrap=false → ScrollArea::both (horizontal scroll); wrap=true → vertical only
			let scroll = egui::ScrollArea::both()
				.auto_shrink([false, false])
				.stick_to_bottom(follow);
			scroll.show(ui, |ui| {
				let mut binding = display.as_str();
				// When wrap is on, constrain width to force line-wrapping
				let desired_w = if wrap { ui.available_width() } else { f32::INFINITY };
				ui.add(
					egui::TextEdit::multiline(&mut binding)
						.font(egui::TextStyle::Monospace)
						.desired_width(desired_w)
						.desired_rows(30),
				);
			});
		},
		None => {
			widgets::empty_state(ui, "📜", "No log loaded", "Click Refresh to load");
		},
	}
}
