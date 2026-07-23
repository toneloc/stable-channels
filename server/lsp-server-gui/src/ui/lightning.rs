use egui::Ui;

use crate::app::LspServerApp;
use crate::state::{LightningTab, StatusMessage};
use crate::ui::layout::{card, page_scrolled, FORM_WIDTH};
use crate::ui::widgets;

const HELP_BOLT11_INVOICE: &str = "A one-time Lightning invoice that can include amount, description, expiry, routing hints, and payment hash. Enter an amount only when the invoice is zero-amount.";
const HELP_GENERATED_BOLT11_INVOICE: &str =
    "A one-time Lightning invoice another payer can settle before it expires.";
const HELP_BOLT12_OFFER: &str =
	"A reusable Lightning offer. The payer requests an invoice from the recipient and can include amount, quantity, or a payer note when the offer allows it.";
const HELP_GENERATED_BOLT12_OFFER: &str =
    "A reusable offer that another wallet can use to request an invoice and pay this node.";
const HELP_AMOUNT: &str =
	"The payment amount in the selected display unit. Lightning sends are tracked internally in millisatoshis.";
const HELP_ZERO_AMOUNT: &str = "Use this only when the BOLT11 invoice does not specify an amount.";
const HELP_DESCRIPTION: &str =
    "Human-readable payment description included in the invoice or offer.";
const HELP_EXPIRY: &str = "How long the invoice or offer should remain payable, in seconds.";
const HELP_QUANTITY: &str =
    "The number of offered items or units requested when the BOLT12 offer supports quantities.";
const HELP_PAYER_NOTE: &str =
    "Optional note sent with the BOLT12 payment request. Avoid secrets or sensitive information.";
const HELP_NODE_ID: &str = "The recipient's Lightning node public key in hex.";
const HELP_KEYSEND: &str =
	"A spontaneous Lightning payment that includes the payment secret material needed by the recipient to settle without a prior invoice.";
const HELP_LAST_PAYMENT_ID: &str =
    "The local identifier used to look up this payment record later.";

pub fn render(ui: &mut Ui, app: &mut LspServerApp) {
    ui.heading("Lightning Payments");
    ui.add_space(10.0);

    if app.render_disconnected_gate(ui) {
        return;
    }

    ui.horizontal(|ui| {
        if ui
            .selectable_label(
                app.state.lightning_tab == LightningTab::Bolt11Send,
                "BOLT11 Send",
            )
            .clicked()
        {
            app.state.lightning_tab = LightningTab::Bolt11Send;
        }
        if ui
            .selectable_label(
                app.state.lightning_tab == LightningTab::Bolt11Receive,
                "BOLT11 Receive",
            )
            .clicked()
        {
            app.state.lightning_tab = LightningTab::Bolt11Receive;
        }
        if ui
            .selectable_label(
                app.state.lightning_tab == LightningTab::Bolt12Send,
                "BOLT12 Send",
            )
            .clicked()
        {
            app.state.lightning_tab = LightningTab::Bolt12Send;
        }
        if ui
            .selectable_label(
                app.state.lightning_tab == LightningTab::Bolt12Receive,
                "BOLT12 Receive",
            )
            .clicked()
        {
            app.state.lightning_tab = LightningTab::Bolt12Receive;
        }
        if ui
            .selectable_label(
                app.state.lightning_tab == LightningTab::SpontaneousSend,
                "Keysend",
            )
            .clicked()
        {
            app.state.lightning_tab = LightningTab::SpontaneousSend;
        }
    });

    ui.separator();
    ui.add_space(10.0);

    // One outer scroll for the selected sub-form, bounded to FORM_WIDTH like the On-chain/Tools tabs.
    page_scrolled(ui, |ui| {
        let form_w = ui.available_width().min(FORM_WIDTH);
        ui.vertical(|ui| {
            ui.set_width(form_w);
            match app.state.lightning_tab {
                LightningTab::Bolt11Send => render_bolt11_send(ui, app),
                LightningTab::Bolt11Receive => render_bolt11_receive(ui, app),
                LightningTab::Bolt12Send => render_bolt12_send(ui, app),
                LightningTab::Bolt12Receive => render_bolt12_receive(ui, app),
                LightningTab::SpontaneousSend => render_spontaneous_send(ui, app),
            }
        });
    });
}

fn render_bolt11_send(ui: &mut Ui, app: &mut LspServerApp) {
    card(ui, "Pay BOLT11 Invoice", |ui| {
        let unit_label = crate::ui::unit_label(app.state.display_unit);
        let form = &mut app.state.forms.bolt11_send;

        widgets::label_with_info(ui, "Invoice:", HELP_BOLT11_INVOICE);
        ui.add(
            egui::TextEdit::multiline(&mut form.invoice)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );

        ui.add_space(5.0);

        egui::Grid::new("bolt11_send_grid")
            .num_columns(2)
            .spacing([10.0, 5.0])
            .show(ui, |ui| {
                widgets::label_with_info(
                    ui,
                    &format!("Amount ({}, for zero-amount invoices):", unit_label),
                    HELP_ZERO_AMOUNT,
                );
                ui.text_edit_singleline(&mut form.amount_msat);
                ui.end_row();
            });

        // preview: read field text locally to avoid borrow conflict with the &self method
        let amt = app.state.forms.bolt11_send.amount_msat.clone();
        if let Some(preview) = app.amount_entry_preview(&amt) {
            ui.weak(preview);
        }

        ui.add_space(10.0);

        let is_pending = app.state.tasks.bolt11_send.is_some();
        if is_pending {
            widgets::loading_row(ui, "Sending...");
        } else if ui.button("Pay Invoice").clicked() {
            app.send_bolt11();
        }

        if let Some(payment_id) = &app.state.last_payment_id {
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                widgets::label_with_info(ui, "Last Payment ID:", HELP_LAST_PAYMENT_ID);
                ui.monospace(crate::ui::truncate_id(payment_id, 8, 8));
                if ui.small_button("Copy").clicked() {
                    ui.output_mut(|o| o.copied_text = payment_id.clone());
                }
            });
        }
    });
}

fn render_bolt11_receive(ui: &mut Ui, app: &mut LspServerApp) {
    card(ui, "Generate BOLT11 Invoice", |ui| {
        let unit_label = crate::ui::unit_label(app.state.display_unit);
        let form = &mut app.state.forms.bolt11_receive;

        egui::Grid::new("bolt11_receive_grid")
            .num_columns(2)
            .spacing([10.0, 5.0])
            .show(ui, |ui| {
                widgets::label_with_info(
                    ui,
                    &format!("Amount ({}, optional):", unit_label),
                    HELP_AMOUNT,
                );
                ui.text_edit_singleline(&mut form.amount_msat);
                ui.end_row();

                widgets::label_with_info(ui, "Description:", HELP_DESCRIPTION);
                ui.text_edit_singleline(&mut form.description);
                ui.end_row();

                widgets::label_with_info(ui, "Expiry (seconds):", HELP_EXPIRY);
                ui.text_edit_singleline(&mut form.expiry_secs);
                ui.end_row();
            });

        // preview: read field text locally to avoid borrow conflict with fmt_msat
        let amt = app.state.forms.bolt11_receive.amount_msat.clone();
        if let Some(preview) = app.amount_entry_preview(&amt) {
            ui.weak(preview);
        }

        ui.add_space(10.0);

        let is_pending = app.state.tasks.bolt11_receive.is_some();
        if is_pending {
            widgets::loading_row(ui, "Generating...");
        } else if ui.button("Generate Invoice").clicked() {
            app.generate_bolt11_invoice();
        }

        if let Some(invoice) = &app.state.generated_invoice.clone() {
            ui.add_space(10.0);
            ui.separator();
            widgets::label_with_info(ui, "Generated Invoice:", HELP_GENERATED_BOLT11_INVOICE);
            ui.add(
                egui::TextEdit::multiline(&mut invoice.as_str())
                    .desired_rows(4)
                    .desired_width(f32::INFINITY)
                    .interactive(false),
            );
            if ui.button("Copy Invoice").clicked() {
                ui.output_mut(|o| o.copied_text = invoice.clone());
                app.state.status_message = Some(StatusMessage::success("Copied"));
            }
        }
    });
}

fn render_bolt12_send(ui: &mut Ui, app: &mut LspServerApp) {
    card(ui, "Pay BOLT12 Offer", |ui| {
        let unit_label = crate::ui::unit_label(app.state.display_unit);
        let form = &mut app.state.forms.bolt12_send;

        widgets::label_with_info(ui, "Offer:", HELP_BOLT12_OFFER);
        ui.add(
            egui::TextEdit::multiline(&mut form.offer)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );

        ui.add_space(5.0);

        egui::Grid::new("bolt12_send_grid")
            .num_columns(2)
            .spacing([10.0, 5.0])
            .show(ui, |ui| {
                widgets::label_with_info(
                    ui,
                    &format!("Amount ({}, optional):", unit_label),
                    HELP_AMOUNT,
                );
                ui.text_edit_singleline(&mut form.amount_msat);
                ui.end_row();

                widgets::label_with_info(ui, "Quantity (optional):", HELP_QUANTITY);
                ui.text_edit_singleline(&mut form.quantity);
                ui.end_row();

                widgets::label_with_info(ui, "Payer Note (optional):", HELP_PAYER_NOTE);
                ui.text_edit_singleline(&mut form.payer_note);
                ui.end_row();
            });

        // preview: read field text locally to avoid borrow conflict with fmt_msat
        let amt = app.state.forms.bolt12_send.amount_msat.clone();
        if let Some(preview) = app.amount_entry_preview(&amt) {
            ui.weak(preview);
        }

        ui.add_space(10.0);

        let is_pending = app.state.tasks.bolt12_send.is_some();
        if is_pending {
            widgets::loading_row(ui, "Sending...");
        } else if ui.button("Pay Offer").clicked() {
            app.send_bolt12();
        }

        if let Some(payment_id) = &app.state.last_payment_id {
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                widgets::label_with_info(ui, "Last Payment ID:", HELP_LAST_PAYMENT_ID);
                ui.monospace(crate::ui::truncate_id(payment_id, 8, 8));
                if ui.small_button("Copy").clicked() {
                    ui.output_mut(|o| o.copied_text = payment_id.clone());
                }
            });
        }
    });
}

fn render_bolt12_receive(ui: &mut Ui, app: &mut LspServerApp) {
    card(ui, "Generate BOLT12 Offer", |ui| {
        let unit_label = crate::ui::unit_label(app.state.display_unit);
        let form = &mut app.state.forms.bolt12_receive;

        egui::Grid::new("bolt12_receive_grid")
            .num_columns(2)
            .spacing([10.0, 5.0])
            .show(ui, |ui| {
                widgets::label_with_info(ui, "Description (required):", HELP_DESCRIPTION);
                ui.text_edit_singleline(&mut form.description);
                ui.end_row();

                widgets::label_with_info(
                    ui,
                    &format!("Amount ({}, optional):", unit_label),
                    HELP_AMOUNT,
                );
                ui.text_edit_singleline(&mut form.amount_msat);
                ui.end_row();

                widgets::label_with_info(ui, "Expiry (seconds, optional):", HELP_EXPIRY);
                ui.text_edit_singleline(&mut form.expiry_secs);
                ui.end_row();

                widgets::label_with_info(ui, "Quantity (optional):", HELP_QUANTITY);
                ui.text_edit_singleline(&mut form.quantity);
                ui.end_row();
            });

        // preview: read field text locally to avoid borrow conflict with fmt_msat
        let amt = app.state.forms.bolt12_receive.amount_msat.clone();
        if let Some(preview) = app.amount_entry_preview(&amt) {
            ui.weak(preview);
        }

        ui.add_space(10.0);

        let is_pending = app.state.tasks.bolt12_receive.is_some();
        if is_pending {
            widgets::loading_row(ui, "Generating...");
        } else if ui.button("Generate Offer").clicked() {
            app.generate_bolt12_offer();
        }

        if let Some(offer) = &app.state.generated_offer.clone() {
            ui.add_space(10.0);
            ui.separator();
            widgets::label_with_info(ui, "Generated Offer:", HELP_GENERATED_BOLT12_OFFER);
            ui.add(
                egui::TextEdit::multiline(&mut offer.as_str())
                    .desired_rows(4)
                    .desired_width(f32::INFINITY)
                    .interactive(false),
            );
            if ui.button("Copy Offer").clicked() {
                ui.output_mut(|o| o.copied_text = offer.clone());
                app.state.status_message = Some(StatusMessage::success("Copied"));
            }
        }
    });
}

fn render_spontaneous_send(ui: &mut Ui, app: &mut LspServerApp) {
    card(ui, "Spontaneous Payment (Keysend)", |ui| {
        let unit_label = crate::ui::unit_label(app.state.display_unit);
        let form = &mut app.state.forms.spontaneous_send;

        ui.horizontal(|ui| {
            widgets::label_with_info(ui, "Payment type:", HELP_KEYSEND);
            ui.label("Keysend");
        });
        ui.add_space(5.0);

        egui::Grid::new("spontaneous_send_grid")
            .num_columns(2)
            .spacing([10.0, 5.0])
            .show(ui, |ui| {
                widgets::label_with_info(ui, "Node ID (hex):", HELP_NODE_ID);
                ui.text_edit_singleline(&mut form.node_id);
                ui.end_row();

                widgets::label_with_info(ui, &format!("Amount ({}):", unit_label), HELP_AMOUNT);
                ui.text_edit_singleline(&mut form.amount_msat);
                ui.end_row();
            });

        // preview: read field text locally to avoid borrow conflict with fmt_msat
        let amt = app.state.forms.spontaneous_send.amount_msat.clone();
        if let Some(preview) = app.amount_entry_preview(&amt) {
            ui.weak(preview);
        }

        ui.add_space(10.0);

        let is_pending = app.state.tasks.spontaneous_send.is_some();
        if is_pending {
            widgets::loading_row(ui, "Sending...");
        } else if ui.button("Send Keysend").clicked() {
            app.spontaneous_send();
        }

        if let Some(payment_id) = &app.state.last_payment_id {
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                widgets::label_with_info(ui, "Last Payment ID:", HELP_LAST_PAYMENT_ID);
                ui.monospace(crate::ui::truncate_id(payment_id, 8, 8));
                if ui.small_button("Copy").clicked() {
                    ui.output_mut(|o| o.copied_text = payment_id.clone());
                }
            });
        }
    });
}
