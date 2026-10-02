use dioxus::prelude::*;

use crate::actions;
use crate::state::{AppCtx, Op};
use crate::ui::log_view::{LogKind, LogView};
use crate::ui::widgets::{Empty, Icon, Spinner, TextInput};

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

/// "Lines:" field plus Refresh, shared by both log tabs.
#[component]
pub fn LogToolbar(lines: String, loading: bool, oninput: EventHandler<String>, onrefresh: EventHandler<MouseEvent>) -> Element {
	rsx! {
		div { class: "row",
			span { class: "field-label", "Lines" }
			div { style: "width: 90px;", TextInput { value: lines, small: true, oninput: move |v| oninput.call(v) } }
			button { class: "btn sm", disabled: loading, onclick: move |e| onrefresh.call(e),
				if loading { Spinner {} } else { Icon { name: "refresh", size: 14 } }
				"Refresh"
			}
		}
	}
}

#[component]
pub fn AuditLog() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let lines = forms.read().audit_log.max_lines.clone();
	let loading = ctx.busy(Op::AuditLog);
	let log = ctx.data.read().audit_log.as_ref().map(|r| r.content.clone());
	rsx! {
		div { class: "card stack", style: "gap: 14px;",
			LogToolbar { lines, loading, oninput: move |v| forms.write().audit_log.max_lines = v, onrefresh: move |_| actions::fetch_audit_log(ctx) }
			match log {
				Some(content) if content.is_empty() => rsx! { Empty { icon: "file", title: "No audit events yet." } },
				Some(content) => {
					let formatted = content.lines().map(format_audit_line).collect::<Vec<_>>().join("\n");
					rsx! { LogView { kind: LogKind::Audit, text: formatted } }
				},
				None => rsx! { Empty { icon: "file", title: "No audit log loaded", hint: "Click Refresh to load" } },
			}
		}
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
