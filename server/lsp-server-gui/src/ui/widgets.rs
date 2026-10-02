//! Reusable building blocks for every screen.

use dioxus::prelude::*;

use crate::format::truncate_id;
use crate::format::format_sats;
use crate::state::{ActiveTab, AppCtx, ConnectionStatus, Op, Tooltip};

/// Stroke icon (24px grid, currentColor).
#[component]
pub fn Icon(name: &'static str, #[props(default = 18)] size: u32) -> Element {
	let body = match name {
		"home" => rsx! { path { d: "M3 10.5 12 3l9 7.5V20a1 1 0 0 1-1 1h-5v-6H9v6H4a1 1 0 0 1-1-1z" } },
		"coins" => rsx! {
			circle { cx: "8", cy: "8", r: "6" }
			path { d: "M18.09 10.37A6 6 0 1 1 10.34 18" }
			path { d: "M7 6h1v4" }
			path { d: "m16.71 13.88.7.71-2.82 2.82" }
		},
		"wallet" => rsx! {
			path { d: "M20 12V8H6a2 2 0 0 1-2-2c0-1.1.9-2 2-2h12v4" }
			path { d: "M4 6v12c0 1.1.9 2 2 2h14v-4" }
			path { d: "M18 12a2 2 0 0 0 0 4h4v-4z" }
		},
		"link" => rsx! {
			path { d: "M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" }
			path { d: "M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" }
		},
		"users" => rsx! {
			path { d: "M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" }
			circle { cx: "9", cy: "7", r: "4" }
			path { d: "M22 21v-2a4 4 0 0 0-3-3.87" }
			path { d: "M16 3.13a4 4 0 0 1 0 7.75" }
		},
		"receipt" => rsx! {
			path { d: "M4 2v20l2-1 2 1 2-1 2 1 2-1 2 1 2-1 2 1V2l-2 1-2-1-2 1-2-1-2 1-2-1-2 1z" }
			path { d: "M16 8h-6" }
			path { d: "M16 12H8" }
			path { d: "M13 16H8" }
		},
		"forward" => rsx! {
			path { d: "M15 17l5-5-5-5" }
			path { d: "M4 18v-2a4 4 0 0 1 4-4h12" }
		},
		"zap" => rsx! { path { d: "M13 2 3 14h9l-1 8 10-12h-9l1-8z" } },
		"cube" => rsx! {
			path { d: "M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" }
			path { d: "M3.3 7 12 12l8.7-5" }
			path { d: "M12 22V12" }
		},
		"shield" => rsx! {
			path { d: "M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" }
			path { d: "m9 12 2 2 4-4" }
		},
		"wrench" => rsx! {
			path { d: "M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" }
		},
		"graph" => rsx! {
			circle { cx: "18", cy: "5", r: "3" }
			circle { cx: "6", cy: "12", r: "3" }
			circle { cx: "18", cy: "19", r: "3" }
			path { d: "m8.59 13.51 6.83 3.98" }
			path { d: "m15.41 6.51-6.82 3.98" }
		},
		"file" => rsx! {
			path { d: "M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5z" }
			path { d: "M14 2v6h6" }
			path { d: "M16 13H8" }
			path { d: "M16 17H8" }
			path { d: "M10 9H8" }
		},
		"sliders" => rsx! {
			path { d: "M21 4h-7" }
			path { d: "M10 4H3" }
			path { d: "M21 12h-9" }
			path { d: "M8 12H3" }
			path { d: "M21 20h-5" }
			path { d: "M12 20H3" }
			path { d: "M14 2v4" }
			path { d: "M8 10v4" }
			path { d: "M16 18v4" }
		},
		"copy" => rsx! {
			rect { x: "8", y: "8", width: "14", height: "14", rx: "2" }
			path { d: "M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" }
		},
		"refresh" => rsx! {
			path { d: "M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" }
			path { d: "M21 3v5h-5" }
			path { d: "M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" }
			path { d: "M8 16H3v5" }
		},
		"plus" => rsx! {
			path { d: "M12 5v14" }
			path { d: "M5 12h14" }
		},
		"arrow-up" => rsx! {
			path { d: "M12 19V5" }
			path { d: "m5 12 7-7 7 7" }
		},
		"arrow-down" => rsx! {
			path { d: "M12 5v14" }
			path { d: "m19 12-7 7-7-7" }
		},
		"arrow-right" => rsx! {
			path { d: "M5 12h14" }
			path { d: "m12 5 7 7-7 7" }
		},
		"arrow-up-right" => rsx! {
			path { d: "M7 17 17 7" }
			path { d: "M7 7h10v10" }
		},
		"arrow-down-left" => rsx! {
			path { d: "M17 7 7 17" }
			path { d: "M17 17H7V7" }
		},
		"x" => rsx! {
			path { d: "M18 6 6 18" }
			path { d: "m6 6 12 12" }
		},
		"info" => rsx! {
			circle { cx: "12", cy: "12", r: "10" }
			path { d: "M12 16v-4" }
			path { d: "M12 8h.01" }
		},
		"chevron-right" => rsx! { path { d: "m9 18 6-6-6-6" } },
		"chevron-up" => rsx! { path { d: "m18 15-6-6-6 6" } },
		"chevron-down" => rsx! { path { d: "m6 9 6 6 6-6" } },
		"more" => rsx! {
			circle { cx: "12", cy: "5", r: "1" }
			circle { cx: "12", cy: "12", r: "1" }
			circle { cx: "12", cy: "19", r: "1" }
		},
		"search" => rsx! {
			circle { cx: "11", cy: "11", r: "8" }
			path { d: "m21 21-4.3-4.3" }
		},
		"external" => rsx! {
			path { d: "M15 3h6v6" }
			path { d: "M10 14 21 3" }
			path { d: "M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" }
		},
		"plug" => rsx! {
			path { d: "M12 22v-5" }
			path { d: "M9 8V2" }
			path { d: "M15 8V2" }
			path { d: "M18 8v5a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4V8z" }
		},
		"alert" => rsx! {
			path { d: "m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3" }
			path { d: "M12 9v4" }
			path { d: "M12 17h.01" }
		},
		"check" => rsx! { path { d: "M20 6 9 17l-5-5" } },
		"download" => rsx! {
			path { d: "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" }
			path { d: "m7 10 5 5 5-5" }
			path { d: "M12 15V3" }
		},
		"upload" => rsx! {
			path { d: "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" }
			path { d: "m17 8-5-5-5 5" }
			path { d: "M12 3v12" }
		},
		"key" => rsx! {
			circle { cx: "7.5", cy: "15.5", r: "5.5" }
			path { d: "m21 2-9.6 9.6" }
			path { d: "m15.5 7.5 3 3L22 7l-3-3" }
		},
		"edit" => rsx! { path { d: "M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5z" } },
		"power" => rsx! {
			path { d: "M18.36 6.64a9 9 0 1 1-12.73 0" }
			path { d: "M12 2v10" }
		},
		"list" => rsx! {
			path { d: "M8 6h13" }
			path { d: "M8 12h13" }
			path { d: "M8 18h13" }
			path { d: "M3 6h.01" }
			path { d: "M3 12h.01" }
			path { d: "M3 18h.01" }
		},
		"sun" => rsx! {
			circle { cx: "12", cy: "12", r: "4" }
			path { d: "M12 2v2" }
			path { d: "M12 20v2" }
			path { d: "m4.93 4.93 1.41 1.41" }
			path { d: "m17.66 17.66 1.41 1.41" }
			path { d: "M2 12h2" }
			path { d: "M20 12h2" }
			path { d: "m6.34 17.66-1.41 1.41" }
			path { d: "m19.07 4.93-1.41 1.41" }
		},
		"moon" => rsx! { path { d: "M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9z" } },
		"monitor" => rsx! {
			rect { x: "2", y: "3", width: "20", height: "14", rx: "2" }
			path { d: "M8 21h8" }
			path { d: "M12 17v4" }
		},
		"pen" => rsx! {
			path { d: "M12 20h9" }
			path { d: "M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4z" }
		},
		"split" => rsx! {
			path { d: "M16 3h5v5" }
			path { d: "M8 3H3v5" }
			path { d: "M12 22v-8.3a4 4 0 0 0-1.17-2.83L3 3" }
			path { d: "m15 9 6-6" }
		},
		"merge" => rsx! {
			path { d: "m8 6 4-4 4 4" }
			path { d: "M12 2v10.3a4 4 0 0 1-1.17 2.83L4 22" }
			path { d: "m20 22-5-5" }
		},
		"activity" => rsx! { path { d: "M22 12h-4l-3 9L9 3l-3 9H2" } },
		"csv" => rsx! {
			path { d: "M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5z" }
			path { d: "M14 2v6h6" }
			path { d: "M8 13h8" }
			path { d: "M8 17h8" }
			path { d: "M12 11v8" }
		},
		"clock" => rsx! {
			circle { cx: "12", cy: "12", r: "10" }
			path { d: "M12 6v6l4 2" }
		},
		_ => rsx! { circle { cx: "12", cy: "12", r: "9" } },
	};
	rsx! {
		svg {
			class: "icon",
			width: "{size}",
			height: "{size}",
			view_box: "0 0 24 24",
			fill: "none",
			stroke: "currentColor",
			stroke_width: "2",
			stroke_linecap: "round",
			stroke_linejoin: "round",
			"aria-hidden": "true",
			{body}
		}
	}
}

/// Stable Channels mark: a balance scale holding a bitcoin coin (vector trace of photos/sc-icon-egui.png).
#[component]
pub fn BrandMark(#[props(default = 34)] height: u32) -> Element {
	rsx! {
		svg {
			class: "brand-mark",
			height: "{height}",
			view_box: "160 236 720 552",
			fill: "none",
			stroke: "currentColor",
			stroke_linecap: "round",
			stroke_linejoin: "round",
			role: "img",
			"aria-label": "Stable Channels",
			g { stroke_width: "30",
				circle { cx: "512", cy: "283", r: "34", fill: "currentColor", stroke: "none" }
				path { d: "M512 300V770M330 770H694M268 360H636M268 360L185 520M268 360L353 520M185 520H355M185 520A85 64 0 0 0 355 520" }
				circle { cx: "748", cy: "362", r: "113" }
			}
			path {
				stroke_width: "18",
				d: "M716 314H758A23 23 0 0 1 758 360H730M730 360H763A29 29 0 0 1 763 418H716M730 314V418M740 294V314M758 294V314M740 418V438M758 418V438",
			}
		}
	}
}

/// Tinted circle with a status icon.
#[component]
pub fn Bubble(icon: &'static str, tone: &'static str, #[props(default = false)] small: bool) -> Element {
	let class = if small { format!("bubble sm {tone}") } else { format!("bubble {tone}") };
	rsx! {
		span { class: "{class}", Icon { name: icon, size: if small { 14 } else { 16 } } }
	}
}

/// Small "i" icon that shows `text` in the floating tooltip layer.
#[component]
pub fn InfoTip(text: String) -> Element {
	let ctx = use_context::<AppCtx>();
	let enter_text = text.clone();
	rsx! {
		span {
			class: "tip-icon",
			tabindex: "0",
			role: "img",
			"aria-label": "{text}",
			onmouseenter: move |e| show_tip(ctx, enter_text.clone(), e),
			onmousemove: move |e| move_tip(ctx, e),
			onmouseleave: move |_| hide_tip(ctx),
			Icon { name: "info", size: 14 }
		}
	}
}

/// Wraps content so hovering it shows `tip` (e.g. exact values behind formatted ones).
#[component]
pub fn Hover(tip: String, children: Element) -> Element {
	let ctx = use_context::<AppCtx>();
	let enter_tip = tip.clone();
	rsx! {
		span {
			onmouseenter: move |e| show_tip(ctx, enter_tip.clone(), e),
			onmousemove: move |e| move_tip(ctx, e),
			onmouseleave: move |_| hide_tip(ctx),
			{children}
		}
	}
}

fn show_tip(ctx: AppCtx, text: String, e: MouseEvent) {
	let point = e.client_coordinates();
	let mut tooltip = ctx.tooltip;
	tooltip.set(Some(Tooltip { text, x: point.x, y: point.y }));
}

fn move_tip(ctx: AppCtx, e: MouseEvent) {
	let point = e.client_coordinates();
	let mut tooltip = ctx.tooltip;
	tooltip.with_mut(|tip| {
		if let Some(tip) = tip.as_mut() {
			tip.x = point.x;
			tip.y = point.y;
		}
	});
}

fn hide_tip(ctx: AppCtx) {
	let mut tooltip = ctx.tooltip;
	tooltip.set(None);
}

/// Floating tooltip layer rendered once at the root.
#[component]
pub fn TooltipLayer() -> Element {
	let ctx = use_context::<AppCtx>();
	let tip = ctx.tooltip.read().clone();
	match tip {
		Some(tip) => rsx! {
			div {
				class: "tooltip",
				style: "left: min({tip.x + 14.0}px, calc(100vw - 316px)); top: {tip.y + 18.0}px;",
				"{tip.text}"
			}
		},
		None => rsx! {},
	}
}

#[component]
pub fn Card(
	title: Option<String>, sub: Option<String>, help: Option<String>, actions: Option<Element>,
	icon: Option<Element>, #[props(default)] class: String, children: Element,
) -> Element {
	let has_head = title.is_some() || actions.is_some();
	rsx! {
		section { class: "card {class}",
			if has_head {
				div { class: "card-head",
					if let Some(icon) = icon {
						div { class: "card-icon", {icon} }
					}
					div { class: "stack tight", style: "gap: 2px;",
						if let Some(title) = title {
							div { class: "row nowrap", style: "gap: 6px;",
								span { class: "card-title", "{title}" }
								if let Some(help) = help {
									InfoTip { text: help }
								}
							}
						}
						if let Some(sub) = sub {
							span { class: "card-sub", "{sub}" }
						}
					}
					if let Some(actions) = actions {
						div { class: "card-actions", {actions} }
					}
				}
			}
			{children}
		}
	}
}

#[component]
pub fn Stat(
	title: String, help: Option<String>, value: String, #[props(default)] sub: String,
	onclick: Option<EventHandler<MouseEvent>>,
) -> Element {
	let clickable = onclick.is_some();
	rsx! {
		div {
			class: if clickable { "card stat clickable" } else { "card stat" },
			role: clickable.then_some("button"),
			onclick: move |e| {
				if let Some(handler) = onclick.as_ref() {
					handler.call(e);
				}
			},
			div { class: "stat-title",
				"{title}"
				if let Some(help) = help {
					InfoTip { text: help }
				}
			}
			div { class: "stat-value", "{value}" }
			if !sub.is_empty() {
				div { class: "stat-sub", "{sub}" }
			}
		}
	}
}

#[component]
pub fn Pill(tone: &'static str, children: Element) -> Element {
	rsx! { span { class: "pill {tone}", {children} } }
}

/// Key/value row inside a `div.kv` grid.
#[component]
pub fn Kv(label: String, help: Option<String>, children: Element) -> Element {
	rsx! {
		div { class: "k",
			"{label}"
			if let Some(help) = help {
				InfoTip { text: help }
			}
		}
		div { class: "v", {children} }
	}
}

/// Icon button that copies `value` and confirms in the toast.
#[component]
pub fn CopyBtn(value: String, #[props(default = "Copy")] label: &'static str) -> Element {
	let ctx = use_context::<AppCtx>();
	rsx! {
		button {
			class: "icon-btn",
			title: "{label}",
			"aria-label": "{label}",
			// Copying inside a clickable row must not also open the row.
			onclick: move |e| {
				e.stop_propagation();
				crate::actions::copy(ctx, &value);
			},
			Icon { name: "copy", size: 14 }
		}
	}
}

/// Monospace truncated id with the full value on hover and a copy button.
#[component]
pub fn IdCopy(value: String, #[props(default = 8)] head: usize, #[props(default = 8)] tail: usize) -> Element {
	let short = truncate_id(&value, head, tail);
	rsx! {
		span { class: "id",
			Hover { tip: value.clone(), span { class: "mono", "{short}" } }
			CopyBtn { value: value.clone() }
		}
	}
}

#[component]
pub fn Spinner(#[props(default = false)] large: bool) -> Element {
	rsx! { span { class: if large { "spinner lg" } else { "spinner" }, role: "status" } }
}

#[component]
pub fn Loading(label: String) -> Element {
	rsx! {
		span { class: "loading", Spinner {} "{label}" }
	}
}

#[component]
pub fn Empty(icon: &'static str, title: String, #[props(default)] hint: String, children: Element) -> Element {
	rsx! {
		div { class: "empty",
			div { class: "glyph", Icon { name: icon, size: 24 } }
			div { class: "title", "{title}" }
			if !hint.is_empty() {
				div { class: "hint", "{hint}" }
			}
			{children}
		}
	}
}

/// Shared "not connected" view: Open Settings (and Retry after an error).
#[component]
pub fn Gate() -> Element {
	let ctx = use_context::<AppCtx>();
	let status = ctx.conn.read().status.clone();
	let server_url = ctx.conn.read().server_url.clone();
	let open_settings = move |_| {
		let mut nav = ctx.nav;
		nav.write().active_tab = ActiveTab::Settings;
	};
	rsx! {
		div { class: "card gate",
			match status {
				ConnectionStatus::Error(e) => rsx! {
					div { class: "empty",
						div { class: "glyph danger", Icon { name: "alert", size: 24 } }
						div { class: "title", "Can't reach the LSP at {server_url}" }
						div { class: "hint break", "{e}" }
						div { class: "hint", "Make sure the daemon is running." }
						div { class: "row", style: "margin-top: 10px; justify-content: center;",
							button { class: "btn", onclick: open_settings, Icon { name: "sliders", size: 16 } "Open Settings" }
							button { class: "btn primary", onclick: move |_| crate::actions::connect(ctx), Icon { name: "refresh", size: 16 } "Retry" }
						}
					}
				},
				_ => rsx! {
					div { class: "empty",
						div { class: "glyph", Icon { name: "plug", size: 24 } }
						div { class: "title", "Not connected to an LSP" }
						div { class: "hint", "Open Settings to configure the connection." }
						div { class: "row", style: "margin-top: 10px; justify-content: center;",
							button { class: "btn primary", onclick: open_settings, Icon { name: "sliders", size: 16 } "Open Settings" }
						}
					}
				},
			}
		}
	}
}

/// Labelled form control with optional help icon, hint and live preview.
#[component]
pub fn Field(
	label: String, help: Option<String>, #[props(default)] hint: String, preview: Option<String>,
	children: Element,
) -> Element {
	rsx! {
		label { class: "field",
			span { class: "field-label",
				"{label}"
				if let Some(help) = help {
					InfoTip { text: help }
				}
			}
			{children}
			if let Some(preview) = preview {
				span { class: "field-preview", "{preview}" }
			}
			if !hint.is_empty() {
				span { class: "field-hint", "{hint}" }
			}
		}
	}
}

#[component]
pub fn TextInput(
	value: String, oninput: EventHandler<String>, #[props(default)] placeholder: String,
	#[props(default = false)] mono: bool, #[props(default = false)] disabled: bool,
	#[props(default = false)] password: bool, #[props(default = false)] small: bool,
) -> Element {
	let mut class = String::from("input");
	if mono {
		class.push_str(" mono");
	}
	if small {
		class.push_str(" sm");
	}
	rsx! {
		input {
			class: "{class}",
			r#type: if password { "password" } else { "text" },
			value: "{value}",
			placeholder: "{placeholder}",
			disabled,
			spellcheck: "false",
			autocomplete: "off",
			oninput: move |e| oninput.call(e.value()),
		}
	}
}

#[component]
pub fn TextArea(
	value: String, oninput: Option<EventHandler<String>>, #[props(default = 3)] rows: u32,
	#[props(default)] placeholder: String, #[props(default = true)] mono: bool,
) -> Element {
	let readonly = oninput.is_none();
	rsx! {
		textarea {
			class: if mono { "textarea mono" } else { "textarea" },
			rows: "{rows}",
			placeholder: "{placeholder}",
			readonly,
			spellcheck: "false",
			value: "{value}",
			oninput: move |e| {
				if let Some(handler) = oninput {
					handler.call(e.value());
				}
			},
		}
	}
}

#[component]
pub fn Check(
	checked: bool, onchange: EventHandler<bool>, label: String, #[props(default = false)] danger: bool,
	help: Option<String>,
) -> Element {
	rsx! {
		label { class: if danger { "check danger" } else { "check" },
			input { r#type: "checkbox", checked, onchange: move |e| onchange.call(e.checked()) }
			span { "{label}" }
			if let Some(help) = help {
				InfoTip { text: help }
			}
		}
	}
}

/// Segmented-control button.
#[component]
pub fn SegBtn(active: bool, onclick: EventHandler<MouseEvent>, children: Element) -> Element {
	rsx! {
		button {
			class: if active { "on" } else { "" },
			"aria-pressed": if active { "true" } else { "false" },
			onclick: move |e| onclick.call(e),
			{children}
		}
	}
}

/// Modal dialog; Escape and the backdrop both invoke `onclose`.
#[component]
pub fn Modal(
	title: String, sub: Option<String>, icon: Option<Element>, onclose: EventHandler<()>,
	#[props(default = false)] wide: bool, footer: Option<Element>, children: Element,
) -> Element {
	rsx! {
		div {
			class: "modal-backdrop",
			onclick: move |_| onclose.call(()),
			onkeydown: move |e| {
				if e.key() == Key::Escape {
					onclose.call(());
				}
			},
			div {
				class: if wide { "modal wide" } else { "modal" },
				role: "dialog",
				"aria-modal": "true",
				"aria-label": "{title}",
				tabindex: "-1",
				onmounted: move |e| async move {
					let _ = e.set_focus(true).await;
				},
				onclick: move |e| e.stop_propagation(),
				div { class: "modal-head",
					if let Some(icon) = icon {
						{icon}
					}
					div { class: "stack tight", style: "gap: 2px;",
						h2 { "{title}" }
						if let Some(sub) = sub {
							span { class: "sub", "{sub}" }
						}
					}
					button { class: "icon-btn", "aria-label": "Close", onclick: move |_| onclose.call(()), Icon { name: "x", size: 18 } }
				}
				div { class: "modal-body", {children} }
				if let Some(footer) = footer {
					div { class: "modal-foot", {footer} }
				}
			}
		}
	}
}

/// Sortable table header button with direction arrow.
#[component]
pub fn SortTh(
	label: String, active: bool, descending: bool, onclick: EventHandler<MouseEvent>, help: Option<String>,
	#[props(default)] class: String,
) -> Element {
	let button_class = if active { "sort on" } else { "sort" };
	let arrow = if descending { "chevron-down" } else { "chevron-up" };
	rsx! {
		th { class: "{class}",
			span { class: "th",
				button { class: button_class, onclick: move |e| onclick.call(e),
					"{label}"
					if active {
						Icon { name: arrow, size: 12 }
					}
				}
				if let Some(help) = help {
					InfoTip { text: help }
				}
			}
		}
	}
}

/// Plain table header with optional help.
#[component]
pub fn Th(label: String, help: Option<String>, #[props(default)] class: String) -> Element {
	rsx! {
		th { class: "{class}",
			span { class: "th",
				"{label}"
				if let Some(help) = help {
					InfoTip { text: help }
				}
			}
		}
	}
}

/// Amber-outbound over blue-inbound liquidity bar.
#[component]
pub fn LiquidityBar(frac: f64) -> Element {
	let pct = (frac.clamp(0.0, 1.0) * 100.0).round();
	rsx! {
		div { class: "liq", div { class: "out", style: "width: {pct}%;" } }
	}
}

/// Refresh button that turns into a spinner while `busy`.
#[component]
pub fn RefreshBtn(
	busy: bool, onclick: EventHandler<MouseEvent>, #[props(default = "Refresh")] label: &'static str, op: Option<Op>,
) -> Element {
	rsx! {
		if let Some(op) = op {
			Updated { op }
		}
		button { class: "btn sm", disabled: busy, onclick: move |e| onclick.call(e),
			if busy {
				Spinner {}
			} else {
				Icon { name: "refresh", size: 14 }
			}
			"{label}"
		}
	}
}

/// Last-result row (label, truncated id, copy).
#[component]
pub fn LastId(label: String, help: Option<String>, value: String, #[props(default = 8)] keep: usize) -> Element {
	rsx! {
		div { class: "row", style: "gap: 8px;",
			span { class: "muted small", "{label}" }
			if let Some(help) = help {
				InfoTip { text: help }
			}
			IdCopy { value, head: keep, tail: keep }
		}
	}
}

/// "Updated 2m ago" for the last successful response of `op`.
#[component]
pub fn Updated(op: Op) -> Element {
	let ctx = use_context::<AppCtx>();
	let now = *ctx.now.read();
	let at = ctx.data.read().fetched_at.get(&op).copied();
	match at {
		Some(at) => rsx! {
			Hover { tip: crate::format::local_datetime(at),
				span { class: "updated", "Updated {crate::format::ago(at, now)}" }
			}
		},
		None => rsx! {},
	}
}

/// Node shown by its gossip alias (when public) with the truncated pubkey underneath.
#[component]
pub fn Peer(node_id: String, #[props(default = 6)] keep: usize) -> Element {
	let ctx = use_context::<AppCtx>();
	let alias = ctx.data.read().alias(&node_id);
	rsx! {
		span { class: "peer",
			match alias {
				Some(alias) => rsx! {
					span { class: "peer-alias", title: "{alias}", "{alias}" }
					IdCopy { value: node_id.clone(), head: keep, tail: keep }
				},
				None => rsx! { IdCopy { value: node_id.clone(), head: keep, tail: keep } },
			}
		}
	}
}

/// Amount in the display unit with a secondary line (sats, or ≈USD when showing sats).
#[component]
pub fn Amount(msat: u64, #[props(default = false)] strong: bool) -> Element {
	let ctx = use_context::<AppCtx>();
	let primary = ctx.fmt_msat(msat);
	let unit = ctx.unit();
	let price = ctx.price_value();
	let secondary = match unit {
		crate::state::DisplayUnit::Sats => price
			.filter(|p| *p > 0.0)
			.map(|_| format!("≈ {}", crate::format::format_amount_msat(msat, crate::state::DisplayUnit::Usd, price))),
		_ => Some(format!("{} sats", format_sats(msat / 1000))),
	};
	rsx! {
		span { class: "amount",
			span { class: if strong { "num strong" } else { "num" }, "{primary}" }
			if let Some(secondary) = secondary {
				span { class: "amount-sub num", "{secondary}" }
			}
		}
	}
}

/// Absolute local date with the relative age underneath.
#[component]
pub fn When(ts: u64) -> Element {
	let ctx = use_context::<AppCtx>();
	let now = *ctx.now.read();
	rsx! {
		Hover { tip: format!("unix: {ts}"),
			span { class: "amount",
				span { class: "num", "{crate::format::local_datetime(ts)}" }
				span { class: "amount-sub", "{crate::format::relative_short(ts, now)}" }
			}
		}
	}
}

/// Right-hand side panel with the full record behind a table row; Escape and the backdrop close it.
#[component]
pub fn SidePanel(
	title: String, sub: Option<String>, icon: Option<Element>, onclose: EventHandler<()>, footer: Option<Element>,
	children: Element,
) -> Element {
	rsx! {
		div {
			class: "drawer-backdrop",
			onclick: move |_| onclose.call(()),
			onkeydown: move |e| {
				if e.key() == Key::Escape {
					onclose.call(());
				}
			},
			aside {
				class: "drawer",
				role: "dialog",
				"aria-label": "{title}",
				tabindex: "-1",
				onmounted: move |e| async move {
					let _ = e.set_focus(true).await;
				},
				onclick: move |e| e.stop_propagation(),
				div { class: "modal-head",
					if let Some(icon) = icon {
						{icon}
					}
					div { class: "stack tight", style: "gap: 2px; min-width: 0;",
						h2 { "{title}" }
						if let Some(sub) = sub {
							span { class: "sub mono", "{sub}" }
						}
					}
					button { class: "icon-btn", "aria-label": "Close", onclick: move |_| onclose.call(()), Icon { name: "x", size: 18 } }
				}
				div { class: "modal-body", {children} }
				if let Some(footer) = footer {
					div { class: "modal-foot", {footer} }
				}
			}
		}
	}
}
