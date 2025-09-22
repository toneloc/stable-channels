    use eframe::{egui, App, Frame};
    use ldk_node::bitcoin::Network;
    use ldk_node::lightning_invoice::Bolt11Invoice;
    use ldk_node::{Builder, Event, Node};
    use ldk_node::{
        bitcoin::secp256k1::PublicKey,
        lightning::ln::msgs::SocketAddress,
    };
    use ldk_node::config::{EsploraSyncConfig, BackgroundSyncConfig};

    use std::str::FromStr;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use image::{GrayImage, Luma};
    use qrcode::{QrCode, Color};
    use egui::{CollapsingHeader, Color32, CursorIcon, OpenUrl, RichText, Sense, TextureOptions, Vec2};
    use serde_json::json;

    use crate::audit::*;
    use crate::stable::update_balances;
    use crate::types::*;
    use crate::price_feeds::{get_cached_price, get_latest_price};
    use crate::stable;
    use crate::constants::*;
    use crate::config::AppConfig;
    use std::path::PathBuf;

    // Configuration will be loaded from AppConfig

    // Data will find the relevant path based on the OS
    fn user_data_dir(config: &AppConfig) -> PathBuf {
        config.get_user_data_dir()
    }

    pub struct UserApp {
        pub node: Arc<Node>,
        pub status_message: String,
        pub btc_price: f64,
        show_onboarding: bool,
        qr_texture: Option<egui::TextureHandle>,
        waiting_for_payment: bool,
        stable_channel: Arc<Mutex<StableChannel>>,
        background_started: bool,
        audit_log_path: String,
        show_log_window: bool,
        log_contents: String,
        log_last_read: std::time::Instant,
        config: AppConfig,
        
        // UI fields
        pub invoice_amount: String,
        pub invoice_result: String,
        pub invoice_to_pay: String,
        pub on_chain_address: String,
        pub on_chain_amount: String,
        pub show_advanced: bool, 
        balance_last_update: std::time::Instant,
        confirm_close_popup: bool,
        pub stable_message: String,

        // Balance fields
        pub lightning_balance_btc: f64,
        pub onchain_balance_btc: f64,
        pub lightning_balance_usd: f64,
        pub onchain_balance_usd: f64,
        pub total_balance_btc: f64,
        pub total_balance_usd: f64,
    }

    impl UserApp {
        pub fn new() -> Result<Self, String> {
            println!("Initializing user node...");

            // Load configuration
            let config = AppConfig::load().map_err(|e| format!("Failed to load config: {}", e))?;
            
            // Validate configuration
            if let Err(errors) = config.validate() {
                eprintln!("Configuration validation errors:");
                for error in errors {
                    eprintln!("  - {}", error);
                }
                eprintln!("Please set the required environment variables.");
            }

            let data_dir = user_data_dir(&config);
            
            let lsp_pubkey = config.lsp_pubkey
                .parse::<PublicKey>()
                .map_err(|e| format!("Invalid LSP pubkey: {}", e))?;

            let audit_log_path = config.get_audit_log_path("user");
            set_audit_log_path(&audit_log_path);

            let mut builder = Builder::new();
            
            let network = match config.network.to_lowercase().as_str() {
                "signet" => Network::Signet,
                "testnet" => Network::Testnet,
                "bitcoin" => Network::Bitcoin,
                _ => {
                    println!("Warning: Unknown network in config, defaulting to Bitcoin");
                    Network::Bitcoin
                }
            };

            println!("[Init] Setting network to: {:?}", network);
            builder.set_network(network);

            let esplora_cfg = EsploraSyncConfig {
                background_sync_config: Some(BackgroundSyncConfig {
                    onchain_wallet_sync_interval_secs: ONCHAIN_WALLET_SYNC_INTERVAL_SECS,
                    lightning_wallet_sync_interval_secs: LIGHTNING_WALLET_SYNC_INTERVAL_SECS,
                    fee_rate_cache_update_interval_secs: FEE_RATE_CACHE_UPDATE_INTERVAL_SECS
                }),
            };            

            builder.set_chain_source_esplora(config.chain_source_url.clone(), Some(esplora_cfg));
            builder.set_storage_dir_path(data_dir.to_string_lossy().into_owned());
            builder.set_listening_addresses(vec![format!("127.0.0.1:{}", config.user_port).parse().unwrap()]).unwrap();
            let _ = builder.set_node_alias(config.user_node_alias.clone());

            // Let's set up our LSP
            let lsp_address = config.lsp_address
                .parse::<SocketAddress>()
                .map_err(|e| format!("Invalid LSP address: {}", e))?;
                
            builder.set_liquidity_source_lsps2(
                lsp_pubkey,
                lsp_address.clone(),
                None,
            );
            builder.set_liquidity_source_lsps1(
                lsp_pubkey,
                lsp_address,
                None,
            );

            let node = Arc::new(builder.build().expect("Failed to build node"));
            node.start().expect("Failed to start node");

            println!("User node started: {}", node.node_id());

            // We try to connect to the "GATEWAY NODE" ... a well-connected Lightning node
            if let (Ok(gateway_pubkey), Ok(gateway_address)) = (PublicKey::from_str(&config.gateway_pubkey), SocketAddress::from_str(&config.gateway_address)) {
                if let Err(e) = node.connect(gateway_pubkey, gateway_address, true) {
                    println!("Failed to connect to Gateway node: {}", e);
                }
            }
            
            // And the LSP
            if let Ok(socket_addr) = SocketAddress::from_str(&config.lsp_address) {
                if let Err(e) = node.connect(lsp_pubkey, socket_addr, true) {
                    println!("Failed to connect to LSP node: {}", e);
                }
            }

            let mut btc_price = crate::price_feeds::get_cached_price();
            if btc_price <= 0.0 {
                if let Ok(price) = get_latest_price(&ureq::Agent::new()) {
                    btc_price = price;
                }
            }

            let sc_init = StableChannel {
                channel_id: ldk_node::lightning::ln::types::ChannelId::from_bytes([0; 32]),
                counterparty: lsp_pubkey,
                is_stable_receiver: true,
                expected_usd: USD::from_f64(config.expected_usd),
                expected_btc: Bitcoin::from_usd(USD::from_f64(config.expected_usd), btc_price),
                stable_receiver_btc: Bitcoin::default(),
                stable_receiver_usd: USD::default(),
                stable_provider_btc: Bitcoin::default(),
                stable_provider_usd: USD::default(),
                latest_price: btc_price,
                risk_level: 0,
                payment_made: false,
                timestamp: 0,
                formatted_datetime: "2021-06-01 12:00:00".to_string(),
                sc_dir: "/".to_string(),
                prices: String::new(),
                onchain_btc: Bitcoin::from_sats(0),
                onchain_usd: USD(0.0),
                note: Some(String::new()),
            };
            let stable_channel = Arc::new(Mutex::new(sc_init));

            let show_onboarding = node.list_channels().is_empty();

            let app = Self {
                node: Arc::clone(&node),
                status_message: String::new(),
                invoice_result: String::new(),
                show_onboarding,
                qr_texture: None,
                waiting_for_payment: false,
                stable_channel: Arc::clone(&stable_channel),
                background_started: false,
                btc_price,
                invoice_amount: "0".to_string(),        
                invoice_to_pay: String::new(),
                on_chain_address: String::new(),
                on_chain_amount: "0".to_string(),  
                lightning_balance_btc: 0.0,
                onchain_balance_btc: 0.0,
                lightning_balance_usd: 0.0,
                onchain_balance_usd: 0.0,
                total_balance_btc: 0.0,
                total_balance_usd: 0.0,
                show_log_window: false,
                log_contents: String::new(),
                log_last_read: std::time::Instant::now(),
                audit_log_path,
                show_advanced: false,
                balance_last_update: std::time::Instant::now() - Duration::from_secs(10),
                confirm_close_popup: false,
                stable_message: String::new(),
                config,
            };

            {
                let mut sc = app.stable_channel.lock().unwrap();
                stable::check_stability(&app.node, &mut sc, btc_price);
                update_balances(&app.node, &mut sc);
            }

            let node_arc = Arc::clone(&app.node);
            let sc_arc = Arc::clone(&app.stable_channel);

            std::thread::spawn(move || {
                use std::{thread::sleep, time::{Duration, SystemTime, UNIX_EPOCH}};

                fn current_unix_time() -> i64 {
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                        .try_into()
                        .unwrap_or(0)
                }

                loop {
                    let price = match get_latest_price(&ureq::Agent::new()) {
                        Ok(p) if p > 0.0 => p,
                        _ => crate::price_feeds::get_cached_price()
                    };

                    if price > 0.0 {
                        if let Ok(mut sc) = sc_arc.lock() {
                            stable::check_stability(&*node_arc, &mut sc, price);
                            update_balances(&*node_arc, &mut sc);

                            sc.latest_price = price;
                            sc.timestamp = current_unix_time();
                        }
                    }
                    sleep(Duration::from_secs(BALANCE_UPDATE_INTERVAL_SECS));
                }
            });

            Ok(app)
        }
        // fn get_app_data_dir(component: &str) -> PathBuf {
        //     let mut path = dirs::data_local_dir()
        //         .unwrap_or_else(|| PathBuf::from("./data"))
        //         .join("com.stablechannels");
            
        //     if !component.is_empty() {
        //         path = path.join(component);
        //     }
            
        //     // Ensure the directory exists
        //     std::fs::create_dir_all(&path).unwrap_or_else(|e| {
        //         eprintln!("Warning: Failed to create data directory: {}", e);
        //     });
            
        //     path
        // }
    
        fn start_background_if_needed(&mut self) {
            if self.background_started {
                return;
            }

            let node_arc = Arc::clone(&self.node);
            let sc_arc = Arc::clone(&self.stable_channel);

            std::thread::spawn(move || {
                loop {
                    // Always try to get the latest price first
                    let price = match crate::price_feeds::get_latest_price(&ureq::Agent::new()) {
                        Ok(p) if p > 0.0 => p,
                        _ => crate::price_feeds::get_cached_price()
                    };

                    // Only proceed if we have a valid price and active channels
                    if price > 0.0 && !node_arc.list_channels().is_empty() {
                        if let Ok(mut sc) = sc_arc.lock() {
                            crate::stable::check_stability(&*node_arc, &mut sc, price);
                            crate::stable::update_balances(&*node_arc, &mut sc);
                        }
                    }

                    // Sleep between checks, but be ready to interrupt if needed
                    std::thread::sleep(Duration::from_secs(BALANCE_UPDATE_INTERVAL_SECS));
                }
            });

            self.background_started = true;
        }

        fn get_jit_invoice(&mut self, ctx: &egui::Context) {
            let latest_price = {
                let sc = self.stable_channel.lock().unwrap();
                sc.latest_price
            };
            let description = ldk_node::lightning_invoice::Bolt11InvoiceDescription::Direct(
                ldk_node::lightning_invoice::Description::new(
                    "Stable Channel Wallet onboarding".to_string(),
                )
                .unwrap(),
            );

            // let max_proportional_lsp_fee_limit_ppm_msat = Some(20_000);

            // let result = self.node.bolt11_payment().receive_variable_amount_via_jit_channel(
            //     &description, 
            //     3600, 
            //     max_proportional_lsp_fee_limit_ppm_msat
            // );
            
            let msats = USD::to_msats(USD::from_f64(self.config.expected_usd), latest_price);
            
            // Round to the nearest sat (i.e., nearest 1_000 msats); ties round up.
            let msats_rounded = ((msats.saturating_add(500)) / 1_000) * 1_000;

            let result = self.node.bolt11_payment().receive_via_jit_channel(
                msats_rounded,
                &description,
                INVOICE_EXPIRY_SECS,
                Some(MAX_PROPORTIONAL_LSP_FEE_LIMIT_PPM_MSAT)
            );

            audit_event("JIT_INVOICE_ATTEMPT", json!({
                "expected_usd": self.config.expected_usd,
                "btc_price": latest_price
            }));

            match result {
                Ok(invoice) => {
                    self.invoice_result = invoice.to_string();
                    audit_event("JIT_INVOICE_GENERATED", json!({
                        "invoice": self.invoice_result,
                        "amount_msats": USD::to_msats(USD::from_f64(self.config.expected_usd), latest_price)
                    }));
                    let code = QrCode::new(&self.invoice_result).unwrap();
                    let bits = code.to_colors();
                    let width = code.width();
                    let scale = 4;
                    let mut imgbuf =
                        GrayImage::new((width * scale) as u32, (width * scale) as u32);
                    for y in 0..width {
                        for x in 0..width {
                            let color = if bits[y * width + x] == Color::Dark {
                                0
                            } else {
                                255
                            };
                            for dy in 0..scale {
                                for dx in 0..scale {
                                    imgbuf.put_pixel(
                                        (x * scale + dx) as u32,
                                        (y * scale + dy) as u32,
                                        Luma([color]),
                                    );
                                }
                            }
                        }
                    }
                    let (w, h) = (imgbuf.width() as usize, imgbuf.height() as usize);
                    let mut rgba = Vec::with_capacity(w * h * 4);
                    for p in imgbuf.pixels() {
                        let lum = p[0];
                        rgba.extend_from_slice(&[lum, lum, lum, 255]);
                    }
                    let tex = ctx.load_texture(
                        "qr_code",
                        egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba),
                        TextureOptions::LINEAR,
                    );
                    self.qr_texture = Some(tex);
                    self.status_message =
                        "Invoice generated. Pay it to create a JIT channel.".to_string();
                    self.waiting_for_payment = true;
                }
                Err(e) => {
                    audit_event("JIT_INVOICE_FAILED", json!({
                        "error": format!("{e}")
                    }));
                    self.invoice_result = format!("Error: {e:?}");
                    self.status_message = format!("Failed to generate invoice: {}", e);
                }
            }
        }

        pub fn generate_invoice(&mut self) -> bool {
            if let Ok(amount) = self.invoice_amount.parse::<u64>() {
                let msats = amount * 1000;
                match self.node.bolt11_payment().receive(
                    msats,
                    &ldk_node::lightning_invoice::Bolt11InvoiceDescription::Direct(
                        ldk_node::lightning_invoice::Description::new("Invoice".to_string()).unwrap()
                    ),
                    INVOICE_EXPIRY_SECS,
                ) {
                    Ok(invoice) => {
                        self.invoice_result = invoice.to_string();
                        self.status_message = "Invoice generated".to_string();
                        audit_event("INVOICE_GENERATED", json!({
                            "amount_msats": msats,
                            "invoice": self.invoice_result
                        }));
                        true
                    },
                    Err(e) => {
                        self.status_message = format!("Error: {}", e);
                        audit_event("INVOICE_GENERATION_FAILED", json!({
                            "amount_msats": msats,
                            "error": format!("{e}")
                        }));
                        false
                    }
                }
            } else {
                self.status_message = "Invalid amount".to_string();
                audit_event("INVOICE_INPUT_INVALID", json!({
                    "raw_input": self.invoice_amount
                }));
                false
            }
        }

        pub fn pay_invoice(&mut self) -> bool {
            match Bolt11Invoice::from_str(&self.invoice_to_pay) {
                Ok(invoice) => {
                    match self.node.bolt11_payment().send(&invoice, None) {
                        Ok(payment_id) => {
                            self.status_message = format!("Payment sent, ID: {}", payment_id);
                            self.invoice_to_pay.clear();
                            self.update_balances();
                            true
                        },
                        Err(e) => {
                            self.status_message = format!("Payment error: {}", e);
                            false
                        }
                    }
                },
                Err(e) => {
                    self.status_message = format!("Invalid invoice: {}", e);
                    false
                }
            }
        }

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
        
        fn close_active_channel(&mut self) {
            let channels = self.node.list_channels();
            if let Some(ch) = channels.first() {
                match self.node.close_channel(&ch.user_channel_id, ch.counterparty_node_id) {
                    Ok(_)  => self.status_message = format!("Closing channel {}", ch.channel_id),
                    Err(e) => self.status_message = format!("Error closing channel: {}", e),
                }
            } else {
                self.status_message = "No channel to close".into();
            }
        }

        pub fn get_address(&mut self) -> bool {
            match self.node.onchain_payment().new_address() {
                Ok(address) => {
                    self.on_chain_address = address.to_string();
                    self.status_message = "Address generated".to_string();
                    true
                },
                Err(e) => {
                    self.status_message = format!("Error: {}", e);
                    false
                }
            }
        }

        // for onchain deposits ...
        // fn get_lsps1_channel(&mut self) {
        //     let lsp_balance_sat = 10_000;
        //     let client_balance_sat = 10_000;
        //     let lsps1 = self.node.lsps1_liquidity();
        //     match lsps1.request_channel(lsp_balance_sat, client_balance_sat, 2016, false) {
        //         Ok(status) => {
        //             self.status_message =
        //                 format!("LSPS1 channel order initiated! Status: {status:?}");
        //         }
        //         Err(e) => {
        //             self.status_message = format!("LSPS1 channel request failed: {e:?}");
        //         }
        //     }
        // }

        fn send_stable_message(&mut self) {
            let amt = 1; 
            let custom_str = self.stable_message.clone();
            let custom_tlv = ldk_node::CustomTlvRecord {
                type_num: STABLE_CHANNEL_TLV_TYPE,
                value: custom_str.as_bytes().to_vec(),
            };
    
            let mut sc = self.stable_channel.lock().unwrap();
            match self.node.spontaneous_payment().send_with_custom_tlvs(
                amt,
                sc.counterparty,
                None,
                vec![custom_tlv],
            ) {
                Ok(_payment_id) => {
                    sc.payment_made = true;
                    self.status_message = format!("Sent stable message: {}", self.stable_message);
                }
                Err(e) => {
                    self.status_message = format!("Failed to send stable message: {}", e);
                }
            }
        }  

        fn process_events(&mut self) {
            while let Some(event) = self.node.next_event() {
                match event {
                    Event::ChannelReady { channel_id, .. } => {
                        let txid_str = self.node
                            .list_channels()
                            .iter()
                            .find(|ch| ch.channel_id == channel_id)
                            .and_then(|ch| ch.funding_txo.as_ref())
                            .map(|outpoint| outpoint.txid.to_string())
                            .unwrap_or_else(|| "unknown".to_string());
                        
                        let mut sc = self.stable_channel.lock().unwrap();
                        update_balances(&self.node, &mut sc);

                        audit_event("CHANNEL_READY", json!({
                            "channel_id": channel_id.to_string()
                        }));
                        self.status_message = format!("Channel {channel_id} is now ready\nTXID: {txid_str}");
                        self.show_onboarding = false;
                        self.waiting_for_payment = false;
                    }

                    Event::ChannelPending {
                        channel_id,
                        user_channel_id,
                        former_temporary_channel_id,
                        counterparty_node_id,
                        funding_txo,
                    } => {
                        // stringify auxiliary fields without relying on `Serialize` impls
                        let temp_id_str = hex::encode(former_temporary_channel_id.0);
                    
                        let funding_str = funding_txo.txid.as_raw_hash().to_string();

                        let mut sc = self.stable_channel.lock().unwrap();
                        update_balances(&self.node, &mut sc);
                    
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
                    
                        self.status_message = format!("Channel {channel_id} is now ready\nTXID: {funding_str}");
                    }
                    
                    Event::PaymentReceived { amount_msat, payment_hash, .. } => {
                        audit_event("PAYMENT_RECEIVED", json!({
                            "amount_msat": amount_msat,
                            "payment_hash": format!("{payment_hash}")
                        }));
                        self.status_message = format!("Received payment of {} msats", amount_msat);
                        let mut sc = self.stable_channel.lock().unwrap();
                        update_balances(&self.node, &mut sc);
                        self.show_onboarding = false;
                        self.waiting_for_payment = false;
                    }
                    
                    
                    Event::PaymentSuccessful { payment_id: _, payment_hash, payment_preimage: _, fee_paid_msat: _ } => {
                        audit_event("PAYMENT_SUCCESSFUL", json!({
                            "payment_hash": format!("{payment_hash}"),
                        }));
                        self.status_message = format!("Sent payment {}", payment_hash);
                        let mut sc = self.stable_channel.lock().unwrap();
                        update_balances(&self.node, &mut sc);
                    }
        
                    Event::ChannelClosed { channel_id, reason, .. } => {
                        audit_event("CHANNEL_CLOSED", json!({
                            "channel_id": format!("{channel_id}"),
                            "reason": format!("{:?}", reason)
                        }));
                        self.status_message = format!("Channel {channel_id} has been closed");
                        if self.node.list_channels().is_empty() {
                            self.show_onboarding = true;
                            self.waiting_for_payment = false;
                        }
                    }
        
                    _ => {
                        audit_event("EVENT_IGNORED", json!({
                            "event_type": format!("{:?}", event)
                        }));
                    }
                }
        
                let _ = self.node.event_handled();
            }
        }

        fn format_currency(v: f64) -> String {
            let s = format!("{:.2}", v); // "112226.70"
            let (int, frac) = s.split_once('.').unwrap();
        
            let int_with_commas = int
                .chars()
                .rev()
                .collect::<Vec<_>>()
                .chunks(3)
                .map(|c| c.iter().collect::<String>())
                .collect::<Vec<_>>()
                .join(",");
        
            let int_with_commas = int_with_commas.chars().rev().collect::<String>();
            format!("${}.{}", int_with_commas, frac)
        }

        fn show_waiting_for_payment_screen(&mut self, ctx: &egui::Context) {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_space(10.0);
                ui.vertical_centered(|ui| {
                    ui.heading(
                        egui::RichText::new("You are sending yourself bitcoin to your Stable Channels Wallet to make it stable.")
                            .size(16.0)
                            .strong()
                            .color(egui::Color32::WHITE),
                    );
                    ui.add_space(3.0);
                    ui.label("This is a Bolt11 Lightning invoice.");
                    ui.add_space(8.0);
                    if let Some(ref qr) = self.qr_texture {
                        ui.image(qr);
                    } else {
                        ui.label("Lightning QR Missing");
                    }
                    ui.add_space(8.0);
                    ui.add(
                        egui::TextEdit::multiline(&mut self.invoice_result)
                            .frame(true)
                            .desired_width(400.0)
                            .desired_rows(3)
                            .hint_text("Invoice..."),
                    );
                    ui.add_space(8.0);
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Copy Invoice")
                                    .color(egui::Color32::BLACK)
                                    .size(16.0),
                            )
                            .min_size(egui::vec2(120.0, 36.0))
                            .fill(egui::Color32::from_gray(220))
                            .rounding(6.0),
                        )
                        .clicked()
                    {
                        ui.output_mut(|o| o.copied_text = self.invoice_result.clone());
                    }
                    ui.add_space(5.0);
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Back")
                                    .color(egui::Color32::BLACK)
                                    .size(16.0),
                            )
                            .min_size(egui::vec2(120.0, 36.0))
                            .fill(egui::Color32::from_gray(220))
                            .rounding(6.0),
                        )
                        .clicked()
                    {
                        self.waiting_for_payment = false;
                    }
                    ui.add_space(8.0);
                });
            });
        }

        fn show_onboarding_screen(&mut self, ctx: &egui::Context) {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(30.0);
                    
                    ui.label(
                        egui::RichText::new("Get started in 3 steps")
                            .italics()
                            .size(16.0)
                            .color(egui::Color32::LIGHT_GRAY),
                    );

                    ui.add_space(50.0);

                    ui.heading(
                        egui::RichText::new("Step 1: Tap Stabilize ⚡")
                            .color(egui::Color32::WHITE),
                    );
                    // ui.label(
                    //     egui::RichText::new("One tap to start.")
                    //         .color(egui::Color32::GRAY),
                    // );
                    ui.add_space(20.0);
                    ui.heading(
                        egui::RichText::new("Step 2: Fund your wallet 💸")
                            .color(egui::Color32::WHITE),
                    );
                    ui.label(
                        egui::RichText::new("Send yourself bitcoin over Lightning")
                            .color(egui::Color32::GRAY),
                    );
                    ui.add_space(20.0);
                    ui.heading(
                        egui::RichText::new("Step 3: Enjoy your stabilized BTC 🔧")
                            .color(egui::Color32::WHITE),
                    );
                    ui.label(
                        egui::RichText::new("Self-custody. 100% bitcoin under the hood.")
                            .color(egui::Color32::GRAY),
                    );

                    // ui.add_space(20.0);
                    // self.show_onchain_send_section(ui);

                    ui.add_space(35.0);

                    let subtle_orange =
                        egui::Color32::from_rgba_premultiplied(247, 147, 26, 200);
                    let btn = egui::Button::new(
                        egui::RichText::new("Stabilize")
                            .color(egui::Color32::WHITE)
                            .strong()
                            .size(18.0),
                        )
                    .min_size(egui::vec2(200.0, 55.0))
                    .fill(subtle_orange)
                    .rounding(8.0);

                    ui.add_space(50.0);

                    if ui.add(btn).clicked() {
                        self.status_message =
                            "Getting JIT channel invoice...".to_string();
                        self.get_jit_invoice(ctx);
                    }
                    // if !self.status_message.is_empty() {
                    //     ui.add_space(40.0);
                    //     ui.label(self.status_message.clone());
                    // }
                    ui.add_space(50.0);

                    ui.label(
                        egui::RichText::new("Stable Channels is for bitcoiners who only want bitcoin.")
                            .size(14.0)
                            .italics()
                            .color(egui::Color32::LIGHT_GRAY),
                    );

                    ui.add_space(5.0);

                    let resp = ui
                        .add(
                            egui::Label::new(
                                egui::RichText::new("Learn more")
                                    .underline()
                                    .color(egui::Color32::from_rgb(255, 149, 0)),
                            )
                            .sense(Sense::click()),
                        )
                        .on_hover_cursor(CursorIcon::PointingHand);
                    
                    if resp.clicked() {
                        ui.output_mut(|o| {
                            o.open_url = Some(OpenUrl {
                                url: "https://www.stablechannels.com".to_owned(),
                                new_tab: true,
                            });
                        });
                    }
                
                    ui.add_space(30.0);

                    ui.horizontal(|ui| {
                        ui.label("Node ID: ");
                        let node_id = self.node.node_id().to_string();
                        let node_id_short = format!(
                            "{}...{}",
                            &node_id[0..10],
                            &node_id[node_id.len() - 10..]
                        );
                        ui.monospace(node_id_short);
                        if ui.small_button("Copy").clicked() {
                            ui.output_mut(|o| o.copied_text = node_id);
                        }
                    });

                    ui.add_space(30.0);

                    CollapsingHeader::new("Advanced Features")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.add_space(10.0);

                            ui.group(|ui| {
                                ui.heading("Withdraw On-chain");
                                ui.horizontal(|ui| {
                                    ui.label("On-chain Balance:");
                                    if let Ok(sc) = self.stable_channel.lock() {
                                        ui.monospace(format!("{:.8} BTC", sc.onchain_btc.to_btc()));
                                        ui.monospace(format!("(${:.2})", sc.onchain_usd.0));
                                    } else {
                                        ui.label("Error: could not lock stable_channel");
                                    }
                                });

                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    ui.label("Address:");
                                    ui.text_edit_singleline(&mut self.on_chain_address);
                                });

                                if ui.button("Withdraw all to address").clicked() {
                                    match ldk_node::bitcoin::Address::from_str(&self.on_chain_address) {
                                        Ok(addr) => match addr.require_network(ldk_node::bitcoin::Network::Bitcoin) {
                                            Ok(valid_addr) => match self.node.onchain_payment().send_all_to_address(&valid_addr, false, None) {
                                                Ok(txid) => {
                                                    self.status_message = format!("On-chain TX sent: {}", txid);
                                                    self.update_balances();
                                                }
                                                Err(e) => {
                                                    self.status_message = format!("On-chain TX failed: {}", e);
                                                }
                                            },
                                            Err(_) => {
                                                self.status_message = "Invalid address for this network".to_string();
                                            }
                                        },
                                        Err(_) => {
                                            self.status_message = "Invalid address format".to_string();
                                        }
                                    }
                                }

                                if !self.status_message.is_empty() {
                                    ui.add_space(8.0);
                                    ui.label(self.status_message.clone());
                                }
                            });
                        });

                        ui.add_space(30.0);
                });
            });
        }

        fn show_main_screen(&mut self, ctx: &egui::Context) {
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Your node ID:")
                                    .strong()
                                    .color(Color32::from_rgb(247, 147, 26))
                            );
                            let nid = self.node.node_id().to_string();
                            ui.monospace(
                                RichText::new(&nid[..8])
                                    .color(Color32::WHITE)
                            );
                        
                            ui.separator();
                        
                            ui.label(
                                RichText::new("Stable Channel ID:")
                                    .strong()
                                    .color(Color32::from_rgb(247, 147, 26))
                            );
                            let cid = self.node
                                .list_channels()
                                .get(0)
                                .map(|ch| ch.channel_id.to_string())
                                .unwrap_or_default();
                            ui.monospace(
                                RichText::new(&cid[..8.min(cid.len())])
                                    .color(Color32::WHITE)
                            );
                        
                            ui.separator();
                        
                            ui.label(
                                RichText::new("Stable status:")
                                    .strong()
                                    .color(Color32::from_rgb(247, 147, 26))
                            );
                            let dot_size = 12.0;
                            let (rect, _) = ui.allocate_exact_size(Vec2::splat(dot_size), Sense::hover());
                            ui.painter()
                                .circle_filled(rect.center(), dot_size * 0.5, Color32::GREEN);
                        });
                        ui.add_space(10.0);
                        ui.add_space(30.0);
        
                        ui.group(|ui| {
                            let sc = self.stable_channel.lock().unwrap();
                        
                            // Select correct stable values
                            let stable_usd = if sc.is_stable_receiver {
                                sc.stable_receiver_usd
                            } else {
                                sc.stable_provider_usd
                            };
                        
                            let pegged_btc = if sc.is_stable_receiver {
                                sc.stable_receiver_btc
                            } else {
                                sc.stable_provider_btc
                            };
                                                
                            // Main heading
                            ui.heading("Stable Balance");
                        
                            ui.add_space(8.0);

                            // Show USD stable balance, or "---" if < MIN_DISPLAY_USD
                            let stable_usd_display = if stable_usd.0 < MIN_DISPLAY_USD {
                                "---".to_string()
                            } else {
                                format!("{:.2}", stable_usd.0)
                            };                        
                        
                            ui.label(
                                egui::RichText::new(stable_usd_display)
                                    .size(24.0)
                                    .strong(),
                            );
                        
                            ui.add_space(12.0);
                        
                            // Agreed Peg USD
                            ui.label(
                                egui::RichText::new(format!("Agreed Peg USD: {:.2}", sc.expected_usd))
                                    .size(14.0)
                                    .color(egui::Color32::GRAY),
                            );
                        
                            ui.add_space(8.0);
                        
                            ui.separator();
                        
                            ui.add_space(8.0);
                        
                            // Bitcoin Holdings
                            ui.label(
                                egui::RichText::new("Bitcoin Holdings")
                                    .size(16.0)
                                    .strong(),
                            );
                            ui.add_space(8.0);
                        
                            egui::Grid::new("bitcoin_holdings_grid")
                                .spacing(Vec2::new(10.0, 6.0))
                                .show(ui, |ui| {
                                    // Use consistent sources (BTC as f64) and explicit formatting.
                                    let pegged_btc_f64 = pegged_btc.to_btc();
                                    let native_btc_f64 = self.onchain_balance_btc;
                                    let total_btc_f64  = pegged_btc_f64 + native_btc_f64;
                            
                                    ui.label("Pegged Bitcoin (Lightning):");
                                    ui.label(
                                        egui::RichText::new(format!("{:.8} BTC", pegged_btc_f64))
                                            .monospace(),
                                    );
                                    ui.end_row();
                            
                                    ui.label("Native Bitcoin (On-Chain):");
                                    ui.label(
                                        egui::RichText::new(format!("{:.8} BTC", native_btc_f64))
                                            .monospace(),
                                    );
                                    ui.end_row();
                            
                                    ui.label("Total Bitcoin:");
                                    ui.label(
                                        egui::RichText::new(format!("{:.8} BTC", total_btc_f64))
                                            .monospace()
                                            .strong(),
                                    );
                                    ui.end_row();
                            });

                        });

                        ui.add_space(20.0);

                        // Stability Allocation
                        // ui.group(|ui| {
                        //     ui.add_space(10.0);
                        
                        //     ui.vertical_centered(|ui| {
                        //         ui.heading("Stability Allocation");
                        
                        //         ui.add_space(20.0);
                        
                        //         let mut risk_level = self.stable_channel.lock().unwrap().risk_level;

                        //         ui.add_sized(
                        //             [100.0, 20.0], 
                        //             egui::Slider::new(&mut risk_level, 0..=100)
                        //                 .show_value(false)
                        //         );
                        
                        //         if ui.ctx().input(|i| i.pointer.any_down()) {
                        //             self.stable_channel.lock().unwrap().risk_level = risk_level;
                        //         }
                        
                        //         ui.add_space(10.0);
                        
                        //         ui.label(
                        //             egui::RichText::new(format!(
                        //                 "{}% BTC, {}% USD",
                        //                 risk_level,
                        //                 100 - risk_level
                        //             ))
                        //             .size(16.0)
                        //             .color(egui::Color32::GRAY),
                        //         );
                        
                        //         ui.add_space(20.0);
                        
                        //         if ui.add(
                        //             egui::Button::new(
                        //                 egui::RichText::new("Set Allocation")
                        //                     .size(16.0)
                        //                     .color(egui::Color32::WHITE)
                        //             )
                        //             .min_size(egui::vec2(150.0, 40.0))
                        //             .fill(egui::Color32::from_rgb(247, 147, 26))
                        //             .rounding(6.0)
                        //         ).clicked() {
                        //             // No action needed
                        //         }
                        //         ui.add_space(10.0);

                        //     });
                        // });
                        
                        // ui.add_space(20.0);
        
                        ui.group(|ui| {
                            let sc = self.stable_channel.lock().unwrap();
                            ui.add_space(20.0);
                            ui.heading("Bitcoin Price");
                            ui.add_space(10.0);


                            let price_ok = sc.latest_price.is_finite() && sc.latest_price > 0.0;

                            if price_ok {
                                ui.label(
                                    egui::RichText::new(Self::format_currency(sc.latest_price))
                                        .size(20.0)
                                        .strong()
                                );
                            } else {
                                ui.label(
                                    egui::RichText::new("Fetching latest price ...")
                                        .italics()
                                        .color(egui::Color32::LIGHT_GRAY)
                                        .size(16.0),
                                );
                            }

                            ui.add_space(20.0);
        
                            let last_updated_text = if !price_ok || sc.timestamp == 0 {
                                "Fetching latest price ...".to_string()
                            } else {
                                let secs = SystemTime::now()
                                    .duration_since(UNIX_EPOCH + std::time::Duration::from_secs(sc.timestamp as u64))
                                    .map(|d| d.as_secs())
                                    .unwrap_or(0);
                                format!("Last updated: {}s ago", secs)
                            };
                            
                            ui.label(
                                egui::RichText::new(last_updated_text)
                                    .size(12.0)
                                    .color(egui::Color32::GRAY),
                            );
                        });
                        ui.add_space(20.0);
        
                        // Begin advanced section.
                        CollapsingHeader::new("Show advanced features")
                            .default_open(false)
                            
                            .show(ui, |ui| {
                                if !self.status_message.is_empty() {
                                    ui.label(self.status_message.clone());
                                    ui.add_space(10.0);
                                }

                                ui.group(|ui| {
                                    ui.heading("Send Message to LSP");
                                    ui.add_space(8.0);
                                    ui.label("Please send your email address to the LSP, if you haven't already");
                                    ui.add_space(4.0);

                                    ui.add(egui::TextEdit::singleline(&mut self.stable_message)
                                        .hint_text("Enter message..."));
                                    ui.add_space(4.0);

                                if ui.button("Send Message").clicked() {
                                    if !self.stable_message.trim().is_empty() {
                                        self.send_stable_message();
                                        self.stable_message.clear(); // reset box
                                    }
                                }
                            });

                                ui.add_space(20.0);

                                ui.group(|ui| {
                                    ui.heading("Withdraw On-chain");
                                    ui.horizontal(|ui| {
                                        ui.label("On-chain Balance:");
                                        ui.monospace(format!("{:.8} BTC", self.onchain_balance_btc));
                                        ui.monospace(format!("(${:.2})", self.onchain_balance_usd));
                                    });
                                
                                    ui.add_space(8.0);
                                    ui.horizontal(|ui| {
                                        ui.label("Address:");
                                        ui.text_edit_singleline(&mut self.on_chain_address);
                                    });
                                
                                    if ui.button("Send On-chain").clicked() {
                                        match ldk_node::bitcoin::Address::from_str(&self.on_chain_address) {
                                            Ok(addr) => match addr.require_network(ldk_node::bitcoin::Network::Bitcoin) {
                                                Ok(valid_addr) => match self.node.onchain_payment().send_all_to_address(&valid_addr, false, None) {
                                                    Ok(txid) => {
                                                        self.status_message = format!("On-chain TX sent: {}", txid);
                                                        self.update_balances();
                                                    }
                                                    Err(e) => {
                                                        self.status_message = format!("On-chain TX failed: {}", e);
                                                    }
                                                },
                                                Err(_) => {
                                                    self.status_message = "Invalid address for this network".to_string();
                                                }
                                            },
                                            Err(_) => {
                                                self.status_message = "Invalid address format".to_string();
                                            }
                                        }
                                    }
                                });
        
                                ui.group(|ui| {
                                    ui.heading("Lightning Channels");
                                    ui.add_space(5.0);
                                    let channels = self.node.list_channels();
                                    if channels.is_empty() {
                                        ui.label("No channels found.");
                                    } else {
                                        for ch in channels {
                                            ui.label(format!(
                                                "Channel: {} – {} sats",
                                                ch.channel_id,
                                                ch.channel_value_sats
                                            ));
                                        }
                                    }
                                });
                                ui.add_space(20.0);

                                if ui.button("Close Stable Channel").clicked() {
                                    self.confirm_close_popup = true;
                                }
                                
                                let mut clicked_yes = false;
                                let mut clicked_cancel = false;
                                
                                egui::Window::new("Confirm Close")
                                    .collapsible(false)
                                    .resizable(false)
                                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                                    .open(&mut self.confirm_close_popup)   
                                    .show(ctx, |ui| {
                                        ui.label("Are you sure you want to close your Stable Channel?");
                                        ui.label("Your on-chain funds will appear under \"Advanced features\" after the transaction processes.");
                                        ui.add_space(10.0);
                                        ui.horizontal(|ui| {
                                            if ui.button("Yes, close").clicked() {
                                                clicked_yes = true;
                                            }
                                            if ui.button("Cancel").clicked() {
                                                clicked_cancel = true; 
                                            }
                                        });
                                    });
                                    
                                if clicked_yes {
                                    self.close_active_channel();
                                    self.confirm_close_popup = false;
                                } else if clicked_cancel {
                                    self.confirm_close_popup = false;
                                }
    
        
                                // ui.group(|ui| {
                                //     ui.label("Generate Invoice");
                                //     ui.horizontal(|ui| {
                                //         ui.label("Amount (sats):");
                                //         ui.text_edit_singleline(&mut self.invoice_amount);
                                //         if ui.button("Get Invoice").clicked() {
                                //             self.generate_invoice();
                                //         }
                                //     });
                                //     if !self.invoice_result.is_empty() {
                                //         ui.text_edit_multiline(&mut self.invoice_result);
                                //         if ui.button("Copy").clicked() {
                                //             ui.output_mut(|o| {
                                //                 o.copied_text = self.invoice_result.clone();
                                //             });
                                //         }
                                //     }
                                // });
        
                                // ui.group(|ui| {
                                //     ui.label("Pay Invoice");
                                //     ui.text_edit_multiline(&mut self.invoice_to_pay);
                                //     if ui.button("Pay Invoice").clicked() {
                                //         self.pay_invoice();
                                //     }
                                // });
        
                                // if ui.button("Create New Channel").clicked() {
                                //     self.show_onboarding = true;
                                // }
                                // if ui.button("Get On-chain Address").clicked() {
                                //     self.get_address();
                                // }
                                if ui.button("View Logs").clicked() {
                                    self.show_log_window = true;
                                }
                            });
                    }); // end vertical_centered
                }); // end ScrollArea
            }); // end CentralPanel
        }
        
        fn show_log_window_if_open(&mut self, ctx: &egui::Context) {
            if !self.show_log_window {
                return;
            }
        
            if self.log_last_read.elapsed() > Duration::from_millis(500) {
                self.log_contents = std::fs::read_to_string(&self.audit_log_path)
                    .unwrap_or_else(|_| "Log file not found.".to_string());
                self.log_last_read = std::time::Instant::now();
            }
        
            egui::Window::new("Audit Log")
                .resizable(true)
                .vscroll(true)
                .open(&mut self.show_log_window)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.log_contents)
                                .font(egui::TextStyle::Monospace)
                                .code_editor()
                                .desired_rows(20)
                                .lock_focus(true)
                                .desired_width(f32::INFINITY)
                                .interactive(false),
                        );
                    });
                });
        }
        
    }    

    impl App for UserApp {
        fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) { 
            // Set dark background for Windows
            ctx.set_visuals(egui::Visuals::dark());
            
            // Explicitly set the background color
            let mut visuals = egui::Visuals::dark();
            visuals.window_fill = egui::Color32::from_rgb(25, 25, 25); // Dark gray background
            visuals.panel_fill = egui::Color32::from_rgb(25, 25, 25);  // Dark gray panels
            ctx.set_visuals(visuals);
            
            self.process_events();

            self.show_onboarding = self.node.list_channels().is_empty() && !self.waiting_for_payment;

            self.start_background_if_needed();

            if self.balance_last_update.elapsed() >= Duration::from_secs(2) {
                self.update_balances();
                self.balance_last_update = std::time::Instant::now();
            }

            if self.waiting_for_payment {
                self.show_waiting_for_payment_screen(ctx);
            } else if self.show_onboarding {
                self.show_onboarding_screen(ctx);
            } else {
                self.show_main_screen(ctx);
            }
            self.show_log_window_if_open(ctx);

            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    pub fn run() {
        println!("Starting User Interface...");
        let native_options = eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size([460.0, 700.0])
                .with_decorations(true)
                .with_transparent(false),
            ..Default::default()
        };

        let app_result = UserApp::new();
        match app_result {
            Ok(app) => {
                eframe::run_native(
                    "Stable Channels Wallet",
                    native_options,
                    Box::new(|cc| {
                        // Set dark theme with explicit background colors for Windows
                        let mut visuals = egui::Visuals::dark();
                        visuals.window_fill = egui::Color32::from_rgb(25, 25, 25); // Dark gray background
                        visuals.panel_fill = egui::Color32::from_rgb(25, 25, 25);  // Dark gray panels
                        cc.egui_ctx.set_visuals(visuals);
    
                        Ok(Box::new(app))
                    }),
                ).unwrap();
            }
            Err(e) => {
                eprintln!("Failed to initialize app: {}", e);
                std::process::exit(1);
            }
        }
    }

