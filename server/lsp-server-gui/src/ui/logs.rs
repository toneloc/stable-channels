use eframe::egui;

use crate::app::LspServerApp;
use crate::state::LogsTab;

pub fn render(ui: &mut egui::Ui, app: &mut LspServerApp) {
	ui.horizontal(|ui| {
		if ui.selectable_label(app.state.logs_tab == LogsTab::Audit, "Audit").clicked() {
			app.state.logs_tab = LogsTab::Audit;
		}
		if ui.selectable_label(app.state.logs_tab == LogsTab::ChannelLedger, "Channel Ledger").clicked() {
			app.state.logs_tab = LogsTab::ChannelLedger;
		}
		if ui.selectable_label(app.state.logs_tab == LogsTab::Ldk, "LDK server").clicked() {
			app.state.logs_tab = LogsTab::Ldk;
		}
	});
	ui.separator();
	match app.state.logs_tab {
		LogsTab::Audit => crate::ui::audit_log::render(ui, app),
		LogsTab::ChannelLedger => crate::ui::channel_ledger::render(ui, app),
		LogsTab::Ldk => crate::ui::ldk_log::render(ui, app),
	}
}
