        use axum::extract::Path as AxumPath;
        use ldk_node::{
            bitcoin::{Network, Address, secp256k1::PublicKey},
            lightning_invoice::{Bolt11Invoice, Description, Bolt11InvoiceDescription},
            lightning::ln::msgs::SocketAddress,
            config::ChannelConfig,
            lightning_types::payment::PaymentHash,
            Builder, Node, Event, liquidity::LSPS2ServiceConfig, CustomTlvRecord,
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

        use stable_channels::audit::{audit_event, set_audit_log_path};
        use stable_channels::price_feeds::get_cached_price;
        use stable_channels::stable;
        use stable_channels::types::{USD, Bitcoin, StableChannel};

        // HTTP
        use axum::{routing::{get, post}, Json, Router};
        use anyhow::Result;

        static APP: Lazy<Mutex<ServerApp>> = Lazy::new(|| {
            Mutex::new(ServerApp::new_with_mode("lsp"))
        });

        const LSP_DATA_DIR: &str = "/home/ubuntu/.local/share/StableChannels/lsp";
        const LSP_NODE_ALIAS: &str = "lsp";
        const LSP_PORT: u16 = 9737;

        const DEFAULT_NETWORK: &str = "bitcoin";
        const DEFAULT_CHAIN_SOURCE_URL: &str = "https://mempool.space/api";        
        const EXPECTED_USD: f64 = 100.0;

        #[derive(Deserialize, Debug)]
        struct ReallocationSignedMessage {
            payload: String,
            signature: String,
        }

        #[derive(Deserialize, Debug)]
        struct ReallocationPayload {
            #[serde(rename = "type")]
            kind: String,
            channel_id: String,
            proposed_allocation: ReallocationAllocation,
        }

        #[derive(Deserialize, Debug)]
        struct ReallocationAllocation {
            pct_btc: u8,
            pct_usd: u8,
        }


        #[derive(Serialize, Deserialize, Clone, Debug)]
        struct StableChannelEntry {
            channel_id: String,
            expected_usd: f64,
            #[serde(default)]
            native_btc: f64,
            note: Option<String>,
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
            pub note: Option<String>,
        }

        #[derive(Serialize)]
        struct Balance {
            total_sats: u64,
            total_usd:  f64,
            lightning_sats: u64,
            lightning_usd:  f64,
            onchain_sats:   u64,
            onchain_usd:    f64,
        }

        #[derive(Deserialize)]
        struct PayReq { invoice: String }

        #[derive(Deserialize)]
        struct EditStableChannelReq {
            channel_id: String,
            target_usd: Option<String>,
            note: Option<String>,
        }

        #[derive(Serialize)]
        struct EditStableChannelRes {
            ok: bool,
            status: String,
        }

        #[derive(Deserialize)]
        struct OnchainSendReq {
            address: String,
            amount:  String,   // sats, still as string for reuse
        }

        #[derive(Deserialize)]
        struct ConnectReq {
            node_id: String,
            address: String,
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
                .route("/api/edit_stable_channel", post(edit_stable_channel_handler))
                .route("/api/onchain_send", post(onchain_send_handler))
                .route("/api/onchain_address", get(get_onchain_address))
                .route("/api/connect", post(connect_handler));


            let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
            println!("Backend running at http://0.0.0.0:8080");
            axum::serve(listener, app).await?;
            Ok(())
        }

        /* ---- handlers ---------------------------------------------------- */

        // GET /api/balance
        async fn get_balance() -> Json<Balance> {
            let (total_usd, lightning_usd, onchain_usd, lightning_sats, onchain_sats) = {
                let mut app = APP.lock().unwrap();

                // Refresh cached price + app.{lightning, onchain, total}_* fields
                app.update_balances();

                // Pull sat balances from LDK directly to avoid any float rounding
                let b = app.node.list_balances();
                let lightning_sats = b.total_lightning_balance_sats;
                let onchain_sats   = b.total_onchain_balance_sats;

                (
                    app.total_balance_usd,
                    app.lightning_balance_usd,
                    app.onchain_balance_usd,
                    lightning_sats,
                    onchain_sats,
                )
            };

            Json(Balance {
                total_sats: lightning_sats + onchain_sats,
                total_usd,
                lightning_sats,
                lightning_usd,
                onchain_sats,
                onchain_usd,
            })
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

                    // If the channel is stabilized, pull the data
                    let expected_usd = is_stable.map(|sc| sc.expected_usd.0);
                    let note = is_stable.and_then(|sc| sc.note.clone());

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
                        note,
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

        async fn edit_stable_channel_handler(Json(req): Json<EditStableChannelReq>) -> Json<EditStableChannelRes> {
            let mut app = APP.lock().unwrap();
            app.selected_channel_id = req.channel_id;
        
            if let Some(t) = req.target_usd {
                app.stable_channel_amount = t;
            }
        
            let note = req.note;
        
            if let Some(n) = note.as_ref() {
                let selected_id = app.selected_channel_id.clone();
                if let Some(sc) = app.stable_channels.iter_mut()
                    .find(|sc| sc.channel_id.to_string() == selected_id)
                {
                    sc.note = Some(n.clone()); // sc.note is Option<String>
                    app.status_message = "Note updated".to_string();
                }
            }
        
            app.edit_stable_channel(note);
        
            Json(EditStableChannelRes {
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

        async fn connect_handler(Json(req): Json<ConnectReq>) -> Json<String> {
            let mut app = APP.lock().unwrap();
            app.connect_node_id      = req.node_id.clone();
            app.connect_node_address = req.address.clone();

            let ok = app.connect_to_node();
            Json(if ok {
                format!("Connected to {}", req.node_id)
            } else {
                app.status_message.clone()   
            })
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

                // println!("[Init] Setting Bitcoin RPC connection");
                // builder.set_chain_source_bitcoind_rpc(
                //     "127.0.0.1".into(), 8332,
                //     "".into(),
                //     "".into(),
                // );

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
                        advertise_service: true, // Need this?
                        channel_opening_fee_ppm: 0, // Add fees here
                        channel_over_provisioning_ppm: 1_000_000, // Overcollateralize Stable Position by 100%
                        min_channel_opening_fee_msat: 0, 
                        min_channel_lifetime: 100, 
                        max_client_to_self_delay: 1024,
                        min_payment_size_msat: 0,
                        max_payment_size_msat: 100_000_000_000,
                        client_trusts_lsp: true
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
                // Refresh price first
                let current_price = get_cached_price();
                if current_price > 0.0 {
                    self.btc_price = current_price;
                }

                // Pull raw balances from LDK
                let balances = self.node.list_balances();
                let lightning_sats = balances.total_lightning_balance_sats;
                let onchain_sats   = balances.total_onchain_balance_sats;

                // Convert to BTC + USD
                self.lightning_balance_btc = lightning_sats as f64 / 100_000_000.0;
                self.onchain_balance_btc   = onchain_sats   as f64 / 100_000_000.0;

                self.lightning_balance_usd = self.lightning_balance_btc * self.btc_price;
                self.onchain_balance_usd   = self.onchain_balance_btc   * self.btc_price;

                self.total_balance_btc = self.lightning_balance_btc + self.onchain_balance_btc;
                self.total_balance_usd = self.lightning_balance_usd + self.onchain_balance_usd;

                // ---- Stable-channel aggregate view ------------------------------------
                let stable_receiver_total_usd: f64 = self
                    .stable_channels
                    .iter()
                    .map(|sc| sc.stable_receiver_usd.0)
                    .sum();

                let native_total_btc: f64 = self
                    .stable_channels
                    .iter()
                    .map(|sc| sc.native_channel_btc.to_btc())
                    .sum();

                // ---- Debug prints: where is everything? --------------------------------
                println!(
                    "[Balances] price=${:.2} | onchain={} sats ({:.8} BTC, ~${:.2}) \
                    | ln={} sats ({:.8} BTC, ~${:.2}) | total={} sats ({:.8} BTC, ~${:.2})",
                    self.btc_price,
                    onchain_sats,
                    self.onchain_balance_btc,
                    self.onchain_balance_usd,
                    lightning_sats,
                    self.lightning_balance_btc,
                    self.lightning_balance_usd,
                    onchain_sats + lightning_sats,
                    self.total_balance_btc,
                    self.total_balance_usd,
                );

                println!(
                    "[Balances/stable] entries={} | receiver_USD≈${:.2} | native_BTC≈{:.8} BTC",
                    self.stable_channels.len(),
                    stable_receiver_total_usd,
                    native_total_btc,
                );
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
                        // We check that the payment is with 1%
                        Event::ChannelReady { channel_id, .. } => {
                            if let Some(chan) = self.node.list_channels()
                                .into_iter()
                                .find(|c| c.channel_id == channel_id)
                            {
                                // We need to divide this by 2.0 to account for how much the user put in
                                let funded_usd = chan.channel_value_sats as f64 / 2.0 / 100_000_000.0 * self.btc_price;
                                let tolerance = 0.01; // 1% tolerance band
                                let lower = EXPECTED_USD * (1.0 - tolerance);
                                let upper = EXPECTED_USD * (1.0 + tolerance);
                        
                                if funded_usd >= lower && funded_usd <= upper {
                                    // Good: within tolerance → designate as stable
                                    self.selected_channel_id   = channel_id.to_string();
                                    self.stable_channel_amount = EXPECTED_USD.to_string();
                                    self.edit_stable_channel(None);
                        
                                    audit_event("CHANNEL_READY_STABLE", json!({
                                        "channel_id": channel_id.to_string(),
                                        "funded_usd": funded_usd
                                    }));
                                    self.status_message = format!(
                                        "Channel {} is stable at ${} (funded ≈ ${:.2})",
                                        channel_id, EXPECTED_USD, funded_usd
                                    );
                                } else {
                                    // Outside tolerance → don’t designate
                                    audit_event("CHANNEL_READY_NOT_STABLE", json!({
                                        "channel_id": channel_id.to_string(),
                                        "funded_usd": funded_usd
                                    }));
                                    self.status_message = format!(
                                        "Channel {} funded at ${:.2}, not within tolerance of ${}",
                                        channel_id, funded_usd, EXPECTED_USD
                                    );
                                }
                            }
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
                        // Event::PaymentReceived { amount_msat, payment_hash, .. } => {
                        //     audit_event("PAYMENT_RECEIVED", json!({"amount_msat": amount_msat, "payment_hash": format!("{}", payment_hash)}));
                        //     self.status_message = format!("Received payment of {} msats", amount_msat);
                        //     self.update_balances();
                        // }
                        Event::PaymentReceived { amount_msat, payment_hash, custom_records, payment_id: _ } => {
                            self.handle_payment_received(amount_msat, payment_hash, custom_records)
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

            fn handle_payment_received(
                &mut self,
                amount_msat: u64,
                payment_hash: PaymentHash,
                custom_records: Vec<CustomTlvRecord>,
            ) {
                let mut decoded_payload: Option<String> = None;
            
                for tlv in custom_records {
                    if tlv.type_num == 13377331 {
                        if let Ok(s) = String::from_utf8(tlv.value) {
                            decoded_payload = Some(s);
                        }
                    }
                }
            
                self.status_message = match &decoded_payload {
                    Some(msg) => format!("Received {} msats with TLV: {}", amount_msat, msg),
                    None => format!("Received {} msats (no TLV)", amount_msat),
                };

                // Lets add in the reallocation logic here ... 
                // we need to incldue the channel id AND a sigbature of it  .,, 
                // we need to update the stable channels.json with the udpated allocation
                // we need to update the stability logic to make sure that it works with the native btc exposure .. 
                // we need to sign the message and verify 
                if amount_msat == 1 {
                    // 1 msat marker payment: interpret as a control / reallocation message
                    match &decoded_payload {
                        Some(raw) => {
                            // Log raw first
                            audit_event("REALLOCATE_MESSAGE_RECEIVED", json!({
                                "payment_hash": format!("{payment_hash}"),
                                "raw": raw,
                            }));

                            // Try to parse + (later) apply; for now at least logs all parse/verify results
                            self.handle_reallocation_message(raw, &payment_hash);
                        }
                        None => {
                            // No TLV payload, still log it distinctly
                            audit_event("MESSAGE_RECEIVED_EMPTY_TLV", json!({
                                "payment_hash": format!("{payment_hash}"),
                                "amount_msat": amount_msat,
                            }));
                        }
                    }
                } else {
                    audit_event("PAYMENT_RECEIVED", json!({
                        "amount_msat": amount_msat,
                        "payment_hash": format!("{}", payment_hash),
                        "decoded_tlv": decoded_payload,
                    }));
                }

                self.update_balances();
            }

            fn handle_reallocation_message(
                &mut self,
                raw_msg: &str,
                payment_hash: &PaymentHash,
            ) {
                // 1) Outer envelope: {"payload":"...", "signature":"..."}
                let signed: ReallocationSignedMessage =
                    match serde_json::from_str::<ReallocationSignedMessage>(raw_msg) {
                        Ok(v) => {
                            audit_event("REALLOCATE_PARSED_SIGNED_OK", json!({
                                "payment_hash": format!("{}", payment_hash),
                            }));
                            v
                        }
                        Err(e) => {
                            audit_event("REALLOCATE_PARSE_SIGNED_FAILED", json!({
                                "error": format!("{e}"),
                                "raw": raw_msg,
                                "payment_hash": format!("{}", payment_hash),
                            }));
                            self.status_message = "Reallocation: malformed signed JSON".to_string();
                            return;
                        }
                    };

                // 2) Inner payload: type + channel_id + proposed_allocation
                let payload: ReallocationPayload =
                    match serde_json::from_str::<ReallocationPayload>(&signed.payload) {
                        Ok(v) => v,
                        Err(e) => {
                            audit_event("REALLOCATE_PARSE_PAYLOAD_FAILED", json!({
                                "error": format!("{e}"),
                                "payload": signed.payload,
                                "payment_hash": format!("{}", payment_hash),
                            }));
                            self.status_message = "Reallocation: malformed payload".to_string();
                            return;
                        }
                    };

                audit_event("REALLOCATE_PARSED_PAYLOAD_OK", json!({
                    "payment_hash": format!("{}", payment_hash),
                    "payload_channel_id": &payload.channel_id,
                    "payload_type": &payload.kind,
                }));

                if payload.kind != "TRADE_REALLOCATE_V1" {
                    audit_event("REALLOCATE_UNHANDLED_TYPE", json!({
                        "kind": payload.kind,
                        "payment_hash": format!("{}", payment_hash),
                    }));
                    self.status_message = format!("Reallocation: ignoring type {}", payload.kind);
                    return;
                }

                // Optionally clamp / sanity-check percentages
                let pct_btc = payload.proposed_allocation.pct_btc.min(100);
                let pct_usd = payload.proposed_allocation.pct_usd.min(100);

                if pct_btc as u16 + pct_usd as u16 != 100 {
                    audit_event("REALLOCATE_PERCENT_MISMATCH", json!({
                        "pct_btc": pct_btc,
                        "pct_usd": pct_usd,
                        "payment_hash": format!("{}", payment_hash),
                    }));
                }

                let chan_id_str = payload.channel_id.clone();

                // 3) Find the channel to get the counterparty pubkey
                let channel_opt = self
                    .node
                    .list_channels()
                    .into_iter()
                    .find(|c| c.channel_id.to_string() == chan_id_str);

                let channel = match channel_opt {
                    Some(ch) => ch,
                    None => {
                        audit_event("REALLOCATE_CHANNEL_NOT_FOUND_FOR_VERIFY", json!({
                            "channel_id": chan_id_str,
                            "pct_btc": pct_btc,
                            "pct_usd": pct_usd,
                            "payment_hash": format!("{}", payment_hash),
                        }));
                        self.status_message = format!(
                            "Reallocation: unknown channel {}",
                            chan_id_str
                        );
                        return;
                    }
                };

                // 4) Use the channel’s counterparty node ID as verifying key
                let pkey = channel.counterparty_node_id;

                let sig_ok = self
                    .node
                    .verify_signature(signed.payload.as_bytes(), &signed.signature, &pkey);

                // Signature was wrong; return
                if !sig_ok {
                    audit_event("REALLOCATE_SIGNATURE_INVALID", json!({
                        "channel_id": chan_id_str,
                        "pct_btc": pct_btc,
                        "pct_usd": pct_usd,
                        "payment_hash": format!("{}", payment_hash),
                    }));
                    self.status_message = format!(
                        "Reallocation: signature NOT verified for channel {}",
                        chan_id_str
                    );
                    println!(
                        "Reallocation message NOT verified for channel {}",
                        chan_id_str
                    );
                    return;
                }

                // At this point: verified message from correct pubkey
                audit_event("REALLOCATE_SIGNATURE_VALID", json!({
                    "channel_id": chan_id_str,
                    "pct_btc": pct_btc,
                    "pct_usd": pct_usd,
                    "payment_hash": format!("{}", payment_hash),
                }));

                // ---- apply to StableChannel in an inner scope to avoid borrow conflicts ----
                let mut updated = false;
                let mut applied_target_stable_usd: f64 = 0.0;
                let mut applied_native_btc_str: String = String::new();

                {
                    if let Some(sc) = self
                        .stable_channels
                        .iter_mut()
                        .find(|sc| sc.channel_id == channel.channel_id)
                    {
                        // Refresh balances so we know current receiver USD
                        let (ok, sc_ref) = stable::update_balances(&self.node, sc);
                        if !ok {
                            audit_event("REALLOCATE_BALANCE_UPDATE_FAILED", json!({
                                "channel_id": chan_id_str,
                                "payment_hash": format!("{}", payment_hash),
                            }));
                        } else {
                            // Total receiver USD right now
                            let total_receiver_usd = sc_ref.stable_receiver_usd.0;

                            // USD portion the user wants hedged
                            let target_stable_usd = total_receiver_usd * (pct_usd as f64 / 100.0);

                            // *** THIS IS WHERE expected_usd IS CHANGED ***
                            sc_ref.expected_usd = USD::from_f64(target_stable_usd);

                            // Remaining is native channel BTC exposure
                            let target_native_usd = (total_receiver_usd - target_stable_usd).max(0.0);
                            let native_usd = USD::from_f64(target_native_usd);
                            sc_ref.native_channel_btc = Bitcoin::from_usd(native_usd, sc_ref.latest_price);

                            // Optional: update note
                            let mut new_note = format!(
                                "Target allocation: {}% BTC / {}% USD (via MESSAGE {})",
                                pct_btc, pct_usd, payment_hash,
                            );
                            if let Some(existing) = sc_ref.note.as_ref() {
                                if !existing.trim().is_empty() {
                                    new_note.push('\n');
                                    new_note.push_str(existing);
                                }
                            }
                            sc_ref.note = Some(new_note);

                            applied_target_stable_usd = target_stable_usd;
                            applied_native_btc_str = sc_ref.native_channel_btc.to_string();
                            updated = true;
                        }
                    } else {
                        audit_event("TRADE_REALLOCATE_STABLE_ENTRY_NOT_FOUND", json!({
                            "channel_id": chan_id_str,
                            "pct_btc": pct_btc,
                            "pct_usd": pct_usd,
                            "payment_hash": format!("{}", payment_hash),
                        }));
                    }
                } // <- mutable borrow of self.stable_channels ends here

                if updated {
                    self.save_stable_channels();

                    audit_event("TRADE_REALLOCATE_APPLIED", json!({
                        "channel_id": chan_id_str,
                        "pct_btc": pct_btc,
                        "pct_usd": pct_usd,
                        "target_stable_usd": applied_target_stable_usd,
                        "native_channel_btc": applied_native_btc_str,
                        "payment_hash": format!("{}", payment_hash),
                    }));
                }

                self.status_message = format!(
                    "Reallocation: message VERIFIED for channel {} ({}% BTC / {}% USD)",
                    chan_id_str, pct_btc, pct_usd
                );
                println!(
                    "Reallocation message VERIFIED for channel {} ({}% BTC / {}% USD)",
                    chan_id_str, pct_btc, pct_usd
                );
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

            pub fn edit_stable_channel(&mut self, note: Option<String>) {
                if self.selected_channel_id.is_empty() {
                    self.status_message = "Please select a channel ID".to_string();
                    audit_event("STABLE_EDIT_NO_CHANNEL", json!({}));  
                    return;
                }

                let amount = match self.stable_channel_amount.parse::<f64>() {
                    Ok(val) => val,
                    Err(_) => {
                        self.status_message = "Invalid amount format".to_string();
                        audit_event("STABLE_EDIT_AMOUNT_INVALID", json!({
                            "raw_input": self.stable_channel_amount
                        }));
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

                        // --- preserve existing note + native_channel_btc if present ---
                        let mut note = note.clone();
                        let mut native_channel_btc = Bitcoin::from_btc(0.0);

                        if let Some(existing) = self
                            .stable_channels
                            .iter()
                            .find(|sc| sc.channel_id == channel.channel_id)
                        {
                            if note.is_none() {
                                note = existing.note.clone();
                            }
                            // <-- this is the key change: do NOT overwrite with 0
                            native_channel_btc = existing.native_channel_btc;
                        }

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
                            note,
                            native_channel_btc,
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
                            "Channel {} edited as stable with target ${}",
                            channel_id_str, amount
                        );
                        audit_event("STABLE_EDITED", json!({
                            "channel_id": channel_id_str,
                            "target_usd": amount
                        }));
                        self.selected_channel_id.clear();
                        self.stable_channel_amount = EXPECTED_USD.to_string();
                        return;
                    }
                }

                audit_event("STABLE_EDIT_CHANNEL_NOT_FOUND", json!({
                    "channel_id": self.selected_channel_id
                }));
                self.status_message = format!(
                    "No channel found matching: {}",
                    self.selected_channel_id
                );
            }

            pub fn save_stable_channels(&mut self) {
                let entries: Vec<StableChannelEntry> = self.stable_channels.iter().map(|sc| StableChannelEntry {
                    channel_id: sc.channel_id.to_string(),
                    expected_usd: sc.expected_usd.0,
                    native_btc: sc.native_channel_btc.to_btc(),            
                    note: sc.note.clone(),  
                }).collect();
            
                let file_path = FilePath::new(LSP_DATA_DIR).join("stablechannels.json");
            
                if let Some(parent) = file_path.parent() {
                    fs::create_dir_all(parent).unwrap_or_else(|e| {
                        eprintln!("Failed to create directory: {}", e);
                    });
                }
            
                match serde_json::to_string_pretty(&entries) {
                    Ok(json) => {
                        if let Err(e) = fs::write(&file_path, json) {
                            eprintln!("Error writing stable channels file: {}", e);
                            self.status_message = format!("Failed to save stable channels: {}", e);
                        } else {
                            println!("Saved stable channels to {}", file_path.display());
                            self.status_message = "Stable channels saved successfully".to_string();
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
                                                expected_btc: Bitcoin::from_btc(0.0),
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
                                                note: entry.note.clone(),
                                                native_channel_btc: Bitcoin::from_btc(entry.native_btc)
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
