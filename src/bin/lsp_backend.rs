use axum::http;
use axum::extract::Path as AxumPath;
use ldk_node::{
    bitcoin::{Network, Address, secp256k1::PublicKey},
    lightning_invoice::{Bolt11Invoice, Description, Bolt11InvoiceDescription},
    lightning::ln::msgs::SocketAddress,
    config::ChannelConfig,
    Builder, Node, Event, liquidity::LSPS2ServiceConfig
};
use std::{sync::Mutex, time::{Duration, Instant}};
use std::path::Path as FilePath;
use std::str::FromStr;
use std::sync::Arc;
use serde::{Serialize, Deserialize};
use serde_json::json;
use std::fs;
use hex;
use once_cell::sync::Lazy;

// HTTP
use axum::{routing::{get, post}, Json, Router};
use anyhow::Result;

use stable_channels::audit::{audit_event, set_audit_log_path};
use stable_channels::types::*;
use stable_channels::stable;
use stable_channels::price_feeds::get_cached_price;

static APP: Lazy<Mutex<ServerApp>> = Lazy::new(|| {
    Mutex::new(ServerApp::new_with_mode("lsp"))
});

const LSP_DATA_DIR: &str = "data/lsp";
const LSP_NODE_ALIAS: &str = "lsp";
const LSP_PORT: u16 = 9737;

const DEFAULT_NETWORK: &str = "bitcoin";
const DEFAULT_CHAIN_SOURCE_URL: &str = "https://blockstream.info/api/";
const EXPECTED_USD: f64 = 100.0;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct StableChannelEntry {
    channel_id: String,
    expected_usd: f64,
    native_btc: f64,
}
pub struct ServerApp {
    // core + balances …
    node: Arc<Node>,
    btc_price: f64,
    status_message: String,
    last_update: Instant,
    last_stability_check: Instant,

    lightning_balance_btc: f64,
    onchain_balance_btc:    f64,
    total_balance_btc:      f64,
    lightning_balance_usd:  f64,
    onchain_balance_usd:    f64,
    total_balance_usd:      f64,

    // invoice / on-chain helpers
    invoice_amount:   String,
    invoice_result:   String,
    invoice_to_pay:   String,
    on_chain_address: String,
    on_chain_amount:  String,

    // channel-management helpers that methods still use
    open_channel_node_id: String,
    open_channel_address: String,
    open_channel_amount:  String,

    connect_node_id:     String,
    connect_node_address:String,

    channel_id_to_close: String,

    selected_channel_id:   String,
    stable_channel_amount: String,

    // stable-channel bookkeeping
    stable_channels: Vec<StableChannel>,
}


// API responses
#[derive(Serialize)]
pub struct ChannelInfo {
    pub id: String,
    pub remote_pubkey: String,
    pub capacity_sats: u64,
    pub local_balance_sats: u64,
    pub local_balance_usd:  f64,
    pub remote_balance_sats: u64,
    pub remote_balance_usd:  f64,
    pub status: String,
    pub is_channel_ready: bool,  
    pub is_usable: bool,         
    pub is_stable: bool,   
    pub expected_usd: Option<f64>,
}

#[derive(Serialize)]
struct Balance { sats: u64, usd: f64 }

#[derive(Deserialize)]
struct PayReq { invoice: String }

#[derive(Deserialize)]
struct DesignateStableChannelReq {
    channel_id: String,
    target_usd: String,
}

#[derive(Serialize)]
struct DesignateStableChannelRes {
    ok: bool,
    status: String,
}

#[derive(Deserialize)]
struct OnchainSendReq {
    address: String,
    amount:  String,   // sats, still as string for reuse
}


#[tokio::main]
async fn main() -> Result<()> {
    // ── periodic upkeep task ───────────────────────────────────────────────
    tokio::spawn(async {
        loop {
            {
                let mut app = APP.lock().unwrap();

                app.poll_events();

                if app.last_update.elapsed() >= Duration::from_secs(30) {
                    let p = get_cached_price();
                    if p > 0.0 { app.btc_price = p; }
                    app.update_balances();
                    app.last_update = Instant::now();
                }

                if app.last_stability_check.elapsed() >= Duration::from_secs(30) {
                    app.check_and_update_stable_channels();
                    app.last_stability_check = Instant::now();
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    // ── HTTP router & server ───────────────────────────────────────────────
    let app = Router::new()
        .route("/api/balance", get(get_balance))
        .route("/api/pay",     post(pay_handler))
        .route("/api/channels", get(get_channels))
        .route("/api/price", get(get_price))
        .route("/api/close_channel/{id}", post(post_close_channel))
        .route("/api/designate_stable_channel", post(designate_stable_channel_handler))
        .route("/api/onchain_send", post(onchain_send_handler))
        .route("/api/onchain_address", get(get_onchain_address));
;

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    println!("Backend running at http://0.0.0.0:8080");
    axum::serve(listener, app).await?;
    Ok(())
}

/* ---- handlers ---------------------------------------------------- */

async fn get_balance() -> Json<Balance> {
    let (sats, usd) = {
        let mut app = APP.lock().unwrap();

        // pull latest BTC price + node balances (just in case the 30-sec loop
        // hasn’t fired yet)
        app.update_balances();

        (
            (app.total_balance_btc * 100_000_000.0) as u64, // convert BTC → sats
            app.total_balance_usd,
        )
    };

    Json(Balance { sats, usd })
}

/// GET /api/channels
pub async fn get_channels() -> Json<Vec<ChannelInfo>> {
    let app = APP.lock().expect("APP mutex poisoned");
    let price = app.btc_price;                       // cache once

    let out: Vec<ChannelInfo> = app
        .node
        .list_channels()
        .into_iter()
        .map(|c| {
            let is_stable = app
                .stable_channels
                .iter()
                .find(|sc| sc.channel_id == c.channel_id);

            let expected_usd = is_stable.map(|sc| sc.expected_usd.0);

            let local_sat   = c.outbound_capacity_msat / 1_000;
            let remote_sat  = c.inbound_capacity_msat / 1_000;

            ChannelInfo {
                id: hex::encode(c.channel_id.0),
                remote_pubkey: c.counterparty_node_id.to_string(),
                capacity_sats: c.channel_value_sats,

                local_balance_sats:  local_sat,
                local_balance_usd:   local_sat as f64 / 100_000_000.0 * price,

                remote_balance_sats: remote_sat,
                remote_balance_usd:  remote_sat as f64 / 100_000_000.0 * price,

                expected_usd,        // Some(x) or None
                status: if c.is_channel_ready { "open".into() } else { "pending".into() },
                is_channel_ready: c.is_channel_ready,
                is_usable:       c.is_usable,
                is_stable:       is_stable.is_some(),
            }
        })
        .collect();

    Json(out)
}


/// GET /api/price
async fn get_price() -> Json<f64> {
        let price = get_cached_price();
        Json(price)
}

/// POST /api/close_channel
async fn post_close_channel(AxumPath(id): AxumPath<String>) -> String {
    let mut app = APP.lock().unwrap();
    for chan in app.node.list_channels() {
        if hex::encode(chan.channel_id.0) == id {
            let res = app.node.close_channel(&chan.user_channel_id, chan.counterparty_node_id);
            return match res {
                Ok(_) => format!("Closing channel {}", id),
                Err(e) => format!("Error closing channel {}: {}", id, e),
            };
        }
    }
    format!("Channel {} not found", id)
}

async fn designate_stable_channel_handler(Json(req): Json<DesignateStableChannelReq>) -> Json<DesignateStableChannelRes> {
    println!("hi");
    let mut app = APP.lock().unwrap();
    app.selected_channel_id = req.channel_id;
    app.stable_channel_amount = req.target_usd;
    app.designate_stable_channel();
    Json(DesignateStableChannelRes {
        ok: app.status_message.starts_with("Channel") || app.status_message.contains("stable"),
        status: app.status_message.clone(),
    })
}

async fn pay_handler(Json(req): Json<PayReq>) -> Json<String> {
    let mut app = APP.lock().unwrap();
    app.invoice_to_pay = req.invoice;
    let ok = app.pay_invoice();
    Json(app.status_message.clone())
}

async fn onchain_send_handler(Json(req): Json<OnchainSendReq>) -> Json<String> {
    let mut app = APP.lock().unwrap();
    app.on_chain_address = req.address;
    app.on_chain_amount  = req.amount;
    app.send_onchain();                    // updates status_message
    Json(app.status_message.clone())
}

async fn get_onchain_address() -> Json<String> {
    let mut app = APP.lock().unwrap();
    if app.get_address() {
        Json(app.on_chain_address.clone())
    } else {
        Json(app.status_message.clone())
    }
}



impl ServerApp {
    pub fn new_with_mode(mode: &str) -> Self {
        let (data_dir, node_alias, port) = match mode.to_lowercase().as_str() {
            "lsp" => (LSP_DATA_DIR, LSP_NODE_ALIAS, LSP_PORT),
            _ => panic!("Invalid mode"),
        };

        let mut builder = Builder::new();

        let network = match DEFAULT_NETWORK.to_lowercase().as_str() {
            "signet" => Network::Signet,
            "testnet" => Network::Testnet,
            "bitcoin" => Network::Bitcoin,
            _ => {
                println!("Warning: Unknown network in config, defaulting to Signet");
                Network::Bitcoin
            }
        };

        println!("[Init] Setting network to: {:?}", network);
        builder.set_network(network);
        println!("[Init] Setting Esplora API URL: {}", DEFAULT_CHAIN_SOURCE_URL);
        builder.set_chain_source_esplora(DEFAULT_CHAIN_SOURCE_URL.to_string(), None);
        println!("[Init] Setting storage directory: {}", data_dir);
        builder.set_storage_dir_path(data_dir.to_string());

        let audit_log_path = format!("{}/audit_log.txt", LSP_DATA_DIR);
        set_audit_log_path(&audit_log_path);

        let listen_addr = format!("0.0.0.0:{}", port).parse().unwrap();
        println!("[Init] Setting listening address: {}", listen_addr);
        builder.set_listening_addresses(vec![listen_addr]).unwrap();
        println!("[Init] Setting node alias: {}", node_alias);
        let _ = builder.set_node_alias(node_alias.to_string()).ok();

        if node_alias == LSP_NODE_ALIAS {
            println!("[Init] Configuring LSP parameters...");
            let service_config = LSPS2ServiceConfig {
                require_token: None,
                advertise_service: true,
                channel_opening_fee_ppm: 0,
                channel_over_provisioning_ppm: 1_000_000,
                min_channel_opening_fee_msat: 0,
                min_channel_lifetime: 100,
                max_client_to_self_delay: 1024,
                min_payment_size_msat: 0,
                max_payment_size_msat: 100_000_000_000,
            };
            builder.set_liquidity_provider_lsps2(service_config);
        }

        let node = Arc::new(match builder.build() {
            Ok(n) => {
                println!("[Init] Node built successfully");
                n
            }
            Err(e) => panic!("[Init] Failed to build node: {:?}", e),
        });
        
        node.start().expect("Failed to start node");

        println!("[Init] Node ID: {}", node.node_id());
        
        if let Some(addrs) = node.listening_addresses() {
            println!("[Init] Listening on: {:?}", addrs);
        }

        let btc_price = get_cached_price();
        println!("[Init] Initial BTC price: {}", btc_price);

        let mut app = Self {
            node,
            btc_price,
            status_message: String::new(),
            last_update: Instant::now(),
            last_stability_check: Instant::now(),
        
            lightning_balance_btc: 0.0,
            onchain_balance_btc:   0.0,
            total_balance_btc:     0.0,
            lightning_balance_usd: 0.0,
            onchain_balance_usd:   0.0,
            total_balance_usd:     0.0,
        
            invoice_amount: String::new(),
            invoice_result: String::new(),
            invoice_to_pay: String::new(),
            on_chain_address: String::new(),
            on_chain_amount: String::new(),
        
            open_channel_node_id: String::new(),
            open_channel_address: String::new(),
            open_channel_amount:  String::new(),
        
            connect_node_id:     String::new(),
            connect_node_address:String::new(),
        
            channel_id_to_close: String::new(),
        
            selected_channel_id:   String::new(),
            stable_channel_amount: EXPECTED_USD.to_string(),
        
            stable_channels: Vec::new(),
        };

        app.update_balances();
        app.update_channel_info();

        if node_alias == LSP_NODE_ALIAS {
            app.load_stable_channels();
        }

        app
    }

    pub fn new() -> Self {
        Self::new_with_mode("lsp")
    }
}

impl ServerApp {
    pub fn update_balances(&mut self) {
        let current_price = get_cached_price();
        if current_price > 0.0 {
            self.btc_price = current_price;
        }

        let balances = self.node.list_balances();
        self.lightning_balance_btc = balances.total_lightning_balance_sats as f64 / 100_000_000.0;
        self.onchain_balance_btc = balances.total_onchain_balance_sats as f64 / 100_000_000.0;
        self.lightning_balance_usd = self.lightning_balance_btc * self.btc_price;
        self.onchain_balance_usd = self.onchain_balance_btc * self.btc_price;
        self.total_balance_btc = self.lightning_balance_btc + self.onchain_balance_btc;
        self.total_balance_usd = self.lightning_balance_usd + self.onchain_balance_usd;
    }

    pub fn check_and_update_stable_channels(&mut self) {
        let current_price = get_cached_price();
        if current_price > 0.0 {
            self.btc_price = current_price;
        }
    
        let mut channels_updated = false;
        for sc in &mut self.stable_channels {
            if !stable::channel_exists(&self.node, &sc.channel_id) {
                continue;
            }
    
            sc.latest_price = current_price;
            stable::check_stability(&self.node, sc, current_price);
    
            if sc.payment_made {
                channels_updated = true;
            }
        }
    
        if channels_updated {
            self.save_stable_channels();
        }
    }

    pub fn poll_events(&mut self) {
        while let Some(event) = self.node.next_event() {
            match event {
                Event::ChannelReady { channel_id, .. } => {
                    audit_event("CHANNEL_READY", json!({"channel_id": channel_id.to_string()}));
                    self.status_message = format!("Channel {} is now ready", channel_id);
                    self.update_balances();
                }
                Event::ChannelPending {
                    channel_id,
                    user_channel_id,
                    former_temporary_channel_id,
                    counterparty_node_id,
                    funding_txo,
                } => {
                    let temp_id_str = hex::encode(former_temporary_channel_id.0);
                
                    let funding_str = funding_txo.txid.as_raw_hash().to_string();
                
                    audit_event(
                        "CHANNEL_PENDING",
                        json!({
                            "channel_id":            channel_id.to_string(),
                            "user_channel_id":       format!("{:?}", user_channel_id),
                            "temp_channel_id":       temp_id_str,
                            "counterparty_node_id":  counterparty_node_id.to_string(),
                            "funding_txo":           funding_str,
                        }),
                    );
                
                    self.status_message = format!("Channel {} is pending confirmation", channel_id);
                }
                Event::PaymentSuccessful { payment_hash, .. } => {
                    audit_event("PAYMENT_SUCCESSFUL", json!({"payment_hash": format!("{}", payment_hash)}));
                    self.status_message = format!("Sent payment {}", payment_hash);
                    self.update_balances();
                }
                Event::PaymentReceived { amount_msat, payment_hash, .. } => {
                    audit_event("PAYMENT_RECEIVED", json!({"amount_msat": amount_msat, "payment_hash": format!("{}", payment_hash)}));
                    self.status_message = format!("Received payment of {} msats", amount_msat);
                    self.update_balances();
                }
                Event::ChannelClosed { channel_id, reason, .. } => {
                    audit_event("CHANNEL_CLOSED", json!({"channel_id": format!("{}", channel_id), "reason": format!("{:?}", reason)}));
                    self.status_message = format!("Channel {} has been closed", channel_id);
                    self.update_balances();
                }
                _ => {
                    audit_event("EVENT_IGNORED", json!({"event_type": format!("{:?}", event)}));
                }
            }
            let _ = self.node.event_handled();
        }
    }

    pub fn generate_invoice(&mut self) -> bool {
        if let Ok(amount) = self.invoice_amount.parse::<u64>() {
            let msats = amount * 1000;
            match self.node.bolt11_payment().receive(
                msats,
                &Bolt11InvoiceDescription::Direct(Description::new("Invoice".to_string()).unwrap()),
                3600,
            ) {
                Ok(invoice) => {
                    self.invoice_result = invoice.to_string();
                    self.status_message = "Invoice generated".to_string();
                    audit_event("INVOICE_GENERATED", json!({"amount_msats": msats, "invoice": self.invoice_result}));
                    true
                }
                Err(e) => {
                    self.status_message = format!("Error: {}", e);
                    audit_event("INVOICE_GENERATION_FAILED", json!({"amount_msats": msats, "error": format!("{}", e)}));
                    false
                }
            }
        } else {
            self.status_message = "Invalid amount".to_string();
            audit_event("INVOICE_INPUT_INVALID", json!({"raw_input": self.invoice_amount}));
            false
        }
    }

    pub fn pay_invoice(&mut self) -> bool {
        match Bolt11Invoice::from_str(&self.invoice_to_pay) {
            Ok(invoice) => match self.node.bolt11_payment().send(&invoice, None) {
                Ok(payment_id) => {
                    self.status_message = format!("Payment sent, ID: {}", payment_id);
                    audit_event("PAYMENT_SENT", json!({"invoice": self.invoice_to_pay, "payment_id": format!("{}", payment_id)}));
                    self.invoice_to_pay.clear();
                    self.update_balances();
                    true
                }
                Err(e) => {
                    self.status_message = format!("Payment error: {}", e);
                    audit_event("PAYMENT_SEND_FAILED", json!({"invoice": self.invoice_to_pay, "error": format!("{}", e)}));
                    false
                }
            },
            Err(e) => {
                self.status_message = format!("Invalid invoice: {}", e);
                audit_event("PAYMENT_INVOICE_INVALID", json!({"raw_input": self.invoice_to_pay, "error": format!("{}", e)}));
                false
            }
        }
    }

    pub fn get_address(&mut self) -> bool {
        match self.node.onchain_payment().new_address() {
            Ok(address) => {
                self.on_chain_address = address.to_string();
                self.status_message = "Address generated".to_string();
                audit_event("ONCHAIN_ADDRESS_GENERATED", json!({"address": self.on_chain_address}));
                true
            }
            Err(e) => {
                self.status_message = format!("Error: {}", e);
                audit_event("ONCHAIN_ADDRESS_FAILED", json!({"error": format!("{}", e)}));
                false
            }
        }
    }

    pub fn send_onchain(&mut self) -> bool {
        if let Ok(amount) = self.on_chain_amount.parse::<u64>() {
            match Address::from_str(&self.on_chain_address) {
                Ok(addr) => match addr.require_network(Network::Bitcoin) {
                    Ok(valid_addr) => match self.node.onchain_payment().send_to_address(&valid_addr, amount, None) {
                        Ok(txid) => {
                            self.status_message = format!("Transaction sent: {}", txid);
                            audit_event("ONCHAIN_SEND_SUCCESS", json!({"txid": format!("{}", txid), "amount_sat": amount}));
                            self.update_balances();
                            true
                        }
                        Err(e) => {
                            self.status_message = format!("Transaction error: {}", e);
                            audit_event("ONCHAIN_SEND_FAILED", json!({"amount_sat": amount, "error": format!("{}", e)}));
                            false
                        }
                    },
                    Err(_) => {
                        self.status_message = "Invalid address for this network".to_string();
                        audit_event("ONCHAIN_ADDRESS_INVALID_NET", json!({"address": self.on_chain_address}));
                        false
                    }
                },
                Err(_) => {
                    self.status_message = "Invalid address".to_string();
                    audit_event("ONCHAIN_ADDRESS_INVALID", json!({"address": self.on_chain_address}));
                    false
                }
            }
        } else {
            self.status_message = "Invalid amount".to_string();
            audit_event("ONCHAIN_AMOUNT_INVALID", json!({"raw_input": self.on_chain_amount}));
            false
        }
    }

    pub fn update_channel_info(&mut self) -> String {
        let channels = self.node.list_channels();
        if channels.is_empty() {
            return "No channels found.".to_string();
        }
    
        let mut info = String::new();
        info.push_str("[Channel Information Table]\n");
    
        for (i, channel) in channels.iter().enumerate() {
            let is_stable = self.stable_channels.iter().any(|sc| sc.channel_id == channel.channel_id);
    
            let id_str = hex::encode(channel.channel_id.0);
            let peer_str = channel.counterparty_node_id.to_string();
            let txid_str = channel.funding_txo
                .as_ref()
                .map(|o| format!("{}:{}", o.txid, o.vout))
                .unwrap_or("-".to_string());
    
            let value = channel.channel_value_sats;
            let outbound = channel.outbound_capacity_msat / 1000;
            let inbound = channel.inbound_capacity_msat / 1000;
            let usable = channel.is_usable;
    
            info.push_str(&format!(
                "{} | ID: {} | Peer: {} | TXID: {} | Capacity: {} sats | Ours: {} | Theirs: {} | Usable: {} | Stable: {}\n",
                i + 1,
                id_str,
                peer_str,
                txid_str,
                value,
                outbound,
                inbound,
                usable,
                is_stable
            ));
        }
    
        info
    }
  
    pub fn open_channel(&mut self) -> bool {
        match PublicKey::from_str(&self.open_channel_node_id) {
            Ok(node_id) => match SocketAddress::from_str(&self.open_channel_address) {
                Ok(net_address) => match self.open_channel_amount.parse::<u64>() {
                    Ok(sats) => {
                        let push_msat = (sats / 2) * 1000;
                        let channel_config: Option<ChannelConfig> = None;

                        match self.node.open_announced_channel(
                            node_id,
                            net_address,
                            sats,
                            Some(push_msat),
                            channel_config,
                        ) {
                            Ok(_) => {
                                self.status_message = format!("Channel opening initiated with {} for {} sats", node_id, sats);
                                true
                            }
                            Err(e) => {
                                self.status_message = format!("Error opening channel: {}", e);
                                false
                            }
                        }
                    }
                    Err(_) => {
                        self.status_message = "Invalid amount format".to_string();
                        false
                    }
                },
                Err(_) => {
                    self.status_message = "Invalid network address format".to_string();
                    false
                }
            },
            Err(_) => {
                self.status_message = "Invalid node ID format".to_string();
                false
            }
        }
    }

    pub fn close_specific_channel(&mut self) {
        if self.channel_id_to_close.is_empty() {
            self.status_message = "Please enter a channel ID to close".to_string();
            return;
        }

        let input = self.channel_id_to_close.trim();
        if input.len() == 64 && input.chars().all(|c| c.is_ascii_hexdigit()) {
            if let Ok(bytes) = hex::decode(input) {
                for channel in self.node.list_channels() {
                    if channel.channel_id.0.to_vec() == bytes {
                        let result = self.node.close_channel(&channel.user_channel_id, channel.counterparty_node_id);
                        self.status_message = match result {
                            Ok(_) => format!("Closing channel: {}", input),
                            Err(e) => format!("Error closing channel: {}", e),
                        };
                        self.channel_id_to_close.clear();
                        return;
                    }
                }
            }
            self.status_message = "Channel ID not found.".to_string();
        } else {
            for channel in self.node.list_channels() {
                if channel.channel_id.to_string() == input {
                    let result = self.node.close_channel(&channel.user_channel_id, channel.counterparty_node_id);
                    self.status_message = match result {
                        Ok(_) => format!("Closing channel: {}", input),
                        Err(e) => format!("Error closing channel: {}", e),
                    };
                    self.channel_id_to_close.clear();
                    return;
                }
            }
            self.status_message = "Channel not found.".to_string();
        }
    }

    pub fn designate_stable_channel(&mut self) {
        if self.selected_channel_id.is_empty() {
            self.status_message = "Please select a channel ID".to_string();
            audit_event("STABLE_DESIGNATE_NO_CHANNEL", json!({}));  
            return;
        }

        let amount = match self.stable_channel_amount.parse::<f64>() {
            Ok(val) => val,
            Err(_) => {
                self.status_message = "Invalid amount format".to_string();
                audit_event("STABLE_DESIGNATE_AMOUNT_INVALID", json!({"raw_input": self.stable_channel_amount}));
                return;
            }
        };

        let channel_id_str = self.selected_channel_id.trim().to_string();

        for channel in self.node.list_channels() {
            if channel.channel_id.to_string() == channel_id_str {
                let expected_usd = USD::from_f64(amount);
                let expected_btc = Bitcoin::from_usd(expected_usd, self.btc_price);

                let unspendable = channel.unspendable_punishment_reserve.unwrap_or(0);
                let our_balance_sats = (channel.outbound_capacity_msat / 1000) + unspendable;
                let their_balance_sats = channel.channel_value_sats - our_balance_sats;

                let stable_provider_btc = Bitcoin::from_sats(our_balance_sats);
                let stable_receiver_btc = Bitcoin::from_sats(their_balance_sats);
                let stable_provider_usd = USD::from_bitcoin(stable_provider_btc, self.btc_price);
                let stable_receiver_usd = USD::from_bitcoin(stable_receiver_btc, self.btc_price);

                let stable_channel = StableChannel {
                    channel_id: channel.channel_id,
                    counterparty: channel.counterparty_node_id,
                    is_stable_receiver: false,
                    expected_usd,
                    expected_btc,
                    stable_receiver_btc,
                    stable_receiver_usd,
                    stable_provider_btc,
                    stable_provider_usd,
                    latest_price: self.btc_price,
                    risk_level: 0,
                    payment_made: false,
                    timestamp: 0,
                    formatted_datetime: "".to_string(),
                    sc_dir: LSP_DATA_DIR.to_string(),
                    prices: "".to_string(),
                    onchain_btc: Bitcoin::from_sats(0),
                    onchain_usd: USD(0.0),
                };

                let mut found = false;
                for sc in &mut self.stable_channels {
                    if sc.channel_id == channel.channel_id {
                        *sc = stable_channel.clone();
                        found = true;
                        break;
                    }
                }

                if !found {
                    self.stable_channels.push(stable_channel);
                }

                self.save_stable_channels();

                self.status_message = format!(
                    "Channel {} designated as stable with target ${}",
                    channel_id_str, amount
                );
                audit_event("STABLE_DESIGNATED", json!({"channel_id": channel_id_str, "target_usd": amount}));
                self.selected_channel_id.clear();
                self.stable_channel_amount = EXPECTED_USD.to_string();
                return;
            }
        }
        audit_event("STABLE_DESIGNATE_CHANNEL_NOT_FOUND", json!({"channel_id": self.selected_channel_id}));
        self.status_message = format!("No channel found matching: {}", self.selected_channel_id);
    }

    pub fn save_stable_channels(&mut self) {
        let entries: Vec<StableChannelEntry> = self.stable_channels.iter().map(|sc| StableChannelEntry {
            channel_id: sc.channel_id.to_string(),
            expected_usd: sc.expected_usd.0,
            native_btc: sc.expected_btc.to_btc(),
        }).collect();

        let file_path = FilePath::new(LSP_DATA_DIR).join("stablechannels.json");

        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).unwrap_or_else(|e| {
                eprintln!("Failed to create directory: {}", e);
            });
        }

        match serde_json::to_string_pretty(&entries) {
            Ok(json) => {
                match fs::write(&file_path, json) {
                    Ok(_) => {
                        println!("Saved stable channels to {}", file_path.display());
                        self.status_message = "Stable channels saved successfully".to_string();
                    }
                    Err(e) => {
                        eprintln!("Error writing stable channels file: {}", e);
                        self.status_message = format!("Failed to save stable channels: {}", e);
                    }
                }
            }
            Err(e) => {
                eprintln!("Error serializing stable channels: {}", e);
                self.status_message = format!("Failed to serialize stable channels: {}", e);
            }
        }
    }

    pub fn load_stable_channels(&mut self) {
        let file_path = FilePath::new(LSP_DATA_DIR).join("stablechannels.json");

        if !file_path.exists() {
            println!("No existing stable channels file found.");
            return;
        }

        match fs::read_to_string(&file_path) {
            Ok(contents) => {
                match serde_json::from_str::<Vec<StableChannelEntry>>(&contents) {
                    Ok(entries) => {
                        self.stable_channels.clear();

                        for entry in entries {
                            for channel in self.node.list_channels() {
                                if channel.channel_id.to_string() == entry.channel_id {
                                    let unspendable = channel.unspendable_punishment_reserve.unwrap_or(0);
                                    let our_balance_sats = (channel.outbound_capacity_msat / 1000) + unspendable;
                                    let their_balance_sats = channel.channel_value_sats - our_balance_sats;

                                    let stable_provider_btc = Bitcoin::from_sats(our_balance_sats);
                                    let stable_receiver_btc = Bitcoin::from_sats(their_balance_sats);
                                    let stable_provider_usd = USD::from_bitcoin(stable_provider_btc, self.btc_price);
                                    let stable_receiver_usd = USD::from_bitcoin(stable_receiver_btc, self.btc_price);

                                    let stable_channel = StableChannel {
                                        channel_id: channel.channel_id,
                                        counterparty: channel.counterparty_node_id,
                                        is_stable_receiver: false,
                                        expected_usd: USD::from_f64(entry.expected_usd),
                                        expected_btc: Bitcoin::from_btc(entry.native_btc),
                                        stable_receiver_btc,
                                        stable_receiver_usd,
                                        stable_provider_btc,
                                        stable_provider_usd,
                                        latest_price: self.btc_price,
                                        risk_level: 0,
                                        payment_made: false,
                                        timestamp: 0,
                                        formatted_datetime: "".to_string(),
                                        sc_dir: LSP_DATA_DIR.to_string(),
                                        prices: "".to_string(),
                                        onchain_btc: Bitcoin::from_sats(0),
                                        onchain_usd: USD(0.0),
                                    };

                                    self.stable_channels.push(stable_channel);
                                    break;
                                }
                            }
                        }

                        println!("Loaded {} stable channels", self.stable_channels.len());
                        self.status_message = format!("Loaded {} stable channels", self.stable_channels.len());
                    }
                    Err(e) => {
                        eprintln!("Error parsing stable channels file: {}", e);
                        self.status_message = format!("Failed to parse stable channels: {}", e);
                    }
                }
            }
            Err(e) => {
                eprintln!("Error reading stable channels file: {}", e);
                self.status_message = format!("Failed to read stable channels file: {}", e);
            }
        }
    }
	
	pub fn connect_to_node(&mut self) -> bool {
	    match PublicKey::from_str(&self.connect_node_id) {
		Ok(node_id) => match SocketAddress::from_str(&self.connect_node_address) {
		    Ok(address) => {
		        match self.node.connect(node_id, address, true) {
		            Ok(_) => {
		                self.status_message = format!("Connected to node {}", node_id);
		                true
		            }
		            Err(e) => {
		                self.status_message = format!("Connect error: {}", e);
		                false
		            }
		        }
		    }
		    Err(_) => {
		        self.status_message = "Invalid address format".to_string();
		        false
		    }
		},
		Err(_) => {
		    self.status_message = "Invalid node ID format".to_string();
		    false
		}
	    }
	}
}
