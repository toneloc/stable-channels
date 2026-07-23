use egui::{Color32, RichText, Ui};

/// Bright near-white for primary values and headings.
pub const PRIMARY: Color32 = Color32::from_rgb(0xF2, 0xF2, 0xF2);
/// Readable medium gray for labels/captions (replaces overused `.weak()`).
pub const SECONDARY: Color32 = Color32::from_rgb(0xA8, 0xA8, 0xA8);
/// Bitcoin amber accent.
pub const AMBER: Color32 = Color32::from_rgb(0xF7, 0x93, 0x1A);

/// Comfortable width for input forms so they don't stretch across the page.
pub const FORM_WIDTH: f32 = 560.0;

/// Number of responsive columns for a given available width.
pub fn columns_for_width(w: f32) -> usize {
    if w >= 1000.0 {
        3
    } else if w >= 680.0 {
        2
    } else {
        1
    }
}

/// Fill the full available width, left-aligned (no cap, no centering).
pub fn page(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    add(ui);
}

/// Like `page` but wraps the content in a vertical scroll (for detail/form tabs).
pub fn page_scrolled(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            add(ui);
        });
}

/// Horizontally-scrollable region that fills the window width but never renders narrower than `min_width` (scrolls instead of cropping).
pub fn h_scroll(ui: &mut Ui, min_width: f32, add: impl FnOnce(&mut Ui)) {
    let avail = ui.available_width();
    egui::ScrollArea::horizontal()
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.set_width(avail.max(min_width));
            add(ui);
        });
}

/// Split `total` width across columns by weight (proportional fill). Returns per-column widths.
pub fn weighted_widths(total: f32, weights: &[f32]) -> Vec<f32> {
    let sum: f32 = weights.iter().sum();
    weights.iter().map(|w| total * w / sum).collect()
}

/// A titled card that fills the width it is given and separates from the panel.
pub fn card(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(12.0))
        .rounding(egui::Rounding::same(8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(RichText::new(title).color(AMBER).strong().size(15.0));
                ui.add_space(6.0);
            }
            add(ui);
        });
}

// Side-by-side cards are done inline at the call site with `ui.columns` (Recipe C), not via a helper — that lets each card body borrow `&mut app` without overlap.

/// Label:value rows — SECONDARY labels, PRIMARY values, bounded spacing.
#[allow(dead_code)]
pub fn kv_grid(ui: &mut Ui, id_salt: &str, rows: &[(&str, &str)]) {
    egui::Grid::new(id_salt)
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            for (k, v) in rows {
                ui.label(RichText::new(*k).color(SECONDARY));
                ui.label(RichText::new(*v).color(PRIMARY));
                ui.end_row();
            }
        });
}

/// Row list for `kv_grid_custom`: label paired with a closure that renders the value cell.
#[allow(dead_code)]
pub type KvRows<'a> = Vec<(&'a str, Box<dyn FnOnce(&mut Ui) + 'a>)>;

/// Row list for key/value grids whose labels can include help text.
pub type KvInfoRows<'a> = Vec<(&'a str, Option<&'a str>, Box<dyn FnOnce(&mut Ui) + 'a>)>;

/// Like `kv_grid` but each value cell is rendered by a closure (copy buttons, pills…).
#[allow(dead_code)]
pub fn kv_grid_custom(ui: &mut Ui, id_salt: &str, rows: KvRows) {
    egui::Grid::new(id_salt)
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            for (k, v) in rows {
                ui.label(RichText::new(k).color(SECONDARY));
                v(ui);
                ui.end_row();
            }
        });
}

/// Like `kv_grid_custom`, but label cells may include compact info affordances.
pub fn kv_grid_custom_info(ui: &mut Ui, id_salt: &str, rows: KvInfoRows) {
    egui::Grid::new(id_salt)
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            for (k, help, v) in rows {
                match help {
                    Some(help) => crate::ui::widgets::muted_label_with_info(ui, k, help),
                    None => {
                        ui.label(RichText::new(k).color(SECONDARY));
                    }
                }
                v(ui);
                ui.end_row();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_scale_with_width() {
        assert_eq!(columns_for_width(1400.0), 3);
        assert_eq!(columns_for_width(1000.0), 3);
        assert_eq!(columns_for_width(800.0), 2);
        assert_eq!(columns_for_width(680.0), 2);
        assert_eq!(columns_for_width(500.0), 1);
        assert_eq!(columns_for_width(0.0), 1);
    }

    #[test]
    fn text_roles_are_distinct() {
        assert_ne!(PRIMARY, SECONDARY);
        assert_ne!(SECONDARY, AMBER);
    }
}
