use egui::Ui;
use egui_extras::{Column, TableBuilder};
#[cfg(target_arch = "wasm32")]
use web_sys::js_sys;

use crate::app::LspServerApp;
use crate::state::OnchainTab;
use crate::ui::layout::{card, kv_grid_custom_info, page, page_scrolled, FORM_WIDTH};
use crate::ui::truncate_id;
use crate::ui::widgets;

const HELP_ADDRESS: &str =
	"The Bitcoin address for the current network. Verify it belongs to the intended recipient and network before sending.";
const HELP_AMOUNT: &str =
    "The on-chain amount to send. Miner fees are separate unless Send All is selected.";
const HELP_SEND_ALL: &str =
    "Spend the wallet's available on-chain balance, subtracting the miner fee from the output.";
const HELP_FEE_RATE: &str =
    "Optional miner fee rate in sat/vB. Higher rates can confirm faster but cost more.";
const HELP_LAST_TXID: &str =
    "The transaction id for the most recently created or broadcast on-chain transaction.";
const HELP_PAYMENT_ID: &str = "The local payment record associated with this on-chain transaction.";
const HELP_TXID: &str =
    "The Bitcoin transaction id. Use it to inspect the transaction in a block explorer or node.";
const HELP_DIRECTION: &str =
    "Whether this transaction sent funds out of the wallet or received funds into it.";
const HELP_STATUS: &str = "The current confirmation or lifecycle state for this on-chain payment.";
const HELP_TIME: &str = "The latest update time for this payment.";
const HELP_ONCHAIN_TOTAL: &str = "The total balance tracked by the node's on-chain wallet.";
const HELP_SPENDABLE: &str =
	"Currently spendable on-chain funds. This excludes funds still waiting on confirmations or kept as reserve.";
const HELP_ANCHOR_RESERVE: &str =
	"Emergency on-chain reserve kept so the node can spend anchor outputs if one of its channels closes.";
const HELP_PENDING_BROADCAST: &str =
    "The sweep transaction has been prepared or queued but is not yet confirmed on-chain.";
const HELP_BROADCAST_AWAITING_CONFIRMATION: &str =
    "The sweep transaction was broadcast and is waiting for its first confirmation.";
const HELP_AWAITING_THRESHOLD_CONFIRMATIONS: &str =
	"The sweep transaction is confirmed but needs more confirmations before the balance is considered safe or spendable.";

pub fn render(ui: &mut Ui, app: &mut LspServerApp) {
    ui.heading("On-chain Transactions");
    ui.add_space(10.0);

    if app.render_disconnected_gate(ui) {
        return;
    }

    ui.horizontal(|ui| {
        if ui
            .selectable_label(app.state.onchain_tab == OnchainTab::Send, "Send")
            .clicked()
        {
            app.state.onchain_tab = OnchainTab::Send;
        }
        if ui
            .selectable_label(app.state.onchain_tab == OnchainTab::Receive, "Receive")
            .clicked()
        {
            app.state.onchain_tab = OnchainTab::Receive;
        }
        if ui
            .selectable_label(app.state.onchain_tab == OnchainTab::History, "History")
            .clicked()
        {
            app.state.onchain_tab = OnchainTab::History;
            // Fetch payments if not already loaded
            if app.state.payments.is_none() {
                app.fetch_payments();
            }
        }
    });

    ui.separator();
    ui.add_space(10.0);

    match app.state.onchain_tab {
        OnchainTab::Send => render_send(ui, app),
        OnchainTab::Receive => render_receive(ui, app),
        OnchainTab::History => render_history(ui, app),
    }
}

fn render_send(ui: &mut Ui, app: &mut LspServerApp) {
    page(ui, |ui| {
        let form_w = ui.available_width().min(FORM_WIDTH);
        ui.vertical(|ui| {
            ui.set_width(form_w);
            card(ui, "Send On-chain", |ui| {
                // Pre-read amount for preview so the &self method can borrow app without conflicting with form borrow
                let unit_label = crate::ui::unit_label(app.state.display_unit);
                let amt = app.state.forms.onchain_send.amount_sats.clone();
                let preview_str = app.amount_entry_preview(&amt);

                let form = &mut app.state.forms.onchain_send;

                egui::Grid::new("onchain_send_grid")
                    .num_columns(2)
                    .spacing([10.0, 5.0])
                    .show(ui, |ui| {
                        widgets::label_with_info(ui, "Address:", HELP_ADDRESS);
                        ui.text_edit_singleline(&mut form.address);
                        ui.end_row();

                        widgets::label_with_info(
                            ui,
                            &format!("Amount ({}):", unit_label),
                            HELP_AMOUNT,
                        );
                        ui.add_enabled(
                            !form.send_all,
                            egui::TextEdit::singleline(&mut form.amount_sats),
                        );
                        ui.end_row();

                        // Muted preview of the parsed sats amount
                        ui.label("");
                        if let Some(ref s) = preview_str {
                            ui.weak(s.as_str());
                        } else {
                            ui.label("");
                        }
                        ui.end_row();

                        widgets::label_with_info(ui, "Send All:", HELP_SEND_ALL);
                        ui.checkbox(&mut form.send_all, "Send entire balance");
                        ui.end_row();

                        widgets::label_with_info(ui, "Fee Rate (sat/vB, optional):", HELP_FEE_RATE);
                        ui.text_edit_singleline(&mut form.fee_rate_sat_per_vb);
                        ui.end_row();
                    });

                ui.add_space(10.0);

                let send_all = app.state.forms.onchain_send.send_all;

                ui.horizontal(|ui| {
                    let is_pending = app.state.tasks.onchain_send.is_some();
                    if is_pending {
                        ui.spinner();
                        ui.label("Sending...");
                    } else if send_all {
                        // Confirm gate for send-all: require a second checkbox before enabling
                        let id = ui.id().with("send_all_confirm");
                        let mut ok =
                            ui.memory_mut(|m| m.data.get_temp::<bool>(id).unwrap_or(false));
                        ui.checkbox(
                            &mut ok,
                            "I understand this sends my entire on-chain balance",
                        );
                        ui.memory_mut(|m| m.data.insert_temp(id, ok));
                        let btn = egui::Button::new(
                            egui::RichText::new("Send All").color(egui::Color32::WHITE),
                        )
                        .fill(egui::Color32::DARK_RED);
                        if ui.add_enabled(ok, btn).clicked() {
                            app.send_onchain();
                        }
                    } else {
                        if ui.button("Send").clicked() {
                            app.send_onchain();
                        }
                    }
                });

                if let Some(txid) = &app.state.last_txid {
                    ui.add_space(10.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        widgets::label_with_info(ui, "Last TXID:", HELP_LAST_TXID);
                        ui.monospace(crate::ui::truncate_id(txid, 12, 12));
                        if ui.small_button("Copy").clicked() {
                            ui.output_mut(|o| o.copied_text = txid.clone());
                        }
                    });
                }
            });
        });
    });
}

fn render_receive(ui: &mut Ui, app: &mut LspServerApp) {
    page(ui, |ui| {
        let form_w = ui.available_width().min(FORM_WIDTH);
        ui.vertical(|ui| {
            ui.set_width(form_w);
            card(ui, "Receive On-chain", |ui| {
                ui.horizontal(|ui| {
                    let is_pending = app.state.tasks.onchain_receive.is_some();
                    if is_pending {
                        ui.spinner();
                        ui.label("Generating...");
                    } else if ui.button("Generate Address").clicked() {
                        app.generate_onchain_address();
                    }
                });

                if let Some(address) = &app.state.onchain_address {
                    ui.add_space(10.0);
                    ui.separator();
                    widgets::label_with_info(ui, "Address:", HELP_ADDRESS);
                    ui.add(
                        egui::TextEdit::singleline(&mut address.as_str())
                            .desired_width(f32::INFINITY)
                            .interactive(false),
                    );
                    if ui.button("Copy Address").clicked() {
                        ui.output_mut(|o| o.copied_text = address.clone());
                    }
                }
            });
        });
    });
}

fn render_history(ui: &mut Ui, app: &mut LspServerApp) {
    ui.heading("On-chain History");
    ui.add_space(10.0);

    page_scrolled(ui, |ui| {
        // Show balances summary
        if let Some(balances) = &app.state.balances {
            // Pre-read into locals to avoid simultaneous borrows of app
            let total = balances.total_onchain_balance_sats;
            let spendable = balances.spendable_onchain_balance_sats;
            let anchor = balances.total_anchor_channels_reserve_sats;

            let total_str = app.fmt_sats(total);
            let spendable_str = app.fmt_sats(spendable);
            let anchor_str = if anchor > 0 {
                Some(app.fmt_sats(anchor))
            } else {
                None
            };

            card(ui, "Summary", |ui| {
                let mut rows: crate::ui::layout::KvInfoRows = vec![
                    (
                        "Total Balance:",
                        Some(HELP_ONCHAIN_TOTAL),
                        Box::new(|ui: &mut Ui| {
                            ui.label(total_str.as_str());
                        }),
                    ),
                    (
                        "Spendable:",
                        Some(HELP_SPENDABLE),
                        Box::new(|ui: &mut Ui| {
                            ui.label(spendable_str.as_str());
                        }),
                    ),
                ];
                if let Some(ref a) = anchor_str {
                    rows.push((
                        "Anchor Reserve:",
                        Some(HELP_ANCHOR_RESERVE),
                        Box::new(move |ui: &mut Ui| {
                            ui.label(a.as_str());
                        }),
                    ));
                }
                kv_grid_custom_info(ui, "onchain_summary", rows);
            });

            ui.add_space(10.0);

            // Show pending sweeps if any
            if !balances.pending_balances_from_channel_closures.is_empty() {
                // Pre-collect sweep data to avoid holding the balances borrow into render_pending_sweep
                let sweeps: Vec<_> = balances
                    .pending_balances_from_channel_closures
                    .iter()
                    .filter_map(|s| s.balance_type.clone())
                    .collect();

                card(ui, "Pending Sweeps", |ui| {
                    for balance_type in &sweeps {
                        render_pending_sweep(ui, app, balance_type);
                        ui.add_space(3.0);
                    }
                });
                ui.add_space(10.0);
            }
        }

        // Neutral informational callout — not an error, so a plain card, not error_banner
        card(ui, "Transaction History", |ui| {
            ui.weak("Full on-chain transaction history is not yet available.");
            ui.weak("ldk-node does not currently expose BDK wallet transaction history.");
        });

        if let Some(txid) = &app.state.last_txid {
            ui.add_space(10.0);
            ui.separator();
            ui.horizontal(|ui| {
                widgets::label_with_info(ui, "Last Sent TXID:", HELP_LAST_TXID);
                ui.monospace(truncate_id(txid, 8, 8));
                if ui.small_button("Copy").clicked() {
                    ui.output_mut(|o| o.copied_text = txid.clone());
                }
            });
        }
    });
}

fn render_pending_sweep(
    ui: &mut Ui,
    app: &mut LspServerApp,
    balance_type: &sc_rest_client::ldk_server_grpc::types::pending_sweep_balance::BalanceType,
) {
    use sc_rest_client::ldk_server_grpc::types::pending_sweep_balance::BalanceType;

    match balance_type {
        BalanceType::PendingBroadcast(b) => {
            let amt = app.fmt_sats(b.amount_satoshis);
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::YELLOW, "Pending Broadcast");
                widgets::info_icon(ui, HELP_PENDING_BROADCAST);
                ui.label(format!("{} sats", amt));
            });
        }
        BalanceType::BroadcastAwaitingConfirmation(b) => {
            let amt = app.fmt_sats(b.amount_satoshis);
            let txid = b.latest_spending_txid.clone();
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::YELLOW, "Awaiting Confirmation");
                widgets::info_icon(ui, HELP_BROADCAST_AWAITING_CONFIRMATION);
                ui.label(format!("{} sats", amt));
            });
            ui.horizontal(|ui| {
                widgets::label_with_info(ui, "TXID:", HELP_TXID);
                ui.monospace(truncate_id(&txid, 8, 8));
                if ui.small_button("Copy").clicked() {
                    ui.output_mut(|o| o.copied_text = txid.clone());
                }
            });
        }
        BalanceType::AwaitingThresholdConfirmations(b) => {
            let amt = app.fmt_sats(b.amount_satoshis);
            let height = b.confirmation_height;
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::GREEN, "Awaiting Threshold");
                widgets::info_icon(ui, HELP_AWAITING_THRESHOLD_CONFIRMATIONS);
                ui.label(format!("{} sats (height {})", amt, height));
            });
        }
    }
}

#[allow(dead_code)]
fn render_history_table(ui: &mut Ui, app: &mut LspServerApp) {
    // This function is kept for future use when ldk-node exposes transaction history
    ui.horizontal(|ui| {
        ui.heading("Transaction History");
        if app.state.tasks.payments.is_some() {
            ui.spinner();
        } else if ui.button("Refresh").clicked() {
            app.state.payments_page_token = None;
            app.fetch_payments();
        }
    });

    ui.add_space(10.0);

    if let Some(payments_response) = &app.state.payments {
        use sc_rest_client::ldk_server_grpc::types::payment_kind::Kind;

        // Filter to only onchain payments
        let onchain_payments: Vec<_> = payments_response
            .payments
            .iter()
            .filter(|p| {
                p.kind
                    .as_ref()
                    .and_then(|k| k.kind.as_ref())
                    .map(|k| matches!(k, Kind::Onchain(_)))
                    .unwrap_or(false)
            })
            .collect();

        if onchain_payments.is_empty() {
            ui.label("No on-chain transactions found in payment history.");
        } else {
            page(ui, |ui| {
                ui.label(format!(
                    "{} on-chain transaction(s)",
                    onchain_payments.len()
                ));
                ui.add_space(5.0);

                TableBuilder::new(ui)
                    .striped(true)
                    .resizable(true)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .auto_shrink([false, false])
                    .column(Column::remainder()) // Payment ID
                    .column(Column::remainder()) // TXID
                    .column(Column::auto()) // Amount
                    .column(Column::auto()) // Direction
                    .column(Column::auto()) // Status
                    .column(Column::auto()) // Time
                    .header(22.0, |mut h| {
                        h.col(|ui| {
                            widgets::table_header_with_info(ui, "Payment ID", HELP_PAYMENT_ID);
                        });
                        h.col(|ui| {
                            widgets::table_header_with_info(ui, "TXID", HELP_TXID);
                        });
                        h.col(|ui| {
                            widgets::table_header_with_info(ui, "Amount", HELP_AMOUNT);
                        });
                        h.col(|ui| {
                            widgets::table_header_with_info(ui, "Direction", HELP_DIRECTION);
                        });
                        h.col(|ui| {
                            widgets::table_header_with_info(ui, "Status", HELP_STATUS);
                        });
                        h.col(|ui| {
                            widgets::table_header_with_info(ui, "Time", HELP_TIME);
                        });
                    })
                    .body(|mut body| {
                        for payment in onchain_payments {
                            body.row(24.0, |mut r| {
                                // Payment ID
                                r.col(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.monospace(truncate_id(&payment.id, 5, 4));
                                        if ui.small_button("Copy").clicked() {
                                            ui.output_mut(|o| o.copied_text = payment.id.clone());
                                        }
                                    });
                                });

                                // TXID (from onchain kind)
                                r.col(|ui| {
                                    if let Some(kind) = &payment.kind {
                                        if let Some(Kind::Onchain(onchain)) = &kind.kind {
                                            ui.horizontal(|ui| {
                                                ui.monospace(truncate_id(&onchain.txid, 5, 4));
                                                if ui.small_button("Copy").clicked() {
                                                    ui.output_mut(|o| {
                                                        o.copied_text = onchain.txid.clone()
                                                    });
                                                }
                                            });
                                        } else {
                                            ui.label("-");
                                        }
                                    } else {
                                        ui.label("-");
                                    }
                                });

                                // Amount
                                r.col(|ui| {
                                    if let Some(amount) = payment.amount_msat {
                                        ui.label(app.fmt_sats(amount / 1000));
                                    } else {
                                        ui.label("-");
                                    }
                                });

                                // Direction
                                r.col(|ui| {
                                    let direction = match payment.direction {
                                        0 => "Receive",
                                        1 => "Send",
                                        _ => "Unknown",
                                    };
                                    ui.label(direction);
                                });

                                // Status
                                r.col(|ui| {
                                    match payment.status {
                                        0 => ui.colored_label(egui::Color32::YELLOW, "Pending"),
                                        1 => ui.colored_label(egui::Color32::GREEN, "Confirmed"),
                                        2 => ui.colored_label(egui::Color32::RED, "Failed"),
                                        _ => ui.label("Unknown"),
                                    };
                                });

                                // Time
                                r.col(|ui| {
                                    ui.label(format_timestamp(payment.latest_update_timestamp));
                                });
                            });
                        }
                    });
            });
        }
    } else {
        ui.label("Loading transaction history...");
        if app.state.tasks.payments.is_none() {
            app.fetch_payments();
        }
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
