use egui::{Color32, RichText, Ui};

const AMBER: Color32 = Color32::from_rgb(0xF7, 0x93, 0x1A);

/// A big headline figure with a small muted secondary line, in a framed card.
pub fn stat_card(ui: &mut Ui, title: &str, big_value: &str, secondary: &str) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).small().weak());
            ui.label(RichText::new(big_value).size(24.0).strong());
            if !secondary.is_empty() {
                ui.label(RichText::new(secondary).small().weak());
            }
        });
    });
}

/// A small filled status pill.
pub fn status_pill(ui: &mut Ui, text: &str, color: Color32) {
    let bg = color.linear_multiply(0.25);
    egui::Frame::none()
        .fill(bg)
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::symmetric(8.0, 2.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).small().strong());
        });
}

/// Monospace truncated id + a copy button that confirms via the status bar.
pub fn id_with_copy(ui: &mut Ui, full_id: &str, status: &mut Option<crate::state::StatusMessage>) {
    ui.horizontal(|ui| {
        let short = super::truncate_id(full_id, 8, 8);
        ui.monospace(&short).on_hover_text(full_id);
        if ui.small_button("Copy").clicked() {
            ui.output_mut(|o| o.copied_text = full_id.to_string());
            *status = Some(crate::state::StatusMessage::success("Copied to clipboard"));
        }
    });
}

/// Centered empty-state placeholder.
pub fn empty_state(ui: &mut Ui, icon: &str, title: &str, hint: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(24.0);
        ui.label(RichText::new(icon).size(32.0));
        ui.label(RichText::new(title).strong());
        ui.label(RichText::new(hint).small().weak());
        ui.add_space(24.0);
    });
}

/// Spinner + label loading row.
pub fn loading_row(ui: &mut Ui, label: &str) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(label);
    });
}

/// Amber section header.
pub fn section_header(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(AMBER).strong());
}

pub enum NotConnectedAction {
    None,
    OpenSettings,
    Retry,
}

/// Centered "not connected" state with actions, used by every screen's connection gate.
pub fn not_connected(ui: &mut Ui, status: &crate::state::ConnectionStatus, server_url: &str) -> NotConnectedAction {
    let mut action = NotConnectedAction::None;
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        match status {
            crate::state::ConnectionStatus::Error(e) => {
                ui.label(RichText::new("⚠").size(32.0));
                ui.label(RichText::new(format!("Can't reach the LSP at {}", server_url)).strong());
                ui.label(RichText::new(e).small().weak());
                ui.label(RichText::new("Make sure the daemon is running.").small().weak());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("⚙ Open Settings").clicked() { action = NotConnectedAction::OpenSettings; }
                    if ui.button("⟳ Retry").clicked() { action = NotConnectedAction::Retry; }
                });
            },
            _ => {
                ui.label(RichText::new("🔌").size(32.0));
                ui.label(RichText::new("Not connected to an LSP").strong());
                ui.label(RichText::new("Open Settings to configure the connection.").small().weak());
                ui.add_space(8.0);
                if ui.button("⚙ Open Settings").clicked() { action = NotConnectedAction::OpenSettings; }
            },
        }
        ui.add_space(40.0);
    });
    action
}
