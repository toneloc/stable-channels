use egui::{Context, Ui};
use egui_extras::{Column, TableBuilder};
use hex::DisplayHex;
#[cfg(target_arch = "wasm32")]
use web_sys::js_sys;

use crate::app::LspServerApp;
use crate::ui::layout::page_scrolled;
use crate::ui::truncate_id;
use crate::ui::widgets;

const HELP_PAYMENT_ID: &str =
	"The server-side identifier for this payment record. It is distinct from a payment hash or on-chain transaction id.";
const HELP_PAYMENT_TYPE: &str = "The payment protocol or kind, such as on-chain, BOLT11, BOLT12 offer, BOLT12 refund, spontaneous, stability, or sync.";
const HELP_PAYMENT_AMOUNT: &str =
	"The payment amount recorded for this payment. Any fee paid by this node is shown separately when known.";
const HELP_PAYMENT_FEE: &str =
	"The routing or transaction fee paid by this node when known. Inbound payments usually have no fee paid by this node.";
const HELP_PAYMENT_DIRECTION: &str =
    "Inbound means received by this node. Outbound means sent by this node.";
const HELP_PAYMENT_STATUS: &str =
    "The payment lifecycle state, such as pending, succeeded, or failed.";
const HELP_PAYMENT_TIMESTAMP: &str =
    "The latest update time for this payment. Hover the value for the raw Unix timestamp.";
const HELP_PAYMENT_HASH: &str =
    "The hash locking a Lightning payment. The matching preimage proves settlement.";
const HELP_PREIMAGE: &str =
    "The secret value that satisfies the payment hash and proves the Lightning payment settled.";
const HELP_SECRET: &str =
	"Payment secret material used to bind or authorize the payment request and protect the recipient.";
const HELP_OFFER_ID: &str = "Identifier for the BOLT12 offer involved in this payment.";
const HELP_PAYER_NOTE: &str = "Optional note supplied by the payer in a BOLT12 flow.";
const HELP_QUANTITY: &str = "Quantity requested from a BOLT12 offer when present.";
const HELP_TXID: &str = "The Bitcoin transaction identifier for an on-chain payment.";

// Per-row snapshot extracted from state.payments so the state borrow is released
// before app.fmt_msat / status_pill run in the grid body (see channels.rs).
struct PaymentRow {
    id: String,
    hash: String,
    type_label: String,
    settlement_kind: Option<crate::state::SettlementKind>,
    amount_msat: Option<u64>,
    fee_paid_msat: Option<u64>,
    direction: i32,
    status: i32,
    timestamp: u64,
}

pub fn render(ui: &mut Ui, app: &mut LspServerApp) {
    ui.heading("Payments");
    ui.add_space(10.0);

    if app.render_disconnected_gate(ui) {
        return;
    }

    ui.horizontal(|ui| {
        if app.state.tasks.payments.is_some() {
            ui.spinner();
            ui.label("Loading...");
        } else {
            if ui.button("Refresh").clicked() {
                app.state.payments_page_token = None;
                app.state.payments_appending = false;
                app.fetch_payments();
            }
            if app.state.payments_page_token.is_some() && ui.button("Load More").clicked() {
                app.state.payments_appending = true;
                app.fetch_payments();
            }
        }
    });

    ui.add_space(10.0);

    let loading = app.state.tasks.payments.is_some();

    // Track which payment details button was clicked
    let mut clicked_payment_id: Option<String> = None;

    // Pre-extract per-row data into locals so the &app.state.payments borrow is
    // released before app.fmt_msat / status_pill below; never mutate the underlying list.
    let settlement_kinds = app.state.settlement_kinds.as_ref();
    let rows: Option<(Vec<PaymentRow>, bool)> = app.state.payments.as_ref().map(|resp| {
        let rows = resp
            .payments
            .iter()
            .map(|p| PaymentRow {
                id: p.id.clone(),
                hash: p.kind.as_ref().map(payment_hash).unwrap_or_default(),
                type_label: p
                    .kind
                    .as_ref()
                    .map(|k| format_payment_kind(k))
                    .unwrap_or_else(|| "Unknown".to_string()),
                settlement_kind: settlement_kinds.and_then(|m| m.get(&p.id).copied()),
                amount_msat: p.amount_msat,
                fee_paid_msat: p.fee_paid_msat,
                direction: p.direction,
                status: p.status,
                timestamp: p.latest_update_timestamp,
            })
            .collect();
        (rows, resp.next_page_token.is_some())
    });

    if let Some((rows, more_available)) = rows {
        if rows.is_empty() {
            if !loading {
                ui.label("No payments found.");
            }
        } else {
            let total = rows.len();

            // Filter + sort controls live in egui temp memory (not persisted).
            let filter_id = ui.id().with("pay_filter");
            let status_id = ui.id().with("pay_status");
            let dir_id = ui.id().with("pay_dir");
            let sort_id = ui.id().with("pay_sort");

            let mut filter =
                ui.memory_mut(|m| m.data.get_temp::<String>(filter_id).unwrap_or_default());
            // status filter: -1 = all, else 0/1/2 like payment.status
            let mut status_filter =
                ui.memory_mut(|m| m.data.get_temp::<i32>(status_id).unwrap_or(-1));
            // direction filter: -1 = all, else 0/1 like payment.direction
            let mut dir_filter = ui.memory_mut(|m| m.data.get_temp::<i32>(dir_id).unwrap_or(-1));
            // sort key: (column, descending) where 0 = Amount, 1 = Date
            let mut sort =
                ui.memory_mut(|m| m.data.get_temp::<(u8, bool)>(sort_id).unwrap_or((1, true)));

            ui.horizontal(|ui| {
                ui.label("Filter:");
                ui.add(
                    egui::TextEdit::singleline(&mut filter)
                        .hint_text("id or hash")
                        .desired_width(240.0),
                );
                egui::ComboBox::from_id_salt(status_id)
                    .width(160.0)
                    .selected_text(status_label(status_filter))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut status_filter, -1, "All statuses");
                        ui.selectable_value(&mut status_filter, 0, "Pending");
                        ui.selectable_value(&mut status_filter, 1, "Succeeded");
                        ui.selectable_value(&mut status_filter, 2, "Failed");
                    });
            });

            // Build the rendered view by filtering indices into the loaded rows.
            let needle = filter.trim().to_lowercase();
            let mut view: Vec<usize> = (0..rows.len())
                .filter(|&i| {
                    let r = &rows[i];
                    let matches_text = needle.is_empty()
                        || r.id.to_lowercase().contains(&needle)
                        || r.hash.to_lowercase().contains(&needle);
                    let matches_status = status_filter < 0 || r.status == status_filter;
                    let matches_dir = dir_filter < 0 || r.direction == dir_filter;
                    matches_text && matches_status && matches_dir
                })
                .collect();

            // Sort the view (purely a display ordering; underlying list untouched).
            view.sort_by(|&a, &b| {
                let (ra, rb) = (&rows[a], &rows[b]);
                let ord = match sort.0 {
                    0 => ra
                        .amount_msat
                        .unwrap_or(0)
                        .cmp(&rb.amount_msat.unwrap_or(0)),
                    _ => ra.timestamp.cmp(&rb.timestamp),
                };
                if sort.1 {
                    ord.reverse()
                } else {
                    ord
                }
            });

            page_scrolled(ui, |ui| {
                ui.label(format!("{}/{} payment(s)", view.len(), total));
                ui.add_space(5.0);

                crate::ui::layout::h_scroll(ui, 1200.0, |ui| {
                    // Tighter gap between columns (default ~8.0); reserve it from the width so columns still fill exactly.
                    let gap = 3.0;
                    ui.spacing_mut().item_spacing.x = gap;
                    let cw = crate::ui::layout::weighted_widths(
                        ui.available_width() - gap * 7.0,
                        &[1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
                    );
                    TableBuilder::new(ui)
                        .striped(true)
                        .resizable(false)
                        .vscroll(false)
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .auto_shrink([false, true])
                        .column(Column::exact(cw[0]).clip(true)) // Payment ID
                        .column(Column::exact(cw[1]).clip(true)) // Type
                        .column(Column::exact(cw[2]).clip(true)) // Amount
                        .column(Column::exact(cw[3]).clip(true)) // Fee
                        .column(Column::exact(cw[4]).clip(true)) // Direction
                        .column(Column::exact(cw[5]).clip(true)) // Status
                        .column(Column::exact(cw[6]).clip(true)) // Timestamp
                        .column(Column::exact(cw[7]).clip(true)) // Details
                        .header(24.0, |mut h| {
                            h.col(|ui| {
                                widgets::table_header_with_info(ui, "Payment ID", HELP_PAYMENT_ID);
                            });
                            h.col(|ui| {
                                widgets::table_header_with_info(ui, "Type", HELP_PAYMENT_TYPE);
                            });
                            h.col(|ui| {
                                ui.vertical_centered(|ui| {
                                    ui.horizontal(|ui| {
                                        if ui.button(sort_header("Amount", &sort, 0)).clicked() {
                                            sort = (0, if sort.0 == 0 { !sort.1 } else { true });
                                        }
                                        widgets::info_icon(ui, HELP_PAYMENT_AMOUNT);
                                    });
                                });
                            });
                            h.col(|ui| {
                                ui.vertical_centered(|ui| {
                                    widgets::table_header_with_info(ui, "Fee", HELP_PAYMENT_FEE);
                                });
                            });
                            h.col(|ui| {
                                ui.vertical_centered(|ui| {
                                    ui.horizontal(|ui| {
                                        let dir_hdr = match dir_filter {
                                            0 => "Direction: In",
                                            1 => "Direction: Out",
                                            _ => "Direction",
                                        };
                                        if ui.button(dir_hdr).clicked() {
                                            dir_filter = match dir_filter {
                                                -1 => 0,
                                                0 => 1,
                                                _ => -1,
                                            };
                                        }
                                        widgets::info_icon(ui, HELP_PAYMENT_DIRECTION);
                                    });
                                });
                            });
                            h.col(|ui| {
                                ui.vertical_centered(|ui| {
                                    widgets::table_header_with_info(
                                        ui,
                                        "Status",
                                        HELP_PAYMENT_STATUS,
                                    );
                                });
                            });
                            h.col(|ui| {
                                ui.vertical_centered(|ui| {
                                    ui.horizontal(|ui| {
                                        if ui.button(sort_header("Timestamp", &sort, 1)).clicked() {
                                            sort = (1, if sort.0 == 1 { !sort.1 } else { true });
                                        }
                                        widgets::info_icon(ui, HELP_PAYMENT_TIMESTAMP);
                                    });
                                });
                            });
                            h.col(|ui| {
                                ui.vertical_centered(|ui| {
                                    ui.strong("");
                                });
                            });
                        })
                        .body(|mut body| {
                            for &i in &view {
                                let row = &rows[i];
                                body.row(26.0, |mut r| {
                                    // Payment ID
                                    r.col(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.monospace(truncate_id(&row.id, 5, 4));
                                            if ui.small_button("Copy").clicked() {
                                                ui.output_mut(|o| o.copied_text = row.id.clone());
                                            }
                                        });
                                    });

                                    // Type — settlement keysends override the generic label
                                    r.col(|ui| {
                                        ui.label(payment_type_label_str(
                                            &row.type_label,
                                            row.settlement_kind,
                                        ));
                                    });

                                    // Amount (unit-aware, centered)
                                    r.col(|ui| {
                                        ui.vertical_centered(|ui| match row.amount_msat {
                                            Some(amount) => ui.monospace(app.fmt_msat(amount)),
                                            None => ui.monospace("-"),
                                        });
                                    });

                                    // Fee (unit-aware, centered)
                                    r.col(|ui| {
                                        ui.vertical_centered(|ui| match row.fee_paid_msat {
                                            Some(fee) => ui.monospace(app.fmt_msat(fee)),
                                            None => ui.monospace("-"),
                                        });
                                    });

                                    // Direction badge (0 = Inbound, 1 = Outbound), centered small pill
                                    r.col(|ui| {
                                        ui.vertical_centered(|ui| {
                                            match row.direction {
                                                0 => widgets::status_pill(
                                                    ui,
                                                    "⬇ In",
                                                    egui::Color32::LIGHT_BLUE,
                                                ),
                                                1 => widgets::status_pill(
                                                    ui,
                                                    "⬆ Out",
                                                    egui::Color32::GOLD,
                                                ),
                                                _ => widgets::status_pill(
                                                    ui,
                                                    "Unknown",
                                                    egui::Color32::GRAY,
                                                ),
                                            };
                                        });
                                    });

                                    // Status pill (0 = Pending, 1 = Succeeded, 2 = Failed), centered
                                    r.col(|ui| {
                                        ui.vertical_centered(|ui| {
                                            let (status_text, status_color) =
                                                status_style(row.status);
                                            widgets::status_pill(ui, status_text, status_color);
                                        });
                                    });

                                    // Timestamp (relative text, exact epoch on hover)
                                    r.col(|ui| {
                                        ui.vertical_centered(|ui| {
                                            ui.label(format_timestamp(row.timestamp))
                                                .on_hover_text(format!("unix: {}", row.timestamp));
                                        });
                                    });

                                    // Details button - track click without modifying app state yet
                                    r.col(|ui| {
                                        ui.vertical_centered(|ui| {
                                            if ui.small_button("Details").clicked() {
                                                clicked_payment_id = Some(row.id.clone());
                                            }
                                        });
                                    });
                                });
                            }
                        });
                });
            });

            if more_available {
                ui.add_space(5.0);
                ui.label("More payments available. Click 'Load More' to fetch.");
            }

            // Persist the control state back into temp memory.
            ui.memory_mut(|m| {
                m.data.insert_temp(filter_id, filter);
                m.data.insert_temp(status_id, status_filter);
                m.data.insert_temp(dir_id, dir_filter);
                m.data.insert_temp(sort_id, sort);
            });
        }
    } else {
        if !loading {
            ui.label("No payment data available. Click Refresh to fetch.");
        }
    }

    // Handle the Details button click outside the borrow
    if let Some(payment_id) = clicked_payment_id {
        app.state.payment_details_id = payment_id.clone();
        app.state.payment_details = None;
        app.state.show_payment_details_dialog = true;
        app.fetch_payment_details(payment_id);
    }
}

// Color/text mapping for the status pill (same colors as the old colored_label).
fn status_style(status: i32) -> (&'static str, egui::Color32) {
    match status {
        0 => ("Pending", egui::Color32::YELLOW),
        1 => ("Succeeded", egui::Color32::GREEN),
        2 => ("Failed", egui::Color32::RED),
        _ => ("Unknown", egui::Color32::GRAY),
    }
}

fn status_label(filter: i32) -> &'static str {
    match filter {
        0 => "Pending",
        1 => "Succeeded",
        2 => "Failed",
        _ => "All statuses",
    }
}

// Header label with a sort-direction arrow when this column is the active sort key.
fn sort_header(label: &str, sort: &(u8, bool), col: u8) -> String {
    if sort.0 == col {
        format!("{} {}", label, if sort.1 { "⬇" } else { "⬆" })
    } else {
        label.to_string()
    }
}

// Best-effort payment hash string for substring filtering (empty if none).
fn payment_hash(kind: &sc_rest_client::ldk_server_grpc::types::PaymentKind) -> String {
    use sc_rest_client::ldk_server_grpc::types::payment_kind::Kind;

    match &kind.kind {
        Some(Kind::Onchain(o)) => o.txid.clone(),
        Some(Kind::Bolt11(b)) => b.hash.clone(),
        Some(Kind::Bolt12Offer(o)) => o.hash.clone().unwrap_or_default(),
        Some(Kind::Bolt12Refund(r)) => r.hash.clone().unwrap_or_default(),
        Some(Kind::Spontaneous(s)) => s.hash.clone(),
        None => String::new(),
    }
}

fn format_payment_kind(kind: &sc_rest_client::ldk_server_grpc::types::PaymentKind) -> String {
    use sc_rest_client::ldk_server_grpc::types::payment_kind::Kind;

    match &kind.kind {
        Some(Kind::Onchain(_)) => "On-chain".to_string(),
        Some(Kind::Bolt11(_)) => "BOLT11".to_string(),
        Some(Kind::Bolt12Offer(_)) => "BOLT12 Offer".to_string(),
        Some(Kind::Bolt12Refund(_)) => "BOLT12 Refund".to_string(),
        Some(Kind::Spontaneous(_)) => "Spontaneous".to_string(),
        None => "Unknown".to_string(),
    }
}

/// Grid display wrapper: the kind label is precomputed into `type_label`; apply only the override.
fn payment_type_label_str(
    type_label: &str,
    settlement: Option<crate::state::SettlementKind>,
) -> String {
    match settlement {
        Some(crate::state::SettlementKind::Stability) => "Stability".to_string(),
        Some(crate::state::SettlementKind::Sync) => "Sync".to_string(),
        None => type_label.to_string(),
    }
}

fn format_timestamp(ts: u64) -> String {
    #[cfg(target_arch = "wasm32")]
    let now_secs = (js_sys::Date::now() / 1000.0) as u64;

    #[cfg(not(target_arch = "wasm32"))]
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if now_secs >= ts {
        let secs = now_secs - ts;
        if secs < 60 {
            format!("{}s ago", secs)
        } else if secs < 3600 {
            format!("{}m ago", secs / 60)
        } else if secs < 86400 {
            format!("{}h ago", secs / 3600)
        } else {
            format!("{}d ago", secs / 86400)
        }
    } else {
        format!("{}", ts)
    }
}

pub fn render_dialogs(ctx: &Context, app: &mut LspServerApp) {
    render_payment_details_dialog(ctx, app);
}

fn render_payment_details_dialog(ctx: &Context, app: &mut LspServerApp) {
    if !app.state.show_payment_details_dialog {
        return;
    }

    egui::Window::new("Payment Details")
        .collapsible(false)
        .resizable(true)
        .default_width(500.0)
        .show(ctx, |ui| {
            // Pre-format unit-aware amounts so the &app.state.payment_details borrow
            // below doesn't conflict with app.fmt_msat (which borrows &app.state).
            let amount_str = app
                .state
                .payment_details
                .as_ref()
                .and_then(|r| r.payment.as_ref())
                .and_then(|p| p.amount_msat)
                .map(|a| app.fmt_msat(a))
                .unwrap_or_else(|| "-".to_string());
            let fee_str = app
                .state
                .payment_details
                .as_ref()
                .and_then(|r| r.payment.as_ref())
                .and_then(|p| p.fee_paid_msat)
                .map(|f| app.fmt_msat(f))
                .unwrap_or_else(|| "-".to_string());
            if app.state.tasks.payment_details.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Loading payment details...");
                });
            } else if let Some(response) = &app.state.payment_details {
                if let Some(payment) = &response.payment {
                    egui::ScrollArea::vertical()
                        .max_height(400.0)
                        .show(ui, |ui| {
                            egui::Grid::new("payment_details_grid")
                                .num_columns(2)
                                .spacing([10.0, 5.0])
                                .show(ui, |ui| {
                                    // Payment ID
                                    widgets::strong_label_with_info(
                                        ui,
                                        "Payment ID:",
                                        HELP_PAYMENT_ID,
                                    );
                                    ui.horizontal(|ui| {
                                        ui.monospace(&payment.id);
                                        if ui.small_button("Copy").clicked() {
                                            ui.output_mut(|o| o.copied_text = payment.id.clone());
                                        }
                                    });
                                    ui.end_row();

                                    // Type
                                    widgets::strong_label_with_info(ui, "Type:", HELP_PAYMENT_TYPE);
                                    let payment_type = payment
                                        .kind
                                        .as_ref()
                                        .map(|k| format_payment_kind(k))
                                        .unwrap_or_else(|| "Unknown".to_string());
                                    ui.label(payment_type);
                                    ui.end_row();

                                    // Amount (unit-aware, pre-formatted above)
                                    widgets::strong_label_with_info(
                                        ui,
                                        "Amount:",
                                        HELP_PAYMENT_AMOUNT,
                                    );
                                    ui.label(&amount_str);
                                    ui.end_row();

                                    // Fee (unit-aware, pre-formatted above)
                                    widgets::strong_label_with_info(
                                        ui,
                                        "Fee Paid:",
                                        HELP_PAYMENT_FEE,
                                    );
                                    ui.label(&fee_str);
                                    ui.end_row();

                                    // Direction
                                    widgets::strong_label_with_info(
                                        ui,
                                        "Direction:",
                                        HELP_PAYMENT_DIRECTION,
                                    );
                                    let direction = match payment.direction {
                                        0 => "Inbound",
                                        1 => "Outbound",
                                        _ => "Unknown",
                                    };
                                    ui.label(direction);
                                    ui.end_row();

                                    // Status pill (same colors as the table)
                                    widgets::strong_label_with_info(
                                        ui,
                                        "Status:",
                                        HELP_PAYMENT_STATUS,
                                    );
                                    let (status_text, status_color) = status_style(payment.status);
                                    widgets::status_pill(ui, status_text, status_color);
                                    ui.end_row();

                                    // Timestamp (relative text, exact epoch on hover)
                                    widgets::strong_label_with_info(
                                        ui,
                                        "Last Updated:",
                                        HELP_PAYMENT_TIMESTAMP,
                                    );
                                    ui.label(format_timestamp(payment.latest_update_timestamp))
                                        .on_hover_text(format!(
                                            "unix: {}",
                                            payment.latest_update_timestamp
                                        ));
                                    ui.end_row();

                                    // Kind-specific details
                                    if let Some(kind) = &payment.kind {
                                        render_payment_kind_details(ui, kind);
                                    }
                                });
                        });
                } else {
                    ui.label("Payment not found.");
                }
            } else {
                ui.label("No payment data.");
            }

            ui.add_space(10.0);
            ui.separator();

            if ui.button("Close").clicked() {
                app.state.show_payment_details_dialog = false;
                app.state.payment_details = None;
                app.state.payment_details_id.clear();
            }
        });
}

fn render_payment_kind_details(
    ui: &mut egui::Ui,
    kind: &sc_rest_client::ldk_server_grpc::types::PaymentKind,
) {
    use sc_rest_client::ldk_server_grpc::types::payment_kind::Kind;

    ui.strong("--- Details ---");
    ui.label("");
    ui.end_row();

    match &kind.kind {
        Some(Kind::Onchain(onchain)) => {
            widgets::strong_label_with_info(ui, "Txid:", HELP_TXID);
            if !onchain.txid.is_empty() {
                ui.horizontal(|ui| {
                    ui.monospace(truncate_id(&onchain.txid, 8, 8));
                    if ui.small_button("Copy").clicked() {
                        ui.output_mut(|o| o.copied_text = onchain.txid.clone());
                    }
                });
            } else {
                ui.label("-");
            }
            ui.end_row();
        }
        Some(Kind::Bolt11(bolt11)) => {
            widgets::strong_label_with_info(ui, "Payment Hash:", HELP_PAYMENT_HASH);
            ui.horizontal(|ui| {
                ui.monospace(truncate_id(&bolt11.hash, 8, 8));
                if ui.small_button("Copy").clicked() {
                    ui.output_mut(|o| o.copied_text = bolt11.hash.clone());
                }
            });
            ui.end_row();

            if let Some(preimage) = &bolt11.preimage {
                widgets::strong_label_with_info(ui, "Preimage:", HELP_PREIMAGE);
                ui.horizontal(|ui| {
                    ui.monospace(truncate_id(preimage, 8, 8));
                    if ui.small_button("Copy").clicked() {
                        ui.output_mut(|o| o.copied_text = preimage.clone());
                    }
                });
                ui.end_row();
            }

            if let Some(secret) = &bolt11.secret {
                let secret_hex = secret.to_lower_hex_string();
                widgets::strong_label_with_info(ui, "Secret:", HELP_SECRET);
                ui.horizontal(|ui| {
                    ui.monospace(truncate_id(&secret_hex, 8, 8));
                    if ui.small_button("Copy").clicked() {
                        ui.output_mut(|o| o.copied_text = secret_hex.clone());
                    }
                });
                ui.end_row();
            }
        }
        Some(Kind::Bolt12Offer(offer)) => {
            if let Some(hash) = &offer.hash {
                widgets::strong_label_with_info(ui, "Payment Hash:", HELP_PAYMENT_HASH);
                ui.horizontal(|ui| {
                    ui.monospace(truncate_id(hash, 8, 8));
                    if ui.small_button("Copy").clicked() {
                        ui.output_mut(|o| o.copied_text = hash.clone());
                    }
                });
                ui.end_row();
            }

            if let Some(preimage) = &offer.preimage {
                widgets::strong_label_with_info(ui, "Preimage:", HELP_PREIMAGE);
                ui.horizontal(|ui| {
                    ui.monospace(truncate_id(preimage, 8, 8));
                    if ui.small_button("Copy").clicked() {
                        ui.output_mut(|o| o.copied_text = preimage.clone());
                    }
                });
                ui.end_row();
            }

            if let Some(secret) = &offer.secret {
                let secret_hex = secret.to_lower_hex_string();
                widgets::strong_label_with_info(ui, "Secret:", HELP_SECRET);
                ui.horizontal(|ui| {
                    ui.monospace(truncate_id(&secret_hex, 8, 8));
                    if ui.small_button("Copy").clicked() {
                        ui.output_mut(|o| o.copied_text = secret_hex.clone());
                    }
                });
                ui.end_row();
            }

            if !offer.offer_id.is_empty() {
                widgets::strong_label_with_info(ui, "Offer ID:", HELP_OFFER_ID);
                ui.horizontal(|ui| {
                    ui.monospace(truncate_id(&offer.offer_id, 8, 8));
                    if ui.small_button("Copy").clicked() {
                        ui.output_mut(|o| o.copied_text = offer.offer_id.clone());
                    }
                });
                ui.end_row();
            }

            if let Some(payer_note) = &offer.payer_note {
                widgets::strong_label_with_info(ui, "Payer Note:", HELP_PAYER_NOTE);
                ui.label(payer_note);
                ui.end_row();
            }

            if let Some(quantity) = offer.quantity {
                widgets::strong_label_with_info(ui, "Quantity:", HELP_QUANTITY);
                ui.label(format!("{}", quantity));
                ui.end_row();
            }
        }
        Some(Kind::Bolt12Refund(refund)) => {
            if let Some(hash) = &refund.hash {
                widgets::strong_label_with_info(ui, "Payment Hash:", HELP_PAYMENT_HASH);
                ui.horizontal(|ui| {
                    ui.monospace(truncate_id(hash, 8, 8));
                    if ui.small_button("Copy").clicked() {
                        ui.output_mut(|o| o.copied_text = hash.clone());
                    }
                });
                ui.end_row();
            }

            if let Some(preimage) = &refund.preimage {
                widgets::strong_label_with_info(ui, "Preimage:", HELP_PREIMAGE);
                ui.horizontal(|ui| {
                    ui.monospace(truncate_id(preimage, 8, 8));
                    if ui.small_button("Copy").clicked() {
                        ui.output_mut(|o| o.copied_text = preimage.clone());
                    }
                });
                ui.end_row();
            }

            if let Some(secret) = &refund.secret {
                let secret_hex = secret.to_lower_hex_string();
                widgets::strong_label_with_info(ui, "Secret:", HELP_SECRET);
                ui.horizontal(|ui| {
                    ui.monospace(truncate_id(&secret_hex, 8, 8));
                    if ui.small_button("Copy").clicked() {
                        ui.output_mut(|o| o.copied_text = secret_hex.clone());
                    }
                });
                ui.end_row();
            }
        }
        Some(Kind::Spontaneous(spontaneous)) => {
            widgets::strong_label_with_info(ui, "Payment Hash:", HELP_PAYMENT_HASH);
            ui.horizontal(|ui| {
                ui.monospace(truncate_id(&spontaneous.hash, 8, 8));
                if ui.small_button("Copy").clicked() {
                    ui.output_mut(|o| o.copied_text = spontaneous.hash.clone());
                }
            });
            ui.end_row();

            if let Some(preimage) = &spontaneous.preimage {
                widgets::strong_label_with_info(ui, "Preimage:", HELP_PREIMAGE);
                ui.horizontal(|ui| {
                    ui.monospace(truncate_id(preimage, 8, 8));
                    if ui.small_button("Copy").clicked() {
                        ui.output_mut(|o| o.copied_text = preimage.clone());
                    }
                });
                ui.end_row();
            }
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::SettlementKind;

    #[test]
    fn settlement_overrides_label() {
        assert_eq!(
            payment_type_label_str("Spontaneous", Some(SettlementKind::Stability)),
            "Stability"
        );
        assert_eq!(
            payment_type_label_str("Spontaneous", Some(SettlementKind::Sync)),
            "Sync"
        );
    }

    #[test]
    fn non_settlement_passes_through() {
        assert_eq!(payment_type_label_str("Spontaneous", None), "Spontaneous");
    }
}
