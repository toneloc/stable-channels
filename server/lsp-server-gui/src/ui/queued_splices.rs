//! Queued splice-ins page: the LSP splice-ins queued for the phone, with Cancel and the guarded Release.

use dioxus::prelude::*;

use sc_rest_client::sc_protos::stable::QueuedSpliceInfo;

use crate::actions;
use crate::format::ago;
use crate::state::{AppCtx, Op, QueuedRelease};
use crate::ui::widgets::{Amount, Card, Check, Empty, Gate, Icon, Peer, Pill, RefreshBtn, Spinner, Th};

const HELP_QUEUED_SPLICES: &str = "Splice-ins queued for the phone. A call the node has not answered stays pending: no second call and no new request go out until it locks, the node drops it, or you release it after checking the node.";
const RELEASE_WARNING: &str = "Release clears the call the node has not answered, so a new request can go out. If the node is still working on the earlier splice-in, both can land: the LSP's funds go into the channel twice, with a second on-chain fee. Release only after checking on the node that it holds no splice for this channel. The tick covers this call only.";

/// What each status means, in the order a splice-in moves through them; a collapsed legend at the bottom of the queue card.
const STATUS_GUIDE: [(&str, &str); 8] = [
	("queued", "Not tried yet; the phone is pushed within seconds. If it cannot be, the Note says why."),
	("waking", "Phone pushed; the call goes out once it is online. No call within 60 s fails the try."),
	("splicing", "The node was asked to splice."),
	("negotiated", "Signed; locks once confirmed and the phone is online."),
	("locked", "Done: the funds are in the channel."),
	("failed", "Last try failed; the next ones follow gaps of about 3, 10, 20 and 40 min, then hourly."),
	("stalled", "Needs a look (the Note says why): not signed 10 min after the call, not locked 3 h after signing, no try possible for a day (such as no push token), or the node could not say whether an earlier call landed. No new call while a call is pending."),
	("cancelled", "No more tries. While pending on node, the call already made can still lock."),
];
const PENDING_GUIDE: &str = "A call is out and has not locked, been refused or been dropped; it may never have reached the node. No new call or request until then, or until you release it after checking the node.";
const FAILED_COUNT_GUIDE: &str = "Tries that failed so far.";
const NOTE_GUIDE: &str = "The Note column says why a row is waiting or what went wrong.";

/// Whether a queued splice-in is still under way: open, or with a call pending on the node. The sidebar lists this page, and the Channels list dots the channel, only while one is.
pub fn splice_in_live(row: &QueuedSpliceInfo) -> bool {
	row.pending_on_node || !matches!(row.status.as_str(), "locked" | "cancelled")
}

/// The pill tone for a queued splice's status.
fn queued_splice_tone(status: &str) -> &'static str {
	match status {
		"locked" => "success",
		"failed" | "stalled" => "danger",
		"cancelled" => "muted",
		_ => "info",
	}
}

/// Whether a queued splice belongs on the page: live (a call pending on the node keeps its Release button here), or finished within the last day.
fn queued_splice_is_shown(row: &QueuedSpliceInfo, now: u64) -> bool {
	splice_in_live(row) || now.saturating_sub(row.updated_at.max(0) as u64) < 86_400
}

/// The row an open Release confirmation belongs to, with its tick; None once that row no longer has the confirmed call pending on the node.
fn armed_release(release: Option<&QueuedRelease>, rows: &[QueuedSpliceInfo]) -> Option<(QueuedSpliceInfo, bool)> {
	let release = release?;
	rows.iter().find(|row| release.is_for(row)).map(|row| (row.clone(), release.confirmed))
}

/// The row's note: its last error, and why the phone could not be reached when a later push failed, unless the error already says so.
fn queued_splice_note(row: &QueuedSpliceInfo) -> String {
	let wake_note = if row.last_error.contains(row.wake_note.as_str()) { "" } else { row.wake_note.as_str() };
	[row.last_error.as_str(), wake_note].into_iter().filter(|text| !text.is_empty()).collect::<Vec<_>>().join(" · ")
}

#[component]
pub fn QueuedSplices() -> Element {
	let ctx = use_context::<AppCtx>();
	if !ctx.is_connected() {
		return rsx! { Gate {} };
	}
	let now = *ctx.now.read();
	let busy = ctx.busy(Op::CancelQueuedSplice);
	let loading = ctx.busy(Op::ListQueuedSplices);
	let data = ctx.data.read();
	let listed = data.queued_splices.is_some();
	let rows: Vec<QueuedSpliceInfo> = data
		.queued_splices
		.as_ref()
		.map(|q| q.splices.iter().filter(|row| queued_splice_is_shown(row, now)).cloned().collect())
		.unwrap_or_default();
	drop(data);
	let mut view = ctx.view;
	let armed = armed_release(view.read().queued_release.as_ref(), &rows);
	rsx! {
		Card { title: "Queued splice-ins", help: HELP_QUEUED_SPLICES, class: "flush",
			actions: rsx! { RefreshBtn { busy: loading, onclick: move |_| actions::fetch_queued_splices(ctx), op: Op::ListQueuedSplices } },
			if rows.is_empty() {
				if loading && !listed {
					Empty { icon: "merge", title: "Loading splice-ins...", Spinner { large: true } }
				} else {
					Empty { icon: "merge", title: "No splice-ins in progress", hint: "Splice+ on a channel queues one." }
				}
			} else {
				div { class: "table-wrap",
					table { class: "table",
						thead {
							tr {
								Th { label: "Peer" }
								Th { label: "Amount", class: "right" }
								Th { label: "Status" }
								Th { label: "Note" }
								Th { label: "Updated" }
								th { "" }
							}
						}
						tbody {
							for row in rows {
								{
									let uid = row.user_channel_id.clone();
									let release_row = row.clone();
									let tone = queued_splice_tone(&row.status);
									let note = queued_splice_note(&row);
									let open = !matches!(row.status.as_str(), "locked" | "cancelled");
									let updated = ago(row.updated_at.max(0) as u64, now);
									rsx! {
										tr { key: "{row.user_channel_id}",
											td { Peer { node_id: row.counterparty_node_id.clone() } }
											td { class: "right", Amount { msat: row.amount_sats.saturating_mul(1_000) } }
											td {
												Pill { tone, "{row.status}" }
												if row.attempts > 0 { " · " span { class: "muted", "{row.attempts} failed" } }
												if row.pending_on_node { " · " Pill { tone: "warning", "pending on node" } }
											}
											td { class: "muted", style: "max-width: 420px; white-space: normal;", "{note}" }
											td { class: "muted", "{updated}" }
											td { class: "right",
												if open {
													button { class: "btn sm ghost", disabled: busy, onclick: move |_| actions::cancel_queued_splice(ctx, uid.clone()), "Cancel" }
												}
												if row.pending_on_node {
													" "
													// Opens the confirmation below; nothing is sent from here.
													button { class: "btn sm danger", disabled: busy, title: "Only after checking that the node holds no splice for this channel",
														onclick: move |_| view.write().open_release(&release_row), "Release" }
												}
											}
										}
									}
								}
							}
						}
					}
				}
			}
			if let Some((row, confirmed)) = armed {
				ReleaseConfirm { row, confirmed }
			}
			StatusGuide {}
		}
	}
}

/// The legend at the bottom of the queue card, collapsed until opened: what each status, the pending marker and the failed count mean.
#[component]
fn StatusGuide() -> Element {
	rsx! {
		details { class: "disclosure status-guide",
			summary { Icon { name: "chevron-right", size: 14 } "What the statuses mean" }
			div { class: "body status-guide-grid",
				for (status, meaning) in STATUS_GUIDE {
					div { class: "status-guide-item", key: "{status}",
						Pill { tone: queued_splice_tone(status), "{status}" }
						span { class: "small muted", "{meaning}" }
					}
				}
				div { class: "status-guide-item",
					Pill { tone: "warning", "pending on node" }
					span { class: "small muted", "{PENDING_GUIDE}" }
				}
				div { class: "status-guide-item",
					span { class: "small muted", style: "white-space: nowrap;", "2 failed" }
					span { class: "small muted", "{FAILED_COUNT_GUIDE}" }
				}
				div { class: "status-guide-item",
					span { class: "small muted", "{NOTE_GUIDE}" }
				}
			}
		}
	}
}

/// The forced cancel behind a tick, like force close: it drops the queue's guard against a second splice on this channel.
#[component]
fn ReleaseConfirm(row: QueuedSpliceInfo, confirmed: bool) -> Element {
	let ctx = use_context::<AppCtx>();
	let mut view = ctx.view;
	let busy = ctx.busy(Op::CancelQueuedSplice);
	let called = ago(row.last_attempt_at.max(0) as u64, *ctx.now.read());
	let tick_row = row.clone();
	rsx! {
		div { class: "confirm-box", style: "margin: 12px;",
			div { class: "title", Icon { name: "alert", size: 16 } "Release the splice-in pending on the node" }
			div { class: "row",
				Peer { node_id: row.counterparty_node_id.clone() }
				Amount { msat: row.amount_sats.saturating_mul(1_000) }
				span { class: "small muted", "Call made {called}" }
			}
			if !row.funding_txo.is_empty() {
				span { class: "small muted", "The node reported this call's splice as signed: {row.funding_txo}" }
			}
			span { class: "small muted", "{RELEASE_WARNING}" }
			Check { checked: confirmed, danger: true, label: "I checked the node: it holds no splice for this channel",
				onchange: move |v| view.write().tick_release(&tick_row, v) }
			div { class: "row",
				button { class: "btn ghost", onclick: move |_| view.write().close_release(), "Keep waiting" }
				button { class: "btn danger solid", disabled: !confirmed || busy, onclick: move |_| actions::release_queued_splice(ctx), "Release" }
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use sc_rest_client::sc_protos::stable::ListQueuedSplicesResponse;

	fn queued_row(status: &str, pending_on_node: bool, updated_at: i64) -> QueuedSpliceInfo {
		QueuedSpliceInfo {
			user_channel_id: "7".into(),
			counterparty_node_id: "02aa".into(),
			amount_sats: 50_000,
			status: status.into(),
			pending_on_node,
			updated_at,
			created_at: 1_000,
			last_attempt_at: 2_000,
			..Default::default()
		}
	}

	#[test]
	fn a_splice_in_is_live_while_open_or_pending_on_the_node() {
		for status in ["queued", "waking", "splicing", "negotiated", "failed", "stalled"] {
			assert!(splice_in_live(&queued_row(status, false, 0)), "{status}");
		}
		for status in ["locked", "cancelled"] {
			assert!(!splice_in_live(&queued_row(status, false, 0)), "{status}");
		}
		assert!(splice_in_live(&queued_row("cancelled", true, 0)), "a Cancel leaves the call pending on the node");
	}

	#[test]
	fn a_finished_row_drops_off_after_a_day_but_a_call_pending_on_the_node_stays() {
		let now = 10_000_000u64;
		let ago = |secs: u64| (now - secs) as i64;
		for status in ["queued", "waking", "splicing", "negotiated", "failed", "stalled"] {
			assert!(queued_splice_is_shown(&queued_row(status, false, ago(30 * 86_400)), now), "{status} is open");
		}
		for status in ["locked", "cancelled"] {
			assert!(queued_splice_is_shown(&queued_row(status, false, ago(86_399)), now), "{status} within the day");
			assert!(!queued_splice_is_shown(&queued_row(status, false, ago(86_400)), now), "{status} after the day");
		}
		// A plain Cancel on a pending call: nothing touches the row again until the node answers or it is released.
		assert!(queued_splice_is_shown(&queued_row("cancelled", true, ago(30 * 86_400)), now));
	}

	#[test]
	fn queued_splice_pills_read_by_status() {
		assert_eq!(queued_splice_tone("locked"), "success");
		assert_eq!(queued_splice_tone("failed"), "danger");
		assert_eq!(queued_splice_tone("stalled"), "danger");
		assert_eq!(queued_splice_tone("cancelled"), "muted");
		for status in ["queued", "waking", "splicing", "negotiated"] {
			assert_eq!(queued_splice_tone(status), "info", "{status}");
		}
	}

	#[component]
	fn PageHarness(rows: Option<Vec<QueuedSpliceInfo>>, release: Option<QueuedRelease>) -> Element {
		use crate::state::{Connection, ConnectionStatus, Forms, Nav, Prefs};
		let ctx = use_context_provider(|| {
			let mut conn = Connection::new();
			conn.status = ConnectionStatus::Connected;
			AppCtx::new(conn, Forms::default(), Nav::default(), Prefs::default(), false)
		});
		use_hook(move || {
			let mut data = ctx.data;
			data.write().queued_splices = rows.clone().map(|splices| ListQueuedSplicesResponse { splices });
			let mut view = ctx.view;
			view.write().queued_release = release.clone();
		});
		rsx! { QueuedSplices {} }
	}

	fn page_html(row: QueuedSpliceInfo) -> String {
		page_html_with(row, None)
	}

	fn page_html_with(row: QueuedSpliceInfo, release: Option<QueuedRelease>) -> String {
		render_page(Some(vec![row]), release)
	}

	/// The queue card only: the status guide under it names every status, so row checks stop where it starts.
	fn render_page(rows: Option<Vec<QueuedSpliceInfo>>, release: Option<QueuedRelease>) -> String {
		let html = render_page_full(rows, release);
		match html.find("status-guide") {
			Some(at) => html[..at].to_string(),
			None => html,
		}
	}

	fn render_page_full(rows: Option<Vec<QueuedSpliceInfo>>, release: Option<QueuedRelease>) -> String {
		let mut dom = VirtualDom::new_with_props(PageHarness, PageHarnessProps { rows, release });
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	#[test]
	fn the_page_explains_every_status_under_the_queue() {
		let now = crate::format::now_secs() as i64;
		let html = render_page_full(Some(vec![queued_row("queued", false, now)]), None);
		let guide = html.find("status-guide").expect("the status guide is on the page");
		assert!(html.find(">Cancel<").expect("the queue row") < guide, "the guide comes after the queue");
		let guide = &html[guide..];
		let opening = &html[html[..html.find("status-guide").unwrap()].rfind('<').unwrap()..];
		assert!(opening.starts_with("<details") && !opening[..opening.find('>').unwrap()].contains("open"), "a collapsed disclosure, not a card");
		assert!(guide.contains("What the statuses mean") && guide.contains(NOTE_GUIDE));
		for (status, meaning) in STATUS_GUIDE {
			assert!(guide.contains(&format!(">{status}<")) && guide.contains(meaning), "{status}");
		}
		assert!(guide.contains(">pending on node<") && guide.contains(PENDING_GUIDE));
		assert!(guide.contains(">2 failed<") && guide.contains(FAILED_COUNT_GUIDE));
		let empty = render_page_full(Some(vec![]), None);
		assert!(empty.contains("No splice-ins in progress") && empty.contains("What the statuses mean"), "shown with nothing in progress too");
	}

	#[test]
	fn a_cancelled_call_still_pending_on_the_node_keeps_its_release_button_after_a_day() {
		let two_days_ago = crate::format::now_secs() as i64 - 2 * 86_400;
		let html = page_html(queued_row("cancelled", true, two_days_ago));
		assert!(html.contains("Queued splice-ins"));
		assert!(html.contains("pending on node") && html.contains(">Release<"), "Release offered");
		let released = page_html(queued_row("cancelled", false, two_days_ago));
		assert!(!released.contains("pending on node") && !released.contains(">cancelled<"), "a day-old row with nothing pending is gone");
		assert!(released.contains("No splice-ins in progress"));
	}

	#[test]
	fn the_page_says_when_nothing_is_in_progress() {
		let html = render_page(Some(Vec::new()), None);
		assert!(html.contains("Queued splice-ins") && html.contains("Refresh"), "the card and its Refresh stay");
		assert!(html.contains("No splice-ins in progress") && !html.contains("<table"));
		assert!(render_page(None, None).contains("No splice-ins in progress"), "no list yet and nothing loading");
	}

	#[test]
	fn an_open_row_offers_cancel_and_a_locked_one_is_only_listed() {
		let now = crate::format::now_secs() as i64;
		let open = page_html(queued_row("queued", false, now));
		assert!(open.contains(">queued<") && open.contains(">Cancel<") && !open.contains(">Release<"));
		let locked = page_html(queued_row("locked", false, now));
		assert!(locked.contains(">locked<") && !locked.contains(">Cancel<") && !locked.contains(">Release<"), "listed for a day, nothing to do");
	}

	/// A confirmation for the call `queued_row` makes on channel `uid`.
	fn release_for(uid: &str, confirmed: bool) -> Option<QueuedRelease> {
		Some(QueuedRelease { user_channel_id: uid.into(), ..QueuedRelease::for_call(&queued_row("splicing", true, 0), confirmed) })
	}

	/// Whether the confirmation's final Release button is disabled; None when there is no confirmation.
	fn final_release_disabled(html: &str) -> Option<bool> {
		let at = html.find("btn danger solid")?;
		let tag_start = html[..at].rfind("<button")?;
		let tag_end = at + html[at..].find('>')?;
		Some(html[tag_start..tag_end].contains("disabled"))
	}

	#[test]
	fn release_only_sends_from_a_ticked_confirmation_for_that_channel() {
		let now = crate::format::now_secs() as i64;
		let pending = queued_row("splicing", true, now);
		let plain = page_html_with(pending.clone(), None);
		assert!(plain.contains(">Release<") && !plain.contains("confirm-box"), "the row's Release only opens the confirmation");
		assert_eq!(final_release_disabled(&plain), None);

		let unticked = page_html_with(pending.clone(), release_for("7", false));
		assert!(unticked.contains("confirm-box") && unticked.contains("I checked the node"));
		assert_eq!(final_release_disabled(&unticked), Some(true), "nothing can be sent before the tick");

		let ticked = page_html_with(pending.clone(), release_for("7", true));
		assert_eq!(final_release_disabled(&ticked), Some(false));

		let other_channel = page_html_with(pending, release_for("8", true));
		assert!(!other_channel.contains("confirm-box"), "a tick for another channel arms nothing here");
	}

	#[test]
	fn a_release_confirmation_belongs_to_a_row_still_pending_on_the_node() {
		let rows = vec![queued_row("splicing", true, 0), QueuedSpliceInfo { user_channel_id: "9".into(), ..queued_row("queued", false, 0) }];
		let open = |uid: &str, confirmed: bool| armed_release(release_for(uid, confirmed).as_ref(), &rows);
		assert_eq!(open("7", true).map(|(row, confirmed)| (row.user_channel_id, confirmed)), Some(("7".to_owned(), true)));
		assert_eq!(open("9", true), None, "no call pending on the node, nothing to release");
		assert_eq!(open("8", true), None, "no such row");
		assert_eq!(armed_release(None, &rows), None);
	}

	#[test]
	fn a_release_tick_is_for_one_call_and_only_a_new_call_or_an_answer_ends_it() {
		let call = queued_row("splicing", true, 3_000);
		let release = QueuedRelease::for_call(&call, true);
		assert!(release.is_for(&call));
		// Still the same call: the row stalls, is pushed again, or gets a plain Cancel while the call is pending.
		assert!(release.is_for(&QueuedSpliceInfo { status: "stalled".into(), updated_at: 9_000, ..call.clone() }));
		assert!(release.is_for(&QueuedSpliceInfo { wake_sent_at: 9_000, wake_note: "no usable push token".into(), ..call.clone() }));
		assert!(release.is_for(&QueuedSpliceInfo { status: "cancelled".into(), ..call.clone() }));
		// Not that call any more.
		assert!(!release.is_for(&QueuedSpliceInfo { last_attempt_at: 2_130, ..call.clone() }), "a new call, or the node's answer");
		assert!(!release.is_for(&QueuedSpliceInfo { created_at: 5_000, ..call.clone() }), "a new request on the channel");
		assert!(!release.is_for(&QueuedSpliceInfo { pending_on_node: false, ..call.clone() }), "nothing pending");
		assert!(!release.is_for(&QueuedSpliceInfo { user_channel_id: "8".into(), ..call.clone() }), "another channel");
		let signed = QueuedSpliceInfo { status: "negotiated".into(), funding_txo: "bb:1".into(), ..call };
		assert!(!release.is_for(&signed), "the node signed the call after the operator checked it");
		assert!(QueuedRelease::for_call(&signed, true).is_for(&signed), "a release opened on the signed call is for it");
	}

	#[test]
	fn a_release_ticked_before_the_node_signed_the_call_sends_nothing() {
		let call = queued_row("splicing", true, 3_000);
		let release = QueuedRelease::for_call(&call, true);
		let signed = QueuedSpliceInfo { status: "negotiated".into(), funding_txo: "bb:1".into(), ..call.clone() };
		let listed = |row: QueuedSpliceInfo| ListQueuedSplicesResponse { splices: vec![row] };
		assert!(actions::queued_release_request(&release, &listed(call)).is_some());
		assert_eq!(actions::queued_release_request(&release, &listed(signed.clone())), None);
		assert_eq!(armed_release(Some(&release), &[signed]), None, "the confirmation closes");
	}

	#[test]
	fn the_rows_release_opens_an_unticked_confirmation_for_its_call_and_keep_waiting_closes_it() {
		let call = queued_row("splicing", true, 0);
		let mut view = crate::state::ViewState::default();
		view.open_release(&call);
		assert_eq!(view.queued_release, Some(QueuedRelease::for_call(&call, false)), "opening sends nothing and ticks nothing");
		view.tick_release(&call, true);
		assert_eq!(view.queued_release, Some(QueuedRelease::for_call(&call, true)));
		view.tick_release(&call, false);
		assert_eq!(view.queued_release.as_ref().map(|r| r.confirmed), Some(false), "the tick can be taken back");
		view.close_release();
		assert_eq!(view.queued_release, None);
	}

	#[test]
	fn a_tick_for_one_call_does_not_arm_a_later_call_on_the_channel() {
		let now = crate::format::now_secs() as i64;
		let retry = QueuedSpliceInfo { last_attempt_at: 2_130, ..queued_row("splicing", true, now) };
		let html = page_html_with(retry, release_for("7", true));
		assert!(html.contains(">Release<") && !html.contains("confirm-box"), "the retry needs its own Release and tick");
		assert_eq!(final_release_disabled(&html), None);
		let requeued = QueuedSpliceInfo { created_at: 5_000, ..queued_row("splicing", true, now) };
		assert!(!page_html_with(requeued, release_for("7", true)).contains("confirm-box"));
	}

	#[test]
	fn the_confirmation_shows_when_the_call_was_made_and_a_signed_splice() {
		let now = crate::format::now_secs() as i64;
		let call = QueuedSpliceInfo { last_attempt_at: now - 600, ..queued_row("splicing", true, now) };
		let html = page_html_with(call.clone(), Some(QueuedRelease::for_call(&call, false)));
		assert!(html.contains("Call made 10m ago"), "{html}");
		assert!(!html.contains("as signed"));
		let negotiated = QueuedSpliceInfo { status: "negotiated".into(), funding_txo: "ab12:1".into(), ..call };
		let html = page_html_with(negotiated.clone(), Some(QueuedRelease::for_call(&negotiated, false)));
		assert!(html.contains("splice as signed: ab12:1"), "{html}");
	}

	#[test]
	fn a_failed_row_shows_its_error_and_why_the_retry_could_not_reach_the_phone() {
		let failed = QueuedSpliceInfo {
			last_error: "splice-in refused".into(),
			wake_note: "no usable push token".into(),
			..queued_row("failed", false, 0)
		};
		assert_eq!(queued_splice_note(&failed), "splice-in refused · no usable push token");
		assert_eq!(queued_splice_note(&QueuedSpliceInfo { last_error: String::new(), ..failed.clone() }), "no usable push token");
		assert_eq!(queued_splice_note(&QueuedSpliceInfo { wake_note: String::new(), ..failed.clone() }), "splice-in refused");
		let stalled = QueuedSpliceInfo { last_error: "no splice-in call could be made within 86400s of queueing: no usable push token".into(), ..failed.clone() };
		assert_eq!(queued_splice_note(&stalled), stalled.last_error, "a stall that names the reason does not repeat it");
		let html = page_html(QueuedSpliceInfo { updated_at: crate::format::now_secs() as i64, ..failed });
		assert!(html.contains("splice-in refused · no usable push token"));
	}

	#[component]
	fn ListedHarness(listed: QueuedSpliceInfo) -> Element {
		use crate::state::{Connection, ConnectionStatus, Forms, Nav, Prefs};
		let ctx = use_context_provider(|| {
			let mut conn = Connection::new();
			conn.status = ConnectionStatus::Connected;
			AppCtx::new(conn, Forms::default(), Nav::default(), Prefs::default(), false)
		});
		use_hook(move || {
			let mut view = ctx.view;
			view.write().queued_release = release_for("7", true);
			actions::queued_splices_listed(ctx, ListQueuedSplicesResponse { splices: vec![listed.clone()] });
		});
		let rows = ctx.data.read().queued_splices.as_ref().map_or(0, |q| q.splices.len());
		let armed = ctx.view.read().queued_release.as_ref().map_or("none".to_owned(), |r| r.confirmed.to_string());
		rsx! { "rows={rows} armed={armed}" }
	}

	fn listed(row: QueuedSpliceInfo) -> String {
		let mut dom = VirtualDom::new_with_props(ListedHarness, ListedHarnessProps { listed: row });
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	#[test]
	fn a_new_list_keeps_a_tick_only_while_its_call_is_still_pending() {
		assert_eq!(listed(queued_row("stalled", true, 9_000)), "rows=1 armed=true", "same call, now stalled");
		let answered = QueuedSpliceInfo { last_attempt_at: 2_050, ..queued_row("failed", false, 9_000) };
		assert_eq!(listed(answered), "rows=1 armed=none", "the node answered");
		let later = QueuedSpliceInfo { last_attempt_at: 2_200, ..queued_row("splicing", true, 9_000) };
		assert_eq!(listed(later), "rows=1 armed=none", "a later call");
	}

	#[component]
	fn ReleaseHarness(release: Option<QueuedRelease>) -> Element {
		use crate::state::{Connection, ConnectionStatus, Forms, Nav, Prefs};
		let ctx = use_context_provider(|| {
			let mut conn = Connection::new();
			conn.status = ConnectionStatus::Connected;
			AppCtx::new(conn, Forms::default(), Nav::default(), Prefs::default(), false)
		});
		use_hook(move || {
			let mut view = ctx.view;
			view.write().queued_release = release.clone();
			actions::release_queued_splice(ctx);
		});
		let armed = ctx.view.read().queued_release.is_some();
		rsx! { "armed={armed}" }
	}

	fn release_pressed(release: Option<QueuedRelease>) -> String {
		let mut dom = VirtualDom::new_with_props(ReleaseHarness, ReleaseHarnessProps { release });
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	#[test]
	fn release_goes_ahead_only_from_a_ticked_confirmation() {
		assert_eq!(release_pressed(release_for("7", false)), "armed=true", "unticked: nothing is sent and the box stays");
		assert_eq!(release_pressed(release_for("7", true)), "armed=false", "ticked: the box closes as the release goes out");
		assert_eq!(release_pressed(None), "armed=false");
	}

	#[test]
	fn release_is_sent_only_while_the_daemon_still_lists_the_checked_call() {
		let release = release_for("7", true).unwrap();
		let list = |row: QueuedSpliceInfo| ListQueuedSplicesResponse { splices: vec![row] };
		let request = actions::queued_release_request(&release, &list(queued_row("stalled", true, 9_000))).expect("same call, still pending");
		assert_eq!((request.user_channel_id.as_str(), request.force), ("7", true), "a release is the forced cancel for that channel");
		assert_eq!(
			(request.created_at, request.last_attempt_at, request.funding_txo.as_str()),
			(release.created_at, release.last_attempt_at, release.funding_txo.as_str()),
			"it names the call that was checked, so the daemon releases no other"
		);
		let retried = QueuedSpliceInfo { last_attempt_at: 2_130, ..queued_row("splicing", true, 9_000) };
		assert_eq!(actions::queued_release_request(&release, &list(retried)), None, "the daemon made a new call since the check");
		let answered = QueuedSpliceInfo { last_attempt_at: 2_050, ..queued_row("failed", false, 9_000) };
		assert_eq!(actions::queued_release_request(&release, &list(answered)), None, "the node answered: nothing left to release");
		assert_eq!(actions::queued_release_request(&release, &ListQueuedSplicesResponse::default()), None);
	}

	#[component]
	fn QueueActionHarness(failure: Option<String>) -> Element {
		use crate::state::{Connection, ConnectionStatus, Forms, Nav, Prefs};
		let ctx = use_context_provider(|| {
			let mut conn = Connection::new();
			conn.status = ConnectionStatus::Connected;
			AppCtx::new(conn, Forms::default(), Nav::default(), Prefs::default(), false)
		});
		use_hook(move || {
			let mut data = ctx.data;
			data.write().queued_splices = Some(ListQueuedSplicesResponse { splices: vec![queued_row("splicing", true, 0)] });
			let mut view = ctx.view;
			view.write().queued_release = release_for("7", true);
			match failure.clone() {
				Some(error) => actions::queued_splices_failed(ctx, error),
				None => actions::disconnect(ctx),
			}
		});
		let status = ctx.status.read().clone().map(|m| m.text).unwrap_or_default();
		let rows = ctx.data.read().queued_splices.as_ref().map_or(0, |q| q.splices.len());
		let armed = ctx.view.read().queued_release.is_some();
		rsx! { "status=[{status}] rows={rows} armed={armed}" }
	}

	fn queue_action(failure: Option<&str>) -> String {
		let mut dom = VirtualDom::new_with_props(QueueActionHarness, QueueActionHarnessProps { failure: failure.map(str::to_owned) });
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	#[test]
	fn a_failed_queue_list_is_reported_unless_the_daemon_is_older_and_keeps_its_rows() {
		assert_eq!(queue_action(Some("HTTP 404 Not Found")), "status=[] rows=1 armed=true");
		assert_eq!(queue_action(Some("HTTP request failed: connection refused")), "status=[HTTP request failed: connection refused] rows=1 armed=true");
	}

	#[test]
	fn disconnecting_forgets_the_queue_and_closes_an_open_release() {
		assert_eq!(queue_action(None), "status=[Disconnected] rows=0 armed=false");
	}
}
