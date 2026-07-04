use eframe::egui;

use crate::app::LspServerApp;
use crate::ui::widgets;

/// Render one audit JSON line (`{ts,event,data}`) as a compact single line. Non-JSON returns unchanged.
pub fn format_audit_line(line: &str) -> String {
	let v: serde_json::Value = match serde_json::from_str(line) {
		Ok(v) => v,
		Err(_) => return line.to_string(),
	};
	let ts = v.get("ts").and_then(|t| t.as_str()).unwrap_or("");
	let event = v.get("event").and_then(|e| e.as_str()).unwrap_or("?");
	let data = v.get("data");

	let mut kv: Vec<String> = Vec::new();
	if let Some(obj) = data.and_then(|d| d.as_object()) {
		for (k, val) in obj {
			kv.push(format!("{}={}", k, compact_val(val)));
		}
	}
	kv.sort();

	let force = event == "CHANNEL_CLOSED"
		&& data
			.and_then(|d| d.get("reason_kind"))
			.and_then(|k| k.as_str())
			.map(|k| k.contains("FORCE_CLOSED"))
			.unwrap_or(false);
	let marker = if force { "⚠ " } else { "" };

	if kv.is_empty() {
		format!("{}  {}{}", ts, marker, event)
	} else {
		format!("{}  {}{}  {}", ts, marker, event, kv.join(" "))
	}
}

fn compact_val(v: &serde_json::Value) -> String {
	match v {
		serde_json::Value::String(s) => s.clone(),
		serde_json::Value::Null => "null".to_string(),
		other => other.to_string(),
	}
}

pub fn render(ui: &mut egui::Ui, app: &mut LspServerApp) {
	ui.heading("Audit Log");
	ui.add_space(5.0);

	ui.horizontal(|ui| {
		ui.label("Lines:");
		ui.add(egui::TextEdit::singleline(&mut app.state.forms.audit_log.max_lines).desired_width(80.0));
		let loading = app.state.tasks.audit_log.is_some();
		if ui.add_enabled(!loading, egui::Button::new("Refresh")).clicked() {
			app.fetch_audit_log();
		}
		if loading {
			ui.spinner();
		}
	});

	let formatted: String = app
		.state
		.audit_log
		.as_ref()
		.map(|r| r.content.lines().map(format_audit_line).collect::<Vec<_>>().join("\n"))
		.unwrap_or_default();

	let (filter, wrap, follow) = crate::ui::log_view::controls(ui, "audit_log", &formatted);

	ui.add_space(10.0);

	match &app.state.audit_log {
		Some(resp) if resp.content.is_empty() => {
			ui.label("Empty audit log.");
		},
		Some(_) => {
			let display: String = if filter.is_empty() {
				formatted.clone()
			} else {
				formatted.lines().filter(|line| line.contains(&filter)).collect::<Vec<_>>().join("\n")
			};
			crate::ui::log_view::text_area(ui, &display, wrap, follow);
		},
		None => {
			widgets::empty_state(ui, "📜", "No audit log loaded", "Click Refresh to load");
		},
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn force_close_line_has_marker_and_fields() {
		let line = r#"{"ts":"2026-06-29T14:12:48Z","event":"CHANNEL_CLOSED","data":{"channel_id":"5a9c","closure_initiator":"REMOTE","reason_kind":"COUNTERPARTY_FORCE_CLOSED"}}"#;
		let out = format_audit_line(line);
		assert!(out.contains("⚠"));
		assert!(out.contains("CHANNEL_CLOSED"));
		assert!(out.contains("closure_initiator=REMOTE"));
		assert!(out.contains("reason_kind=COUNTERPARTY_FORCE_CLOSED"));
	}

	#[test]
	fn plain_event_has_no_marker() {
		let line = r#"{"ts":"t","event":"TRADE_APPLIED","data":{"expected_usd":44.38}}"#;
		let out = format_audit_line(line);
		assert!(out.contains("TRADE_APPLIED"));
		assert!(out.contains("expected_usd=44.38"));
		assert!(!out.contains("⚠"));
	}

	#[test]
	fn non_json_passes_through() {
		assert_eq!(format_audit_line("not json at all"), "not json at all");
	}

	#[test]
	fn empty_data_has_no_trailing_separator() {
		let line = r#"{"ts":"t","event":"TRADE_PARSE_PAYLOAD_FAILED","data":{}}"#;
		let out = format_audit_line(line);
		assert_eq!(out, "t  TRADE_PARSE_PAYLOAD_FAILED");
	}
}
