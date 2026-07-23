use egui::Ui;
use egui_extras::{Column, TableBuilder};

use crate::app::LspServerApp;
use crate::ui::layout::{card, kv_grid_custom_info, page_scrolled};
use crate::ui::widgets;

const HELP_ONCHAIN_TOTAL: &str = "The total balance tracked by the node's on-chain wallet.";
const HELP_ONCHAIN_SPENDABLE: &str =
    "The on-chain funds that are currently spendable after confirmation requirements and reserves.";
const HELP_SPENDABLE: &str =
	"Currently spendable on-chain funds. This excludes funds still waiting on confirmations or kept as reserve.";
const HELP_ANCHOR_RESERVE: &str =
	"Emergency on-chain reserve kept so the node can spend anchor outputs if one of its channels closes.";
const HELP_LIGHTNING_TOTAL: &str =
	"Total balance claimable across Lightning channels. This is not the same as immediately sendable capacity.";
const HELP_BALANCE_TYPE: &str = "The claim or balance state for this Lightning balance row.";
const HELP_BALANCE_CHANNEL: &str =
    "The channel ID associated with this balance or sweep when one is available.";
const HELP_BALANCE_AMOUNT: &str =
    "The funds represented by this row, shown in the selected display unit.";
const HELP_BALANCE_EXTRA: &str =
	"Additional block heights, txids, or timing details needed to understand when funds become spendable.";
const HELP_PENDING_SWEEP_BALANCES: &str =
    "On-chain outputs the node is sweeping from channel closures or claim transactions.";
const HELP_CLAIMABLE_ON_CHANNEL_CLOSE: &str =
	"Funds that could be claimed if the channel were force-closed now, less on-chain fees. This does not include unconfirmed splice changes.";
const HELP_AWAITING_CONFIRMATIONS: &str =
	"The channel is closed and this balance is ours, but it needs enough on-chain confirmations before becoming spendable.";
const HELP_CONTENTIOUS_CLAIMABLE: &str =
	"The channel is closed and this balance should be ours, but our spending transaction must confirm before a timeout that could let the counterparty claim it.";
const HELP_MAYBE_TIMEOUT_CLAIMABLE_HTLC: &str =
	"An HTLC we sent that may become claimable after its timeout if the counterparty does not claim it first with the preimage.";
const HELP_MAYBE_PREIMAGE_CLAIMABLE_HTLC: &str =
	"An HTLC we received that is claimable only if we learn and use the payment preimage before the timeout.";
const HELP_COUNTERPARTY_REVOKED_OUTPUT: &str =
	"The counterparty broadcast a revoked commitment transaction, allowing this node to claim penalty outputs from it.";
const HELP_PENDING_BROADCAST: &str =
    "The sweep transaction has been prepared or queued but is not yet confirmed on-chain.";
const HELP_BROADCAST_AWAITING_CONFIRMATION: &str =
    "The sweep transaction was broadcast and is waiting for its first confirmation.";
const HELP_AWAITING_THRESHOLD_CONFIRMATIONS: &str =
	"The sweep transaction is confirmed but needs more confirmations before the balance is considered safe or spendable.";

pub fn render(ui: &mut Ui, app: &mut LspServerApp) {
    ui.heading("Balances");
    ui.add_space(10.0);

    if app.render_disconnected_gate(ui) {
        return;
    }

    ui.horizontal(|ui| {
        if app.state.tasks.balances.is_some() {
            widgets::loading_row(ui, "Loading balances...");
        } else if ui.button("Refresh").clicked() {
            app.fetch_balances();
        }
    });

    ui.add_space(10.0);

    if app.state.balances.is_none() {
        widgets::empty_state(ui, "💰", "No balance data", "Click Refresh to load");
        return;
    }

    // Extract headline totals into locals before borrowing state for iteration.
    let total_onchain = app
        .state
        .balances
        .as_ref()
        .map(|b| b.total_onchain_balance_sats)
        .unwrap_or(0);
    let spendable = app
        .state
        .balances
        .as_ref()
        .map(|b| b.spendable_onchain_balance_sats)
        .unwrap_or(0);
    let reserve = app
        .state
        .balances
        .as_ref()
        .map(|b| b.total_anchor_channels_reserve_sats)
        .unwrap_or(0);
    let total_lightning = app
        .state
        .balances
        .as_ref()
        .map(|b| b.total_lightning_balance_sats)
        .unwrap_or(0);

    // Build a per-amount formatter once, before we borrow state.
    let fmt = |sats: u64| app.fmt_sats(sats);

    let onchain_val = fmt(spendable);
    let onchain_sec = format!("reserve {} | total {}", fmt(reserve), fmt(total_onchain));
    let ln_val = fmt(total_lightning);
    let onchain_total_str = fmt(total_onchain);
    let onchain_spendable_str = fmt(spendable);
    let onchain_reserve_str = fmt(reserve);
    let lightning_total_str = fmt(total_lightning);

    // Snapshot lightning-balance rows into owned strings before the scrolled closure.
    let lightning_rows: Vec<[String; 4]> = app
        .state
        .balances
        .as_ref()
        .map(|b| {
            b.lightning_balances
                .iter()
                .filter_map(|balance| {
                    balance
                        .balance_type
                        .as_ref()
                        .map(|bt| lightning_balance_row(bt, &fmt))
                })
                .collect()
        })
        .unwrap_or_default();

    let pending_sweeps: Vec<(usize, String, &'static str)> = app
        .state
        .balances
        .as_ref()
        .map(|b| {
            b.pending_balances_from_channel_closures
                .iter()
                .enumerate()
                .filter_map(|(i, sweep)| {
                    sweep.balance_type.as_ref().map(|bt| {
                        let (text, help) = pending_sweep_text(bt, &fmt);
                        (i, text, help)
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    page_scrolled(ui, |ui| {
        ui.columns(2, |cols| {
            widgets::stat_card_with_info(
                &mut cols[0],
                "On-chain Spendable",
                HELP_ONCHAIN_SPENDABLE,
                &onchain_val,
                &onchain_sec,
            );
            widgets::stat_card_with_info(
                &mut cols[1],
                "Lightning Total",
                HELP_LIGHTNING_TOTAL,
                &ln_val,
                "",
            );
        });

        ui.add_space(10.0);

        card(ui, "On-chain Balance", |ui| {
            let rows: crate::ui::layout::KvInfoRows = vec![
                (
                    "Total",
                    Some(HELP_ONCHAIN_TOTAL),
                    Box::new(|ui: &mut egui::Ui| {
                        ui.label(&onchain_total_str).on_hover_text(format!(
                            "{} sats",
                            crate::ui::format_sats(total_onchain)
                        ));
                    }),
                ),
                (
                    "Spendable",
                    Some(HELP_SPENDABLE),
                    Box::new(|ui: &mut egui::Ui| {
                        ui.label(&onchain_spendable_str)
                            .on_hover_text(format!("{} sats", crate::ui::format_sats(spendable)));
                    }),
                ),
                (
                    "Anchor Reserve",
                    Some(HELP_ANCHOR_RESERVE),
                    Box::new(|ui: &mut egui::Ui| {
                        ui.label(&onchain_reserve_str)
                            .on_hover_text(format!("{} sats", crate::ui::format_sats(reserve)));
                    }),
                ),
            ];
            kv_grid_custom_info(ui, "onchain_balance", rows);
        });

        ui.add_space(10.0);

        card(ui, "Lightning Balance", |ui| {
            let rows: crate::ui::layout::KvInfoRows = vec![(
                "Total",
                Some(HELP_LIGHTNING_TOTAL),
                Box::new(|ui: &mut egui::Ui| {
                    ui.label(&lightning_total_str)
                        .on_hover_text(format!("{} sats", crate::ui::format_sats(total_lightning)));
                }),
            )];
            kv_grid_custom_info(ui, "lightning_balance", rows);

            if !lightning_rows.is_empty() {
                ui.add_space(8.0);
                ui.label(format!("Details ({} items)", lightning_rows.len()));
                ui.add_space(4.0);
                crate::ui::layout::h_scroll(ui, 600.0, |ui| {
                    TableBuilder::new(ui)
                        .striped(true)
                        .resizable(false)
                        .vscroll(false)
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .auto_shrink([false, true])
                        .column(Column::remainder().at_least(64.0).clip(true)) // Type
                        .column(Column::remainder().at_least(64.0).clip(true)) // Channel
                        .column(Column::auto()) // Amount
                        .column(Column::remainder().at_least(64.0).clip(true)) // Extra
                        .header(22.0, |mut h| {
                            h.col(|ui| {
                                widgets::table_header_with_info(ui, "Type", HELP_BALANCE_TYPE);
                            });
                            h.col(|ui| {
                                widgets::table_header_with_info(
                                    ui,
                                    "Channel",
                                    HELP_BALANCE_CHANNEL,
                                );
                            });
                            h.col(|ui| {
                                widgets::table_header_with_info(ui, "Amount", HELP_BALANCE_AMOUNT);
                            });
                            h.col(|ui| {
                                widgets::table_header_with_info(ui, "Extra", HELP_BALANCE_EXTRA);
                            });
                        })
                        .body(|mut body| {
                            for row in &lightning_rows {
                                body.row(24.0, |mut r| {
                                    r.col(|ui| {
                                        let label = ui.label(&row[0]);
                                        if let Some(help) = lightning_balance_help(&row[0]) {
                                            label.on_hover_text(help);
                                        }
                                    });
                                    r.col(|ui| {
                                        ui.monospace(&row[1]);
                                    });
                                    r.col(|ui| {
                                        ui.monospace(&row[2]);
                                    });
                                    r.col(|ui| {
                                        ui.label(&row[3]);
                                    });
                                });
                            }
                        });
                });
            }
        });

        ui.add_space(10.0);

        if !pending_sweeps.is_empty() {
            card(ui, "Pending Sweep Balances", |ui| {
                widgets::label_with_info(ui, "Sweep outputs", HELP_PENDING_SWEEP_BALANCES);
                ui.add_space(4.0);
                for (i, text, help) in &pending_sweeps {
                    ui.group(|ui| {
                        widgets::label_with_info(ui, &format!("Sweep #{}", i + 1), help);
                        ui.label(text);
                    });
                }
            });
        }
    });
}

fn lightning_balance_row(
    balance: &sc_rest_client::ldk_server_grpc::types::lightning_balance::BalanceType,
    fmt: &dyn Fn(u64) -> String,
) -> [String; 4] {
    use sc_rest_client::ldk_server_grpc::types::lightning_balance::BalanceType;

    match balance {
        BalanceType::ClaimableOnChannelClose(b) => [
            "Claimable on Channel Close".to_string(),
            crate::ui::truncate_id(&b.channel_id, 8, 8),
            fmt(b.amount_satoshis),
            String::new(),
        ],
        BalanceType::ClaimableAwaitingConfirmations(b) => [
            "Awaiting Confirmations".to_string(),
            crate::ui::truncate_id(&b.channel_id, 8, 8),
            fmt(b.amount_satoshis),
            format!("Confirmation Height: {}", b.confirmation_height),
        ],
        BalanceType::ContentiousClaimable(b) => [
            "Contentious Claimable".to_string(),
            crate::ui::truncate_id(&b.channel_id, 8, 8),
            fmt(b.amount_satoshis),
            format!("Timeout Height: {}", b.timeout_height),
        ],
        BalanceType::MaybeTimeoutClaimableHtlc(b) => [
            "Maybe Timeout Claimable HTLC".to_string(),
            crate::ui::truncate_id(&b.channel_id, 8, 8),
            fmt(b.amount_satoshis),
            format!("Claimable Height: {}", b.claimable_height),
        ],
        BalanceType::MaybePreimageClaimableHtlc(b) => [
            "Maybe Preimage Claimable HTLC".to_string(),
            crate::ui::truncate_id(&b.channel_id, 8, 8),
            fmt(b.amount_satoshis),
            format!("Expiry Height: {}", b.expiry_height),
        ],
        BalanceType::CounterpartyRevokedOutputClaimable(b) => [
            "Counterparty Revoked Output".to_string(),
            crate::ui::truncate_id(&b.channel_id, 8, 8),
            fmt(b.amount_satoshis),
            String::new(),
        ],
    }
}

fn pending_sweep_text(
    balance: &sc_rest_client::ldk_server_grpc::types::pending_sweep_balance::BalanceType,
    fmt: &dyn Fn(u64) -> String,
) -> (String, &'static str) {
    use sc_rest_client::ldk_server_grpc::types::pending_sweep_balance::BalanceType;

    match balance {
        BalanceType::PendingBroadcast(b) => {
            let ch_line = b
                .channel_id
                .as_ref()
                .map(|c| format!("Channel: {}\n", crate::ui::truncate_id(c, 8, 8)))
                .unwrap_or_default();
            (
                format!(
                    "Type: Pending Broadcast\n{}Amount: {}",
                    ch_line,
                    fmt(b.amount_satoshis)
                ),
                HELP_PENDING_BROADCAST,
            )
        }
        BalanceType::BroadcastAwaitingConfirmation(b) => {
            let ch_line = b
                .channel_id
                .as_ref()
                .map(|c| format!("Channel: {}\n", crate::ui::truncate_id(c, 8, 8)))
                .unwrap_or_default();
            (
                format!(
                    "Type: Broadcast Awaiting Confirmation\n{}Amount: {}\nTXID: {}",
                    ch_line,
                    fmt(b.amount_satoshis),
                    crate::ui::truncate_id(&b.latest_spending_txid, 8, 8)
                ),
                HELP_BROADCAST_AWAITING_CONFIRMATION,
            )
        }
        BalanceType::AwaitingThresholdConfirmations(b) => {
            let ch_line = b
                .channel_id
                .as_ref()
                .map(|c| format!("Channel: {}\n", crate::ui::truncate_id(c, 8, 8)))
                .unwrap_or_default();
            (
                format!(
                    "Type: Awaiting Threshold Confirmations\n{}Amount: {}\nConfirmed at height: {}",
                    ch_line,
                    fmt(b.amount_satoshis),
                    b.confirmation_height
                ),
                HELP_AWAITING_THRESHOLD_CONFIRMATIONS,
            )
        }
    }
}

fn lightning_balance_help(label: &str) -> Option<&'static str> {
    match label {
        "Claimable on Channel Close" => Some(HELP_CLAIMABLE_ON_CHANNEL_CLOSE),
        "Awaiting Confirmations" => Some(HELP_AWAITING_CONFIRMATIONS),
        "Contentious Claimable" => Some(HELP_CONTENTIOUS_CLAIMABLE),
        "Maybe Timeout Claimable HTLC" => Some(HELP_MAYBE_TIMEOUT_CLAIMABLE_HTLC),
        "Maybe Preimage Claimable HTLC" => Some(HELP_MAYBE_PREIMAGE_CLAIMABLE_HTLC),
        "Counterparty Revoked Output" => Some(HELP_COUNTERPARTY_REVOKED_OUTPUT),
        _ => None,
    }
}
