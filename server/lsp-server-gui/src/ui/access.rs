//! Access page: make, list and revoke the daemon's tokens.

use chrono::{Datelike, NaiveDate, Timelike};
use dioxus::prelude::*;
use sc_rest_client::ldk_server_grpc::permissions::{
	ADMIN_PERMISSION, ALL_PERMISSIONS, INVOICE_PERMISSIONS, READONLY_PERMISSIONS,
};
use sc_rest_client::sc_protos::access::{TokenInfo, LIST_TOKENS_PATH};

use crate::actions;
use crate::state::{permission_for, AccessForm, AppCtx, Dialog, Expiry, Op, RevokeTokenForm};
use crate::ui::widgets::{
	Bubble, Card, Check, CopyBtn, Empty, Field, Gate, Group, Icon, Modal, NoAccess, Pill, RefreshBtn, SegBtn, Spinner, TextInput, Th,
};
use crate::ui::{close_dialog, open_dialog};

const HELP_NAME: &str = "1 to 64 letters, digits, '-' or '_'.";
/// Expiry shortcuts in days; 0 is never.
const EXPIRY_PRESETS: [(u32, &str); 5] = [(0, "Never"), (1, "1 day"), (7, "7 days"), (30, "30 days"), (365, "1 year")];
const WEEKDAYS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];

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

/// Unix seconds the token stops working at, 0 for never; a picked time must exist locally and be after `now`.
pub fn expiry_secs(expiry: &Expiry, now: u64) -> Result<u64, String> {
	use chrono::TimeZone;
	match *expiry {
		Expiry::Never => Ok(0),
		Expiry::Days(days) => Ok(now + u64::from(days) * 86_400),
		Expiry::At { day: None, .. } => Err("Pick the day the token expires".to_string()),
		Expiry::At { day: Some(day), hour, minute } => {
			let secs = day
				.and_hms_opt(hour, minute, 0)
				.and_then(|t| chrono::Local.from_local_datetime(&t).earliest())
				.ok_or_else(|| "A clock change skips that time. Pick another".to_string())?
				.timestamp();
			if secs <= now as i64 {
				return Err("That time has passed. Pick a later one".to_string());
			}
			Ok(secs as u64)
		},
	}
}

/// The line under the picker: when the token expires, or what is wrong with the choice (flagged true).
pub fn expiry_line(expiry: &Expiry, now: u64) -> (bool, String) {
	match expiry {
		Expiry::Never => (false, "Works until you revoke it".to_string()),
		Expiry::At { day: None, .. } => (false, "Pick a day on the calendar".to_string()),
		_ => match expiry_secs(expiry, now) {
			Ok(secs) => (false, format!("Expires {}", crate::format::local_datetime(secs))),
			Err(e) => (true, e),
		},
	}
}

/// One month of the calendar from Monday: blanks before the 1st and after the last day fill whole weeks.
pub fn month_cells(month: NaiveDate) -> Vec<Option<NaiveDate>> {
	let first = month.with_day(1).unwrap_or(month);
	let mut cells = vec![None; first.weekday().num_days_from_monday() as usize];
	cells.extend(first.iter_days().take_while(|d| d.month() == first.month()).map(Some));
	while cells.len() % 7 != 0 {
		cells.push(None);
	}
	cells
}

/// The first day of the month `months` away.
pub fn shift_month(month: NaiveDate, months: i32) -> NaiveDate {
	let first = month.with_day(1).unwrap_or(month);
	let step = chrono::Months::new(months.unsigned_abs());
	if months >= 0 { first.checked_add_months(step) } else { first.checked_sub_months(step) }.unwrap_or(first)
}

/// Opens the calendar on the day the current choice lands on, at the current time of day.
fn start_picking(form: &mut AccessForm) {
	if matches!(form.expiry, Expiry::At { .. }) {
		return;
	}
	let now = chrono::Local::now();
	let day = match form.expiry {
		Expiry::Days(days) => now.date_naive().checked_add_days(chrono::Days::new(u64::from(days))),
		_ => None,
	};
	form.calendar_month = Some(day.unwrap_or(now.date_naive()).with_day(1).unwrap_or(now.date_naive()));
	form.expiry = Expiry::At { day, hour: now.hour(), minute: now.minute() / 5 * 5 };
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
				Card { title: "New token: {created.name}", sub: "Use it to log in here or to call the LSP",
					div { class: "notice warning", Icon { name: "alert", size: 16 } "Copy it now. It will not be shown again." }
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
					Group { label: "Permissions",
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
					ExpiryPicker {}
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
							Empty { icon: "key", title: "Loading tokens…", Spinner { large: true } }
						} else {
							Empty { icon: "key", title: "No tokens loaded", hint: "Click Refresh to load" }
						}
					},
				}
			}
		}
	}
}

/// Expiry shortcuts, and a calendar with a time for any other moment.
#[component]
fn ExpiryPicker() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let (expiry, shown) = {
		let f = forms.read();
		(f.access.expiry.clone(), f.access.calendar_month)
	};
	let (is_error, line) = expiry_line(&expiry, crate::format::now_secs());
	let picking = matches!(expiry, Expiry::At { .. });
	rsx! {
		Group { label: "Expires",
			div { class: "seg", style: "align-self: flex-start;",
				for (days, label) in EXPIRY_PRESETS {
					SegBtn {
						key: "{days}",
						active: expiry == if days == 0 { Expiry::Never } else { Expiry::Days(days) },
						onclick: move |_| forms.write().access.expiry = if days == 0 { Expiry::Never } else { Expiry::Days(days) },
						"{label}"
					}
				}
				SegBtn { active: picking, onclick: move |_| start_picking(&mut forms.write().access), "Pick a date" }
			}
			if let Expiry::At { day, hour, minute } = expiry {
				Calendar { day, hour, minute, shown }
			}
			span { class: if is_error { "field-preview error" } else { "field-preview" }, "{line}" }
		}
	}
}

/// A month of days to pick from, days before today greyed out, and the time of day.
#[component]
fn Calendar(day: Option<NaiveDate>, hour: u32, minute: u32, shown: Option<NaiveDate>) -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let today = chrono::Local::now().date_naive();
	let this_month = shift_month(today, 0);
	let month = shown.map(|m| shift_month(m, 0)).unwrap_or(this_month);
	let mut set = move |day: Option<NaiveDate>, hour: u32, minute: u32| forms.write().access.expiry = Expiry::At { day, hour, minute };
	let title = month.format("%B %Y").to_string();
	rsx! {
		div { class: "cal",
			div { class: "cal-month",
				div { class: "cal-head",
					button {
						class: "icon-btn",
						r#type: "button",
						"aria-label": "Previous month",
						disabled: month <= this_month,
						onclick: move |_| forms.write().access.calendar_month = Some(shift_month(month, -1)),
						Icon { name: "chevron-left", size: 16 }
					}
					span { class: "cal-title", "{title}" }
					button {
						class: "icon-btn",
						r#type: "button",
						"aria-label": "Next month",
						onclick: move |_| forms.write().access.calendar_month = Some(shift_month(month, 1)),
						Icon { name: "chevron-right", size: 16 }
					}
				}
				div { class: "cal-grid",
					for name in WEEKDAYS {
						span { key: "{name}", class: "cal-wd", "{name}" }
					}
					for (i, cell) in month_cells(month).into_iter().enumerate() {
						match cell {
							None => rsx! { span { key: "{i}" } },
							Some(d) => {
								let mut class = String::from("cal-day");
								if Some(d) == day {
									class.push_str(" on");
								}
								if d == today {
									class.push_str(" today");
								}
								rsx! {
									button {
										key: "{i}",
										class: "{class}",
										r#type: "button",
										disabled: d < today,
										"aria-pressed": if Some(d) == day { "true" } else { "false" },
										onclick: move |_| set(Some(d), hour, minute),
										"{d.day()}"
									}
								}
							},
						}
					}
				}
			}
			// Grids, not <select>: a native dropdown list can open past the screen edge.
			div { class: "cal-time",
				span { class: "cal-label", "Hour" }
				div { class: "cal-slots", role: "group", "aria-label": "Hour",
					for h in 0..24u32 {
						button {
							key: "{h}",
							class: if h == hour { "cal-slot on" } else { "cal-slot" },
							r#type: "button",
							"aria-pressed": if h == hour { "true" } else { "false" },
							onclick: move |_| set(day, h, minute),
							"{h:02}"
						}
					}
				}
				span { class: "cal-label", "Minute" }
				div { class: "cal-slots", role: "group", "aria-label": "Minute",
					for m in (0..60u32).step_by(5) {
						button {
							key: "{m}",
							class: if m == minute { "cal-slot on" } else { "cal-slot" },
							r#type: "button",
							"aria-pressed": if m == minute { "true" } else { "false" },
							onclick: move |_| set(day, hour, m),
							":{m:02}"
						}
					}
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
					if token.used_by_daemon { Pill { tone: "warning", "used by the LSP" } }
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
				span { class: "small muted", "You are logged in with this token. Revoking it logs you out." }
			}
			if target.is_gui_token {
				span { class: "small muted", "The LSP writes a new GUI token file in its place." }
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
		for text in ["used by the LSP", "GUI token", ">you<", "node:read", "expires ", "ip = 10.0.0.1", "g1", "ops-viewer"] {
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
		assert!(html.contains("Copy it now. It will not be shown again."));
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

	fn day(y: i32, m: u32, d: u32) -> chrono::NaiveDate {
		chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()
	}

	#[test]
	fn an_expiry_is_never_some_days_after_creation_or_a_picked_future_time() {
		let now = 1_790_000_000;
		assert_eq!(expiry_secs(&Expiry::Never, now), Ok(0));
		assert_eq!(expiry_secs(&Expiry::Days(7), now), Ok(now + 7 * 86_400));
		assert!(expiry_secs(&Expiry::At { day: None, hour: 12, minute: 0 }, now).is_err(), "no day picked");
		assert!(expiry_secs(&Expiry::At { day: Some(day(2001, 1, 1)), hour: 0, minute: 0 }, now).is_err(), "past");
		let secs = expiry_secs(&Expiry::At { day: Some(day(2030, 6, 1)), hour: 12, minute: 30 }, now).unwrap();
		assert_eq!(crate::format::local_datetime(secs), "01 Jun 2030, 12:30");
	}

	#[test]
	fn the_expiry_line_says_when_the_token_stops_working() {
		let now = 1_790_000_000;
		assert_eq!(expiry_line(&Expiry::Never, now), (false, "Works until you revoke it".to_string()));
		assert_eq!(expiry_line(&Expiry::Days(1), now), (false, format!("Expires {}", crate::format::local_datetime(now + 86_400))));
		assert_eq!(expiry_line(&Expiry::At { day: None, hour: 9, minute: 0 }, now), (false, "Pick a day on the calendar".to_string()));
		let (error, text) = expiry_line(&Expiry::At { day: Some(day(2001, 1, 1)), hour: 9, minute: 0 }, now);
		assert!(error && text.contains("passed"), "{text}");
	}

	#[test]
	fn the_calendar_starts_on_monday_and_shows_every_day_of_the_month() {
		let nov = month_cells(day(2026, 11, 1));
		assert_eq!(nov.iter().take_while(|c| c.is_none()).count(), 6, "1 Nov 2026 is a Sunday");
		assert_eq!(nov.iter().flatten().count(), 30);
		assert_eq!(nov.iter().flatten().last(), Some(&day(2026, 11, 30)));
		assert_eq!(nov.len() % 7, 0, "whole weeks");
		let feb = month_cells(day(2027, 2, 1));
		assert_eq!(feb.first(), Some(&Some(day(2027, 2, 1))), "1 Feb 2027 is a Monday");
		assert_eq!(feb.len(), 28);
	}

	#[test]
	fn the_calendar_steps_across_years() {
		assert_eq!(shift_month(day(2026, 12, 1), 1), day(2027, 1, 1));
		assert_eq!(shift_month(day(2027, 1, 1), -1), day(2026, 12, 1));
		assert_eq!(shift_month(day(2026, 3, 1), 12), day(2027, 3, 1));
	}

	#[component]
	fn PickerHarness() -> Element {
		let ctx = use_context_provider(|| {
			AppCtx::new(Connection::signed_in("ops", &["admin"]), Forms::default(), Nav { active_tab: ActiveTab::Access, ..Default::default() }, Prefs::default(), false)
		});
		use_hook(move || {
			let mut forms = ctx.forms;
			let mut f = forms.write();
			f.access.expiry = Expiry::At { day: Some(day(2030, 6, 12)), hour: 9, minute: 30 };
			f.access.calendar_month = Some(day(2030, 6, 1));
		});
		rsx! { Access {} }
	}

	#[test]
	fn the_form_uses_its_own_calendar_and_no_browser_date_picker() {
		let mut dom = VirtualDom::new(PickerHarness);
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		assert!(!html.contains("datetime-local"));
		assert!(html.contains("June 2030"));
		assert_eq!(html.matches("class=\"cal-day").count(), 30, "every day of June");
		assert!(html.contains("Expires 12 Jun 2030, 09:30"), "{html}");
		for preset in ["Never", "1 day", "7 days", "30 days", "1 year", "Pick a date"] {
			assert!(html.contains(&format!(">{preset}<")), "missing {preset}");
		}
		assert!(!html.contains("<label class=\"field\"><span class=\"field-label\">Permissions"), "a click between the boxes must not tick the first one");
		assert!(!html.contains("<label class=\"field\"><span class=\"field-label\">Expires"), "a click on the calendar must not press the first preset");
	}

	#[test]
	fn the_time_is_picked_from_hour_and_minute_grids_not_browser_dropdowns() {
		let mut dom = VirtualDom::new(PickerHarness);
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		let cal = &html[html.find("class=\"cal\"").unwrap()..html.find("field-preview").unwrap()];
		assert!(!cal.contains("<select"), "a native dropdown opens past the screen edge");
		let slots = cal.matches("class=\"cal-slot\"").count() + cal.matches("class=\"cal-slot on\"").count();
		assert_eq!(slots, 24 + 12, "24 hours and 12 five-minute steps");
		assert_eq!(cal.matches("class=\"cal-slot on").count(), 2, "09 and 30 are marked");
	}

	#[test]
	fn restrictions_are_put_in_plain_words() {
		assert_eq!(restriction_text("permissions = a,b"), None);
		assert!(restriction_text("time-before = 1790000000").unwrap().starts_with("expires "));
		assert_eq!(restriction_text("ip = 10.0.0.1").as_deref(), Some("ip = 10.0.0.1"));
	}
}
