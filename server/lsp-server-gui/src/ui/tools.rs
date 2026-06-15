use eframe::egui;

use crate::app::LspServerApp;
use crate::ui::widgets;

pub fn render(ui: &mut egui::Ui, app: &mut LspServerApp) {
	ui.heading("Tools");
	ui.add_space(10.0);

	if app.render_disconnected_gate(ui) {
		return;
	}

	render_sign_message(ui, app);
	ui.add_space(20.0);
	render_verify_signature(ui, app);
	ui.add_space(20.0);
	render_export_pathfinding_scores(ui, app);
}

fn render_sign_message(ui: &mut egui::Ui, app: &mut LspServerApp) {
	ui.group(|ui| {
		ui.heading("Sign Message");
		ui.add_space(5.0);

		let form = &mut app.state.forms.sign_message;

		ui.label("Message:");
		ui.add(
			egui::TextEdit::multiline(&mut form.message)
				.desired_rows(3)
				.desired_width(f32::INFINITY),
		);

		ui.add_space(10.0);

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.sign_message.is_some();
			if is_pending {
				ui.spinner();
				ui.label("Signing...");
			} else if ui.button("Sign").clicked() {
				app.sign_message();
			}
		});

		if let Some(signature) = &app.state.sign_result {
			ui.add_space(10.0);
			ui.separator();
			ui.label("Signature:");
			// Pre-extract to avoid borrow conflict between TextEdit and copy button
			let sig_clone = signature.clone();
			ui.add(
				egui::TextEdit::multiline(&mut sig_clone.as_str())
					.desired_rows(2)
					.desired_width(f32::INFINITY)
					.interactive(false),
			);
			if ui.button("Copy Signature").clicked() {
				ui.output_mut(|o| o.copied_text = sig_clone.clone());
				app.state.status_message = Some(crate::state::StatusMessage::success("Copied"));
			}
		}
	});
}

fn render_export_pathfinding_scores(ui: &mut egui::Ui, app: &mut LspServerApp) {
	ui.group(|ui| {
		ui.heading("Export Pathfinding Scores");
		ui.add_space(5.0);

		ui.label("Export the pathfinding scores used by the router.");

		ui.add_space(10.0);

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.export_pathfinding_scores.is_some();
			if is_pending {
				ui.spinner();
				ui.label("Exporting...");
			} else if ui.button("Export Scores").clicked() {
				app.export_pathfinding_scores();
			}
		});

		if let Some(result) = &app.state.export_scores_result {
			ui.add_space(10.0);
			ui.separator();
			let n = result.scores.len();
			ui.horizontal(|ui| {
				widgets::status_pill(ui, &format!("Exported {} bytes", n), egui::Color32::GREEN);
			});
		}
	});
}

fn render_verify_signature(ui: &mut egui::Ui, app: &mut LspServerApp) {
	ui.group(|ui| {
		ui.heading("Verify Signature");
		ui.add_space(5.0);

		let form = &mut app.state.forms.verify_signature;

		egui::Grid::new("verify_sig_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
			ui.label("Message:");
			ui.add(
				egui::TextEdit::multiline(&mut form.message)
					.desired_rows(2)
					.desired_width(f32::INFINITY),
			);
			ui.end_row();

			ui.label("Signature (zbase32):");
			ui.text_edit_singleline(&mut form.signature);
			ui.end_row();

			ui.label("Public Key (hex):");
			ui.text_edit_singleline(&mut form.public_key);
			ui.end_row();
		});

		ui.add_space(10.0);

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.verify_signature.is_some();
			if is_pending {
				ui.spinner();
				ui.label("Verifying...");
			} else if ui.button("Verify").clicked() {
				app.verify_signature();
			}
		});

		if let Some(valid) = &app.state.verify_result {
			ui.add_space(5.0);
			ui.horizontal(|ui| {
				if *valid {
					widgets::status_pill(ui, "VALID", egui::Color32::GREEN);
				} else {
					widgets::status_pill(ui, "INVALID", egui::Color32::RED);
				}
			});
		}
	});
}
