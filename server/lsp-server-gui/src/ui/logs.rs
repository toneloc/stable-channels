use dioxus::prelude::*;

use crate::state::{AppCtx, LogsTab};
use crate::ui::widgets::{Gate, SegBtn};

#[component]
pub fn Logs() -> Element {
	let ctx = use_context::<AppCtx>();
	if !ctx.is_connected() {
		return rsx! { Gate {} };
	}
	let active = ctx.nav.read().logs_tab;
	let mut nav = ctx.nav;
	rsx! {
		div { class: "seg", style: "align-self: flex-start;",
			SegBtn { active: active == LogsTab::Audit, onclick: move |_| nav.write().logs_tab = LogsTab::Audit, "Audit" }
			SegBtn { active: active == LogsTab::ChannelLedger, onclick: move |_| nav.write().logs_tab = LogsTab::ChannelLedger, "Channel History" }
			SegBtn { active: active == LogsTab::Ldk, onclick: move |_| nav.write().logs_tab = LogsTab::Ldk, "LDK server" }
		}
		match active {
			LogsTab::Audit => rsx! { crate::ui::audit_log::AuditLog {} },
			LogsTab::ChannelLedger => rsx! { crate::ui::channel_ledger::ChannelLedger {} },
			LogsTab::Ldk => rsx! { crate::ui::ldk_log::LdkLog {} },
		}
	}
}
