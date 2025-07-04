//! Native dashboard that talks to the local LSP backend (http://127.0.0.1:8080)
//! Uses only the REST API – **no direct LightningNode handle** inside the GUI.
//! Balance →  GET /api/balance
//! Channels → GET /api/channels

use eframe::{egui, App, NativeOptions};
use egui::RichText;
use futures_util::FutureExt; // for now_or_never
use reqwest::Client;
use serde::Deserialize;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

/* ---------- DTOs from the backend ----------------------------- */

#[derive(Debug, Clone, Deserialize, Default)]
struct Balance {
    sats: u64,
    usd:  f64,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ChannelInfo {
    id: String,
    remote_pubkey: String,
    capacity_sats: u64,
    local_balance_sats: u64,
    remote_balance_sats: u64,
    status: String,
}

/* ---------- GUI application state ----------------------------- */

struct Dashboard {
    /* —— async plumbing —— */
    rt: Runtime,
    client: Client,

    /* pending tasks */
    bal_task: Option<JoinHandle<reqwest::Result<Balance>>>,
    ch_task:  Option<JoinHandle<reqwest::Result<Vec<ChannelInfo>>>>,

    /* data */
    balance: Option<Balance>,
    channels: Vec<ChannelInfo>,
    error: Option<String>,
}

/* ---------- entry point --------------------------------------- */

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "LSP Dashboard",
        NativeOptions::default(),
        Box::new(|cc| Ok(Box::new(Dashboard::new(cc)))),
    )
}

/* ---------- impl Dashboard ------------------------------------ */

impl Dashboard {
    fn new(_: &eframe::CreationContext<'_>) -> Self {
        Self {
            rt: Runtime::new().expect("Tokio runtime"),
            client: Client::new(),

            bal_task: None,
            ch_task: None,

            balance: None,
            channels: Vec::new(),
            error: None,
        }
    }

    /* ---- async helpers -------------------------------------- */

    fn fetch_balance(&mut self) {
        if self.bal_task.is_some() { return; }
        let client = self.client.clone();
        self.bal_task = Some(self.rt.spawn(async move {
            client
                .get("http://127.0.0.1:8080/api/balance")
                .send()
                .await?
                .json::<Balance>()
                .await
        }));
    }

    fn fetch_channels(&mut self) {
        if self.ch_task.is_some() { return; }
        let client = self.client.clone();
        self.ch_task = Some(self.rt.spawn(async move {
            client
                .get("http://127.0.0.1:8080/api/channels")
                .send()
                .await?
                .json::<Vec<ChannelInfo>>()
                .await
        }));
    }

    /* ---- egui helpers --------------------------------------- */

    fn show_balance(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.heading("Balances");
            ui.add_space(4.0);

            match &self.balance {
                Some(bal) => ui.label(format!("{} sats  ≈ ${:.2}", bal.sats, bal.usd)),
                None => ui.label("Balance: —"),
            };

            if let Some(err) = &self.error {
                ui.colored_label(egui::Color32::RED, err);
            }

            if ui.button("Refresh Balance").clicked() {
                self.balance = None;
                self.error = None;
                self.fetch_balance();
            }
        });
    }

    fn show_channels(&mut self, ui: &mut egui::Ui) {
        use egui::ScrollArea;

        ui.group(|ui| {
            ui.heading("Channels");
            if ui.button("Refresh Channels").clicked() {
                self.fetch_channels();
            }

            ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                if self.channels.is_empty() {
                    ui.label("(no channels)");
                } else {
                    egui::Grid::new("channels").striped(true).show(ui, |ui| {
                        ui.label(RichText::new("ID").strong());
                        ui.label(RichText::new("Peer").strong());
                        ui.label(RichText::new("Capacity").strong());
                        ui.label(RichText::new("Local").strong());
                        ui.label(RichText::new("Remote").strong());
                        ui.label(RichText::new("Status").strong());
                        ui.end_row();

                        for c in &self.channels {
                            ui.label(&c.id[..8.min(c.id.len())]);
                            ui.label(&c.remote_pubkey[..8.min(c.remote_pubkey.len())]);
                            ui.label(c.capacity_sats.to_string());
                            ui.label(c.local_balance_sats.to_string());
                            ui.label(c.remote_balance_sats.to_string());
                            ui.label(&c.status);
                            ui.end_row();
                        }
                    });
                }
            });
        });
    }
}

/* ---------- eframe glue -------------------------------------- */

impl App for Dashboard {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        /* ---- poll balance task ---- */
        if let Some(t) = &mut self.bal_task {
            if let Some(res) = t.now_or_never() {
                self.bal_task = None;
                match res {
                    Ok(Ok(bal)) => self.balance = Some(bal),
                    Ok(Err(e)) => self.error = Some(e.to_string()),
                    Err(join) => self.error = Some(join.to_string()),
                }
            } else {
                ctx.request_repaint();
            }
        }

        /* ---- poll channels task ---- */
        if let Some(t) = &mut self.ch_task {
            if let Some(res) = t.now_or_never() {
                self.ch_task = None;
                match res {
                    Ok(Ok(chs)) => self.channels = chs,
                    Ok(Err(e)) => self.error = Some(e.to_string()),
                    Err(join) => self.error = Some(join.to_string()),
                }
            } else {
                ctx.request_repaint();
            }
        }

        /* ---- UI ---- */
        egui::CentralPanel::default().show(ctx, |ui| {
            self.show_balance(ui);
            ui.add_space(10.0);
            self.show_channels(ui);
        });

        /* ---- kick off first fetches ---- */
        if self.balance.is_none() && self.error.is_none() && self.bal_task.is_none() {
            self.fetch_balance();
        }
        if self.channels.is_empty() && self.error.is_none() && self.ch_task.is_none() {
            self.fetch_channels();
        }
    }
}
