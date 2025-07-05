//! Lightning Dashboard (UI draft)
//! -------------------------------------------------------------
//! REST backend partially wired – balance and channels use real
//! network requests, the rest are stubbed out for now.

use eframe::{egui, App, NativeOptions};
use egui::{RichText, CollapsingHeader};
use futures_util::FutureExt; // now_or_never
use reqwest::Client;
use serde::Deserialize;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

/* ---------- DTOs ------------------------------------------------ */

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

#[derive(Debug, Clone, Deserialize, Default)]
struct PaymentInfo {
    amount_msat: u64,
    direction:   String,
    status:      String,
    timestamp:   String,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct InvoiceInfo {
    amount_sats: u64,
    bolt11:      String,
    paid:        bool,
    timestamp:   String,
}

/* ---------- GUI State ------------------------------------------ */

struct Dashboard {
    rt: Runtime,
    client: Client,

    bal_task:      Option<JoinHandle<reqwest::Result<Balance>>>,
    ch_task:       Option<JoinHandle<reqwest::Result<Vec<ChannelInfo>>>>,
    price_task:    Option<JoinHandle<reqwest::Result<f64>>>,
    payments_task: Option<JoinHandle<reqwest::Result<Vec<PaymentInfo>>>>,
    invoices_task: Option<JoinHandle<reqwest::Result<Vec<InvoiceInfo>>>>,
    logs_task:     Option<JoinHandle<reqwest::Result<String>>>,

    balance:  Option<Balance>,
    channels: Vec<ChannelInfo>,
    price_usd: Option<f64>,
    payments: Vec<PaymentInfo>,
    invoices: Vec<InvoiceInfo>,
    log_tail: String,

    status_msg: String,

    invoice_amount: String,
    invoice_result: String,
    invoice_to_pay: String,

    open_channel_pubkey: String,
    open_channel_address: String,
    open_channel_sats: String,

    close_channel_id: String,

    onchain_address: String,
    onchain_amount: String,

    show_logs: bool,
    last_log_refresh: Instant,
}

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "LSP Dashboard",
        NativeOptions::default(),
        Box::new(|cc| Ok(Box::new(Dashboard::new(cc)))),
    )
}

impl Dashboard {
    fn new(_: &eframe::CreationContext<'_>) -> Self {
        Self {
            rt: Runtime::new().expect("Tokio runtime"),
            client: Client::new(),

            bal_task: None,
            ch_task: None,
            price_task: None,
            payments_task: None,
            invoices_task: None,
            logs_task: None,

            balance: None,
            channels: Vec::new(),
            price_usd: None,
            payments: Vec::new(),
            invoices: Vec::new(),
            log_tail: String::new(),

            status_msg: String::new(),

            invoice_amount: "1000".into(),
            invoice_result: String::new(),
            invoice_to_pay: String::new(),

            open_channel_pubkey: String::new(),
            open_channel_address: "127.0.0.1:9737".into(),
            open_channel_sats: "100000".into(),

            close_channel_id: String::new(),

            onchain_address: String::new(),
            onchain_amount: "10000".into(),

            show_logs: false,
            last_log_refresh: Instant::now(),
        }
    }

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

    fn fetch_price(&mut self) {
        if self.price_task.is_some() { return; }
        let client = self.client.clone();
        self.price_task = Some(self.rt.spawn(async move {
            let resp = client
                .get("http://127.0.0.1:8080/api/price")
                .send()
                .await?;
            let price = resp.json::<f64>().await?;
            Ok(price)
        }));
    }

    fn fetch_payments(&mut self) {
        if self.payments_task.is_some() { return; }
        self.payments_task = Some(self.rt.spawn(async move {
            // STUB: GET /api/payments
            Ok(Vec::<PaymentInfo>::new())
        }));
    }

    fn fetch_invoices(&mut self) {
        if self.invoices_task.is_some() { return; }
        self.invoices_task = Some(self.rt.spawn(async move {
            // STUB: GET /api/invoices
            Ok(Vec::<InvoiceInfo>::new())
        }));
    }

    fn fetch_logs(&mut self) {
        if self.logs_task.is_some() { return; }
        self.logs_task = Some(self.rt.spawn(async move {
            // STUB: GET /api/logs
            Ok(String::from("(log output placeholder)"))
        }));
    }

    fn show_balance(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.heading("Balances");
            match &self.balance {
                Some(bal) => ui.label(format!("{} sats  ≈ ${:.2}", bal.sats, bal.usd)),
                None => ui.label("Balance: —"),
            };
            if let Some(p) = self.price_usd {
                ui.label(format!("BTC/USD price: ${:.2}", p));
            }
            if ui.button("Refresh").clicked() {
                self.fetch_balance();
                self.fetch_price();
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

    // ---- stub API endpoints ----

    fn fetch_channel_details(&self, id: &str) {
        // TODO: GET /api/channels/{id}
    }

    fn open_channel_stub(&self, peer_pubkey: &str, sat_amount: u64, push_msat: Option<u64>) {
        // TODO: POST /api/channels
    }

    fn delete_channel_stub(&self, id: &str, force: bool) {
        // TODO: DELETE /api/channels/{id}
    }

    fn fetch_payments_stub(&self) {
        // TODO: GET /api/payments
    }

    fn send_payment_stub(&self, bolt11_invoice: &str) {
        // TODO: POST /api/payments
    }

    fn fetch_invoices_stub(&self) {
        // TODO: GET /api/invoices
    }

    fn create_invoice_stub(&self, amount_sats: u64, description: &str) {
        // TODO: POST /api/invoices
    }

    fn fetch_price_stub(&self) {
        // TODO: GET /api/price
    }

    fn fetch_logs_stub(&self) {
        // TODO: GET /api/logs
    }
}

impl App for Dashboard {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        macro_rules! poll_task {
            ($slot:ident => $target:expr) => {
                if let Some(t) = &mut self.$slot {
                    if let Some(res) = t.now_or_never() {
                        self.$slot = None;
                        match res {
                            Ok(Ok(val)) => $target(val),
                            Ok(Err(e)) => self.status_msg = e.to_string(),
                            Err(join_err) => self.status_msg = join_err.to_string(),
                        }
                    } else {
                        ctx.request_repaint();
                    }
                }
            }
        }
        poll_task!(bal_task => |v| self.balance = Some(v));
        poll_task!(ch_task => |v| self.channels = v);
        poll_task!(price_task => |v| self.price_usd = Some(v));
        poll_task!(payments_task => |v| self.payments = v);
        poll_task!(invoices_task => |v| self.invoices = v);
        poll_task!(logs_task => |v| self.log_tail = v);

        egui::CentralPanel::default().show(ctx, |ui| {
            self.show_balance(ui);
            ui.add_space(10.0);
            self.show_channels(ui);
        });

        if self.balance.is_none() && self.bal_task.is_none() {
            self.fetch_balance();
        }
        if self.channels.is_empty() && self.ch_task.is_none() {
            self.fetch_channels();
        }
        if self.price_usd.is_none() && self.price_task.is_none() {
            self.fetch_price();
        }

        ctx.request_repaint_after(Duration::from_millis(100));
    }
}
