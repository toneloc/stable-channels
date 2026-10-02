use dioxus::prelude::*;
use sc_rest_client::sc_protos::stable::{AccountingSnapshot, ChannelLedgerEvent, ChannelLedgerOverview, LedgerRef};

use crate::actions;
use crate::ledger::{
	category_help, completeness_help, completeness_label, completeness_tone, decimal_delta, event_help,
	exact_timestamp, filter_choice_label, format_sats_with_usd, forwarding_path, human_summary,
	latest_state_caption, loaded_events_caption, relative_timestamp, sats_delta, snapshot_rows, status_help,
	status_tone, timeline_order, ForwardingLeg,
};
use crate::state::{AppCtx, ChannelLedgerForm, ChannelLedgerRequestKey, Op};
use crate::ui::widgets::{Empty, Hover, Icon, IdCopy, InfoTip, Pill, SegBtn, Spinner, Stat, TextInput};

const CATEGORIES: [&str; 10] =
	["channel", "payment", "forwarding", "trade", "stability", "peer", "sweep", "reconciliation", "operator", "system"];
const STATUSES: [&str; 6] = ["observed", "pending", "completed", "partial", "failed", "skipped"];
const COMPLETENESS: [&str; 4] = ["observed", "reconstructed", "legacy", "gap"];

/// Edit the ledger form; any change to the identifier or server filters discards loaded data.
fn edit_form(ctx: AppCtx, f: impl FnOnce(&mut ChannelLedgerForm)) {
	let mut forms = ctx.forms;
	let before = ChannelLedgerRequestKey::from(&forms.peek().channel_ledger);
	f(&mut forms.write().channel_ledger);
	if before != ChannelLedgerRequestKey::from(&forms.peek().channel_ledger) {
		// Never display or merge data fetched for the previous identifier/filter set.
		actions::invalidate_channel_ledger(ctx);
	}
}

#[component]
fn FilterSelect(label: &'static str, value: String, choices: Vec<&'static str>, onchange: EventHandler<String>) -> Element {
	rsx! {
		select {
			class: "select sm",
			style: "width: auto; min-width: 150px;",
			"aria-label": "{label}",
			onchange: move |e| onchange.call(e.value()),
			option { value: "", selected: value.is_empty(), "All {label}" }
			for choice in choices {
				option { key: "{choice}", value: "{choice}", selected: value == choice, "{filter_choice_label(label, choice)}" }
			}
		}
	}
}

#[component]
pub fn ChannelLedger() -> Element {
	let ctx = use_context::<AppCtx>();
	// The channel picker lists the node's channels; load them if this tab is opened first.
	use_hook(move || {
		if ctx.data.peek().channels.is_none() {
			actions::fetch_channels(ctx);
		}
	});
	let form = ctx.forms.read().channel_ledger.clone();
	let loading = ctx.busy(Op::ChannelLedger);
	let exporting = ctx.busy(Op::ChannelLedgerExport);
	let has_identifier = !form.identifier.trim().is_empty();
	let has_cursor = ctx.data.read().channel_ledger_cursor.is_some();
	let history = ctx.data.read().channel_ledger.clone();
	let newest_first = form.newest_first;
	let show_technical = form.show_technical;
	let selected = form.identifier.trim().to_owned();
	let choices: Vec<(String, String)> = {
		let data = ctx.data.read();
		data.channels
			.as_ref()
			.map(|list| {
				list.channels
					.iter()
					.map(|ch| {
						let peer = data
							.alias(&ch.counterparty_node_id)
							.unwrap_or_else(|| crate::format::truncate_id(&ch.counterparty_node_id, 8, 6));
						(ch.user_channel_id.clone(), format!("{peer} · {} sats", crate::format::format_sats(ch.channel_value_sats)))
					})
					.collect()
			})
			.unwrap_or_default()
	};
	rsx! {
		div { class: "card stack", style: "gap: 12px;",
			div { class: "row",
				select {
					class: "select sm",
					style: "width: auto; min-width: 260px;",
					"aria-label": "Channel",
					onchange: move |e| {
						let id = e.value();
						edit_form(ctx, |f| f.identifier = id);
						actions::fetch_channel_ledger(ctx, false);
					},
					option { value: "", selected: selected.is_empty(), "Choose a channel" }
					for (id, label) in choices {
						option { key: "{id}", value: "{id}", selected: selected == id, "{label}" }
					}
				}
				div { class: "search", style: "flex: 1 1 260px; max-width: 420px;",
					Icon { name: "search", size: 15 }
					TextInput { value: form.identifier.clone(), small: true, mono: true, placeholder: "or paste any channel, payment or transaction id", oninput: move |v| edit_form(ctx, |f| f.identifier = v) }
				}
				label { class: "check",
					input {
						r#type: "checkbox",
						checked: show_technical,
						onchange: move |e| {
							let on = e.checked();
							edit_form(ctx, |f| f.show_technical = on);
							actions::fetch_channel_ledger(ctx, false);
						},
					}
					"Show technical events"
				}
			}
			if show_technical {
				div { class: "row",
					FilterSelect { label: "Category", value: form.category.clone(), choices: CATEGORIES.to_vec(), onchange: move |v| edit_form(ctx, |f| f.category = v) }
					FilterSelect { label: "Status", value: form.status.clone(), choices: STATUSES.to_vec(), onchange: move |v| edit_form(ctx, |f| f.status = v) }
					FilterSelect { label: "Completeness", value: form.completeness.clone(), choices: COMPLETENESS.to_vec(), onchange: move |v| edit_form(ctx, |f| f.completeness = v) }
				}
			}
			div { class: "row",
				button {
					class: "btn sm primary",
					disabled: loading || !has_identifier,
					title: if has_identifier { "" } else { "Pick a channel or paste an identifier" },
					onclick: move |_| actions::fetch_channel_ledger(ctx, false),
					Icon { name: "refresh", size: 14 }
					"Refresh"
				}
				button { class: "btn sm", disabled: loading || !has_cursor, onclick: move |_| actions::fetch_channel_ledger(ctx, true),
					Icon { name: "chevron-down", size: 14 }
					"Load older"
				}
				button {
					class: "btn sm",
					disabled: exporting || !has_identifier,
					title: if has_identifier { "Export every page for this channel as JSONL" } else { "Pick a channel or paste an identifier" },
					onclick: move |_| actions::export_channel_ledger(ctx),
					Icon { name: "download", size: 14 }
					"Export JSONL"
				}
				div { class: "seg",
					SegBtn { active: newest_first, onclick: move |_| edit_form(ctx, |f| f.newest_first = true), "Newest first" }
					SegBtn { active: !newest_first, onclick: move |_| edit_form(ctx, |f| f.newest_first = false), "Oldest first" }
				}
				if loading || exporting {
					Spinner {}
				}
			}
		}
		match history {
			None => rsx! {
				div { class: "card",
					Empty { icon: "list", title: "Pick a channel", hint: "Its full history appears here, from opening until now." }
				}
			},
			Some(history) if !show_technical => rsx! {
				Timeline { events: history.events.clone(), overview: history.overview.clone(), channel: selected.clone(), newest_first }
			},
			Some(history) => {
				let caption = history.overview.as_ref().and_then(|overview| {
					loaded_events_caption(history.events.len(), overview.matching_events, history.next_cursor.is_some())
				});
				let order = timeline_order(&history.events, newest_first);
				rsx! {
					if let Some(overview) = history.overview.clone() {
						Overview { overview }
					}
					if let Some(caption) = caption {
						span { class: "small muted", "{caption}" }
					}
					if history.events.is_empty() {
						div { class: "card", Empty { icon: "list", title: "No ledger events match these exact filters." } }
					}
					for index in order {
						EventCard { key: "{history.events[index].id}", event: history.events[index].clone() }
					}
				}
			},
		}
	}
}

/// Summary of the channel the history belongs to.
#[component]
fn HistoryHeader(channel: String, overview: Option<ChannelLedgerOverview>, first_ms: Option<i64>, last_ms: Option<i64>) -> Element {
	let ctx = use_context::<AppCtx>();
	let data = ctx.data.read();
	let live = data.channels.as_ref().and_then(|list| list.channels.iter().find(|ch| ch.user_channel_id == channel).cloned());
	let peer = live.as_ref().map(|ch| {
		data.alias(&ch.counterparty_node_id).unwrap_or_else(|| crate::format::truncate_id(&ch.counterparty_node_id, 8, 6))
	});
	drop(data);
	// The ledger's own oldest row dates the history; the loaded page may start much later.
	let first_ms = overview.as_ref().and_then(|o| o.oldest_occurred_at_ms).or(first_ms);
	let latest = overview.and_then(|o| o.latest_accounting);
	let or_dash = |value: Option<String>| value.unwrap_or_else(|| "—".to_owned());
	let capacity = live
		.as_ref()
		.map(|ch| format!("{} sats capacity", crate::format::format_sats(ch.channel_value_sats)))
		.unwrap_or_default();
	let target = latest.as_ref().and_then(|s| s.expected_usd).map(crate::format::format_usd);
	let split = latest.as_ref().and_then(|s| {
		Some(format!("{} / {}", crate::format::format_sats(s.backing_sats?), crate::format::format_sats(s.native_sats?)))
	});
	let since = last_ms.map(|t| format!("last activity {}", relative_timestamp(t))).unwrap_or_default();
	rsx! {
		div { class: "grid-4",
			Stat { title: "Counterparty", value: or_dash(peer), sub: capacity }
			Stat { title: "Stable target", value: or_dash(target), sub: "Current recorded target" }
			Stat { title: "Backing / native", value: or_dash(split), sub: "sats" }
			Stat { title: "History since", value: or_dash(first_ms.map(crate::history::day_label)), sub: since }
		}
	}
}

/// Day-grouped, one line per business event; a row expands into its underlying ledger steps.
#[component]
fn Timeline(events: Vec<ChannelLedgerEvent>, overview: Option<ChannelLedgerOverview>, channel: String, newest_first: bool) -> Element {
	let mut open = use_signal(std::collections::HashSet::<i64>::new);
	let first_ms = events.iter().map(|e| e.occurred_at_ms).min();
	let last_ms = events.iter().map(|e| e.occurred_at_ms).max();
	let mut entries = crate::history::build_entries(&events, &channel);
	if newest_first {
		entries.reverse();
	}
	let mut days: Vec<(String, Vec<crate::history::HistoryEntry>)> = Vec::new();
	for entry in entries {
		let day = crate::history::day_label(entry.occurred_at_ms);
		match days.last_mut() {
			Some((current, list)) if *current == day => list.push(entry),
			_ => days.push((day, vec![entry])),
		}
	}
	rsx! {
		HistoryHeader { channel: channel.clone(), overview, first_ms, last_ms }
		if days.is_empty() {
			div { class: "card", Empty { icon: "list", title: "No history yet for this channel." } }
		}
		for (day, list) in days {
			section { key: "{day}", class: "card flush hist-day",
				div { class: "hist-day-head", "{day}" }
				for entry in list {
					div {
						key: "{entry.key}",
						class: if open.read().contains(&entry.key) { "hist-row open" } else { "hist-row" },
						onclick: move |_| {
							let key = entry.key;
							let mut set = open.write();
							if !set.remove(&key) {
								set.insert(key);
							}
						},
						span { class: "hist-time num", "{exact_timestamp(entry.occurred_at_ms)}" }
						span { class: "hist-summary",
							"{entry.summary}"
							if entry.repeats > 1 {
								span { class: "hist-repeats", " ×{entry.repeats} since {crate::history::day_label(entry.started_at_ms)}" }
							}
						}
						span { class: "hist-amount num",
							if let Some(msat) = entry.amount_msat.filter(|m| *m >= 1_000) {
								"{format_sats_with_usd(msat / 1_000, entry.btc_price)}"
							}
						}
						span {
							if let Some(target) = entry.target_after.filter(|_| entry.target_changed) {
								Pill { tone: "info", "target {crate::format::format_usd(target)}" }
							}
						}
					}
					if open.read().contains(&entry.key) {
						div { class: "hist-steps",
							for event in entry.events.clone() {
								EventCard { key: "{event.id}", event }
							}
						}
					}
				}
			}
		}
	}
}

#[component]
fn Overview(overview: ChannelLedgerOverview) -> Element {
	let state = overview.latest_accounting.clone();
	let sats = |value: Option<u64>, price: Option<f64>| {
		value.map(|value| format_sats_with_usd(value, price)).unwrap_or_else(|| "—".to_owned())
	};
	let price = state.as_ref().and_then(|s| s.btc_price);
	let expected = state
		.as_ref()
		.and_then(|s| s.expected_usd)
		.map(|value| format!("${value:.2}"))
		.unwrap_or_else(|| "—".to_owned());
	let same = overview.matching_events == overview.total_events;
	let events = if same {
		overview.total_events.to_string()
	} else {
		format!("{} / {}", overview.matching_events, overview.total_events)
	};
	let span = (overview.oldest_occurred_at_ms.is_some() || overview.newest_occurred_at_ms.is_some()).then(|| {
		format!(
			"{} -> {}",
			overview.oldest_occurred_at_ms.map(exact_timestamp).unwrap_or_else(|| "—".to_owned()),
			overview.newest_occurred_at_ms.map(exact_timestamp).unwrap_or_else(|| "—".to_owned())
		)
	});
	// Headline sats, with the "≈ $" part moved onto the caption line.
	let split = |text: String, caption: String| match text.split_once(" · ") {
		Some((value, usd)) => (value.to_owned(), format!("{usd} · {caption}")),
		None => (text, caption),
	};
	let (backing, backing_sub) = split(sats(state.as_ref().and_then(|s| s.backing_sats), price), "Stable allocation".to_owned());
	let (native, native_sub) = split(sats(state.as_ref().and_then(|s| s.native_sats), price), "Non-stable allocation".to_owned());
	let (live, live_sub) = split(sats(state.as_ref().and_then(|s| s.live_receiver_sats), price), latest_state_caption(&overview));
	rsx! {
		div { class: "grid-3",
			Stat { title: "Expected USD", value: expected, sub: "Current recorded target" }
			Stat { title: "Backing", value: backing, sub: backing_sub }
			Stat { title: "Native", value: native, sub: native_sub }
			Stat { title: "Live balance", value: live, sub: live_sub }
			Stat { title: "Events", value: events, sub: if same { "Exact identifier total" } else { "Matching current filters / total" } }
			Stat {
				title: "Coverage",
				value: format!("{} direct", overview.observed_events),
				sub: format!("{} reconstructed · {} legacy · {} gaps", overview.reconstructed_events, overview.legacy_events, overview.gap_events),
			}
		}
		if let Some(span) = span {
			div { class: "row small", style: "gap: 6px;", span { class: "muted", "Ledger span:" } span { class: "num", "{span}" } }
		}
	}
}

#[component]
fn EventCard(event: ChannelLedgerEvent) -> Element {
	let ctx = use_context::<AppCtx>();
	// Re-render every second so the relative time stays current.
	let _ = ctx.now.read();
	let pretty = serde_json::from_str::<serde_json::Value>(&event.detail_json)
		.and_then(|value| serde_json::to_string_pretty(&value))
		.unwrap_or_else(|_| event.detail_json.clone());
	let path = forwarding_path(&event);
	rsx! {
		article { class: "card event",
			div { class: "event-head",
				InfoTip { text: event_help(&event) }
				span { class: "event-title", "{human_summary(&event)}" }
				Hover { tip: category_help(&event.category), Pill { tone: "info", "{event.category}" } }
				Hover { tip: status_help(&event.status), Pill { tone: status_tone(&event.status), "{event.status}" } }
				Hover { tip: completeness_help(&event.completeness), Pill { tone: completeness_tone(&event.completeness), "{completeness_label(&event.completeness)}" } }
			}
			div { class: "row between small",
				span { class: "muted num", "{exact_timestamp(event.occurred_at_ms)}" }
				span { class: "muted", "{relative_timestamp(event.occurred_at_ms)}" }
			}
			Accounting { before: event.before.clone(), after: event.after.clone() }
			if let Some(path) = path {
				div { class: "grid-2", style: "gap: 12px;",
					Leg { title: "Incoming channel", help: "Payment arrived through this channel.", leg: path.incoming }
					Leg { title: "Outgoing channel", help: "Payment was forwarded through this channel.", leg: path.outgoing }
				}
			} else if !event.refs.is_empty() {
				References { refs: event.refs.clone() }
			}
			details { class: "disclosure",
				summary { Icon { name: "chevron-right", size: 14 } "Raw JSON" }
				div { class: "body", pre { class: "raw", "{pretty}" } }
			}
		}
	}
}

#[component]
fn Accounting(before: Option<AccountingSnapshot>, after: Option<AccountingSnapshot>) -> Element {
	match (before, after) {
		(Some(before), Some(after)) => {
			let sats = |value: Option<u64>, price: Option<f64>| value.map(|v| format_sats_with_usd(v, price));
			let usd = |value: Option<f64>| value.map(|v| format!("${v:.2}"));
			let rows = vec![
				("Expected USD", usd(before.expected_usd), usd(after.expected_usd), decimal_delta(before.expected_usd, after.expected_usd, "$")),
				("Backing", sats(before.backing_sats, before.btc_price), sats(after.backing_sats, after.btc_price), sats_delta(before.backing_sats, after.backing_sats)),
				("Native", sats(before.native_sats, before.btc_price), sats(after.native_sats, after.btc_price), sats_delta(before.native_sats, after.native_sats)),
				(
					"Live balance",
					sats(before.live_receiver_sats, before.btc_price),
					sats(after.live_receiver_sats, after.btc_price),
					sats_delta(before.live_receiver_sats, after.live_receiver_sats),
				),
			];
			let missing = || "Not recorded".to_owned();
			rsx! {
				div { class: "card inner stack tight",
					span { class: "field-label", "Balance change" }
					div { class: "change",
						for (label, before, after, delta) in rows {
							if before.is_some() || after.is_some() {
								span { key: "{label}", class: "change-label", "{label}" }
								match delta {
									// Unchanged values are shown once instead of "x -> x (+0)".
									Some((_, 0)) => rsx! {
										span { class: "change-same num", "{after.clone().unwrap_or_else(missing)}" }
										span { class: "delta flat", "unchanged" }
									},
									Some((text, sign)) => rsx! {
										span { class: "change-before num", "{before.clone().unwrap_or_else(missing)}" }
										span { class: "change-arrow", Icon { name: "arrow-right", size: 14 } }
										span { class: "change-after num", "{after.clone().unwrap_or_else(missing)}" }
										span { class: if sign > 0 { "delta up" } else { "delta down" }, "{text}" }
									},
									None => rsx! {
										span { class: "change-before num", "{before.clone().unwrap_or_else(missing)}" }
										span { class: "change-arrow", Icon { name: "arrow-right", size: 14 } }
										span { class: "change-after num", "{after.clone().unwrap_or_else(missing)}" }
										span {}
									},
								}
							}
						}
					}
				}
			}
		},
		(None, Some(after)) => rsx! { Snapshot { snapshot: after, title: None } },
		(Some(before), None) => rsx! { Snapshot { snapshot: before, title: "Previous recorded state" } },
		(None, None) => rsx! {},
	}
}

#[component]
fn Snapshot(snapshot: AccountingSnapshot, title: Option<String>) -> Element {
	let rows = snapshot_rows(&snapshot);
	rsx! {
		div { class: "card inner stack tight",
			if let Some(title) = title {
				span { class: "small", style: "color: var(--orange-text); font-weight: 600;", "{title}" }
			}
			div { class: "kv",
				for (label, value) in rows {
					div { class: "k small", "{label}" }
					div { class: "v", "{value}" }
				}
			}
		}
	}
}

#[component]
fn Leg(title: &'static str, help: &'static str, leg: ForwardingLeg) -> Element {
	rsx! {
		div { class: "card inner stack tight",
			Hover { tip: help.to_string(), span { class: "strong", "{title}" } }
			for (label, value) in [("Channel ID", leg.channel_id.clone()), ("User channel ID", leg.user_channel_id.clone()), ("Node ID", leg.node_id.clone())] {
				if let Some(value) = value {
					div { key: "{label}", class: "row", style: "gap: 6px;",
						span { class: "small muted", "{label}:" }
						IdCopy { value }
					}
				}
			}
		}
	}
}

#[component]
fn References(refs: Vec<LedgerRef>) -> Element {
	rsx! {
		div { class: "grid-3", style: "gap: 8px;",
			for (i, reference) in refs.into_iter().enumerate() {
				div { key: "{i}", class: "row nowrap", style: "gap: 6px; min-width: 0;",
					span { class: "small muted", "{reference.role}:" }
					IdCopy { value: reference.value.clone() }
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_balance_change_reads_as_before_to_after_without_raw_arrows() {
		let before = AccountingSnapshot { expected_usd: Some(43.63), backing_sats: Some(52_772), ..Default::default() };
		let after = AccountingSnapshot { expected_usd: Some(43.63), backing_sats: Some(52_444), ..Default::default() };
		let html = dioxus_ssr::render_element(rsx! { Accounting { before: Some(before), after: Some(after) } });
		assert!(!html.contains("-&gt;") && !html.contains("->"), "no text arrows: {html}");
		assert!(!html.contains("$+"), "no '$+' deltas");
		assert!(html.contains("unchanged"));
		assert!(html.contains("\u{2212}328 sats"));
	}
}
