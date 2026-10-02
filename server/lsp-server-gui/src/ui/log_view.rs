use dioxus::prelude::*;

use crate::actions;
use crate::state::{AppCtx, LogViewState};
use crate::ui::widgets::{Check, Icon, TextInput};

#[derive(Clone, Copy, PartialEq)]
pub enum LogKind {
	Audit,
	Ldk,
}

fn state(ctx: AppCtx, kind: LogKind) -> LogViewState {
	let view = ctx.view.read();
	match kind {
		LogKind::Audit => view.audit_view.clone(),
		LogKind::Ldk => view.ldk_view.clone(),
	}
}

fn update(ctx: AppCtx, kind: LogKind, f: impl FnOnce(&mut LogViewState)) {
	let mut view = ctx.view;
	let mut view = view.write();
	match kind {
		LogKind::Audit => f(&mut view.audit_view),
		LogKind::Ldk => f(&mut view.ldk_view),
	}
}

/// Filter / Copy-all / Wrap / Follow-tail control row plus the scrollable monospace log.
#[component]
pub fn LogView(kind: LogKind, text: String) -> Element {
	let ctx = use_context::<AppCtx>();
	let s = state(ctx, kind);
	let display: String = if s.filter.is_empty() {
		text.clone()
	} else {
		text.lines().filter(|line| line.contains(&s.filter)).collect::<Vec<_>>().join("\n")
	};
	let dom_id = match kind {
		LogKind::Audit => "audit-log-view",
		LogKind::Ldk => "ldk-log-view",
	};
	let follow = s.follow;
	let length = display.len();
	// Stick to the bottom whenever the content changes while following the tail.
	use_effect(use_reactive!(|(length, follow)| {
		let _ = length;
		if follow {
			let _ = document::eval(&format!(
				"requestAnimationFrame(() => {{ const el = document.getElementById('{dom_id}'); if (el) el.scrollTop = el.scrollHeight; }});"
			));
		}
	}));
	rsx! {
		div { class: "row",
			div { class: "search",
				Icon { name: "search", size: 15 }
				TextInput { value: s.filter.clone(), small: true, placeholder: "Filter lines", oninput: move |v| update(ctx, kind, |s| s.filter = v) }
			}
			button { class: "btn sm", disabled: text.is_empty(), onclick: move |_| actions::copy(ctx, &text), Icon { name: "copy", size: 14 } "Copy all" }
			Check { checked: s.wrap, label: "Wrap", onchange: move |v| update(ctx, kind, |s| s.wrap = v) }
			Check { checked: s.follow, label: "Follow tail", onchange: move |v| update(ctx, kind, |s| s.follow = v) }
		}
		pre { id: dom_id, class: if s.wrap { "log wrap" } else { "log" }, tabindex: "0", "{display}" }
	}
}
