use eframe::{egui, App, NativeOptions};
use egui::{RichText, CollapsingHeader};
use futures_util::FutureExt; // now_or_never
use reqwest::Client;
use serde::{Deserialize, Serialize};
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
    local_balance_usd:  f64,
    remote_balance_sats: u64,
    remote_balance_usd:  f64,
    status: String,
    is_channel_ready: bool,  
    is_usable: bool,         
    is_stable: bool,   
    expected_usd: Option<f64>,
    note: Option<String>,
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

#[derive(Debug, Clone, Deserialize, Default)]
struct EditStableChannelRes {
    ok: bool,
    status: String,
}

#[derive(Debug, Clone, Serialize)]
struct EditStableChannelReq {
    channel_id: String,
    target_usd: Option<String>,
    note: Option<String>,
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
    edit_task: Option<JoinHandle<reqwest::Result<EditStableChannelRes>>>,
    close_task:     Option<JoinHandle<reqwest::Result<String>>>,
    pay_task:      Option<JoinHandle<reqwest::Result<String>>>,
    onchain_send_task:    Option<JoinHandle<reqwest::Result<String>>>,
    onchain_send_result:  Option<String>,
    pay_result:    Option<String>,
    close_result:   Option<String>,      
    get_address_task: Option<JoinHandle<reqwest::Result<String>>>,
    connect_task:  Option<JoinHandle<reqwest::Result<String>>>,
    connect_result: Option<String>,   


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
    edit_channel_id: String,
    edit_channel_usd: String,
    edit_channel_note: String, 
    edit_stable_result: Option<String>,
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
            close_task: None,
            close_result: None,
            pay_task: None,
            pay_result: None,
            connect_task:      None, 
            connect_result:     None,

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
            open_channel_address: "100.25.168.115:9737".into(),
            open_channel_sats: "100000".into(),
            close_channel_id: String::new(),

            onchain_address: String::new(),
            onchain_amount: "10000".into(),

            show_logs: false,
            last_log_refresh: Instant::now(),
            edit_channel_id: String::new(),
            edit_channel_usd: String::new(),
            edit_channel_note: String::new(),
            edit_stable_result: None,
            edit_task: None,
            onchain_send_task: None,
            onchain_send_result:  None,
            get_address_task: None,

        }
    }

    fn fetch_balance(&mut self) {
        if self.bal_task.is_some() { return; }
        let client = self.client.clone();
        self.bal_task = Some(self.rt.spawn(async move {
            client
                .get("http://100.25.168.115:8080/api/balance")
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
                .get("http://100.25.168.115:8080/api/channels")
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
                .get("http://100.25.168.115:8080/api/price")
                .send()
                .await?;
            let price = resp.json::<f64>().await?;
            Ok(price)
        }));
    }

    fn fetch_onchain_address(&mut self) {
        if self.get_address_task.is_some() { return; }
        let client = self.client.clone();
        self.get_address_task = Some(self.rt.spawn(async move {
            client
                .get("http://100.25.168.115:8080/api/onchain_address")
                .send()
                .await?
                .json::<String>()
                .await
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
        use egui::{RichText, ScrollArea};
    
        fn short(s: &str, n: usize) -> String {
            if s.len() > n { format!("{}…", &s[..n]) } else { s.to_owned() }
        }
    
        ui.group(|ui| {
            ui.heading("Channels");
            if ui.button("Refresh Channels").clicked() {
                self.fetch_channels();
            }
    
            ScrollArea::both()
                .max_height(160.0)
                .auto_shrink([true; 2])
                .show(ui, |ui| {
                    egui::Grid::new("channel_table")
                        .striped(true)
                        .min_col_width(50.0)
                        .show(ui, |ui| {
                            // ── headers ───────────────────────────────────────────
                            for h in [
                                "Notes","ID", "Peer", "Capacity",
                                "Local", "USD",           // local sats / local USD
                                "Remote", "USD",          // remote sats / remote USD
                                "Status", "Ready", "Usable", "Stable $"
                            ] {
                                ui.label(RichText::new(h).strong().small());
                            }
                            ui.end_row();
    
                            // ── rows ─────────────────────────────────────────────
                            for ch in &self.channels {
                                // Note (copy)
                                let note_text = ch.note.clone().unwrap_or_else(|| "---".to_string());

                                
                                ui.horizontal(|ui| {
                                    ui.label(note_text.clone());
                                    if ui.button("📋").clicked() {
                                        ui.output_mut(|o| o.copied_text = note_text);
                                    }
                                });

                                // ID (copy)
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(short(&ch.id, 8)).monospace());
                                    if ui.small_button("⧉").on_hover_text("Copy full ID").clicked() {
                                        ui.output_mut(|o| o.copied_text = ch.id.clone());
                                    }
                                });
    
                                // Peer (copy)
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(short(&ch.remote_pubkey, 8)).monospace());
                                    if ui.small_button("⧉").on_hover_text("Copy full peer key").clicked() {
                                        ui.output_mut(|o| o.copied_text = ch.remote_pubkey.clone());
                                    }
                                });
    
                                ui.label(ch.capacity_sats.to_string());
    
                                // Local sats + USD
                                ui.label(ch.local_balance_sats.to_string());
                                ui.label(format!("{:.2}", ch.local_balance_usd));
    
                                // Remote sats + USD
                                ui.label(ch.remote_balance_sats.to_string());
                                ui.label(format!("{:.2}", ch.remote_balance_usd));
    
                                ui.label(&ch.status);
                                ui.label(ch.is_channel_ready.to_string());
                                ui.label(ch.is_usable.to_string());
    
                                // Stable target USD (Option<f64>)
                                ui.label(
                                    ch.expected_usd
                                        .map(|v| format!("{:.2}", v))
                                        .unwrap_or_else(|| "n/a".into()),
                                );
    
                                ui.end_row();
                            }
                        });
                });


        });
    }
    
    fn edit_stable_channel(&mut self) {
        if self.edit_task.is_some() {
            return;
        }
    
        let client = self.client.clone();
        let channel_id = self.edit_channel_id.trim().to_string();
        let target_usd = self.edit_channel_usd.trim().to_string();
        let note = self.edit_channel_note.trim().to_string();
    
        self.edit_task = Some(self.rt.spawn(async move {
            let req = EditStableChannelReq {
                channel_id,
                target_usd: if target_usd.is_empty() { None } else { Some(target_usd) },
                note: if note.is_empty() { None } else { Some(note) },
            };
            client
                .post("http://100.25.168.115:8080/api/edit_stable_channel")
                .json(&req)
                .send()
                .await?
                .json::<EditStableChannelRes>()
                .await
        }));
    }

    fn close_specific_channel(&mut self) {
        if self.close_task.is_some() { return; }
        let id = self.close_channel_id.trim().to_string();
        if id.is_empty() { return; }
    
        self.close_channel_id.clear();              // clear box immediately
        let client = self.client.clone();
        self.close_task = Some(self.rt.spawn(async move {
            client
                .post(format!("http://100.25.168.115:8080/api/close_channel/{}", id))
                .send()
                .await?
                .text()
                .await
        }));
    }

    fn pay_invoice(&mut self) {
        if self.pay_task.is_some() { return; }
        let inv = self.invoice_to_pay.trim().to_string();
        if inv.is_empty() { return; }
    
        self.invoice_to_pay.clear();           // clear textbox
        let client = self.client.clone();
        self.pay_task = Some(self.rt.spawn(async move {
            #[derive(Serialize)] struct Req { invoice: String }
            client
                .post("http://100.25.168.115:8080/api/pay")
                .json(&Req { invoice: inv })
                .send()
                .await?
                .json::<String>()              // backend returns status string
                .await
        }));
    }

    fn send_onchain(&mut self) {
        if self.onchain_send_task.is_some() { return; }
        let addr  = self.onchain_address.trim().to_string();
        let amt   = self.onchain_amount.trim().to_string();
        if addr.is_empty() || amt.is_empty() { return; }
    
        self.onchain_address.clear();
        self.onchain_amount.clear();
    
        let client = self.client.clone();
        #[derive(Serialize)] struct Req { address: String, amount: String }
        self.onchain_send_task = Some(self.rt.spawn(async move {
            client
                .post("http://100.25.168.115:8080/api/onchain_send")
                .json(&Req { address: addr, amount: amt })
                .send()
                .await?
                .json::<String>()
                .await
        }));
    }

    fn show_onchain_address_section(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.heading("On-chain Address");
            if ui.button("Get Address").clicked() {
                self.fetch_onchain_address();
            }
    
            if !self.onchain_address.is_empty() {
                ui.label(&self.onchain_address);
                if ui.button("Copy").clicked() {
                    ui.output_mut(|o| o.copied_text = self.onchain_address.clone());
                }
            }
        });
    }

    fn connect_to_node(&mut self) {
        if self.connect_task.is_some() { return; }
    
        let node_id = self.open_channel_pubkey.trim().to_owned();
        let address = self.open_channel_address.trim().to_owned();
        if node_id.is_empty() || address.is_empty() { return; }
    
        let client = self.client.clone();
        #[derive(Serialize)] struct Req { node_id: String, address: String }
    
        self.connect_task = Some(self.rt.spawn(async move {
            client
                .post("http://100.25.168.115:8080/api/connect")
                .json(&Req { node_id, address })
                .send()
                .await?
                .json::<String>()        // <— now just a String
                .await
        }));
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
        poll_task!(edit_task => |res: EditStableChannelRes| {
            self.edit_stable_result = Some(res.status);
        });
        poll_task!(close_task => |v| self.close_result = Some(v));
        poll_task!(pay_task => |v| self.pay_result = Some(v));
        poll_task!(onchain_send_task => |v| self.onchain_send_result = Some(v));
        poll_task!(get_address_task => |addr| self.onchain_address = addr);
        poll_task!(connect_task => |v| self.connect_result = Some(v));

        egui::CentralPanel::default().show(ctx, |ui| {
            self.show_balance(ui);
            ui.add_space(10.0);
            self.show_channels(ui);
            ui.group(|ui| {
                ui.heading("Edit Stable Channel");
            
                ui.horizontal(|ui| {
                    ui.label("Channel ID:");
                    ui.text_edit_singleline(&mut self.edit_channel_id);
                });
            
                ui.horizontal(|ui| {
                    ui.label("Target USD amount:");
                    ui.text_edit_singleline(&mut self.edit_channel_usd);
                });
            
                ui.horizontal(|ui| {
                    ui.label("Note:");
                    ui.text_edit_singleline(&mut self.edit_channel_note);
                });
            
                if ui.button("Submit Edits").clicked() {
                    self.edit_stable_channel();
                }
            
                if let Some(msg) = &self.edit_stable_result {
                    ui.label(msg);
                }
            });

            ui.add_space(10.0);
            self.show_onchain_address_section(ui);
            ui.add_space(10.0);

            ui.group(|ui| {
                ui.heading("On-chain Send");
                ui.horizontal(|ui| {
                    ui.label("Address:");
                    ui.text_edit_singleline(&mut self.onchain_address);
                });
                ui.horizontal(|ui| {
                    ui.label("Amount (sats):");
                    ui.text_edit_singleline(&mut self.onchain_amount);
                });
                if ui.button("Send On-chain").clicked() {
                    self.send_onchain();
                }
                if let Some(msg) = &self.onchain_send_result {
                    ui.label(msg);
                }
            });

            ui.add_space(10.0);
            ui.group(|ui| {
                ui.heading("Pay Invoice");
                ui.text_edit_multiline(&mut self.invoice_to_pay);
                if ui.button("Pay Invoice").clicked() {
                    self.pay_invoice();
                }
                if let Some(msg) = &self.pay_result {
                    ui.label(msg);
                }
            });
            ui.add_space(10.0);

            ui.group(|ui| {
                ui.heading("Connect to Node");
                ui.horizontal(|ui| {
                    ui.label("Node ID:");
                    ui.text_edit_singleline(&mut self.open_channel_pubkey);   // reuse existing field
                });
                ui.horizontal(|ui| {
                    ui.label("Address:");
                    ui.text_edit_singleline(&mut self.open_channel_address);  // reuse existing field
                });
                if ui.button("Connect").clicked() {
                    self.connect_to_node();
                }
                if let Some(msg) = &self.connect_result {
                    ui.label(msg);
                }
            });

            
            ui.group(|ui| {
                ui.heading("Close Specific Channel");
                ui.horizontal(|ui| {
                    ui.label("Channel ID:");
                    ui.text_edit_singleline(&mut self.close_channel_id);
                    if ui.button("Close Channel").clicked() {
                        self.close_specific_channel();
                    }
                });
                if let Some(msg) = &self.close_result {
                    ui.label(msg);
                }
            });
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
