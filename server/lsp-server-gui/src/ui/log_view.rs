use eframe::egui;

/// Filter / Copy-all / Wrap / Follow-tail control row. State lives in egui temp memory keyed by `id_salt`.
pub fn controls(ui: &mut egui::Ui, id_salt: &str, copy_source: &str) -> (String, bool, bool) {
	let filter_id = ui.id().with((id_salt, "filter"));
	let wrap_id = ui.id().with((id_salt, "wrap"));
	let follow_id = ui.id().with((id_salt, "follow"));

	let mut filter = ui.memory_mut(|m| m.data.get_temp::<String>(filter_id).unwrap_or_default());
	let mut wrap = ui.memory_mut(|m| m.data.get_temp::<bool>(wrap_id).unwrap_or(false));
	let mut follow = ui.memory_mut(|m| m.data.get_temp::<bool>(follow_id).unwrap_or(true));

	ui.horizontal(|ui| {
		ui.label("Filter:");
		ui.text_edit_singleline(&mut filter);
		if ui.button("Copy all").clicked() && !copy_source.is_empty() {
			ui.output_mut(|o| o.copied_text = copy_source.to_string());
		}
		ui.checkbox(&mut wrap, "Wrap");
		ui.checkbox(&mut follow, "Follow tail");
	});

	ui.memory_mut(|m| m.data.insert_temp(filter_id, filter.clone()));
	ui.memory_mut(|m| m.data.insert_temp(wrap_id, wrap));
	ui.memory_mut(|m| m.data.insert_temp(follow_id, follow));
	(filter, wrap, follow)
}

/// Monospace, scrollable, read-only text area that fills the remaining panel width and height.
pub fn text_area(ui: &mut egui::Ui, display: &str, wrap: bool, follow: bool) {
	let avail = ui.available_size();
	let scroll = egui::ScrollArea::both().auto_shrink([false, false]).stick_to_bottom(follow);
	scroll.show(ui, |ui| {
		let mut binding = display;
		let desired_w = if wrap { avail.x } else { f32::INFINITY };
		ui.add(
			egui::TextEdit::multiline(&mut binding)
				.font(egui::TextStyle::Monospace)
				.desired_width(desired_w)
				.min_size(egui::vec2(avail.x, avail.y)),
		);
	});
}
