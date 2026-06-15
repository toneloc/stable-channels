use egui::Ui;
use crate::app::LspServerApp;
use crate::state::DisplayUnit;

pub fn render(ui: &mut Ui, app: &mut LspServerApp) {
	ui.heading("Settings");
	ui.add_space(8.0);

	crate::ui::widgets::section_header(ui, "Display unit");
	ui.horizontal(|ui| {
		let mut unit = app.state.display_unit;
		ui.selectable_value(&mut unit, DisplayUnit::Usd, "USD");
		ui.selectable_value(&mut unit, DisplayUnit::Btc, "BTC");
		ui.selectable_value(&mut unit, DisplayUnit::Sats, "Sats");
		app.state.display_unit = unit;
	});
	match &app.state.price {
		Some(p) if p.price > 0.0 => { ui.weak(format!("Live rate: ${:.2} / BTC", p.price)); }
		_ => { ui.weak("Live rate: unavailable"); }
	}
	ui.add_space(12.0);
	ui.separator();
	ui.add_space(12.0);

	crate::ui::widgets::section_header(ui, "Connection");
	#[cfg(not(target_arch = "wasm32"))]
	if let Some(path) = &app.state.config_file_path {
		ui.weak(format!("Loaded from: {}", path));
	}
	crate::ui::connection::render_settings(ui, app);
}
