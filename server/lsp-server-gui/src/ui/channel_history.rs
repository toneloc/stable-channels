use eframe::egui;
use sc_rest_client::sc_protos::stable::{
	AccountingSnapshot, ChannelLedgerEvent, ListChannelLedgerEventsResponse,
};

use crate::app::LspServerApp;
use crate::state::StatusMessage;
use crate::ui::widgets;

pub fn render(ui: &mut egui::Ui, app: &mut LspServerApp) {
	ui.heading("Channel Ledger");
	ui.label("Authoritative SQLite history. Identifier matching is exact across channel, payment, transaction, node, and correlation references.");
	ui.add_space(8.0);

	ui.horizontal_wrapped(|ui| {
		ui.label("Identifier:");
		ui.add(
			egui::TextEdit::singleline(&mut app.state.forms.channel_history.identifier)
				.desired_width(300.0)
				.hint_text("user_channel_id / channel_id / payment_id / txid"),
		);
		filter_combo(ui, "ledger_category", "Category", &mut app.state.forms.channel_history.category,
			&["channel", "payment", "forwarding", "trade", "stability", "peer", "sweep", "reconciliation", "operator", "system"]);
		filter_combo(ui, "ledger_status", "Status", &mut app.state.forms.channel_history.status,
			&["observed", "pending", "completed", "partial", "failed", "skipped"]);
		filter_combo(ui, "ledger_completeness", "Completeness", &mut app.state.forms.channel_history.completeness,
			&["observed", "reconstructed", "legacy", "gap"]);
	});

	ui.horizontal(|ui| {
		let loading = app.state.tasks.channel_history.is_some();
		if ui.add_enabled(!loading, egui::Button::new("Refresh")).clicked() {
			app.state.channel_history_cursor = None;
			app.state.channel_history_appending = false;
			app.fetch_channel_history();
		}
		if ui.add_enabled(!loading && app.state.channel_history_cursor.is_some(), egui::Button::new("Load older")).clicked() {
			app.state.channel_history_appending = true;
			app.fetch_channel_history();
		}
		if ui.add_enabled(app.state.channel_history.is_some(), egui::Button::new("Export JSONL")).clicked() {
			if let Some(history) = &app.state.channel_history {
				export_jsonl(history, &mut app.state.status_message);
			}
		}
		if loading { ui.spinner(); }
	});
	ui.separator();

	match &app.state.channel_history {
		Some(history) if history.events.is_empty() => {
			ui.label("No ledger events match these exact filters.");
		},
		Some(history) => {
			for event in &history.events {
				render_event(ui, event);
				ui.add_space(5.0);
			}
		},
		None => {
			widgets::empty_state(ui, "🧾", "Ledger not loaded", "Choose filters and click Refresh");
		},
	}
}

fn filter_combo(ui: &mut egui::Ui, id: &str, label: &str, value: &mut String, choices: &[&str]) {
	egui::ComboBox::from_id_salt(id)
		.selected_text(if value.is_empty() { format!("All {label}") } else { value.clone() })
		.show_ui(ui, |ui| {
			ui.selectable_value(value, String::new(), format!("All {label}"));
			for choice in choices { ui.selectable_value(value, (*choice).to_owned(), *choice); }
		});
}

fn render_event(ui: &mut egui::Ui, event: &ChannelLedgerEvent) {
	egui::Frame::group(ui.style()).show(ui, |ui| {
		ui.horizontal_wrapped(|ui| {
			ui.strong(&event.event_type);
			badge(ui, &event.category, egui::Color32::from_rgb(70, 110, 170));
			badge(ui, &event.status, status_color(&event.status));
			badge(ui, &event.completeness, completeness_color(&event.completeness));
			ui.weak(format!("#{} • {} ms", event.id, event.occurred_at_ms));
		});
		if let Some(delta) = accounting_delta(event.before.as_ref(), event.after.as_ref()) {
			ui.monospace(delta);
		}
		if !event.refs.is_empty() {
			ui.horizontal_wrapped(|ui| {
				for reference in &event.refs {
					ui.small(format!("{}={}", reference.role, reference.value));
				}
			});
		}
		egui::CollapsingHeader::new("Raw JSON").id_salt(("ledger_raw", event.id)).show(ui, |ui| {
			let pretty = serde_json::from_str::<serde_json::Value>(&event.detail_json)
				.and_then(|value| serde_json::to_string_pretty(&value))
				.unwrap_or_else(|_| event.detail_json.clone());
			ui.monospace(pretty);
		});
	});
}

fn badge(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
	egui::Frame::none()
		.fill(color.gamma_multiply(0.22))
		.stroke(egui::Stroke::new(1.0, color))
		.rounding(4.0)
		.inner_margin(egui::Margin::symmetric(6.0, 2.0))
		.show(ui, |ui| { ui.small(text); });
}

fn status_color(status: &str) -> egui::Color32 {
	match status {
		"failed" => egui::Color32::RED,
		"completed" => egui::Color32::GREEN,
		"skipped" => egui::Color32::GRAY,
		_ => egui::Color32::YELLOW,
	}
}

fn completeness_color(completeness: &str) -> egui::Color32 {
	match completeness {
		"observed" => egui::Color32::GREEN,
		"gap" => egui::Color32::RED,
		"legacy" => egui::Color32::GRAY,
		_ => egui::Color32::YELLOW,
	}
}

fn accounting_delta(before: Option<&AccountingSnapshot>, after: Option<&AccountingSnapshot>) -> Option<String> {
	let before = before?;
	let after = after?;
	let mut parts = Vec::new();
	if let (Some(a), Some(b)) = (before.expected_usd, after.expected_usd) {
		parts.push(format!("expected_usd {a:.2} → {b:.2} ({:+.2})", b - a));
	}
	if let (Some(a), Some(b)) = (before.backing_sats, after.backing_sats) {
		parts.push(format!("backing {a} → {b} ({:+})", b as i128 - a as i128));
	}
	if let (Some(a), Some(b)) = (before.native_sats, after.native_sats) {
		parts.push(format!("native {a} → {b} ({:+})", b as i128 - a as i128));
	}
	(!parts.is_empty()).then(|| parts.join("  •  "))
}

fn history_jsonl(history: &ListChannelLedgerEventsResponse) -> String {
	history.events.iter().map(|event| {
		serde_json::json!({
			"ledger_id": event.id,
			"occurred_at_ms": event.occurred_at_ms,
			"event": event.event_type,
			"category": event.category,
			"severity": event.severity,
			"status": event.status,
			"source": event.source,
			"completeness": event.completeness,
			"refs": event.refs.iter().map(|r| serde_json::json!({"role": r.role, "value": r.value})).collect::<Vec<_>>(),
			"data": serde_json::from_str::<serde_json::Value>(&event.detail_json).unwrap_or_else(|_| serde_json::Value::String(event.detail_json.clone())),
		}).to_string()
	}).collect::<Vec<_>>().join("\n")
}

fn export_jsonl(history: &ListChannelLedgerEventsResponse, status: &mut Option<StatusMessage>) {
	let content = history_jsonl(history);
	#[cfg(not(target_arch = "wasm32"))]
	{
		if let Some(path) = rfd::FileDialog::new().set_file_name("channel-ledger.jsonl").save_file() {
			match std::fs::write(&path, content) {
				Ok(()) => *status = Some(StatusMessage::success(format!("Exported {}", path.display()))),
				Err(error) => *status = Some(StatusMessage::error(format!("Export failed: {error}"))),
			}
		}
	}
	#[cfg(target_arch = "wasm32")]
	{
		let _ = content;
		*status = Some(StatusMessage::error("JSONL download is not available in this web build"));
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn accounting_delta_reports_before_after() {
		let before = AccountingSnapshot { backing_sats: Some(10), native_sats: Some(5), ..Default::default() };
		let after = AccountingSnapshot { backing_sats: Some(12), native_sats: Some(3), ..Default::default() };
		let text = accounting_delta(Some(&before), Some(&after)).unwrap();
		assert!(text.contains("backing 10 → 12 (+2)"));
		assert!(text.contains("native 5 → 3 (-2)"));
	}

	#[test]
	fn jsonl_export_preserves_timeline_order_and_raw_detail() {
		let event = |id| ChannelLedgerEvent { id, event_type: format!("E{id}"), detail_json: format!("{{\"id\":{id}}}"), ..Default::default() };
		let jsonl = history_jsonl(&ListChannelLedgerEventsResponse { events: vec![event(1), event(2)], next_cursor: None });
		let lines = jsonl.lines().collect::<Vec<_>>();
		assert!(lines[0].contains("\"ledger_id\":1"));
		assert!(lines[1].contains("\"ledger_id\":2"));
		assert!(lines[0].contains("\"data\":{\"id\":1}"));
	}
}
