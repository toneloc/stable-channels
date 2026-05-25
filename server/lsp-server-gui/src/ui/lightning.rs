use egui::Ui;

use crate::app::LspServerApp;
use crate::state::{ConnectionStatus, LightningTab};

pub fn render(ui: &mut Ui, app: &mut LspServerApp) {
	ui.heading("Lightning Payments");
	ui.add_space(10.0);

	if !matches!(app.state.connection_status, ConnectionStatus::Connected) {
		ui.label("Connect to a server to use lightning payments.");
		return;
	}

	ui.horizontal(|ui| {
		if ui
			.selectable_label(app.state.lightning_tab == LightningTab::Bolt11Send, "BOLT11 Send")
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
			.selectable_label(app.state.lightning_tab == LightningTab::Bolt12Send, "BOLT12 Send")
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
			.selectable_label(app.state.lightning_tab == LightningTab::SpontaneousSend, "Keysend")
			.clicked()
		{
			app.state.lightning_tab = LightningTab::SpontaneousSend;
		}
	});

	ui.separator();
	ui.add_space(10.0);

	match app.state.lightning_tab {
		LightningTab::Bolt11Send => render_bolt11_send(ui, app),
		LightningTab::Bolt11Receive => render_bolt11_receive(ui, app),
		LightningTab::Bolt12Send => render_bolt12_send(ui, app),
		LightningTab::Bolt12Receive => render_bolt12_receive(ui, app),
		LightningTab::SpontaneousSend => render_spontaneous_send(ui, app),
	}
}

fn render_bolt11_send(ui: &mut Ui, app: &mut LspServerApp) {
	ui.group(|ui| {
		ui.heading("Pay BOLT11 Invoice");
		ui.add_space(5.0);

		let form = &mut app.state.forms.bolt11_send;

		ui.label("Invoice:");
		ui.add(
			egui::TextEdit::multiline(&mut form.invoice)
				.desired_rows(3)
				.desired_width(f32::INFINITY),
		);

		ui.add_space(5.0);

		egui::Grid::new("bolt11_send_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
			ui.label("Amount (msat, for zero-amount invoices):");
			ui.text_edit_singleline(&mut form.amount_msat);
			ui.end_row();
		});

		ui.add_space(10.0);

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.bolt11_send.is_some();
			if is_pending {
				ui.spinner();
				ui.label("Sending...");
			} else if ui.button("Pay Invoice").clicked() {
				app.send_bolt11();
			}
		});

		if let Some(payment_id) = &app.state.last_payment_id {
			ui.add_space(5.0);
			ui.horizontal(|ui| {
				ui.label("Last Payment ID:");
				ui.monospace(crate::ui::truncate_id(payment_id, 8, 8));
				if ui.small_button("Copy").clicked() {
					ui.output_mut(|o| o.copied_text = payment_id.clone());
				}
			});
		}
	});
}

fn render_bolt11_receive(ui: &mut Ui, app: &mut LspServerApp) {
	ui.group(|ui| {
		ui.heading("Generate BOLT11 Invoice");
		ui.add_space(5.0);

		let form = &mut app.state.forms.bolt11_receive;

		egui::Grid::new("bolt11_receive_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
			ui.label("Amount (msat, optional):");
			ui.text_edit_singleline(&mut form.amount_msat);
			ui.end_row();

			ui.label("Description:");
			ui.text_edit_singleline(&mut form.description);
			ui.end_row();

			ui.label("Expiry (seconds):");
			ui.text_edit_singleline(&mut form.expiry_secs);
			ui.end_row();
		});

		ui.add_space(10.0);

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.bolt11_receive.is_some();
			if is_pending {
				ui.spinner();
				ui.label("Generating...");
			} else if ui.button("Generate Invoice").clicked() {
				app.generate_bolt11_invoice();
			}
		});

		if let Some(invoice) = &app.state.generated_invoice {
			ui.add_space(10.0);
			ui.separator();
			ui.label("Generated Invoice:");
			ui.add(
				egui::TextEdit::multiline(&mut invoice.as_str())
					.desired_rows(4)
					.desired_width(f32::INFINITY)
					.interactive(false),
			);
			if ui.button("Copy Invoice").clicked() {
				ui.output_mut(|o| o.copied_text = invoice.clone());
			}
		}
	});
}

fn render_bolt12_send(ui: &mut Ui, app: &mut LspServerApp) {
	ui.group(|ui| {
		ui.heading("Pay BOLT12 Offer");
		ui.add_space(5.0);

		let form = &mut app.state.forms.bolt12_send;

		ui.label("Offer:");
		ui.add(
			egui::TextEdit::multiline(&mut form.offer).desired_rows(3).desired_width(f32::INFINITY),
		);

		ui.add_space(5.0);

		egui::Grid::new("bolt12_send_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
			ui.label("Amount (msat, optional):");
			ui.text_edit_singleline(&mut form.amount_msat);
			ui.end_row();

			ui.label("Quantity (optional):");
			ui.text_edit_singleline(&mut form.quantity);
			ui.end_row();

			ui.label("Payer Note (optional):");
			ui.text_edit_singleline(&mut form.payer_note);
			ui.end_row();
		});

		ui.add_space(10.0);

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.bolt12_send.is_some();
			if is_pending {
				ui.spinner();
				ui.label("Sending...");
			} else if ui.button("Pay Offer").clicked() {
				app.send_bolt12();
			}
		});

		if let Some(payment_id) = &app.state.last_payment_id {
			ui.add_space(5.0);
			ui.horizontal(|ui| {
				ui.label("Last Payment ID:");
				ui.monospace(crate::ui::truncate_id(payment_id, 8, 8));
				if ui.small_button("Copy").clicked() {
					ui.output_mut(|o| o.copied_text = payment_id.clone());
				}
			});
		}
	});
}

fn render_bolt12_receive(ui: &mut Ui, app: &mut LspServerApp) {
	ui.group(|ui| {
		ui.heading("Generate BOLT12 Offer");
		ui.add_space(5.0);

		let form = &mut app.state.forms.bolt12_receive;

		egui::Grid::new("bolt12_receive_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
			ui.label("Description (required):");
			ui.text_edit_singleline(&mut form.description);
			ui.end_row();

			ui.label("Amount (msat, optional):");
			ui.text_edit_singleline(&mut form.amount_msat);
			ui.end_row();

			ui.label("Expiry (seconds, optional):");
			ui.text_edit_singleline(&mut form.expiry_secs);
			ui.end_row();

			ui.label("Quantity (optional):");
			ui.text_edit_singleline(&mut form.quantity);
			ui.end_row();
		});

		ui.add_space(10.0);

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.bolt12_receive.is_some();
			if is_pending {
				ui.spinner();
				ui.label("Generating...");
			} else if ui.button("Generate Offer").clicked() {
				app.generate_bolt12_offer();
			}
		});

		if let Some(offer) = &app.state.generated_offer {
			ui.add_space(10.0);
			ui.separator();
			ui.label("Generated Offer:");
			ui.add(
				egui::TextEdit::multiline(&mut offer.as_str())
					.desired_rows(4)
					.desired_width(f32::INFINITY)
					.interactive(false),
			);
			if ui.button("Copy Offer").clicked() {
				ui.output_mut(|o| o.copied_text = offer.clone());
			}
		}
	});
}

fn render_spontaneous_send(ui: &mut Ui, app: &mut LspServerApp) {
	ui.group(|ui| {
		ui.heading("Spontaneous Payment (Keysend)");
		ui.add_space(5.0);

		let form = &mut app.state.forms.spontaneous_send;

		egui::Grid::new("spontaneous_send_grid").num_columns(2).spacing([10.0, 5.0]).show(
			ui,
			|ui| {
				ui.label("Node ID (hex):");
				ui.text_edit_singleline(&mut form.node_id);
				ui.end_row();

				ui.label("Amount (msat):");
				ui.text_edit_singleline(&mut form.amount_msat);
				ui.end_row();
			},
		);

		ui.add_space(10.0);

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.spontaneous_send.is_some();
			if is_pending {
				ui.spinner();
				ui.label("Sending...");
			} else if ui.button("Send Keysend").clicked() {
				app.spontaneous_send();
			}
		});

		if let Some(payment_id) = &app.state.last_payment_id {
			ui.add_space(5.0);
			ui.horizontal(|ui| {
				ui.label("Last Payment ID:");
				ui.monospace(crate::ui::truncate_id(payment_id, 8, 8));
				if ui.small_button("Copy").clicked() {
					ui.output_mut(|o| o.copied_text = payment_id.clone());
				}
			});
		}
	});
}
