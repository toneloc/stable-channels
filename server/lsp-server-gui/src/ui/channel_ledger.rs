use std::collections::HashSet;

use chrono::{TimeZone, Utc};
use eframe::egui;
use egui::{Color32, RichText};
use sc_rest_client::sc_protos::stable::{
    AccountingSnapshot, ChannelLedgerEvent, ChannelLedgerOverview, LedgerRef,
    ListChannelLedgerEventsResponse,
};

use crate::app::LspServerApp;
use crate::state::StatusMessage;
use crate::ui::layout::{AMBER, SECONDARY};
use crate::ui::widgets;

pub fn render(ui: &mut egui::Ui, app: &mut LspServerApp) {
    ui.heading("Channel Ledger");
    ui.add_space(8.0);

    ui.horizontal_wrapped(|ui| {
        ui.label("Identifier:");
        ui.add(
            egui::TextEdit::singleline(&mut app.state.forms.channel_ledger.identifier)
                .desired_width(300.0)
                .hint_text("user_channel_id / channel_id / payment_id / txid"),
        );
        filter_combo(
            ui,
            "ledger_category",
            "Category",
            &mut app.state.forms.channel_ledger.category,
            &[
                "channel",
                "payment",
                "forwarding",
                "trade",
                "stability",
                "peer",
                "sweep",
                "reconciliation",
                "operator",
                "system",
            ],
        );
        filter_combo(
            ui,
            "ledger_status",
            "Status",
            &mut app.state.forms.channel_ledger.status,
            &[
                "observed",
                "pending",
                "completed",
                "partial",
                "failed",
                "skipped",
            ],
        );
        filter_combo(
            ui,
            "ledger_completeness",
            "Completeness",
            &mut app.state.forms.channel_ledger.completeness,
            &["observed", "reconstructed", "legacy", "gap"],
        );
    });

    ui.horizontal_wrapped(|ui| {
        let loading = app.state.tasks.channel_ledger.is_some();
        let exporting = app.state.tasks.channel_ledger_export.is_some();
        let has_identifier = !app.state.forms.channel_ledger.identifier.trim().is_empty();
        if ui
            .add_enabled(!loading && has_identifier, egui::Button::new("Refresh"))
            .on_disabled_hover_text("Enter one exact identifier")
            .clicked()
        {
            app.state.channel_ledger_cursor = None;
            app.state.channel_ledger_appending = false;
            app.fetch_channel_ledger();
        }
        if ui
            .add_enabled(
                !loading && app.state.channel_ledger_cursor.is_some(),
                egui::Button::new("Load older"),
            )
            .clicked()
        {
            app.state.channel_ledger_appending = true;
            app.fetch_channel_ledger();
        }
        if ui
            .add_enabled(
                !exporting && has_identifier,
                egui::Button::new("Export all JSONL"),
            )
            .on_disabled_hover_text("Enter one exact identifier")
            .clicked()
        {
            app.export_channel_ledger();
        }
        ui.separator();
        ui.selectable_value(
            &mut app.state.forms.channel_ledger.newest_first,
            true,
            "Newest first",
        );
        ui.selectable_value(
            &mut app.state.forms.channel_ledger.newest_first,
            false,
            "Oldest first",
        );
        if loading || exporting {
            ui.spinner();
        }
    });
    ui.separator();

    let history = app.state.channel_ledger.clone();
    let newest_first = app.state.forms.channel_ledger.newest_first;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match history {
            Some(history) if history.events.is_empty() => {
                if let Some(overview) = history.overview.as_ref() {
                    render_overview(ui, overview);
                }
                ui.label("No ledger events match these exact filters.");
            }
            Some(history) => {
                if let Some(overview) = history.overview.as_ref() {
                    render_overview(ui, overview);
                    ui.add_space(10.0);
                }
                for index in timeline_order(&history.events, newest_first) {
                    render_event(ui, &history.events[index], &mut app.state.status_message);
                    ui.add_space(6.0);
                }
            }
            None => {
                widgets::empty_state(
                    ui,
                    "🧾",
                    "Ledger not loaded",
                    "Enter one exact identifier and click Refresh",
                );
            }
        });
}

fn filter_combo(ui: &mut egui::Ui, id: &str, label: &str, value: &mut String, choices: &[&str]) {
    ui.scope(|ui| {
        // Match the single-line TextEdit's default two-point vertical margin.
        ui.spacing_mut().button_padding.y = 2.0;
        egui::ComboBox::from_id_salt(id)
            .selected_text(if value.is_empty() {
                format!("All {label}")
            } else {
                filter_choice_label(label, value).to_owned()
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(value, String::new(), format!("All {label}"));
                for choice in choices {
                    ui.selectable_value(
                        value,
                        (*choice).to_owned(),
                        filter_choice_label(label, choice),
                    );
                }
            });
    });
}

fn filter_choice_label<'a>(filter: &str, value: &'a str) -> &'a str {
    if filter == "Completeness" && value == "observed" {
        "direct"
    } else {
        value
    }
}

fn render_overview(ui: &mut egui::Ui, overview: &ChannelLedgerOverview) {
    let state = overview.latest_accounting.as_ref();
    let values = [
        (
            "Expected USD",
            state
                .and_then(|snapshot| snapshot.expected_usd)
                .map(|value| format!("${value:.2}"))
                .unwrap_or_else(|| "—".to_owned()),
            "Current recorded target".to_owned(),
        ),
        (
            "Backing",
            state
                .and_then(|snapshot| {
                    snapshot
                        .backing_sats
                        .map(|value| (value, snapshot.btc_price))
                })
                .map(|(value, price)| format_sats_with_usd(value, price))
                .unwrap_or_else(|| "—".to_owned()),
            "Stable allocation".to_owned(),
        ),
        (
            "Native",
            state
                .and_then(|snapshot| {
                    snapshot
                        .native_sats
                        .map(|value| (value, snapshot.btc_price))
                })
                .map(|(value, price)| format_sats_with_usd(value, price))
                .unwrap_or_else(|| "—".to_owned()),
            "Non-stable allocation".to_owned(),
        ),
        (
            "Live balance",
            state
                .and_then(|snapshot| {
                    snapshot
                        .live_receiver_sats
                        .map(|value| (value, snapshot.btc_price))
                })
                .map(|(value, price)| format_sats_with_usd(value, price))
                .unwrap_or_else(|| "—".to_owned()),
            latest_state_caption(overview),
        ),
        (
            "Events",
            if overview.matching_events == overview.total_events {
                overview.total_events.to_string()
            } else {
                format!("{} / {}", overview.matching_events, overview.total_events)
            },
            if overview.matching_events == overview.total_events {
                "Exact identifier total".to_owned()
            } else {
                "Matching current filters / total".to_owned()
            },
        ),
        (
            "Coverage",
            format!("{} direct", overview.observed_events),
            format!(
                "{} reconstructed · {} legacy · {} gaps",
                overview.reconstructed_events, overview.legacy_events, overview.gap_events
            ),
        ),
    ];
    let column_count = responsive_column_count(ui.available_width(), 320.0, 3);
    for row in values.chunks(column_count) {
        ui.columns(column_count, |columns| {
            for (column, (title, value, caption)) in columns.iter_mut().zip(row.iter()) {
                widgets::stat_card(column, title, value, caption);
            }
        });
    }
    if overview.oldest_occurred_at_ms.is_some() || overview.newest_occurred_at_ms.is_some() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Ledger span:").small().color(SECONDARY));
            ui.label(
                RichText::new(format!(
                    "{} -> {}",
                    overview
                        .oldest_occurred_at_ms
                        .map(exact_timestamp)
                        .unwrap_or_else(|| "—".to_owned()),
                    overview
                        .newest_occurred_at_ms
                        .map(exact_timestamp)
                        .unwrap_or_else(|| "—".to_owned())
                ))
                .small(),
            );
        });
    }
}

fn latest_state_caption(overview: &ChannelLedgerOverview) -> String {
    let source = match overview.latest_accounting_source.as_str() {
        "channels" => "Current SQLite state",
        "ledger" => "Latest complete snapshot",
        _ => "No complete state",
    };
    match overview.latest_accounting_at_ms {
        Some(timestamp) => format!("{source} · {}", relative_timestamp(timestamp)),
        None => source.to_owned(),
    }
}

fn timeline_order(events: &[ChannelLedgerEvent], newest_first: bool) -> Vec<usize> {
    let mut order = (0..events.len()).collect::<Vec<_>>();
    order.sort_by_key(|index| (events[*index].occurred_at_ms, events[*index].id));
    if newest_first {
        order.reverse();
    }
    order
}

fn render_event(ui: &mut egui::Ui, event: &ChannelLedgerEvent, status: &mut Option<StatusMessage>) {
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                widgets::info_icon(ui, event_help(event));
                ui.label(RichText::new(human_summary(event)).strong().size(15.0));
                badge(
                    ui,
                    &event.category,
                    Color32::from_rgb(70, 110, 170),
                    category_help(&event.category),
                );
                badge(
                    ui,
                    &event.status,
                    status_color(&event.status),
                    status_help(&event.status),
                );
                badge(
                    ui,
                    completeness_label(&event.completeness),
                    completeness_color(&event.completeness),
                    completeness_help(&event.completeness),
                );
            });
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(exact_timestamp(event.occurred_at_ms))
                        .small()
                        .color(SECONDARY),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(relative_timestamp(event.occurred_at_ms))
                            .small()
                            .color(SECONDARY),
                    );
                });
            });
            render_accounting(ui, event.before.as_ref(), event.after.as_ref());
            if let Some(path) = forwarding_path(event) {
                ui.add_space(4.0);
                render_forwarding_path(ui, &path, status);
            } else if !event.refs.is_empty() {
                ui.add_space(4.0);
                render_references(ui, &event.refs, status);
            }
            egui::CollapsingHeader::new("Raw JSON")
                .id_salt(("ledger_raw", event.id))
                .show(ui, |ui| {
                    let pretty = serde_json::from_str::<serde_json::Value>(&event.detail_json)
                        .and_then(|value| serde_json::to_string_pretty(&value))
                        .unwrap_or_else(|_| event.detail_json.clone());
                    ui.add(egui::Label::new(RichText::new(pretty).monospace()).wrap());
                });
        });
}

fn render_accounting(
    ui: &mut egui::Ui,
    before: Option<&AccountingSnapshot>,
    after: Option<&AccountingSnapshot>,
) {
    match (before, after) {
        (Some(before), Some(after)) => {
            ui.add_space(6.0);
            ui.label(RichText::new("Accounting change").small().color(AMBER));
            egui::Grid::new(ui.next_auto_id())
                .num_columns(4)
                .spacing([12.0, 5.0])
                .show(ui, |ui| {
                    render_change_row(
                        ui,
                        "Expected USD",
                        before.expected_usd.map(|value| format!("${value:.2}")),
                        after.expected_usd.map(|value| format!("${value:.2}")),
                        decimal_delta(before.expected_usd, after.expected_usd, "$"),
                    );
                    render_change_row(
                        ui,
                        "Backing",
                        before
                            .backing_sats
                            .map(|value| format_sats_with_usd(value, before.btc_price)),
                        after
                            .backing_sats
                            .map(|value| format_sats_with_usd(value, after.btc_price)),
                        sats_delta(before.backing_sats, after.backing_sats),
                    );
                    render_change_row(
                        ui,
                        "Native",
                        before
                            .native_sats
                            .map(|value| format_sats_with_usd(value, before.btc_price)),
                        after
                            .native_sats
                            .map(|value| format_sats_with_usd(value, after.btc_price)),
                        sats_delta(before.native_sats, after.native_sats),
                    );
                    render_change_row(
                        ui,
                        "Live balance",
                        before
                            .live_receiver_sats
                            .map(|value| format_sats_with_usd(value, before.btc_price)),
                        after
                            .live_receiver_sats
                            .map(|value| format_sats_with_usd(value, after.btc_price)),
                        sats_delta(before.live_receiver_sats, after.live_receiver_sats),
                    );
                });
        }
        (None, Some(after)) => {
            ui.add_space(6.0);
            render_snapshot(ui, after);
        }
        (Some(before), None) => {
            ui.add_space(6.0);
            ui.label(
                RichText::new("Previous recorded state")
                    .small()
                    .color(AMBER),
            );
            render_snapshot(ui, before);
        }
        (None, None) => {}
    }
}

fn render_change_row(
    ui: &mut egui::Ui,
    label: &str,
    before: Option<String>,
    after: Option<String>,
    delta: Option<(String, i8)>,
) {
    if before.is_none() && after.is_none() {
        return;
    }
    ui.label(RichText::new(label).small().color(SECONDARY));
    ui.label(before.unwrap_or_else(|| "Not recorded".to_owned()));
    ui.label(format!(
        "-> {}",
        after.unwrap_or_else(|| "Not recorded".to_owned())
    ));
    if let Some((delta, sign)) = delta {
        let color = match sign {
            1 => Color32::GREEN,
            -1 => Color32::RED,
            _ => SECONDARY,
        };
        ui.label(RichText::new(delta).color(color));
    } else {
        ui.label("");
    }
    ui.end_row();
}

fn render_snapshot(ui: &mut egui::Ui, snapshot: &AccountingSnapshot) {
    let rows = snapshot_rows(snapshot);
    egui::Grid::new(ui.next_auto_id())
        .num_columns(2)
        .spacing([12.0, 5.0])
        .show(ui, |ui| {
            for (label, value) in rows {
                ui.label(RichText::new(label).small().color(SECONDARY));
                ui.label(value);
                ui.end_row();
            }
        });
}

fn snapshot_rows(snapshot: &AccountingSnapshot) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    if let Some(value) = snapshot.expected_usd {
        rows.push(("Expected USD", format!("${value:.2}")));
    }
    if let Some(value) = snapshot.backing_sats {
        rows.push(("Backing", format_sats_with_usd(value, snapshot.btc_price)));
    }
    if let Some(value) = snapshot.native_sats {
        rows.push(("Native", format_sats_with_usd(value, snapshot.btc_price)));
    }
    if let Some(value) = snapshot.live_receiver_sats {
        rows.push((
            "Live balance",
            format_sats_with_usd(value, snapshot.btc_price),
        ));
    }
    if let Some(value) = snapshot.amount_sats {
        rows.push(("Amount", format_sats_with_usd(value, snapshot.btc_price)));
    } else if let Some(value) = snapshot.amount_msat {
        rows.push(("Amount", format_msat(value)));
    }
    if let Some(value) = snapshot.amount_usd {
        rows.push(("Recorded amount", format!("${value:.2}")));
    }
    if let Some(value) = snapshot.fee_sats {
        rows.push(("Fee", format_sats_with_usd(value, snapshot.btc_price)));
    } else if let Some(value) = snapshot.fee_msat {
        rows.push(("Fee", format_msat(value)));
    }
    rows
}

fn decimal_delta(before: Option<f64>, after: Option<f64>, prefix: &str) -> Option<(String, i8)> {
    let delta = after? - before?;
    let sign = if delta > 0.0 {
        1
    } else if delta < 0.0 {
        -1
    } else {
        0
    };
    Some((format!("({prefix}{delta:+.2})"), sign))
}

fn sats_delta(before: Option<u64>, after: Option<u64>) -> Option<(String, i8)> {
    let delta = after? as i128 - before? as i128;
    Some((format!("({delta:+} sats)"), delta.signum() as i8))
}

#[cfg(test)]
fn accounting_delta(
    before: Option<&AccountingSnapshot>,
    after: Option<&AccountingSnapshot>,
) -> Option<String> {
    let before = before?;
    let after = after?;
    let mut parts = Vec::new();
    if let (Some(a), Some(b)) = (before.expected_usd, after.expected_usd) {
        parts.push(format!("expected_usd {a:.2} -> {b:.2} ({:+.2})", b - a));
    }
    if let (Some(a), Some(b)) = (before.backing_sats, after.backing_sats) {
        parts.push(format!("backing {a} -> {b} ({:+})", b as i128 - a as i128));
    }
    if let (Some(a), Some(b)) = (before.native_sats, after.native_sats) {
        parts.push(format!("native {a} -> {b} ({:+})", b as i128 - a as i128));
    }
    (!parts.is_empty()).then(|| parts.join("  •  "))
}

fn human_summary(event: &ChannelLedgerEvent) -> String {
    match event.event_type.as_str() {
        "CHANNEL_PENDING" => "Channel opening started".to_owned(),
        "CHANNEL_READY_TRACKED" => "Channel ready".to_owned(),
        "CHANNEL_OPEN_FAILED" => "Channel opening failed".to_owned(),
        "STABLE_EDITED" | "TRADE_APPLIED" | "SYNC_V1_APPLIED" => "Stable target changed".to_owned(),
        "PAYMENT_OUTGOING_RECONCILED" | "OUTGOING_STABLE_DEDUCTED" | "STABLE_SPEND_DEDUCTED" => {
            "Outgoing payment reduced stable backing".to_owned()
        }
        "SPLICE_IN_RECONCILED" => "Splice in completed".to_owned(),
        "SPLICE_OUT_STABLE_RECONCILED" => "Splice out completed".to_owned(),
        "CHANNEL_READY_SPLICE" => match splice_direction(event).as_deref() {
            Some("in") => "Splice in completed".to_owned(),
            Some("out") => "Splice out completed".to_owned(),
            _ => "Splice completed".to_owned(),
        },
        "SPLICE_RECONCILED" => "Splice completed".to_owned(),
        "SPLICE_OUT_STABLE_DEDUCTED" => "Splice out reduced stable backing".to_owned(),
        "STABILITY_PAYMENT_SENT" => "Stability payment sent".to_owned(),
        "EVENT_STREAM_GAP_CLOSED" => "Channel recovered after reconnect".to_owned(),
        "CHANNEL_ACCOUNTING_STATE_COMMITTED" => "Channel accounting state recorded".to_owned(),
        "CHANNEL_CLOSED_COMMITTED" | "CHANNEL_CLOSED" => "Channel closed".to_owned(),
        "STABILITY_PAYMENT_RECORDED" => "Stability payment recorded".to_owned(),
        "MESSAGE_RECEIVED" => "Channel message received".to_owned(),
        "TRADE_SIGNATURE_VALID" => "Channel message signature verified".to_owned(),
        "SYNC_MESSAGE_SENT" => "Accounting sync delivered".to_owned(),
        "PAYMENT_SETTLED" if event_amount_msat(event) == Some(1) => {
            "Accounting sync settled".to_owned()
        }
        unknown => title_case_event(unknown),
    }
}

fn event_help(event: &ChannelLedgerEvent) -> String {
    let explanation = match event.event_type.as_str() {
        "STABLE_EDITED" => "An operator changed the channel's target stable USD amount.",
        "TRADE_APPLIED" => "A validated BTC/USD trade updated the channel's stable allocation.",
        "SYNC_V1_APPLIED" => {
            "A newer signed allocation from the wallet was accepted and applied to this channel."
        }
        "PAYMENT_OUTGOING_RECONCILED" | "OUTGOING_STABLE_DEDUCTED" | "STABLE_SPEND_DEDUCTED" => {
            "An outgoing Lightning payment used stable-backed capacity, so the recorded stable backing was reduced."
        }
        "SPLICE_RECONCILED" => {
            "LDK reported the channel ready after a splice, and the LSP reconciled its current capacity and allocation."
        }
        "SPLICE_IN_RECONCILED" => {
            "A splice in added funds to the channel. The channel became ready again and its accounting was updated."
        }
        "SPLICE_OUT_STABLE_RECONCILED" => {
            "A splice out removed funds from the channel. The channel became ready again and its stable accounting was updated."
        }
        "CHANNEL_READY_SPLICE" => return splice_help(event),
        "SPLICE_OUT_STABLE_DEDUCTED" => {
            "The splice out removed more than the channel's native balance, so the remaining amount reduced its stable backing."
        }
        "STABILITY_PUSH_QUEUED" => {
            "The wallet was offline, so the LSP queued a push notification asking it to reconnect and check stability. No stability payment was sent yet."
        }
        "STABILITY_CHECK_ONLY" => {
            "The channel was above its target, but the LSP cannot pull value from the wallet, so it recorded the check without sending a payment."
        }
        "STABILITY_PAYMENT_SENT" => {
            "The LSP sent a Lightning payment to move the channel's stable value toward its target."
        }
        "STABILITY_PAYMENT_RECORDED" => {
            "A stability payment was associated with this channel and stored for settlement tracking."
        }
        "EVENT_STREAM_CONNECTED" => {
            "The LSP connected to LDK Server's live event stream and resumed listening for activity."
        }
        "EVENT_STREAM_GAP_OPENED" => {
            "The LSP lost the live LDK event stream, so activity during this interval may need reconstruction."
        }
        "EVENT_STREAM_GAP_CLOSED" => {
            "The LSP reconnected to LDK Server and completed its recovery check for the missed interval."
        }
        "CHANNEL_RECONSTRUCTED" => {
            "After reconnecting, the LSP rebuilt this snapshot from current LDK channel data. The channel itself was not recreated."
        }
        "PAYMENT_RECONSTRUCTED" => {
            "After reconnecting, the LSP rebuilt this payment record from LDK's current payment history."
        }
        "PEER_RECONSTRUCTED" => {
            "After reconnecting, the LSP rebuilt this peer snapshot from LDK's current peer list."
        }
        "SWEEP_RECONSTRUCTED" => {
            "After reconnecting, the LSP rebuilt this pending sweep snapshot from LDK's current balances."
        }
        "PAYMENT_FORWARDED_BACKFILL" => {
            "The LSP found a forwarded payment in LDK history that was not observed on the live event stream and added it to the ledger."
        }
        "RECONCILIATION_SCOPE_FAILED" => {
            "Part of the reconnect recovery could not be queried. The affected scope and error are available in Raw JSON."
        }
        "CHANNEL_ACCOUNTING_STATE_COMMITTED" => {
            "The latest expected USD, backing, native balance, and live balance were saved as one accounting snapshot."
        }
        "CHANNEL_READY_TRACKED" => {
            "The channel opening finished. LDK marked the channel ready for Lightning payments, and the LSP began tracking its stable accounting."
        }
        "CHANNEL_PENDING" => {
            "The channel opening started. Its funding transaction was created, and it is waiting for confirmations before it can carry Lightning payments."
        }
        "CHANNEL_OPEN_FAILED" => {
            "The channel opening stopped before the channel became usable. Open Raw JSON to see the recorded reason."
        }
        "CHANNEL_CLOSED_COMMITTED" | "CHANNEL_CLOSED" => {
            "The channel was closed and can no longer carry payments. The LSP stopped tracking it as an active stable channel."
        }
        "MESSAGE_RECEIVED" => {
            "The LSP received a Stable Channels protocol message carried in a custom Lightning record."
        }
        "TRADE_PARSED_PAYLOAD_OK" => {
            "The received trade message had the expected structure and could be decoded."
        }
        "TRADE_SIGNATURE_VALID" => {
            "The cryptographic signature on the received channel message was successfully verified."
        }
        "SYNC_MESSAGE_SENT" | "TRADE_MESSAGE_SENT" => {
            "A Stable Channels protocol message was delivered to the counterparty over Lightning."
        }
        "PAYMENT_SETTLED" if event_amount_msat(event) == Some(1) => {
            "The 1-msat carrier payment used to deliver an accounting sync completed successfully."
        }
        "PAYMENT_SETTLED" | "PAYMENT_SUCCESSFUL" => {
            "The Lightning payment completed successfully."
        }
        "PAYMENT_FAILED" => "The Lightning payment did not complete successfully.",
        _ => {
            return format!(
                "This is {}. Hover the badges for classification details or open Raw JSON for the exact recorded fields.",
                category_help_phrase(&event.category)
            );
        },
    };
    explanation.to_owned()
}

fn splice_direction(event: &ChannelLedgerEvent) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(&event.detail_json)
        .ok()?
        .get("direction")?
        .as_str()
        .map(str::to_owned)
}

fn splice_amount_sats(event: &ChannelLedgerEvent) -> Option<u64> {
    event
        .after
        .as_ref()
        .and_then(|snapshot| snapshot.amount_sats)
        .or_else(|| {
            serde_json::from_str::<serde_json::Value>(&event.detail_json)
                .ok()?
                .get("amount_sats")?
                .as_u64()
        })
}

fn splice_help(event: &ChannelLedgerEvent) -> String {
    let amount = splice_amount_sats(event)
        .map(|amount| format!("{} sats net", format_integer(amount)))
        .unwrap_or_else(|| "funds".to_owned());
    match splice_direction(event).as_deref() {
        Some("in") => format!(
            "A splice in added {amount} to the channel. The channel became ready again and its new balance was stored."
        ),
        Some("out") => format!(
            "A splice out removed {amount} from the channel. The channel became ready again and its stable accounting was reconciled."
        ),
        Some("unchanged") => {
            "LDK reported the channel ready after a splice, but its recorded balance was unchanged. This can be a replay or recovery event."
                .to_owned()
        },
        _ => {
            "LDK reported the channel ready after a splice, and the LSP reconciled its current balance and stable accounting."
                .to_owned()
        },
    }
}

fn category_help_phrase(category: &str) -> &'static str {
    match category {
        "channel" => "a channel lifecycle event",
        "payment" => "a Lightning payment event",
        "forwarding" => "a routed-payment event",
        "trade" => "a trade or stable-allocation event",
        "stability" => "a stabilization or accounting-sync event",
        "peer" => "a peer-connection event",
        "sweep" => "a channel-closing sweep event",
        "reconciliation" => "a recovery or backfill event",
        "operator" => "an operator action",
        "system" => "an internal system event",
        _ => "an unclassified ledger event",
    }
}

fn title_case_event(event_type: &str) -> String {
    let mut words = event_type
        .split('_')
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    if let Some(first) = words.first_mut() {
        if let Some(initial) = first.get_mut(0..1) {
            initial.make_ascii_uppercase();
        }
    }
    if words.is_empty() {
        "Unknown event".to_owned()
    } else {
        words.join(" ")
    }
}

fn event_amount_msat(event: &ChannelLedgerEvent) -> Option<u64> {
    event
        .after
        .as_ref()
        .and_then(|snapshot| snapshot.amount_msat)
        .or_else(|| {
            serde_json::from_str::<serde_json::Value>(&event.detail_json)
                .ok()
                .and_then(|detail| detail.get("amount_msat").and_then(|value| value.as_u64()))
        })
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ForwardingLeg {
    channel_id: Option<String>,
    user_channel_id: Option<String>,
    node_id: Option<String>,
}

impl ForwardingLeg {
    fn is_empty(&self) -> bool {
        self.channel_id.is_none() && self.user_channel_id.is_none() && self.node_id.is_none()
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ForwardingPath {
    incoming: ForwardingLeg,
    outgoing: ForwardingLeg,
}

fn forwarding_path(event: &ChannelLedgerEvent) -> Option<ForwardingPath> {
    if !matches!(
        event.event_type.as_str(),
        "PAYMENT_FORWARDED" | "PAYMENT_FORWARDED_BACKFILL"
    ) {
        return None;
    }
    let detail = serde_json::from_str::<serde_json::Value>(&event.detail_json).ok()?;
    let text = |key: &str| {
        detail.get(key).and_then(|value| match value {
            serde_json::Value::String(value) if !value.is_empty() => Some(value.clone()),
            serde_json::Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
    };
    let path = ForwardingPath {
        incoming: ForwardingLeg {
            channel_id: text("prev_channel_id"),
            user_channel_id: text("prev_user_channel_id"),
            node_id: text("prev_node_id"),
        },
        outgoing: ForwardingLeg {
            channel_id: text("next_channel_id"),
            user_channel_id: text("next_user_channel_id"),
            node_id: text("next_node_id"),
        },
    };
    (!path.incoming.is_empty() || !path.outgoing.is_empty()).then_some(path)
}

fn render_forwarding_path(
    ui: &mut egui::Ui,
    path: &ForwardingPath,
    status: &mut Option<StatusMessage>,
) {
    let legs = [
        (
            "Incoming channel",
            "Payment arrived through this channel.",
            &path.incoming,
        ),
        (
            "Outgoing channel",
            "Payment was forwarded through this channel.",
            &path.outgoing,
        ),
    ];
    let column_count = responsive_column_count(ui.available_width(), 430.0, 2);
    for row in legs.chunks(column_count) {
        ui.columns(column_count, |columns| {
            for (column, (title, help, leg)) in columns.iter_mut().zip(row) {
                egui::Frame::group(column.style())
                    .inner_margin(egui::Margin::same(8.0))
                    .show(column, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(*title).strong())
                            .on_hover_text(*help);
                        render_leg_identifier(ui, "Channel ID", leg.channel_id.as_deref(), status);
                        render_leg_identifier(
                            ui,
                            "User channel ID",
                            leg.user_channel_id.as_deref(),
                            status,
                        );
                        render_leg_identifier(ui, "Node ID", leg.node_id.as_deref(), status);
                    });
            }
        });
    }
}

fn render_leg_identifier(
    ui: &mut egui::Ui,
    label: &str,
    value: Option<&str>,
    status: &mut Option<StatusMessage>,
) {
    let Some(value) = value else {
        return;
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{label}:")).small().color(SECONDARY));
        widgets::id_with_copy(ui, value, status);
    });
}

fn render_references(
    ui: &mut egui::Ui,
    references: &[LedgerRef],
    status: &mut Option<StatusMessage>,
) {
    let column_count = responsive_column_count(ui.available_width(), 360.0, 3);
    for row in references.chunks(column_count) {
        ui.columns(column_count, |columns| {
            for (column, reference) in columns.iter_mut().zip(row) {
                column.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{}:", reference.role))
                            .small()
                            .color(SECONDARY),
                    );
                    widgets::id_with_copy(ui, &reference.value, status);
                });
            }
        });
    }
}

fn responsive_column_count(
    available_width: f32,
    minimum_column_width: f32,
    maximum: usize,
) -> usize {
    ((available_width / minimum_column_width).floor() as usize).clamp(1, maximum)
}

fn badge(ui: &mut egui::Ui, text: &str, color: Color32, help: String) {
    egui::Frame::none()
        .fill(color.gamma_multiply(0.22))
        .stroke(egui::Stroke::new(1.0, color))
        .rounding(4.0)
        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
        .show(ui, |ui| {
            ui.small(text);
        })
        .response
        .on_hover_text(help);
}

fn category_help(category: &str) -> String {
    let meaning = match category {
        "channel" => "Channel lifecycle, readiness, splice, or closure activity",
        "payment" => "Lightning payment activity",
        "forwarding" => "Routed payment activity",
        "trade" => "BTC/USD trade or stable-allocation activity",
        "stability" => "Stabilization payment or accounting-sync activity",
        "peer" => "Peer connection activity",
        "sweep" => "Closing-output sweep activity",
        "reconciliation" => "Recovery, backfill, or event-gap processing",
        "operator" => "Manual edit or configuration activity",
        "system" => "Internal system activity",
        _ => "Unclassified ledger activity",
    };
    format!("Category: {meaning}")
}

fn status_help(status: &str) -> String {
    let meaning = match status {
        "observed" => "Informational event; no workflow completion is implied",
        "pending" => "Operation is still in progress",
        "completed" => "Operation finished or was applied successfully",
        "partial" => "Only part of the operation completed successfully",
        "failed" => "Operation failed or was rejected",
        "skipped" => "Operation was intentionally not performed",
        _ => "Unrecognized event status",
    };
    format!("Status: {meaning}")
}

fn completeness_label(completeness: &str) -> &str {
    match completeness {
        "observed" => "direct",
        other => other,
    }
}

fn completeness_help(completeness: &str) -> String {
    let meaning = match completeness {
        "observed" => "Recorded directly when the event occurred",
        "reconstructed" => "Rebuilt later from other available records",
        "legacy" => "Imported from the older JSONL audit log and may lack structured state",
        "gap" => "Marks known missing or incomplete event coverage",
        _ => "Unrecognized record completeness",
    };
    format!("Completeness: {meaning}")
}

fn status_color(status: &str) -> Color32 {
    match status {
        "failed" => Color32::RED,
        "completed" => Color32::GREEN,
        "skipped" => Color32::GRAY,
        _ => Color32::YELLOW,
    }
}

fn completeness_color(completeness: &str) -> Color32 {
    match completeness {
        "observed" => Color32::GREEN,
        "gap" => Color32::RED,
        "legacy" => Color32::GRAY,
        _ => Color32::YELLOW,
    }
}

fn format_sats_with_usd(sats: u64, btc_price: Option<f64>) -> String {
    let display = format!("{} sats", format_integer(sats));
    match btc_price.filter(|price| price.is_finite() && *price > 0.0) {
        Some(price) => format!("{display} · ≈ ${:.2}", sats_to_usd(sats, price)),
        None => display,
    }
}

fn sats_to_usd(sats: u64, btc_price: f64) -> f64 {
    sats as f64 / 100_000_000.0 * btc_price
}

fn format_integer(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            output.push(',');
        }
        output.push(character);
    }
    output
}

fn format_msat(msat: u64) -> String {
    if msat % 1_000 == 0 {
        format!("{} sats", format_integer(msat / 1_000))
    } else {
        format!("{} msat", format_integer(msat))
    }
}

fn relative_timestamp(timestamp_ms: i64) -> String {
    let seconds = (Utc::now().timestamp_millis() - timestamp_ms) / 1_000;
    if seconds < 0 {
        return "in the future".to_owned();
    }
    match seconds {
        0..=4 => "just now".to_owned(),
        5..=59 => format!("{seconds} seconds ago"),
        60..=119 => "1 minute ago".to_owned(),
        120..=3_599 => format!("{} minutes ago", seconds / 60),
        3_600..=7_199 => "1 hour ago".to_owned(),
        7_200..=86_399 => format!("{} hours ago", seconds / 3_600),
        86_400..=172_799 => "1 day ago".to_owned(),
        172_800..=2_591_999 => format!("{} days ago", seconds / 86_400),
        2_592_000..=5_183_999 => "1 month ago".to_owned(),
        5_184_000..=31_535_999 => format!("{} months ago", seconds / 2_592_000),
        31_536_000..=63_071_999 => "1 year ago".to_owned(),
        _ => format!("{} years ago", seconds / 31_536_000),
    }
}

fn exact_timestamp(timestamp_ms: i64) -> String {
    Utc.timestamp_millis_opt(timestamp_ms)
        .single()
        .map(|timestamp| timestamp.format("%d %b %Y, %H:%M:%S%.3f UTC").to_string())
        .unwrap_or_else(|| format!("{timestamp_ms} ms"))
}

fn snapshot_json(snapshot: &AccountingSnapshot) -> serde_json::Value {
    serde_json::json!({
        "expected_usd": snapshot.expected_usd,
        "backing_sats": snapshot.backing_sats,
        "native_sats": snapshot.native_sats,
        "live_receiver_sats": snapshot.live_receiver_sats,
        "btc_price": snapshot.btc_price,
        "amount_sats": snapshot.amount_sats,
        "amount_msat": snapshot.amount_msat,
        "amount_usd": snapshot.amount_usd,
        "fee_sats": snapshot.fee_sats,
        "fee_msat": snapshot.fee_msat,
    })
}

fn history_jsonl(history: &ListChannelLedgerEventsResponse) -> String {
    let mut events = history.events.iter().collect::<Vec<_>>();
    events.sort_by_key(|event| (event.occurred_at_ms, event.id));
    let mut seen = HashSet::new();
    events
        .into_iter()
        .filter(|event| seen.insert(event.id))
        .map(|event| {
            serde_json::json!({
                "ledger_id": event.id,
                "occurred_at_ms": event.occurred_at_ms,
                "recorded_at_ms": event.recorded_at_ms,
                "event": event.event_type,
                "category": event.category,
                "severity": event.severity,
                "status": event.status,
                "source": event.source,
                "completeness": event.completeness,
                "dedup_key": event.dedup_key,
                "before": event.before.as_ref().map(snapshot_json),
                "after": event.after.as_ref().map(snapshot_json),
                "refs": event.refs.iter().map(|reference| serde_json::json!({
                    "role": reference.role,
                    "value": reference.value,
                })).collect::<Vec<_>>(),
                "data": serde_json::from_str::<serde_json::Value>(&event.detail_json)
                    .unwrap_or_else(|_| serde_json::Value::String(event.detail_json.clone())),
            })
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn export_jsonl(
    history: &ListChannelLedgerEventsResponse,
    status: &mut Option<StatusMessage>,
) {
    let content = history_jsonl(history);
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name("channel-ledger.jsonl")
            .save_file()
        {
            match std::fs::write(&path, content) {
                Ok(()) => {
                    *status = Some(StatusMessage::success(format!(
                        "Exported all {} events to {}",
                        history.events.len(),
                        path.display()
                    )))
                }
                Err(error) => {
                    *status = Some(StatusMessage::error(format!("Export failed: {error}")))
                }
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = content;
        *status = Some(StatusMessage::error(
            "JSONL download is not available in this web build",
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: i64, event_type: &str) -> ChannelLedgerEvent {
        ChannelLedgerEvent {
            id,
            event_type: event_type.to_owned(),
            occurred_at_ms: id * 10,
            status: "completed".to_owned(),
            severity: "info".to_owned(),
            completeness: "observed".to_owned(),
            detail_json: "{}".to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn completeness_display_distinguishes_direct_records_from_status() {
        assert_eq!(completeness_label("observed"), "direct");
        assert_eq!(filter_choice_label("Completeness", "observed"), "direct");
        assert_eq!(filter_choice_label("Status", "observed"), "observed");
        assert!(completeness_help("observed").contains("Recorded directly"));
        assert!(status_help("observed").contains("Informational event"));
    }

    #[test]
    fn responsive_columns_follow_available_width() {
        assert_eq!(responsive_column_count(359.0, 360.0, 3), 1);
        assert_eq!(responsive_column_count(720.0, 360.0, 3), 2);
        assert_eq!(responsive_column_count(1_080.0, 360.0, 3), 3);
        assert_eq!(responsive_column_count(2_000.0, 360.0, 3), 3);
    }

    #[test]
    fn accounting_delta_reports_before_after() {
        let before = AccountingSnapshot {
            backing_sats: Some(10),
            native_sats: Some(5),
            ..Default::default()
        };
        let after = AccountingSnapshot {
            backing_sats: Some(12),
            native_sats: Some(3),
            ..Default::default()
        };
        let text = accounting_delta(Some(&before), Some(&after)).unwrap();
        assert!(text.contains("backing 10 -> 12 (+2)"));
        assert!(text.contains("native 5 -> 3 (-2)"));
        assert!(accounting_delta(None, Some(&after)).is_none());
    }

    #[test]
    fn human_summaries_and_unknown_fallback_are_readable() {
        assert_eq!(
            human_summary(&event(1, "STABLE_EDITED")),
            "Stable target changed"
        );
        assert_eq!(
            human_summary(&event(2, "PAYMENT_OUTGOING_RECONCILED")),
            "Outgoing payment reduced stable backing"
        );
        assert_eq!(
            human_summary(&event(3, "SPLICE_RECONCILED")),
            "Splice completed"
        );
        assert_eq!(human_summary(&event(4, "A_NEW_EVENT")), "A new event");
        assert_eq!(
            human_summary(&event(5, "CHANNEL_PENDING")),
            "Channel opening started"
        );
        assert_eq!(
            human_summary(&event(6, "CHANNEL_READY_TRACKED")),
            "Channel ready"
        );

        let mut splice_in = event(7, "CHANNEL_READY_SPLICE");
        splice_in.detail_json = r#"{"direction":"in","amount_sats":9769}"#.to_owned();
        assert_eq!(human_summary(&splice_in), "Splice in completed");

        let mut splice_out = event(8, "CHANNEL_READY_SPLICE");
        splice_out.detail_json = r#"{"direction":"out","amount_sats":5000}"#.to_owned();
        assert_eq!(human_summary(&splice_out), "Splice out completed");
    }

    #[test]
    fn event_help_explains_operator_facing_titles() {
        assert!(event_help(&event(1, "STABILITY_PUSH_QUEUED")).contains("No stability payment"));
        assert!(event_help(&event(2, "CHANNEL_RECONSTRUCTED")).contains("not recreated"));
        assert!(event_help(&event(3, "SPLICE_RECONCILED")).contains("reconciled"));
        assert!(
            event_help(&event(4, "CHANNEL_PENDING")).contains("funding transaction was created")
        );
        assert!(event_help(&event(5, "CHANNEL_READY_TRACKED")).contains("opening finished"));

        let mut one_msat = event(6, "PAYMENT_SETTLED");
        one_msat.detail_json = r#"{"amount_msat":1}"#.to_owned();
        assert!(event_help(&one_msat).contains("carrier payment"));

        let mut splice_in = event(7, "CHANNEL_READY_SPLICE");
        splice_in.detail_json = r#"{"direction":"in","amount_sats":9769}"#.to_owned();
        assert!(event_help(&splice_in).contains("9,769 sats net"));

        let legacy_splice = event(8, "CHANNEL_READY_SPLICE");
        assert_eq!(human_summary(&legacy_splice), "Splice completed");
        assert!(event_help(&legacy_splice).contains("reconciled"));
    }

    #[test]
    fn forwarded_payment_preserves_incoming_and_outgoing_leg_roles() {
        let mut forwarded = event(5, "PAYMENT_FORWARDED");
        forwarded.detail_json = serde_json::json!({
            "prev_channel_id": "incoming-channel",
            "prev_user_channel_id": "incoming-user-channel",
            "prev_node_id": "incoming-peer",
            "next_channel_id": "outgoing-channel",
            "next_user_channel_id": "outgoing-user-channel",
            "next_node_id": "outgoing-peer",
        })
        .to_string();

        let path = forwarding_path(&forwarded).unwrap();
        assert_eq!(
            path.incoming.channel_id.as_deref(),
            Some("incoming-channel")
        );
        assert_eq!(
            path.incoming.user_channel_id.as_deref(),
            Some("incoming-user-channel")
        );
        assert_eq!(path.incoming.node_id.as_deref(), Some("incoming-peer"));
        assert_eq!(
            path.outgoing.channel_id.as_deref(),
            Some("outgoing-channel")
        );
        assert_eq!(
            path.outgoing.user_channel_id.as_deref(),
            Some("outgoing-user-channel")
        );
        assert_eq!(path.outgoing.node_id.as_deref(), Some("outgoing-peer"));
        assert!(forwarding_path(&event(6, "CHANNEL_RECONSTRUCTED")).is_none());
    }

    #[test]
    fn timeline_keeps_every_event_in_requested_order() {
        let events = vec![
            event(1, "MESSAGE_RECEIVED"),
            event(2, "TRADE_SIGNATURE_VALID"),
            event(3, "STABLE_EDITED"),
        ];
        assert_eq!(timeline_order(&events, false), vec![0, 1, 2]);
        assert_eq!(timeline_order(&events, true), vec![2, 1, 0]);
    }

    #[test]
    fn jsonl_export_is_chronological_complete_and_deduplicated() {
        let mut newest = event(2, "E2");
        newest.detail_json = r#"{"id":2}"#.to_owned();
        newest.before = Some(AccountingSnapshot {
            backing_sats: Some(7),
            ..Default::default()
        });
        let mut oldest = event(1, "E1");
        oldest.detail_json = r#"{"id":1}"#.to_owned();
        let history = ListChannelLedgerEventsResponse {
            events: vec![newest.clone(), oldest, newest],
            next_cursor: None,
            overview: None,
        };
        let jsonl = history_jsonl(&history);
        let lines = jsonl.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"ledger_id\":1"));
        assert!(lines[1].contains("\"ledger_id\":2"));
        assert!(lines[1].contains("\"before\":{\"amount_msat\":null"));
        assert!(lines[1].contains("\"data\":{\"id\":2}"));
    }

    #[test]
    fn sats_values_include_approximate_usd_only_with_recorded_price() {
        assert_eq!(format_sats_with_usd(100_000, None), "100,000 sats");
        assert_eq!(
            format_sats_with_usd(100_000, Some(80_000.0)),
            "100,000 sats · ≈ $80.00"
        );
    }
}
