use crate::ui::layout::{AMBER, SECONDARY};
use egui::{Color32, RichText, Ui};

/// A big headline figure with a small muted secondary line, in a framed card.
#[allow(dead_code)]
pub fn stat_card(ui: &mut Ui, title: &str, big_value: &str, secondary: &str) {
    stat_card_inner(ui, title, None, big_value, secondary);
}

/// A headline figure with a compact info affordance beside the title.
pub fn stat_card_with_info(ui: &mut Ui, title: &str, help: &str, big_value: &str, secondary: &str) {
    stat_card_inner(ui, title, Some(help), big_value, secondary);
}

fn stat_card_inner(ui: &mut Ui, title: &str, help: Option<&str>, big_value: &str, secondary: &str) {
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(12.0))
        .rounding(egui::Rounding::same(8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(title).small().color(SECONDARY));
                    if let Some(help) = help {
                        ui.add_space(4.0);
                        info_icon(ui, help);
                    }
                });
                ui.label(RichText::new(big_value).size(24.0).strong());
                if !secondary.is_empty() {
                    ui.label(RichText::new(secondary).small().color(SECONDARY));
                }
            });
        });
}

/// Draw a small hover-only info affordance with no button chrome.
pub fn info_icon(ui: &mut Ui, help: impl Into<egui::WidgetText>) -> egui::Response {
    let size = egui::vec2(14.0, 14.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let center = rect.center();
    let color = SECONDARY;
    let stroke = if response.hovered() {
        egui::Stroke::new(1.15, AMBER)
    } else {
        egui::Stroke::new(1.0, color)
    };

    if response.hovered() {
        ui.painter()
            .circle_filled(center, 6.5, AMBER.gamma_multiply(0.12));
    }

    ui.painter().circle_stroke(center, 6.0, stroke);
    ui.painter().text(
        center,
        egui::Align2::CENTER_CENTER,
        "i",
        egui::FontId::proportional(9.5),
        if response.hovered() { AMBER } else { color },
    );

    response.on_hover_text(help)
}

/// Plain label with a compact info icon.
pub fn label_with_info(ui: &mut Ui, label: &str, help: &str) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add_space(4.0);
        info_icon(ui, help);
    });
}

/// Muted label with a compact info icon, useful in key/value grids.
pub fn muted_label_with_info(ui: &mut Ui, label: &str, help: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(SECONDARY));
        ui.add_space(4.0);
        info_icon(ui, help);
    });
}

/// Strong label with a compact info icon.
pub fn strong_label_with_info(ui: &mut Ui, label: &str, help: &str) {
    ui.horizontal(|ui| {
        ui.strong(label);
        ui.add_space(4.0);
        info_icon(ui, help);
    });
}

/// Table header label with a compact info icon.
pub fn table_header_with_info(ui: &mut Ui, label: &str, help: &str) {
    ui.horizontal(|ui| {
        ui.strong(label);
        ui.add_space(4.0);
        info_icon(ui, help);
    });
}

/// A small non-interactive status pill: white text on a dimmed color chip.
/// Built as a Button (a Widget) so it stays content-sized and centers cleanly.
pub fn status_pill(ui: &mut Ui, text: &str, color: Color32) {
    let bg = Color32::from_rgb(color.r() / 2, color.g() / 2, color.b() / 2);
    let prev = ui.spacing().button_padding;
    ui.spacing_mut().button_padding = egui::vec2(8.0, 3.0);
    ui.add(
        egui::Button::new(RichText::new(text).color(Color32::WHITE).small())
            .fill(bg)
            .stroke(egui::Stroke::NONE)
            .rounding(egui::Rounding::same(8.0))
            .sense(egui::Sense::hover()),
    );
    ui.spacing_mut().button_padding = prev;
}

/// A destructive-action button: red outline + red text, transparent fill.
pub fn danger_button(ui: &mut Ui, label: &str) -> egui::Response {
    let btn = egui::Button::new(RichText::new(label).color(Color32::RED))
        .small()
        .fill(Color32::TRANSPARENT)
        .stroke(egui::Stroke::new(1.0, Color32::RED));
    ui.add(btn)
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
pub fn not_connected(
    ui: &mut Ui,
    status: &crate::state::ConnectionStatus,
    server_url: &str,
) -> NotConnectedAction {
    let mut action = NotConnectedAction::None;
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        match status {
            crate::state::ConnectionStatus::Error(e) => {
                ui.label(RichText::new("⚠").size(32.0));
                ui.label(RichText::new(format!("Can't reach the LSP at {}", server_url)).strong());
                ui.label(RichText::new(e).small().weak());
                ui.label(
                    RichText::new("Make sure the daemon is running.")
                        .small()
                        .weak(),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("⚙ Open Settings").clicked() {
                        action = NotConnectedAction::OpenSettings;
                    }
                    if ui.button("⟳ Retry").clicked() {
                        action = NotConnectedAction::Retry;
                    }
                });
            }
            _ => {
                ui.label(RichText::new("🔌").size(32.0));
                ui.label(RichText::new("Not connected to an LSP").strong());
                ui.label(
                    RichText::new("Open Settings to configure the connection.")
                        .small()
                        .weak(),
                );
                ui.add_space(8.0);
                if ui.button("⚙ Open Settings").clicked() {
                    action = NotConnectedAction::OpenSettings;
                }
            }
        }
        ui.add_space(40.0);
    });
    action
}
