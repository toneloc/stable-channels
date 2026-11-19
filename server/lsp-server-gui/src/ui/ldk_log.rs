use dioxus::prelude::*;

use crate::actions;
use sc_rest_client::sc_protos::stable::LDK_LOG_PATH;

use crate::state::{permission_for, AppCtx, Op};
use crate::ui::audit_log::LogToolbar;
use crate::ui::log_view::{LogKind, LogView};
use crate::ui::widgets::{Empty, Icon, NoAccess};

#[component]
pub fn LdkLog() -> Element {
	let ctx = use_context::<AppCtx>();
	if !ctx.can(LDK_LOG_PATH) {
		return rsx! { div { class: "card", NoAccess { permission: permission_for(LDK_LOG_PATH) } } };
	}
	let mut forms = ctx.forms;
	let lines = forms.read().ldk_log.max_lines.clone();
	let loading = ctx.busy(Op::LdkLog);
	let log = ctx.data.read().ldk_log.as_ref().map(|r| r.content.clone());
	rsx! {
		div { class: "card stack", style: "gap: 14px;",
			LogToolbar { lines, loading, oninput: move |v| forms.write().ldk_log.max_lines = v, onrefresh: move |_| actions::fetch_ldk_log(ctx) }
			match log {
				Some(content) if content.is_empty() => rsx! {
					Empty { icon: "file", title: "Empty response.",
						div { class: "notice", style: "text-align: left;",
							Icon { name: "info", size: 16 }
							em { "LDK Server may not have `[log] file = \"...\"` set in its config. Uncomment that line and restart LDK Server, then refresh." }
						}
					}
				},
				Some(content) => rsx! { LogView { kind: LogKind::Ldk, text: content } },
				None => rsx! { Empty { icon: "file", title: "No log loaded", hint: "Click Refresh to load" } },
			}
		}
	}
}
