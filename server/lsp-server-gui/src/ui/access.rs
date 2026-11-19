//! Access page: make, list and revoke the daemon's tokens.

use dioxus::prelude::*;
use sc_rest_client::ldk_server_grpc::permissions::{
	ADMIN_PERMISSION, ALL_PERMISSIONS, INVOICE_PERMISSIONS, READONLY_PERMISSIONS,
};
use sc_rest_client::sc_protos::access::{TokenInfo, LIST_TOKENS_PATH};

use crate::actions;
use crate::state::{permission_for, AccessForm, AppCtx, Dialog, Op, RevokeTokenForm};
use crate::ui::widgets::{Bubble, Card, Check, CopyBtn, Empty, Field, Gate, Icon, Modal, NoAccess, Pill, RefreshBtn, Spinner, TextInput, Th};
use crate::ui::{close_dialog, open_dialog};

const HELP_NAME: &str = "1 to 64 letters, digits, '-' or '_'.";
const HELP_EXPIRY: &str = "Local time after which the token stops working; leave empty for no expiry.";

/// Permissions a preset stands for; "custom" (or anything else) starts empty.
pub fn preset_permissions(preset: &str) -> Vec<String> {
	let list: &[&str] = match preset {
		"readonly" => &READONLY_PERMISSIONS,
		"invoice" => &INVOICE_PERMISSIONS,
		"admin" => &[ADMIN_PERMISSION],
		_ => &[],
	};
	list.iter().map(|p| p.to_string()).collect()
}

/// Tick or untick one permission; admin always stands alone.
pub fn toggle_permission(permissions: &mut Vec<String>, permission: &str, on: bool) {
	if !on {
		permissions.retain(|p| p != permission);
	} else if permission == ADMIN_PERMISSION {
		*permissions = vec![ADMIN_PERMISSION.to_string()];
	} else if !permissions.iter().any(|p| p == permission) {
		permissions.retain(|p| p != ADMIN_PERMISSION);
		permissions.push(permission.to_string());
	}
}

/// Unix seconds for a "YYYY-MM-DDTHH:MM" local time; empty means no expiry (0), a time not after `now` is an error.
pub fn expiry_secs(input: &str, now: u64) -> Result<u64, String> {
	use chrono::TimeZone;
	let input = input.trim();
	if input.is_empty() {
		return Ok(0);
	}
	let naive = chrono::NaiveDateTime::parse_from_str(input, "%Y-%m-%dT%H:%M")
		.or_else(|_| chrono::NaiveDateTime::parse_from_str(input, "%Y-%m-%dT%H:%M:%S"))
		.map_err(|_| "Expiry must be a date and time".to_string())?;
	let secs = chrono::Local
		.from_local_datetime(&naive)
		.earliest()
		.ok_or_else(|| "Expiry is not a valid local time".to_string())?
		.timestamp();
	if secs <= now as i64 {
		return Err("Expiry must be in the future".to_string());
	}
	Ok(secs as u64)
}

/// A caveat in words for the list; permissions are shown on their own, so they return None.
pub fn restriction_text(caveat: &str) -> Option<String> {
	match caveat.split_once('=').map(|(k, v)| (k.trim(), v.trim())) {
		Some(("permissions", _)) => None,
		Some(("time-before", v)) => match v.parse::<u64>() {
			Ok(ts) => Some(format!("expires {}", crate::format::local_datetime(ts))),
			Err(_) => Some(caveat.to_string()),
		},
		_ => Some(caveat.to_string()),
	}
}

/// Forget the token shown once.
pub fn dismiss_created(form: &mut AccessForm) {
	form.created = None;
}

#[component]
pub fn Access() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	// Leaving the page drops the token shown once.
	use_drop(move || {
		if let Ok(mut f) = forms.try_write() {
			dismiss_created(&mut f.access);
		}
	});
	use_effect(move || actions::fetch_tokens(ctx));
	if !ctx.is_connected() {
		return rsx! { Gate {} };
	}
	if !ctx.can(LIST_TOKENS_PATH) {
		return rsx! { Card { NoAccess { permission: permission_for(LIST_TOKENS_PATH) } } };
	}
	let form = forms.read().access.clone();
	let tokens: Option<Vec<TokenInfo>> = ctx.data.read().tokens.as_ref().map(|t| t.tokens.clone());
	let loading = ctx.busy(Op::ListTokens);
	let creating = ctx.busy(Op::CreateToken);
	rsx! {
		div { class: "stack",
			if let Some(created) = form.created.clone() {
				Card { title: "New token: {created.name}", sub: "Use it to log in to the GUI or call the daemon",
					div { class: "notice warning", Icon { name: "alert", size: 16 } "Copy it now: it will not be shown again" }
					div { class: "row nowrap", style: "margin-top: 10px;",
						span { class: "mono break", "{created.hex}" }
						CopyBtn { value: created.hex.clone(), label: "Copy token" }
					}
					div { class: "row", style: "margin-top: 10px;",
						button { class: "btn sm", onclick: move |_| dismiss_created(&mut forms.write().access), "I have copied it" }
					}
				}
			}
			Card { title: "Create token", sub: "A token limits what its holder can do",
				div { class: "stack", style: "gap: 14px;",
					Field { label: "Name", help: HELP_NAME,
						TextInput { value: form.name.clone(), placeholder: "ops-viewer", oninput: move |v| forms.write().access.name = v }
					}
					Field { label: "Preset",
						select {
							class: "select",
							value: form.preset.clone(),
							onchange: move |e| {
								let preset = e.value();
								let mut f = forms.write();
								f.access.permissions = preset_permissions(&preset);
								f.access.preset = preset;
							},
							option { value: "custom", selected: form.preset.is_empty() || form.preset == "custom", "Custom" }
							option { value: "readonly", selected: form.preset == "readonly", "Read-only" }
							option { value: "invoice", selected: form.preset == "invoice", "Invoice" }
							option { value: "admin", selected: form.preset == "admin", "Admin" }
						}
					}
					Field { label: "Permissions",
						div { class: "row", style: "gap: 6px 16px;",
							for permission in ALL_PERMISSIONS {
								Check {
									key: "{permission}",
									checked: form.permissions.iter().any(|p| p == permission),
									label: permission.to_string(),
									onchange: move |on| toggle_permission(&mut forms.write().access.permissions, permission, on),
								}
							}
						}
					}
					Field { label: "Expires", help: HELP_EXPIRY,
						input {
							class: "input",
							r#type: "datetime-local",
							value: form.expires.clone(),
							oninput: move |e| forms.write().access.expires = e.value(),
						}
					}
					div { class: "row",
						button { class: "btn primary", disabled: creating, onclick: move |_| actions::create_token(ctx),
							if creating { Spinner {} }
							"Create token"
						}
					}
				}
			}
			Card { class: "flush", title: "Tokens", actions: rsx! { RefreshBtn { busy: loading, onclick: move |_| actions::fetch_tokens(ctx), op: Op::ListTokens } },
				match tokens {
					Some(tokens) if !tokens.is_empty() => rsx! {
						div { class: "table-wrap",
							table { class: "table", style: "min-width: 720px;",
								thead {
									tr {
										Th { label: "Name" }
										Th { label: "Permissions" }
										Th { label: "Restrictions" }
										th { class: "right", "Actions" }
									}
								}
								tbody {
									for token in tokens {
										TokenRow { key: "{token.id}", token }
									}
								}
							}
						}
					},
					_ => rsx! {
						if loading {
							Empty { icon: "key", title: "Loading tokens...", Spinner { large: true } }
						} else {
							Empty { icon: "key", title: "No tokens loaded", hint: "Click Refresh to load" }
						}
					},
				}
			}
		}
	}
}

#[component]
fn TokenRow(token: TokenInfo) -> Element {
	let ctx = use_context::<AppCtx>();
	let revoking = ctx.busy(Op::RevokeToken);
	let restrictions: Vec<String> = token.caveats.iter().filter_map(|c| restriction_text(c)).collect();
	let can_revoke = !token.used_by_daemon;
	let permissions = token.permissions.join(", ");
	let (id, name, is_you, is_gui_token) = (token.id.clone(), token.name.clone(), token.is_you, token.is_gui_token);
	rsx! {
		tr {
			td {
				div { class: "strong", "{token.name}" }
				div { class: "small muted mono break", "{token.id}" }
				div { class: "row", style: "gap: 4px; margin-top: 4px;",
					if token.used_by_daemon { Pill { tone: "warning", "used by this daemon" } }
					if token.is_gui_token { Pill { tone: "info", "GUI token" } }
					if token.is_you { Pill { tone: "success", "you" } }
				}
			}
			td { span { class: "small", "{permissions}" } }
			td {
				if restrictions.is_empty() {
					span { class: "muted", "none" }
				}
				for text in restrictions {
					div { key: "{text}", class: "small", "{text}" }
				}
			}
			td { class: "right",
				if can_revoke {
					button {
						class: "btn sm danger",
						disabled: revoking,
						onclick: move |_| {
							let mut forms = ctx.forms;
							forms.write().access.revoke = RevokeTokenForm { id: id.clone(), name: name.clone(), is_you, is_gui_token };
							open_dialog(ctx, Dialog::RevokeToken);
						},
						"Revoke"
					}
				}
			}
		}
	}
}

#[component]
pub fn RevokeTokenDialog() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let target = forms.read().access.revoke.clone();
	let pending = ctx.busy(Op::RevokeToken);
	let mut cancel = move || {
		forms.write().access.revoke = Default::default();
		close_dialog(ctx);
	};
	rsx! {
		Modal {
			title: "Revoke token",
			sub: "The token stops working at once",
			icon: rsx! { Bubble { icon: "key", tone: "orange" } },
			onclose: move |_| cancel(),
			footer: rsx! {
				button { class: "btn ghost", onclick: move |_| cancel(), "Cancel" }
				button { class: "btn danger", disabled: pending, onclick: move |_| actions::revoke_token(ctx),
					if pending { Spinner {} }
					"Revoke token"
				}
			},
			p { "Revoke the token \"{target.name}\"?" }
			if target.is_you {
				span { class: "small muted", "This is the token you are logged in with: revoking it logs you out." }
			}
			if target.is_gui_token {
				span { class: "small muted", "The daemon writes a new GUI token file in its place." }
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use dioxus::prelude::*;
	use sc_rest_client::sc_protos::access::{ListTokensResponse, TokenInfo};

	use super::*;
	use crate::state::{ActiveTab, AppCtx, Connection, CreatedToken, Forms, Nav, Prefs};

	fn token(id: &str, name: &str, daemon: bool, gui: bool, you: bool) -> TokenInfo {
		TokenInfo {
			id: id.into(),
			name: name.into(),
			permissions: vec!["node:read".into()],
			caveats: vec!["time-before = 1790000000".into(), "ip = 10.0.0.1".into()],
			used_by_daemon: daemon,
			is_you: you,
			is_gui_token: gui,
		}
	}

	#[component]
	fn Harness(permissions: Vec<&'static str>, created: bool, tab: ActiveTab) -> Element {
		let ctx = use_context_provider(|| {
			AppCtx::new(Connection::signed_in("ops", &permissions), Forms::default(), Nav { active_tab: tab, ..Default::default() }, Prefs::default(), false)
		});
		use_hook(move || {
			let mut data = ctx.data;
			data.write().tokens = Some(ListTokensResponse {
				tokens: vec![
					token("d1", "sc-daemon", true, false, false),
					token("g1", "sc-gui-admin-1", false, true, false),
					token("m1", "me", false, false, true),
					token("o1", "ops-viewer", false, false, false),
				],
			});
			if created {
				let mut forms = ctx.forms;
				forms.write().access.created = Some(CreatedToken { name: "fresh".into(), hex: "deadbeef00".into() });
			}
		});
		rsx! { crate::ui::Sidebar {} Access {} }
	}

	fn render(permissions: &[&'static str], created: bool) -> String {
		let mut dom = VirtualDom::new_with_props(Harness, HarnessProps { permissions: permissions.to_vec(), created, tab: ActiveTab::Access });
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	#[test]
	fn the_access_tab_is_hidden_without_macaroons_manage() {
		let viewer = render(&["node:read"], false);
		assert!(!viewer.contains("<span>Access</span>"), "no sidebar entry");
		assert!(viewer.contains("needs macaroons:manage"), "a page that was reached anyway says what it needs");
		assert!(!viewer.contains("sc-daemon"));
		let manager = render(&["macaroons:manage"], false);
		assert!(manager.contains("<span>Access</span>"), "sidebar entry");
		assert!(manager.contains("sc-daemon"));
	}

	#[test]
	fn the_token_list_shows_tags_and_no_revoke_for_the_daemon_token() {
		let html = render(&["admin"], false);
		for text in ["used by this daemon", "GUI token", ">you<", "node:read", "expires ", "ip = 10.0.0.1", "g1", "ops-viewer"] {
			assert!(html.contains(text), "missing {text}");
		}
		assert!(!html.contains("time-before"), "the expiry is shown in words");
		assert_eq!(html.matches(">Revoke<").count(), 3, "every row but the daemon's");
	}

	thread_local! {
		static CTX: std::cell::Cell<Option<AppCtx>> = const { std::cell::Cell::new(None) };
	}

	fn shown_token(ctx: AppCtx) -> bool {
		ctx.forms.peek().access.created.is_some()
	}

	#[component]
	fn SessionEndHarness() -> Element {
		let ctx = use_context_provider(|| {
			AppCtx::new(Connection::signed_in("ops", &["admin"]), Forms::default(), Nav::default(), Prefs::default(), false)
		});
		CTX.with(|c| c.set(Some(ctx)));
		rsx! {}
	}

	#[test]
	fn the_new_token_is_gone_after_the_session_ends() {
		let mut dom = VirtualDom::new(SessionEndHarness);
		dom.rebuild_in_place();
		let ctx = CTX.with(|c| c.get()).unwrap();
		dom.in_scope(ScopeId::ROOT, || {
			let mut forms = ctx.forms;
			forms.write().access.created = Some(CreatedToken { name: "fresh".into(), hex: "deadbeef00".into() });
			assert!(shown_token(ctx));
			crate::actions::disconnect(ctx);
			assert!(!shown_token(ctx), "the token must not outlive its session");
		});
	}

	#[component]
	fn LeavingHarness() -> Element {
		let ctx = use_context_provider(|| {
			AppCtx::new(Connection::signed_in("ops", &["admin"]), Forms::default(), Nav { active_tab: ActiveTab::Access, ..Default::default() }, Prefs::default(), false)
		});
		CTX.with(|c| c.set(Some(ctx)));
		let tab = ctx.nav.read().active_tab;
		rsx! { if tab == ActiveTab::Access { Access {} } }
	}

	#[test]
	fn the_new_token_is_gone_after_leaving_the_access_page() {
		let mut dom = VirtualDom::new(LeavingHarness);
		dom.rebuild_in_place();
		let ctx = CTX.with(|c| c.get()).unwrap();
		dom.in_scope(ScopeId::ROOT, || {
			let mut forms = ctx.forms;
			forms.write().access.created = Some(CreatedToken { name: "fresh".into(), hex: "deadbeef00".into() });
			assert!(shown_token(ctx), "still shown while the page is open");
			let mut nav = ctx.nav;
			nav.write().active_tab = ActiveTab::Overview;
		});
		dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
		assert!(!shown_token(ctx), "leaving the page drops the token");
	}

	#[test]
	fn a_new_token_is_shown_once_with_a_warning() {
		let html = render(&["admin"], true);
		assert!(html.contains("deadbeef00") && html.contains("fresh"));
		assert!(html.contains("Copy it now: it will not be shown again"));
		assert!(!render(&["admin"], false).contains("will not be shown again"));
		let mut form = crate::state::AccessForm { created: Some(CreatedToken { name: "x".into(), hex: "secret".into() }), ..Default::default() };
		dismiss_created(&mut form);
		assert!(form.created.is_none());
	}

	#[test]
	fn presets_fill_the_permissions_and_admin_stays_alone() {
		use sc_rest_client::ldk_server_grpc::permissions::READONLY_PERMISSIONS;
		assert_eq!(preset_permissions("readonly"), READONLY_PERMISSIONS.iter().map(|p| p.to_string()).collect::<Vec<_>>());
		assert_eq!(preset_permissions("admin"), vec!["admin".to_string()]);
		assert!(preset_permissions("custom").is_empty());
		let mut perms = vec!["admin".to_string()];
		toggle_permission(&mut perms, "node:read", true);
		assert_eq!(perms, vec!["node:read".to_string()], "a specific permission replaces admin");
		toggle_permission(&mut perms, "admin", true);
		assert_eq!(perms, vec!["admin".to_string()], "admin replaces the rest");
		toggle_permission(&mut perms, "admin", false);
		assert!(perms.is_empty());
	}

	#[test]
	fn an_expiry_is_a_local_time_in_the_future_or_empty() {
		assert_eq!(expiry_secs("", 100), Ok(0));
		assert!(expiry_secs("nonsense", 100).is_err());
		assert!(expiry_secs("2001-01-01T00:00", 1_790_000_000).is_err(), "past");
		let secs = expiry_secs("2030-06-01T12:30", 1_790_000_000).unwrap();
		assert!(secs > 1_790_000_000);
	}

	#[test]
	fn restrictions_are_put_in_plain_words() {
		assert_eq!(restriction_text("permissions = a,b"), None);
		assert!(restriction_text("time-before = 1790000000").unwrap().starts_with("expires "));
		assert_eq!(restriction_text("ip = 10.0.0.1").as_deref(), Some("ip = 10.0.0.1"));
	}
}
