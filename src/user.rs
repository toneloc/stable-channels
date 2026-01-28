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
    use chrono::{TimeZone, Utc};

    use stable_channels::audit::*;
    use stable_channels::stable::update_balances;
    use stable_channels::types::*;
    use stable_channels::price_feeds::{get_cached_price, get_cached_price_no_fetch};
    use stable_channels::stable;
    use stable_channels::constants::*;
    use stable_channels::db::Database;

    #[derive(Clone, Debug)]
    pub enum TradeAction {
        BuyBtc,
        SellBtc,
    }

    #[derive(Clone, Debug)]
    pub struct PendingTrade {
        pub action: TradeAction,
        pub amount_usd: f64,
        pub new_btc_percent: u8,
    }

    #[derive(Clone, Debug)]
    pub struct PendingSplice {
        pub direction: String,  // "in" or "out"
        pub amount_sats: u64,
        pub address: Option<String>,  // For splice_out
    }

    #[derive(Clone)]
    struct Toast {
        message: String,
        emoji: String,
        created_at: std::time::Instant,
        duration_secs: f32,
    }

    impl Toast {
        fn new(message: &str, emoji: &str, duration_secs: f32) -> Self {
            Self {
                message: message.to_string(),
                emoji: emoji.to_string(),
                created_at: std::time::Instant::now(),
                duration_secs,
            }
        }

        fn is_expired(&self) -> bool {
            self.created_at.elapsed().as_secs_f32() > self.duration_secs
        }

        fn progress(&self) -> f32 {
            (self.created_at.elapsed().as_secs_f32() / self.duration_secs).min(1.0)
        }
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

        // UI fields
        pub invoice_amount: String,
        pub invoice_result: String,
        pub invoice_to_pay: String,
        pub on_chain_address: String,
        pub on_chain_amount: String,
        pub onchain_send_address: String,
        pub onchain_send_amount: String,
        pub splice_in_amount: String,
        pub splice_out_amount: String,
        pub splice_out_address: String,
        pending_splice: Option<PendingSplice>,
        pub show_onchain_receive: bool,
        pub show_onchain_send: bool,
        pub show_advanced: bool, 
        balance_last_update: std::time::Instant,
        confirm_close_popup: bool,
        pub stable_message: String,
        show_confirm_trade: bool,
        trade_amount_input: String,
        pending_trade: Option<PendingTrade>,
        trade_error: String,

        // Balance fields
        pub lightning_balance_btc: f64,
        pub onchain_balance_btc: f64,
        pub lightning_balance_usd: f64,
        pub onchain_balance_usd: f64,
        pub total_balance_btc: f64,
        pub total_balance_usd: f64,

        // Database
        db: Database,

        // Toast notifications
        toasts: Vec<Toast>,

        // Network
        network: Network,
    }

    impl UserApp {
        pub fn new() -> Result<Self, String> {
            println!("Initializing user node...");

            let data_dir = get_user_data_dir();

            let lsp_pubkey = DEFAULT_LSP_PUBKEY
                .parse::<PublicKey>()
                .map_err(|e| format!("Invalid LSP pubkey: {}", e))?;

            let audit_log_path = audit_log_path_for("user");
            set_audit_log_path(&audit_log_path);

            let mut builder = Builder::new();

            let network = match DEFAULT_NETWORK.to_lowercase().as_str() {
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

            builder.set_chain_source_esplora(DEFAULT_CHAIN_URL.to_string(), Some(esplora_cfg));
            builder.set_storage_dir_path(data_dir.to_string_lossy().into_owned());
            builder.set_listening_addresses(vec![format!("127.0.0.1:{}", DEFAULT_USER_PORT).parse().unwrap()]).unwrap();
            let _ = builder.set_node_alias(DEFAULT_USER_ALIAS.to_string());

            // Let's set up our LSP
            let lsp_address = DEFAULT_LSP_ADDRESS
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
            if let (Ok(gateway_pubkey), Ok(gateway_address)) = (PublicKey::from_str(DEFAULT_GATEWAY_PUBKEY), SocketAddress::from_str(DEFAULT_GATEWAY_ADDRESS)) {
                if let Err(e) = node.connect(gateway_pubkey, gateway_address, true) {
                    println!("Failed to connect to Gateway node: {}", e);
                }
            }

            // And the LSP
            if let Ok(socket_addr) = SocketAddress::from_str(DEFAULT_LSP_ADDRESS) {
                if let Err(e) = node.connect(lsp_pubkey, socket_addr, true) {
                    println!("Failed to connect to LSP node: {}", e);
                }
            }

            // Use non-blocking cache read at startup - background thread will fetch price
            let btc_price = get_cached_price_no_fetch();

            // Set initial timestamp if we have a valid cached price
            let initial_timestamp = if btc_price > 0.0 {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0)
            } else {
                0
            };

            let sc_init = StableChannel {
                channel_id: ldk_node::lightning::ln::types::ChannelId::from_bytes([0; 32]),
                counterparty: lsp_pubkey,
                is_stable_receiver: true,
                expected_usd: USD::from_f64(DEFAULT_EXPECTED_USD),
                expected_btc: Bitcoin::from_usd(USD::from_f64(DEFAULT_EXPECTED_USD), btc_price),
                stable_receiver_btc: Bitcoin::default(),
                stable_receiver_usd: USD::default(),
                stable_provider_btc: Bitcoin::default(),
                stable_provider_usd: USD::default(),
                latest_price: btc_price,
                risk_level: 0,
                payment_made: false,
                timestamp: initial_timestamp,
                formatted_datetime: "2021-06-01 12:00:00".to_string(),
                sc_dir: "/".to_string(),
                prices: String::new(),
                onchain_btc: Bitcoin::from_sats(0),
                onchain_usd: USD(0.0),
                note: Some(String::new()),
                allocation: Allocation::default(),
                native_channel_btc: Bitcoin::from_sats(0),
            };
            let stable_channel = Arc::new(Mutex::new(sc_init));

            let show_onboarding = node.list_channels().is_empty();

            // Initialize SQLite database
            let db = Database::open(&data_dir)
                .map_err(|e| format!("Failed to open database: {}", e))?;

            let mut app = Self {
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
                onchain_send_address: String::new(),
                onchain_send_amount: String::new(),
                splice_in_amount: String::new(),
                splice_out_amount: String::new(),
                splice_out_address: String::new(),
                pending_splice: None,
                show_onchain_receive: false,
                show_onchain_send: false,
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
                show_confirm_trade: false,
                trade_amount_input: String::new(),
                pending_trade: None,
                trade_error: String::new(),
                db,
                toasts: Vec::new(),
                network,
            };

            {
                let mut sc = app.stable_channel.lock().unwrap();
                if let Some(payment_info) = stable::check_stability(&app.node, &mut sc, btc_price) {
                    // Record sent stability payment
                    let amount_usd = (payment_info.amount_msat as f64 / 1000.0 / 100_000_000.0) * payment_info.btc_price;
                    let _ = app.db.record_payment(
                        Some(&payment_info.payment_id),
                        "stability",
                        "sent",
                        payment_info.amount_msat,
                        Some(amount_usd),
                        Some(payment_info.btc_price),
                        Some(&payment_info.counterparty),
                        "completed",
                    );
                }
                update_balances(&app.node, &mut sc);
            }

            // Load persisted allocation from database
            app.load_user_allocation();

            // Background thread is started via start_background_if_needed() in the update loop

            Ok(app)
        }
    
        fn start_background_if_needed(&mut self) {
            if self.background_started {
                return;
            }

            let node_arc = Arc::clone(&self.node);
            let sc_arc = Arc::clone(&self.stable_channel);
            let db = self.db.clone();

            std::thread::spawn(move || {
                fn current_unix_time() -> i64 {
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                        .try_into()
                        .unwrap_or(0)
                }

                loop {
                    // Fetch price (network call) OUTSIDE of lock
                    let price = match stable_channels::price_feeds::get_latest_price(&ureq::Agent::new()) {
                        Ok(p) if p > 0.0 => p,
                        _ => stable_channels::price_feeds::get_cached_price()
                    };

                    // Only proceed if we have a valid price
                    if price > 0.0 {
                        // Brief lock to update values
                        if let Ok(mut sc) = sc_arc.lock() {
                            if !node_arc.list_channels().is_empty() {
                                if let Some(payment_info) = stable_channels::stable::check_stability(&*node_arc, &mut sc, price) {
                                    // Record sent stability payment
                                    let amount_usd = (payment_info.amount_msat as f64 / 1000.0 / 100_000_000.0) * payment_info.btc_price;
                                    let _ = db.record_payment(
                                        Some(&payment_info.payment_id),
                                        "stability",
                                        "sent",
                                        payment_info.amount_msat,
                                        Some(amount_usd),
                                        Some(payment_info.btc_price),
                                        Some(&payment_info.counterparty),
                                        "completed",
                                    );
                                }
                                stable_channels::stable::update_balances(&*node_arc, &mut sc);
                            }
                            sc.latest_price = price;
                            sc.timestamp = current_unix_time();
                        }
                    }

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
            
            let msats = USD::to_msats(USD::from_f64(DEFAULT_EXPECTED_USD), latest_price);

            // Round to the nearest sat (i.e., nearest 1_000 msats); ties round up.
            let msats_rounded = ((msats.saturating_add(500)) / 1_000) * 1_000;

            let result = self.node.bolt11_payment().receive_via_jit_channel(
                msats_rounded,
                &description,
                INVOICE_EXPIRY_SECS,
                Some(MAX_PROPORTIONAL_LSP_FEE_LIMIT_PPM_MSAT)
            );

            audit_event("JIT_INVOICE_ATTEMPT", json!({
                "expected_usd": DEFAULT_EXPECTED_USD,
                "btc_price": latest_price
            }));

            match result {
                Ok(invoice) => {
                    self.invoice_result = invoice.to_string();
                    audit_event("JIT_INVOICE_GENERATED", json!({
                        "invoice": self.invoice_result,
                        "amount_msats": USD::to_msats(USD::from_f64(DEFAULT_EXPECTED_USD), latest_price)
                    }));
                    let code = QrCode::new(&self.invoice_result).unwrap();
                    let bits = code.to_colors();
                    let width = code.width();
                    let scale = 4;
                    let border = scale * 2; // 2 modules of border
                    let img_size = (width * scale) as u32;
                    let bordered_size = img_size + (border * 2) as u32;

                    // Create image with border (white background)
                    let mut imgbuf = GrayImage::from_pixel(bordered_size, bordered_size, Luma([255]));

                    // Draw QR code in the center
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
                                        (x * scale + dx) as u32 + border as u32,
                                        (y * scale + dy) as u32 + border as u32,
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

        // TODO - for onchain deposits ...
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

        fn change_allocation(&mut self, btc_pct_i32: i32, fee_usd: f64, trade_amount_usd: f64) {
            // Clamp to [0, 100]
            let pct_btc: u8 = btc_pct_i32.clamp(0, 100) as u8;
            let pct_usd: u8 = 100 - pct_btc;

            // Create the new allocation
            let new_allocation = Allocation::from_btc_percent(pct_btc);

            // Grab channel_id, counterparty, price, and old allocation from StableChannel
            let (channel_id_str, counterparty, price, old_btc_pct) = {
                let sc = self.stable_channel.lock().unwrap();
                (sc.channel_id.to_string(), sc.counterparty, sc.latest_price, sc.allocation.btc_percent())
            };

            // Determine trade action
            let action = if pct_btc > old_btc_pct { "buy" } else { "sell" };

            // Build payload that the LSP will parse
            let payload = json!({
                "type": ALLOCATION_UPDATE_TYPE,
                "channel_id": channel_id_str,
                "allocation": {
                    "usd_weight": new_allocation.usd_weight,
                    "btc_weight": new_allocation.btc_weight,
                },
            });

            // Serialize payload and sign it with the node's key
            let payload_str = payload.to_string();
            let signature = self.node.sign_message(payload_str.as_bytes());

            // Envelope we actually send over the wire:
            // { "payload": "<JSON string above>", "signature": "<recoverable sig string>" }
            let signed_msg = json!({
                "payload": payload_str,
                "signature": signature,
            });

            let signed_str = signed_msg.to_string();

            // Build custom TLV record
            let custom_tlv = ldk_node::CustomTlvRecord {
                type_num: STABLE_CHANNEL_TLV_TYPE,
                value: signed_str.as_bytes().to_vec(),
            };

            // Calculate fee in msats (1% fee sent to LSP)
            let fee_msats = if price > 0.0 && fee_usd > 0.0 {
                let fee_btc = fee_usd / price;
                let fee_sats = (fee_btc * 100_000_000.0) as u64;
                fee_sats * 1000 // convert to msats
            } else {
                1 // minimum 1 msat if no fee
            };
            let amt_msat: u64 = fee_msats.max(1);

            match self.node.spontaneous_payment().send_with_custom_tlvs(
                amt_msat,
                counterparty,
                None,
                vec![custom_tlv],
            ) {
                Ok(payment_id) => {
                    // Update local stable channel with new allocation
                    {
                        let mut sc = self.stable_channel.lock().unwrap();
                        sc.allocation = new_allocation;
                        // Recalculate expected_usd based on current balance and new allocation
                        let total_usd = sc.stable_receiver_usd.0;
                        sc.expected_usd = USD::from_f64(total_usd * new_allocation.usd_weight);
                        // Calculate native BTC exposure
                        let native_usd = USD::from_f64(total_usd * new_allocation.btc_weight);
                        sc.native_channel_btc = Bitcoin::from_usd(native_usd, sc.latest_price);
                    }

                    // Persist allocation to database
                    self.save_user_allocation();

                    // Record the trade in database
                    let payment_id_str = format!("{payment_id}");
                    let amount_btc = if self.btc_price > 0.0 { trade_amount_usd / self.btc_price } else { 0.0 };
                    self.record_trade(action, "BTC", trade_amount_usd, amount_btc, fee_usd, old_btc_pct, pct_btc, Some(&payment_id_str), "completed");

                    self.status_message = format!(
                        "Sent trade order (fee: ${:.2})",
                        fee_usd,
                    );
                    audit_event("ALLOCATION_MESSAGE_SENT", json!({
                        "payment_id": payment_id_str,
                        "channel_id": channel_id_str,
                        "pct_btc": pct_btc,
                        "pct_usd": pct_usd,
                        "usd_weight": new_allocation.usd_weight,
                        "btc_weight": new_allocation.btc_weight,
                        "fee_usd": fee_usd,
                        "fee_msats": amt_msat,
                    }));
                }
                Err(e) => {
                    // Record the failed trade
                    let amount_btc = if self.btc_price > 0.0 { trade_amount_usd / self.btc_price } else { 0.0 };
                    self.record_trade(action, "BTC", trade_amount_usd, amount_btc, fee_usd, old_btc_pct, pct_btc, None, "failed");

                    self.status_message = format!("Failed to send trade order: {}", e);
                    audit_event("ALLOCATION_MESSAGE_FAILED", json!({
                        "channel_id": channel_id_str,
                        "pct_btc": pct_btc,
                        "pct_usd": pct_usd,
                        "error": format!("{e}"),
                    }));
                }
            }
        }

        /// Save user's stable channel allocation to database
        fn save_user_allocation(&self) {
            let sc = self.stable_channel.lock().unwrap();

            // Only save if we have a valid channel
            if sc.channel_id == ldk_node::lightning::ln::types::ChannelId::from_bytes([0; 32]) {
                return;
            }

            let channel_id_str = sc.channel_id.to_string();
            let note_ref = sc.note.as_deref();

            if let Err(e) = self.db.save_channel(
                &channel_id_str,
                sc.allocation.usd_weight,
                sc.allocation.btc_weight,
                sc.expected_usd.0,
                note_ref,
            ) {
                eprintln!("Failed to save channel allocation: {}", e);
            }
        }

        /// Load user's stable channel allocation from database
        fn load_user_allocation(&mut self) {
            let channel_id_str = {
                let sc = self.stable_channel.lock().unwrap();
                sc.channel_id.to_string()
            };

            // Try to load from database first
            if let Ok(Some(record)) = self.db.load_channel(&channel_id_str) {
                let mut sc = self.stable_channel.lock().unwrap();
                if let Ok(allocation) = Allocation::new(record.usd_weight, record.btc_weight) {
                    sc.allocation = allocation;
                    sc.expected_usd = USD::from_f64(record.expected_usd);
                    if record.note.is_some() {
                        sc.note = record.note;
                    }
                    return;
                }
            }

            // Fallback: migrate from legacy JSON file if it exists
            self.migrate_from_json();
        }

        /// Migrate data from legacy stablechannels.json to SQLite
        fn migrate_from_json(&mut self) {
            let file_path = get_user_data_dir().join("stablechannels.json");

            if !file_path.exists() {
                return;
            }

            let contents = match std::fs::read_to_string(&file_path) {
                Ok(c) => c,
                Err(_) => return,
            };

            let entries: Vec<serde_json::Value> = match serde_json::from_str(&contents) {
                Ok(e) => e,
                Err(_) => return,
            };

            let mut sc = self.stable_channel.lock().unwrap();
            let channel_id_str = sc.channel_id.to_string();

            // Find matching entry and migrate
            if let Some(entry) = entries.iter().find(|e| {
                e.get("channel_id").and_then(|v| v.as_str()) == Some(&channel_id_str)
            }) {
                let mut usd_weight = 1.0;
                let mut btc_weight = 0.0;
                let mut expected_usd = 0.0;
                let note: Option<String> = entry.get("note")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                if let Some(alloc) = entry.get("allocation") {
                    usd_weight = alloc.get("usd_weight").and_then(|v| v.as_f64()).unwrap_or(1.0);
                    btc_weight = alloc.get("btc_weight").and_then(|v| v.as_f64()).unwrap_or(0.0);
                }

                if let Some(exp) = entry.get("expected_usd").and_then(|v| v.as_f64()) {
                    expected_usd = exp;
                }

                // Apply to current state
                if let Ok(allocation) = Allocation::new(usd_weight, btc_weight) {
                    sc.allocation = allocation;
                    sc.expected_usd = USD::from_f64(expected_usd);
                    if note.is_some() {
                        sc.note = note.clone();
                    }

                    // Save to database
                    let _ = self.db.save_channel(
                        &channel_id_str,
                        usd_weight,
                        btc_weight,
                        expected_usd,
                        note.as_deref(),
                    );

                    println!("Migrated channel {} from JSON to SQLite", channel_id_str);
                }
            }
        }

        /// Record a trade in the database
        fn record_trade(&self, action: &str, asset_type: &str, amount_usd: f64, amount_btc: f64, fee_usd: f64, old_btc_pct: u8, new_btc_pct: u8, payment_id: Option<&str>, status: &str) {
            let channel_id_str = {
                let sc = self.stable_channel.lock().unwrap();
                sc.channel_id.to_string()
            };

            if let Err(e) = self.db.record_trade(
                &channel_id_str,
                action,
                asset_type,
                amount_usd,
                amount_btc,
                self.btc_price,
                fee_usd,
                Some(old_btc_pct),
                Some(new_btc_pct),
                payment_id,
                status,
            ) {
                eprintln!("Failed to record trade: {}", e);
            }
        }

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

                        {
                            let mut sc = self.stable_channel.lock().unwrap();
                            update_balances(&self.node, &mut sc);
                        }
                        self.update_balances(); // Update UI immediately

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

                        {
                            let mut sc = self.stable_channel.lock().unwrap();
                            update_balances(&self.node, &mut sc);
                        }
                        self.update_balances(); // Update UI immediately
                    
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

                        // Record payment in database
                        let (amount_usd, btc_price) = {
                            let sc = self.stable_channel.lock().unwrap();
                            let price = sc.latest_price;
                            let usd = if price > 0.0 {
                                Some((amount_msat as f64 / 1000.0 / 100_000_000.0) * price)
                            } else {
                                None
                            };
                            (usd, if price > 0.0 { Some(price) } else { None })
                        };
                        let _ = self.db.record_payment(
                            Some(&format!("{payment_hash}")),
                            "stability",
                            "received",
                            amount_msat,
                            amount_usd,
                            btc_price,
                            None,
                            "completed",
                        );

                        self.status_message = format!("Received payment of {} msats", amount_msat);
                        {
                            let mut sc = self.stable_channel.lock().unwrap();
                            update_balances(&self.node, &mut sc);
                        }
                        self.update_balances(); // Update UI immediately
                        self.show_onboarding = false;
                        self.waiting_for_payment = false;

                        // Show toast notification
                        let sats = amount_msat / 1000;
                        let toast_msg = if let Some(usd) = amount_usd {
                            format!("Received {} sats (${:.2})", sats, usd)
                        } else {
                            format!("Received {} sats", sats)
                        };
                        self.show_toast(&toast_msg, "+");
                    }


                    Event::PaymentSuccessful { payment_id: _, payment_hash, payment_preimage: _, fee_paid_msat: _ } => {
                        audit_event("PAYMENT_SUCCESSFUL", json!({
                            "payment_hash": format!("{payment_hash}"),
                        }));

                        // Note: We record outgoing trades separately in change_allocation
                        // This captures other outgoing payments (invoices, etc.)
                        // For now we don't record here to avoid double-counting trade fees

                        self.status_message = format!("Sent payment {}", payment_hash);
                        {
                            let mut sc = self.stable_channel.lock().unwrap();
                            update_balances(&self.node, &mut sc);
                        }
                        self.update_balances(); // Update UI immediately

                        // Show toast for sent payment
                        self.show_toast("Payment sent", "-");
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

                    Event::SplicePending { channel_id, new_funding_txo, .. } => {
                        audit_event("SPLICE_PENDING", json!({
                            "channel_id": format!("{channel_id}"),
                            "funding_txo": format!("{new_funding_txo}")
                        }));

                        // Record the on-chain transaction if we have pending splice info
                        if let Some(splice) = self.pending_splice.take() {
                            let btc_price = {
                                let sc = self.stable_channel.lock().unwrap();
                                if sc.latest_price > 0.0 { Some(sc.latest_price) } else { None }
                            };
                            let _ = self.db.record_onchain_tx(
                                &new_funding_txo.txid.to_string(),
                                &splice.direction,
                                splice.amount_sats,
                                splice.address.as_deref(),
                                btc_price,
                                "pending",
                            );
                        }

                        self.status_message = format!("Splice pending - tx: {}", new_funding_txo.txid);
                        self.show_toast("Splice pending", "~");
                        self.update_balances();
                    }

                    Event::SpliceFailed { channel_id, user_channel_id, .. } => {
                        audit_event("SPLICE_FAILED", json!({
                            "channel_id": format!("{channel_id}"),
                            "user_channel_id": format!("{:?}", user_channel_id)
                        }));
                        self.pending_splice = None;  // Clear pending splice on failure
                        self.status_message = "Splice failed".to_string();
                        self.show_toast("Splice failed", "!");
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

        fn format_time_ago(secs: u64) -> String {
            // Round to nearest 10 seconds
            let rounded = ((secs + 5) / 10) * 10;

            if rounded < 60 {
                format!("{}s ago", rounded)
            } else {
                let mins = rounded / 60;
                let remaining_secs = rounded % 60;
                if remaining_secs == 0 {
                    format!("{}m ago", mins)
                } else {
                    format!("{}m {}s ago", mins, remaining_secs)
                }
            }
        }

        /// Format a unix timestamp as a UTC datetime string
        fn format_timestamp(timestamp: i64) -> String {
            match Utc.timestamp_opt(timestamp, 0) {
                chrono::LocalResult::Single(dt) => dt.format("%Y-%m-%d %H:%M UTC").to_string(),
                _ => "Invalid time".to_string(),
            }
        }

        /// Format a price with comma separators (e.g., 100000 -> "100,000")
        fn format_price(price: f64) -> String {
            let price_int = price as i64;
            let formatted = price_int.to_string()
                .as_bytes()
                .rchunks(3)
                .rev()
                .map(|chunk| std::str::from_utf8(chunk).unwrap())
                .collect::<Vec<_>>()
                .join(",");
            format!("${}", formatted)
        }

        /// Show a toast notification
        fn show_toast(&mut self, message: &str, emoji: &str) {
            self.toasts.push(Toast::new(message, emoji, 6.0));
        }

        /// Render toast notifications
        fn render_toasts(&mut self, ctx: &egui::Context) {
            // Remove expired toasts
            self.toasts.retain(|t| !t.is_expired());

            if self.toasts.is_empty() {
                return;
            }

            // Request repaint for animation
            ctx.request_repaint();

            let screen_rect = ctx.screen_rect();
            let toast_width = 280.0;
            let toast_height = 60.0;
            let margin = 20.0;
            let spacing = 10.0;

            for (i, toast) in self.toasts.iter().enumerate() {
                let y_offset = margin + (i as f32 * (toast_height + spacing));
                let progress = toast.progress();

                // Fade in/out animation
                let alpha = if progress < 0.1 {
                    (progress / 0.1).min(1.0)
                } else if progress > 0.8 {
                    ((1.0 - progress) / 0.2).max(0.0)
                } else {
                    1.0
                };

                let bg_color = egui::Color32::from_rgba_unmultiplied(40, 120, 60, (220.0 * alpha) as u8);
                let text_color = egui::Color32::from_rgba_unmultiplied(255, 255, 255, (255.0 * alpha) as u8);

                egui::Area::new(egui::Id::new(format!("toast_{}", i)))
                    .fixed_pos(egui::pos2(
                        screen_rect.right() - toast_width - margin,
                        screen_rect.top() + y_offset,
                    ))
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        egui::Frame::none()
                            .fill(bg_color)
                            .rounding(12.0)
                            .inner_margin(egui::Margin::symmetric(16, 12))
                            .shadow(egui::epaint::Shadow {
                                offset: [0, 2],
                                blur: 8,
                                spread: 0,
                                color: egui::Color32::from_black_alpha(60),
                            })
                            .show(ui, |ui| {
                                ui.set_min_width(toast_width - 32.0);
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(&toast.emoji).size(24.0));
                                    ui.add_space(8.0);
                                    ui.vertical(|ui| {
                                        ui.label(egui::RichText::new(&toast.message)
                                            .size(14.0)
                                            .color(text_color)
                                            .strong());
                                    });
                                });
                            });
                    });
            }
        }

        fn show_price_info(&self, ui: &mut egui::Ui) {
            let (price, timestamp) = {
                let sc = self.stable_channel.lock().unwrap();
                (sc.latest_price, sc.timestamp)
            };

            let price_ok = price.is_finite() && price > 0.0;

            ui.vertical_centered(|ui| {
                if price_ok {
                    ui.label(
                        egui::RichText::new(format!("BTC/USD: {}", Self::format_currency(price)))
                            .size(14.0)
                            .color(egui::Color32::LIGHT_GRAY),
                    );

                    if timestamp > 0 {
                        let secs = SystemTime::now()
                            .duration_since(UNIX_EPOCH + std::time::Duration::from_secs(timestamp as u64))
                            .map(|d| d.as_secs())
                            .unwrap_or(0);

                        ui.label(
                            egui::RichText::new(Self::format_time_ago(secs))
                                .size(11.0)
                                .color(egui::Color32::DARK_GRAY),
                        );
                    }
                } else {
                    ui.label(
                        egui::RichText::new("BTC/USD: Loading...")
                            .italics()
                            .color(egui::Color32::GRAY)
                            .size(14.0),
                    );
                }
            });
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
                    ui.label("Bolt11 Lightning invoice.");
                    ui.add_space(8.0);
                    if let Some(ref qr) = self.qr_texture {
                        ui.image(qr);
                    } else {
                        ui.label("Lightning QR Missing");
                    }
                    ui.add_space(10.0);
                    self.show_price_info(ui);
                    ui.add_space(10.0);
                    ui.add(
                        egui::TextEdit::multiline(&mut self.invoice_result)
                            .frame(true)
                            .desired_width(400.0)
                            .desired_rows(3)
                            .hint_text("Invoice..."),
                    );
                    ui.add_space(8.0);
        
                    // Button 1: Copy
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
                        self.status_message = "Invoice copied".to_string();
                    }
        
                    ui.add_space(5.0);
        
                    // Button 2: Back
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
        
                    // ↓↓↓ Moved the message to be **below both buttons**
                    ui.add_space(6.0);
                    if !self.status_message.is_empty() {
                        ui.label(
                            egui::RichText::new(&self.status_message)
                                .color(egui::Color32::WHITE),
                        );
                    }
                    // ↑↑↑ end move
        
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

                    ui.add_space(20.0);
                    self.show_price_info(ui);
                    ui.add_space(30.0);

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
                                        Ok(addr) => match addr.require_network(self.network) {
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
                                RichText::new("Stability:")
                                    .strong()
                                    .color(Color32::from_rgb(247, 147, 26))
                            );

                            // Calculate overcollateralization ratio
                            let (_collateral_ratio, status_color, status_text) = {
                                let sc = self.stable_channel.lock().unwrap();

                                // Total channel value = user's side + LSP's side
                                let total_channel_usd = sc.stable_receiver_usd.0 + sc.stable_provider_usd.0;

                                // User's stable portion (the USD-pegged amount)
                                let user_stable_usd = sc.stable_receiver_usd.0 * sc.allocation.usd_weight;

                                if user_stable_usd < 1.0 {
                                    (0.0, Color32::GRAY, "Waiting for balance...".to_string())
                                } else {
                                    // Ratio = total channel / stable requirement
                                    let ratio = total_channel_usd / user_stable_usd;
                                    let color = if ratio >= 1.7 {
                                        Color32::GREEN      // 70%+ overcollateralized
                                    } else if ratio >= 1.3 {
                                        Color32::YELLOW     // 30-70% overcollateralized
                                    } else {
                                        Color32::RED        // Under 30% overcollateralized
                                    };
                                    let pct = ((ratio - 1.0) * 100.0).max(0.0);
                                    let text = format!("{:.0}% overcollateralized\nChannel total: ${:.2}\nYour stable: ${:.2}", pct, total_channel_usd, user_stable_usd);
                                    (ratio, color, text)
                                }
                            };

                            let dot_size = 12.0;
                            let (rect, response) = ui.allocate_exact_size(Vec2::splat(dot_size), Sense::hover());
                            ui.painter()
                                .circle_filled(rect.center(), dot_size * 0.5, status_color);

                            if response.hovered() {
                                egui::show_tooltip(ui.ctx(), ui.layer_id(), egui::Id::new("stability_tooltip"), |ui| {
                                    ui.label(&status_text);
                                });
                            }
                        });
                        ui.add_space(10.0);
                        ui.add_space(30.0);
        
                        ui.group(|ui| {
                            let sc = self.stable_channel.lock().unwrap();

                            // Select correct values based on role
                            let total_usd = if sc.is_stable_receiver {
                                sc.stable_receiver_usd
                            } else {
                                sc.stable_provider_usd
                            };

                            let pegged_btc = if sc.is_stable_receiver {
                                sc.stable_receiver_btc
                            } else {
                                sc.stable_provider_btc
                            };

                            // Calculate portions based on allocation
                            let stable_usd_value = total_usd.0 * sc.allocation.usd_weight;
                            let native_usd_value = total_usd.0 * sc.allocation.btc_weight;
                            let total_btc_f64 = pegged_btc.to_btc() + self.onchain_balance_btc;

                            ui.add_space(8.0);

                            // Total Balance - prominent display
                            ui.heading("Your Total Balance");
                            ui.add_space(6.0);

                            let total_display = if total_usd.0 <= MIN_DISPLAY_USD {
                                "---".to_string()
                            } else {
                                format!("${:.2}", total_usd.0)
                            };

                            ui.label(
                                egui::RichText::new(total_display)
                                    .size(28.0)
                                    .strong(),
                            );

                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!("{:.8} BTC", total_btc_f64))
                                    .size(13.0)
                                    .color(egui::Color32::GRAY)
                                    .monospace(),
                            );

                            ui.add_space(16.0);
                            ui.separator();
                            ui.add_space(12.0);

                            // Breakdown section
                            if total_usd.0 > MIN_DISPLAY_USD {
                                egui::Grid::new("balance_breakdown_grid")
                                    .spacing(Vec2::new(12.0, 8.0))
                                    .show(ui, |ui| {
                                        // Stabilized BTC row
                                        ui.label(
                                            egui::RichText::new("Stabilized BTC (USD)")
                                                .size(14.0)
                                                .color(egui::Color32::from_rgb(100, 200, 100)),
                                        );
                                        ui.label(
                                            egui::RichText::new(format!("${:.2}", stable_usd_value))
                                                .size(14.0)
                                                .strong(),
                                        );
                                        ui.end_row();

                                        // Bitcoin BTC row (only show if there's BTC allocation)
                                        if sc.allocation.btc_percent() > 0 {
                                            ui.label(
                                                egui::RichText::new("Bitcoin (BTC)")
                                                    .size(14.0)
                                                    .color(egui::Color32::from_rgb(247, 147, 26)),
                                            );
                                            ui.label(
                                                egui::RichText::new(format!("${:.2}", native_usd_value))
                                                    .size(14.0)
                                                    .strong(),
                                            );
                                            ui.end_row();
                                        }
                                    });
                            }

                            ui.add_space(8.0);
                        });

                        ui.add_space(20.0);

                        // Buy/Sell BTC Section
                        ui.group(|ui| {
                            ui.add_space(10.0);
                            ui.vertical_centered(|ui| {
                                ui.heading("Buy / Sell BTC");
                                ui.add_space(15.0);

                                // Get current allocation info
                                let (current_btc_pct, current_usd_pct, stable_usd, native_usd, total_usd) = {
                                    let sc = self.stable_channel.lock().unwrap();
                                    let total = sc.stable_receiver_usd.0;
                                    let usd_pct = sc.allocation.usd_percent();
                                    let btc_pct = sc.allocation.btc_percent();
                                    let stable = total * sc.allocation.usd_weight;
                                    let native = total * sc.allocation.btc_weight;
                                    (btc_pct, usd_pct, stable, native, total)
                                };

                                // Amount input
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("$").size(18.0));
                                    let response = ui.add(
                                        egui::TextEdit::singleline(&mut self.trade_amount_input)
                                            .desired_width(100.0)
                                            .font(egui::TextStyle::Heading)
                                            .hint_text("0.00")
                                    );
                                    if response.changed() {
                                        self.trade_error.clear();
                                    }
                                });

                                // Show error if any
                                if !self.trade_error.is_empty() {
                                    ui.add_space(5.0);
                                    ui.label(
                                        egui::RichText::new(&self.trade_error)
                                            .size(12.0)
                                            .color(egui::Color32::from_rgb(255, 100, 100))
                                    );
                                }

                                ui.add_space(15.0);

                                // Softer colors
                                let buy_green = egui::Color32::from_rgb(46, 125, 50);
                                let buy_green_disabled = egui::Color32::from_rgb(40, 60, 40);
                                let sell_red = egui::Color32::from_rgb(183, 28, 28);
                                let sell_red_disabled = egui::Color32::from_rgb(60, 40, 40);

                                // Buy/Sell buttons
                                let can_buy = current_usd_pct > 0 && stable_usd > 0.01;
                                let can_sell = current_btc_pct > 0 && native_usd > 0.01;

                                // Buy BTC row
                                ui.horizontal(|ui| {
                                    let buy_btn = egui::Button::new(
                                        egui::RichText::new("Buy BTC")
                                            .size(14.0)
                                            .color(if can_buy { egui::Color32::WHITE } else { egui::Color32::DARK_GRAY })
                                    )
                                    .min_size(egui::vec2(100.0, 35.0))
                                    .fill(if can_buy { buy_green } else { buy_green_disabled });

                                    if ui.add_enabled(can_buy, buy_btn).clicked() {
                                        self.trade_error.clear();
                                        match self.trade_amount_input.parse::<f64>() {
                                            Ok(amount) if amount <= 0.0 => {
                                                self.trade_error = "Enter a positive amount".to_string();
                                            }
                                            Ok(amount) if amount > stable_usd => {
                                                self.trade_error = format!("Max ${:.2} available", stable_usd);
                                            }
                                            Ok(amount) => {
                                                let new_native_usd = native_usd + amount;
                                                let new_btc_pct = ((new_native_usd / total_usd) * 100.0).round() as u8;
                                                self.pending_trade = Some(PendingTrade {
                                                    action: TradeAction::BuyBtc,
                                                    amount_usd: amount,
                                                    new_btc_percent: new_btc_pct.min(100),
                                                });
                                                self.show_confirm_trade = true;
                                            }
                                            Err(_) => {
                                                self.trade_error = "Enter a valid number".to_string();
                                            }
                                        }
                                    }

                                    ui.add_space(10.0);

                                    // Buy Max button with frame
                                    if can_buy {
                                        let buy_max_btn = egui::Button::new(
                                            egui::RichText::new("Buy Max")
                                                .size(12.0)
                                                .color(egui::Color32::from_rgb(100, 180, 100))
                                        )
                                        .min_size(egui::vec2(70.0, 28.0))
                                        .fill(egui::Color32::TRANSPARENT)
                                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 180, 100)));

                                        if ui.add(buy_max_btn).clicked() {
                                            self.trade_error.clear();
                                            self.pending_trade = Some(PendingTrade {
                                                action: TradeAction::BuyBtc,
                                                amount_usd: stable_usd,
                                                new_btc_percent: 100,
                                            });
                                            self.show_confirm_trade = true;
                                        }
                                    }
                                });

                                ui.add_space(8.0);

                                // Sell BTC row
                                ui.horizontal(|ui| {
                                    let sell_btn = egui::Button::new(
                                        egui::RichText::new("Sell BTC")
                                            .size(14.0)
                                            .color(if can_sell { egui::Color32::WHITE } else { egui::Color32::DARK_GRAY })
                                    )
                                    .min_size(egui::vec2(100.0, 35.0))
                                    .fill(if can_sell { sell_red } else { sell_red_disabled });

                                    if ui.add_enabled(can_sell, sell_btn).clicked() {
                                        self.trade_error.clear();
                                        match self.trade_amount_input.parse::<f64>() {
                                            Ok(amount) if amount <= 0.0 => {
                                                self.trade_error = "Enter a positive amount".to_string();
                                            }
                                            Ok(amount) if amount > native_usd => {
                                                self.trade_error = format!("Max ${:.2} available", native_usd);
                                            }
                                            Ok(amount) => {
                                                let new_native_usd = native_usd - amount;
                                                let new_btc_pct = ((new_native_usd / total_usd) * 100.0).round() as u8;
                                                self.pending_trade = Some(PendingTrade {
                                                    action: TradeAction::SellBtc,
                                                    amount_usd: amount,
                                                    new_btc_percent: new_btc_pct,
                                                });
                                                self.show_confirm_trade = true;
                                            }
                                            Err(_) => {
                                                self.trade_error = "Enter a valid number".to_string();
                                            }
                                        }
                                    }

                                    ui.add_space(10.0);

                                    // Sell Max button with frame
                                    if can_sell {
                                        let sell_max_btn = egui::Button::new(
                                            egui::RichText::new("Sell Max")
                                                .size(12.0)
                                                .color(egui::Color32::from_rgb(200, 120, 120))
                                        )
                                        .min_size(egui::vec2(70.0, 28.0))
                                        .fill(egui::Color32::TRANSPARENT)
                                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(200, 120, 120)));

                                        if ui.add(sell_max_btn).clicked() {
                                            self.trade_error.clear();
                                            self.pending_trade = Some(PendingTrade {
                                                action: TradeAction::SellBtc,
                                                amount_usd: native_usd,
                                                new_btc_percent: 0,
                                            });
                                            self.show_confirm_trade = true;
                                        }
                                    }
                                });

                                ui.add_space(10.0);
                            });

                            // On-chain & Splice section
                            ui.add_space(20.0);
                            ui.separator();
                            ui.add_space(10.0);

                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("On-chain").size(16.0).strong());
                                ui.add_space(10.0);
                                ui.label(egui::RichText::new(format!("{:.8} BTC", self.onchain_balance_btc)).size(14.0).color(egui::Color32::GRAY));
                            });

                            ui.add_space(10.0);

                            ui.horizontal(|ui| {
                                // Deposit (Splice In) button
                                let deposit_btn = egui::Button::new(
                                    egui::RichText::new("Deposit")
                                        .size(13.0)
                                        .color(egui::Color32::WHITE)
                                )
                                .min_size(egui::vec2(90.0, 32.0))
                                .fill(egui::Color32::from_rgb(70, 130, 180));

                                if ui.add(deposit_btn).clicked() {
                                    self.show_onchain_receive = !self.show_onchain_receive;
                                    self.show_onchain_send = false;
                                    if self.show_onchain_receive && self.on_chain_address.is_empty() {
                                        self.get_address();
                                    }
                                }

                                ui.add_space(10.0);

                                // Withdraw (Splice Out) button
                                let withdraw_btn = egui::Button::new(
                                    egui::RichText::new("Withdraw")
                                        .size(13.0)
                                        .color(egui::Color32::WHITE)
                                )
                                .min_size(egui::vec2(90.0, 32.0))
                                .fill(egui::Color32::from_rgb(180, 100, 70));

                                if ui.add(withdraw_btn).clicked() {
                                    self.show_onchain_send = !self.show_onchain_send;
                                    self.show_onchain_receive = false;
                                }
                            });

                            // Deposit section (Splice In)
                            if self.show_onchain_receive {
                                ui.add_space(10.0);
                                ui.group(|ui| {
                                    ui.label(egui::RichText::new("Deposit to Channel (Splice In)").strong());
                                    ui.add_space(5.0);

                                    // Show on-chain address for receiving
                                    ui.label(egui::RichText::new("1. Send BTC to this address:").size(12.0));
                                    ui.add_space(3.0);

                                    if self.on_chain_address.is_empty() {
                                        if ui.button("Generate Address").clicked() {
                                            self.get_address();
                                        }
                                    } else {
                                        ui.label(
                                            egui::RichText::new(&self.on_chain_address)
                                                .monospace()
                                                .size(10.0)
                                        );

                                        ui.add_space(5.0);
                                        ui.horizontal(|ui| {
                                            if ui.button("Copy").clicked() {
                                                ui.output_mut(|o| o.copied_text = self.on_chain_address.clone());
                                                self.status_message = "Address copied!".to_string();
                                            }
                                            if ui.button("New Address").clicked() {
                                                self.get_address();
                                            }
                                        });

                                        // QR Code
                                        ui.add_space(10.0);
                                        if let Ok(qr) = QrCode::new(self.on_chain_address.as_bytes()) {
                                            let qr_size = 150;
                                            let module_size = qr_size / qr.width();
                                            let mut img = GrayImage::new(qr_size as u32, qr_size as u32);

                                            for y in 0..qr.width() {
                                                for x in 0..qr.width() {
                                                    let color = if qr[(x, y)] == Color::Dark {
                                                        Luma([0u8])
                                                    } else {
                                                        Luma([255u8])
                                                    };
                                                    for dy in 0..module_size {
                                                        for dx in 0..module_size {
                                                            let px = x * module_size + dx;
                                                            let py = y * module_size + dy;
                                                            if px < qr_size && py < qr_size {
                                                                img.put_pixel(px as u32, py as u32, color);
                                                            }
                                                        }
                                                    }
                                                }
                                            }

                                            let pixels: Vec<egui::Color32> = img
                                                .pixels()
                                                .map(|p| {
                                                    let v = p.0[0];
                                                    egui::Color32::from_rgb(v, v, v)
                                                })
                                                .collect();

                                            let texture = ctx.load_texture(
                                                "onchain_qr",
                                                egui::ColorImage {
                                                    size: [qr_size, qr_size],
                                                    pixels,
                                                },
                                                TextureOptions::NEAREST,
                                            );
                                            ui.image(&texture);
                                        }
                                    }

                                    // Splice In section
                                    ui.add_space(15.0);
                                    ui.separator();
                                    ui.add_space(10.0);

                                    ui.label(egui::RichText::new("2. Splice funds into channel:").size(12.0));
                                    ui.add_space(5.0);

                                    ui.label(
                                        egui::RichText::new(format!("On-chain balance: {} sats", (self.onchain_balance_btc * 100_000_000.0) as u64))
                                            .size(11.0)
                                            .color(egui::Color32::GRAY)
                                    );

                                    ui.add_space(5.0);

                                    ui.horizontal(|ui| {
                                        ui.label("Amount (sats):");
                                        let amount_edit = egui::TextEdit::singleline(&mut self.splice_in_amount)
                                            .desired_width(120.0)
                                            .hint_text("0");
                                        ui.add(amount_edit);
                                    });

                                    ui.add_space(8.0);

                                    if ui.button("Splice In to Channel").clicked() {
                                        if let Ok(amount_sats) = self.splice_in_amount.parse::<u64>() {
                                            let target_channel_id = {
                                                let sc = self.stable_channel.lock().unwrap();
                                                sc.channel_id
                                            };

                                            // Find the channel to get its user_channel_id
                                            let channel_info = self.node.list_channels()
                                                .into_iter()
                                                .find(|ch| ch.channel_id == target_channel_id);

                                            if let Some(ch) = channel_info {
                                                match self.node.splice_in(&ch.user_channel_id, ch.counterparty_node_id, amount_sats) {
                                                    Ok(()) => {
                                                        self.pending_splice = Some(PendingSplice {
                                                            direction: "in".to_string(),
                                                            amount_sats,
                                                            address: None,
                                                        });
                                                        self.status_message = format!("Splice-in initiated: {} sats", amount_sats);
                                                        self.show_toast("Splice-in started", "+");
                                                        self.splice_in_amount.clear();
                                                        self.update_balances();
                                                    }
                                                    Err(e) => {
                                                        self.status_message = format!("Splice-in failed: {}", e);
                                                    }
                                                }
                                            } else {
                                                self.status_message = "No channel found".to_string();
                                            }
                                        } else {
                                            self.status_message = "Enter a valid amount".to_string();
                                        }
                                    }
                                });
                            }

                            // Withdraw section (Splice Out)
                            if self.show_onchain_send {
                                ui.add_space(10.0);
                                ui.group(|ui| {
                                    ui.label(egui::RichText::new("Withdraw from Channel (Splice Out)").strong());
                                    ui.add_space(5.0);

                                    ui.horizontal(|ui| {
                                        ui.label("To address:");
                                    });
                                    let addr_edit = egui::TextEdit::singleline(&mut self.splice_out_address)
                                        .font(egui::TextStyle::Monospace)
                                        .desired_width(280.0)
                                        .hint_text("bc1q...");
                                    ui.add(addr_edit);

                                    ui.add_space(5.0);

                                    ui.horizontal(|ui| {
                                        ui.label("Amount (sats):");
                                        let amount_edit = egui::TextEdit::singleline(&mut self.splice_out_amount)
                                            .desired_width(120.0)
                                            .hint_text("0");
                                        ui.add(amount_edit);
                                    });

                                    ui.add_space(8.0);

                                    if ui.button("Splice Out from Channel").clicked() {
                                        if let Ok(amount_sats) = self.splice_out_amount.parse::<u64>() {
                                            match ldk_node::bitcoin::Address::from_str(&self.splice_out_address) {
                                                Ok(addr) => match addr.require_network(self.network) {
                                                    Ok(valid_addr) => {
                                                        let target_channel_id = {
                                                            let sc = self.stable_channel.lock().unwrap();
                                                            sc.channel_id
                                                        };

                                                        // Find the channel to get its user_channel_id
                                                        let channel_info = self.node.list_channels()
                                                            .into_iter()
                                                            .find(|ch| ch.channel_id == target_channel_id);

                                                        if let Some(ch) = channel_info {
                                                            let out_address = self.splice_out_address.clone();
                                                            match self.node.splice_out(&ch.user_channel_id, ch.counterparty_node_id, &valid_addr, amount_sats) {
                                                                Ok(()) => {
                                                                    self.pending_splice = Some(PendingSplice {
                                                                        direction: "out".to_string(),
                                                                        amount_sats,
                                                                        address: Some(out_address),
                                                                    });
                                                                    self.status_message = format!("Splice-out initiated: {} sats", amount_sats);
                                                                    self.show_toast("Splice-out started", "-");
                                                                    self.splice_out_address.clear();
                                                                    self.splice_out_amount.clear();
                                                                    self.update_balances();
                                                                }
                                                                Err(e) => {
                                                                    self.status_message = format!("Splice-out failed: {}", e);
                                                                }
                                                            }
                                                        } else {
                                                            self.status_message = "No channel found".to_string();
                                                        }
                                                    }
                                                    Err(e) => {
                                                        self.status_message = format!("Invalid network: {}", e);
                                                    }
                                                },
                                                Err(e) => {
                                                    self.status_message = format!("Invalid address: {}", e);
                                                }
                                            }
                                        } else {
                                            self.status_message = "Enter a valid amount".to_string();
                                        }
                                    }

                                    ui.add_space(10.0);

                                    let channel_balance_sats = (self.lightning_balance_btc * 100_000_000.0) as u64;
                                    ui.label(
                                        egui::RichText::new(format!("Channel balance: {} sats", channel_balance_sats))
                                            .size(11.0)
                                            .color(egui::Color32::GRAY)
                                    );
                                });
                            }

                            ui.add_space(10.0);

                            // Confirmation dialog
                            if self.show_confirm_trade {
                                if let Some(trade) = self.pending_trade.clone() {
                                    let price = self.btc_price.max(1.0);
                                    let action_str = match trade.action {
                                        TradeAction::BuyBtc => "Buy",
                                        TradeAction::SellBtc => "Sell",
                                    };
                                    let new_btc_pct = trade.new_btc_percent;
                                    let trade_amount = trade.amount_usd;

                                    egui::Window::new(format!("Confirm {} BTC", action_str))
                                        .collapsible(false)
                                        .resizable(false)
                                        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                                        .show(ctx, |ui| {
                                            ui.add_space(8.0);

                                            let fee = trade_amount * 0.01;
                                            let amount_after_fee = trade_amount - fee;
                                            let btc_after_fee = amount_after_fee / price;

                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "{} ${:.2} worth of BTC",
                                                    action_str, trade_amount
                                                ))
                                                .size(16.0)
                                                .strong()
                                            );

                                            ui.add_space(10.0);

                                            egui::Grid::new("trade_details")
                                                .spacing(egui::vec2(10.0, 4.0))
                                                .show(ui, |ui| {
                                                    ui.label("Amount:");
                                                    ui.label(format!("${:.2}", trade_amount));
                                                    ui.end_row();

                                                    ui.label("Fee (1%):");
                                                    ui.label(
                                                        egui::RichText::new(format!("-${:.2}", fee))
                                                            .color(egui::Color32::from_rgb(200, 150, 100))
                                                    );
                                                    ui.end_row();

                                                    ui.label("You receive:");
                                                    ui.label(
                                                        egui::RichText::new(format!("≈ {:.8} BTC", btc_after_fee))
                                                            .strong()
                                                    );
                                                    ui.end_row();

                                                    ui.label("Price:");
                                                    ui.label(
                                                        egui::RichText::new(format!("${:.2}/BTC", price))
                                                            .color(egui::Color32::GRAY)
                                                    );
                                                    ui.end_row();
                                                });

                                            ui.add_space(12.0);
                                            ui.horizontal(|ui| {
                                                if ui.button("Confirm").clicked() {
                                                    self.change_allocation(new_btc_pct as i32, fee, trade_amount);
                                                    self.show_confirm_trade = false;
                                                    self.pending_trade = None;
                                                    self.trade_amount_input.clear();
                                                }
                                                if ui.button("Cancel").clicked() {
                                                    self.show_confirm_trade = false;
                                                    self.pending_trade = None;
                                                }
                                            });

                                            ui.add_space(8.0);
                                        });
                                }
                            }
                        });
                        
                        ui.add_space(20.0);
        
                        ui.group(|ui| {
                            let sc = self.stable_channel.lock().unwrap();
                            ui.add_space(20.0);
                            ui.heading("Bitcoin Price");
                            ui.add_space(10.0);

                            let price_ok = sc.latest_price.is_finite() && sc.latest_price > 0.0;

                            ui.vertical_centered(|ui| {
                                if price_ok {
                                    ui.label(
                                        egui::RichText::new(Self::format_currency(sc.latest_price))
                                            .size(20.0)
                                            .strong()
                                    );

                                    if sc.timestamp > 0 {
                                        let secs = SystemTime::now()
                                            .duration_since(UNIX_EPOCH + std::time::Duration::from_secs(sc.timestamp as u64))
                                            .map(|d| d.as_secs())
                                            .unwrap_or(0);

                                        ui.add_space(8.0);
                                        ui.label(
                                            egui::RichText::new(Self::format_time_ago(secs))
                                                .size(12.0)
                                                .color(egui::Color32::GRAY),
                                        );
                                    }
                                } else {
                                    ui.label(
                                        egui::RichText::new("Fetching latest price ...")
                                            .italics()
                                            .color(egui::Color32::LIGHT_GRAY)
                                            .size(16.0),
                                    );
                                }
                            });

                            ui.add_space(20.0);
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
                                            Ok(addr) => match addr.require_network(self.network) {
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

                                // Trade History section
                                ui.group(|ui| {
                                    ui.heading("Trade History");
                                    ui.add_space(5.0);

                                    let channel_id_str = {
                                        let sc = self.stable_channel.lock().unwrap();
                                        sc.channel_id.to_string()
                                    };

                                    match self.db.get_recent_trades(&channel_id_str, 10) {
                                        Ok(trades) if trades.is_empty() => {
                                            ui.label(egui::RichText::new("No trades yet").color(egui::Color32::GRAY));
                                        }
                                        Ok(trades) => {
                                            egui::Grid::new("trades_grid")
                                                .striped(true)
                                                .spacing(egui::vec2(8.0, 4.0))
                                                .show(ui, |ui| {
                                                    // Header
                                                    ui.label(egui::RichText::new("ID").strong().size(11.0));
                                                    ui.label(egui::RichText::new("Action").strong().size(11.0));
                                                    ui.label(egui::RichText::new("USD").strong().size(11.0));
                                                    ui.label(egui::RichText::new("BTC").strong().size(11.0));
                                                    ui.label(egui::RichText::new("Price").strong().size(11.0));
                                                    ui.label(egui::RichText::new("Fee").strong().size(11.0));
                                                    ui.label(egui::RichText::new("Time (UTC)").strong().size(11.0));
                                                    ui.end_row();

                                                    for trade in trades.iter().take(5) {
                                                        let action_color = if trade.action == "buy" {
                                                            egui::Color32::from_rgb(100, 200, 100)
                                                        } else {
                                                            egui::Color32::from_rgb(200, 100, 100)
                                                        };
                                                        // Use payment_id as trade ID if available, otherwise show db id
                                                        let trade_id = trade.payment_id.as_ref()
                                                            .map(|pid| pid.chars().take(8).collect::<String>())
                                                            .unwrap_or_else(|| format!("#{}", trade.id));
                                                        ui.label(egui::RichText::new(&trade_id).size(11.0).color(egui::Color32::GRAY));
                                                        ui.label(egui::RichText::new(trade.action.to_uppercase()).size(11.0).color(action_color));
                                                        ui.label(egui::RichText::new(format!("${:.2}", trade.amount_usd)).size(11.0));
                                                        ui.label(egui::RichText::new(format!("{:.8}", trade.amount_btc)).size(11.0));
                                                        ui.label(egui::RichText::new(Self::format_price(trade.btc_price)).size(11.0));
                                                        ui.label(egui::RichText::new(format!("${:.2}", trade.fee_usd)).size(11.0));
                                                        ui.label(egui::RichText::new(Self::format_timestamp(trade.created_at)).size(11.0).color(egui::Color32::GRAY));
                                                        ui.end_row();
                                                    }
                                                });
                                        }
                                        Err(_) => {
                                            ui.label(egui::RichText::new("Error loading trades").color(egui::Color32::RED));
                                        }
                                    }
                                });
                                ui.add_space(20.0);

                                // Payments section
                                ui.group(|ui| {
                                    ui.heading("Recent Payments");
                                    ui.add_space(5.0);

                                    match self.db.get_recent_payments(10) {
                                        Ok(payments) if payments.is_empty() => {
                                            ui.label(egui::RichText::new("No payments yet").color(egui::Color32::GRAY));
                                        }
                                        Ok(payments) => {
                                            egui::Grid::new("payments_grid")
                                                .striped(true)
                                                .spacing(egui::vec2(8.0, 4.0))
                                                .show(ui, |ui| {
                                                    // Header
                                                    ui.label(egui::RichText::new("ID").strong().size(11.0));
                                                    ui.label(egui::RichText::new("Type").strong().size(11.0));
                                                    ui.label(egui::RichText::new("Direction").strong().size(11.0));
                                                    ui.label(egui::RichText::new("Amount").strong().size(11.0));
                                                    ui.label(egui::RichText::new("USD").strong().size(11.0));
                                                    ui.label(egui::RichText::new("Time (UTC)").strong().size(11.0));
                                                    ui.end_row();

                                                    for payment in payments.iter().take(5) {
                                                        let (dir_color, dir_label) = if payment.direction == "received" {
                                                            (egui::Color32::from_rgb(100, 200, 100), "Received")
                                                        } else {
                                                            (egui::Color32::from_rgb(200, 150, 100), "Sent")
                                                        };

                                                        let type_label = if payment.payment_type == "stability" {
                                                            "Stability"
                                                        } else {
                                                            "Manual"
                                                        };

                                                        // Payment ID (truncated)
                                                        let id_display = payment.payment_id.as_ref()
                                                            .map(|id| if id.len() > 8 { format!("{}...", &id[..8]) } else { id.clone() })
                                                            .unwrap_or_else(|| format!("#{}", payment.id));
                                                        ui.label(egui::RichText::new(id_display).size(11.0).color(egui::Color32::GRAY));

                                                        ui.label(egui::RichText::new(type_label).size(11.0));
                                                        ui.label(egui::RichText::new(dir_label).size(11.0).color(dir_color));
                                                        ui.label(egui::RichText::new(format!("{} sats", payment.amount_msat / 1000)).size(11.0));

                                                        if let Some(usd) = payment.amount_usd {
                                                            ui.label(egui::RichText::new(format!("${:.2}", usd)).size(11.0));
                                                        } else {
                                                            ui.label(egui::RichText::new("—").size(11.0));
                                                        }

                                                        ui.label(egui::RichText::new(Self::format_timestamp(payment.created_at)).size(11.0).color(egui::Color32::GRAY));
                                                        ui.end_row();
                                                    }
                                                });
                                        }
                                        Err(_) => {
                                            ui.label(egui::RichText::new("Error loading payments").color(egui::Color32::RED));
                                        }
                                    }
                                });
                                ui.add_space(20.0);

                                // On-chain transactions section
                                ui.group(|ui| {
                                    ui.heading("On-chain Transactions");
                                    ui.add_space(5.0);

                                    match self.db.get_recent_onchain_txs(10) {
                                        Ok(txs) if txs.is_empty() => {
                                            ui.label(egui::RichText::new("No on-chain transactions yet").color(egui::Color32::GRAY));
                                        }
                                        Ok(txs) => {
                                            egui::Grid::new("onchain_txs_grid")
                                                .striped(true)
                                                .spacing(egui::vec2(8.0, 4.0))
                                                .show(ui, |ui| {
                                                    // Header
                                                    ui.label(egui::RichText::new("TxID").strong().size(11.0));
                                                    ui.label(egui::RichText::new("Type").strong().size(11.0));
                                                    ui.label(egui::RichText::new("Amount").strong().size(11.0));
                                                    ui.label(egui::RichText::new("Status").strong().size(11.0));
                                                    ui.label(egui::RichText::new("Time (UTC)").strong().size(11.0));
                                                    ui.end_row();

                                                    for tx in txs.iter().take(5) {
                                                        let (dir_color, dir_label) = if tx.direction == "in" {
                                                            (egui::Color32::from_rgb(100, 200, 100), "Deposit")
                                                        } else {
                                                            (egui::Color32::from_rgb(200, 150, 100), "Withdraw")
                                                        };

                                                        // TxID (truncated)
                                                        let txid_display = if tx.txid.len() > 12 {
                                                            format!("{}...", &tx.txid[..12])
                                                        } else {
                                                            tx.txid.clone()
                                                        };
                                                        ui.label(egui::RichText::new(txid_display).size(11.0).color(egui::Color32::GRAY));

                                                        ui.label(egui::RichText::new(dir_label).size(11.0).color(dir_color));
                                                        ui.label(egui::RichText::new(format!("{} sats", tx.amount_sats)).size(11.0));

                                                        let status_color = match tx.status.as_str() {
                                                            "confirmed" => egui::Color32::from_rgb(100, 200, 100),
                                                            "pending" => egui::Color32::from_rgb(200, 200, 100),
                                                            _ => egui::Color32::GRAY,
                                                        };
                                                        ui.label(egui::RichText::new(&tx.status).size(11.0).color(status_color));

                                                        ui.label(egui::RichText::new(Self::format_timestamp(tx.created_at)).size(11.0).color(egui::Color32::GRAY));
                                                        ui.end_row();
                                                    }
                                                });
                                        }
                                        Err(_) => {
                                            ui.label(egui::RichText::new("Error loading transactions").color(egui::Color32::RED));
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

            // Render toast notifications on top
            self.render_toasts(ctx);

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

