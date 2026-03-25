use eframe::{egui, App, Frame};
use ldk_node::bitcoin::Network;
use ldk_node::config::{AnchorChannelsConfig, BackgroundSyncConfig, Config, EsploraSyncConfig};
use ldk_node::lightning::offers::offer::Offer;
use ldk_node::lightning_invoice::Bolt11Invoice;
use ldk_node::{bitcoin::secp256k1::PublicKey, lightning::ln::msgs::SocketAddress};
use ldk_node::{Builder, Event, Node};

use chrono::{TimeZone, Utc};
use egui::{Color32, CursorIcon, OpenUrl, RichText, Sense, TextureOptions};
use image::{GrayImage, Luma};
use ldk_node::lightning::ln::channelmanager::PaymentId;
use ldk_node::payment::{PaymentDirection, PaymentKind};
use qrcode::{Color, QrCode};
use serde_json::json;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read as IoRead, Write};
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use stable_channels::audit::*;
use stable_channels::constants::*;
use stable_channels::db::{DailyPriceRecord, Database, PaymentRecord, TradeRecord};
use stable_channels::historical_prices::get_seed_prices;
use stable_channels::price_feeds::get_cached_price_no_fetch;
use stable_channels::stable;
use stable_channels::stable::update_balances;
use stable_channels::types::*;

#[derive(Clone, Debug, PartialEq)]
pub enum Tab {
    Home,
    Settings,
    History,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FundTab {
    Lightning,
    Onchain,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TransferTab {
    Send,
    Receive,
}

#[derive(Clone, Debug)]
pub enum TradeAction {
    BuyBtc,
    SellBtc,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChartPeriod {
    Day1,
    Week1,
    Month1,
    Year1,
    Year3,
    All,
}

impl ChartPeriod {
    fn label(&self) -> &'static str {
        match self {
            ChartPeriod::Day1 => "1D",
            ChartPeriod::Week1 => "1W",
            ChartPeriod::Month1 => "1M",
            ChartPeriod::Year1 => "1Y",
            ChartPeriod::Year3 => "3Y",
            ChartPeriod::All => "ALL",
        }
    }

    fn days(&self) -> u32 {
        match self {
            ChartPeriod::Day1 => 1,
            ChartPeriod::Week1 => 7,
            ChartPeriod::Month1 => 30,
            ChartPeriod::Year1 => 365,
            ChartPeriod::Year3 => 1095,
            ChartPeriod::All => 9999, // All available data
        }
    }
}

#[derive(Clone, Debug)]
pub struct PendingTrade {
    pub action: TradeAction,
    pub amount_usd: f64,
    pub btc_price: f64,
    pub fee_usd: f64,
    pub btc_amount: f64,
    pub net_amount_usd: f64,
}

/// Trade payment that's been sent but not yet confirmed by LDK.
/// Stored in a map keyed by PaymentId so we can finalize on PaymentSuccessful
/// or revert on PaymentFailed.
#[derive(Clone, Debug)]
struct PendingTradePayment {
    new_expected_usd: f64,
    price: f64,
    trade_db_id: i64,
    action: String,
}

/// Outgoing payment awaiting LDK confirmation.
#[derive(Clone, Debug)]
struct PendingPayment {
    payment_db_id: i64,
}

#[derive(Clone, Debug)]
pub struct PendingSplice {
    pub direction: String, // "in" or "out"
    pub amount_sats: u64,
    pub address: Option<String>, // For splice_out
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
    onchain_qr_texture: Option<egui::TextureHandle>,
    waiting_for_payment: bool,
    trigger_fund_wallet: bool,
    fund_tab: FundTab,
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
    pending_splice: Arc<std::sync::Mutex<Option<PendingSplice>>>,
    auto_sweep_in_progress: Arc<std::sync::atomic::AtomicBool>,
    /// On-chain sats at the time auto-sweep was initiated; used to detect confirmation.
    auto_sweep_onchain_at_start: Arc<std::sync::atomic::AtomicU64>,
    pub show_onchain_receive: bool,
    pub show_onchain_send: bool,
    pub show_advanced: bool,
    settings_show_sats: bool,
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

    // Cached fee rate (sat/vB) — fetched asynchronously
    cached_fee_rate: Option<u64>,
    fee_rate_receiver: Option<std::sync::mpsc::Receiver<u64>>,

    // Database
    db: Database,

    // Toast notifications
    toasts: Vec<Toast>,

    // Network
    network: Network,

    // New UI state
    current_tab: Tab,
    show_transfer_modal: bool,
    show_buy_modal: bool,
    show_sell_modal: bool,
    modal_opened_at: std::time::Instant,
    transfer_tab: TransferTab,
    send_input: String,
    send_all: bool,
    send_amount: String,
    send_amount_usd: String,
    send_amount_editing: bool, // true = user is editing BTC field, false = USD field
    send_error: String,
    bolt12_offer: String,

    // Lightning receive state
    show_lightning_receive: bool,
    lightning_receive_amount: String,
    lightning_receive_amount_usd: String,
    lightning_receive_editing_btc: bool,
    lightning_receive_invoice: String,
    lightning_receive_qr: Option<egui::TextureHandle>,
    lightning_receive_error: String,

    // Chart state
    chart_period: ChartPeriod,
    chart_prices: Vec<DailyPriceRecord>,
    chart_last_update: std::time::Instant,
    intraday_prices: Vec<(i64, f64)>, // (timestamp, price) for 1D chart
    last_price_record: std::time::Instant,

    // Syncing state
    is_syncing: bool,

    // In-flight trade payments awaiting LDK confirmation
    pending_trade_payments: HashMap<PaymentId, PendingTradePayment>,

    // In-flight outgoing payments awaiting LDK confirmation
    pending_payments: HashMap<PaymentId, PendingPayment>,

    // Non-blocking daily price update
    daily_prices_receiver: Option<std::sync::mpsc::Receiver<bool>>,

    // Bar chart display toggle: false = default (Stable=USD, BTC=BTC), true = swapped
    bar_chart_show_btc: bool,
    bar_chart_anim: f32, // 0.0 = idle, >0 = animating after toggle

    // History tables display toggle: false = USD, true = BTC
    history_show_btc: bool,

    // Detail modals for history rows
    selected_payment: Option<PaymentRecord>,
    selected_trade: Option<TradeRecord>,

    // Seed phrase backup
    show_seed_words: bool,
    saved_mnemonic: Option<String>,
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

        // Trust the LSP peer so no on-chain anchor reserve is held for their channel
        let config = Config {
            anchor_channels_config: Some(AnchorChannelsConfig {
                trusted_peers_no_reserve: vec![lsp_pubkey],
                per_channel_reserve_sats: 25_000,
            }),
            ..Default::default()
        };

        let mut builder = Builder::from_config(config);

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
                fee_rate_cache_update_interval_secs: FEE_RATE_CACHE_UPDATE_INTERVAL_SECS,
            }),
            ..Default::default()
        };

        builder.set_chain_source_esplora(DEFAULT_CHAIN_URL.to_string(), Some(esplora_cfg));
        builder.set_storage_dir_path(data_dir.to_string_lossy().into_owned());
        builder
            .set_listening_addresses(vec![format!("127.0.0.1:{}", DEFAULT_USER_PORT)
                .parse()
                .unwrap()])
            .unwrap();
        let _ = builder.set_node_alias(DEFAULT_USER_ALIAS.to_string());

        // Let's set up our LSP
        let lsp_address = DEFAULT_LSP_ADDRESS
            .parse::<SocketAddress>()
            .map_err(|e| format!("Invalid LSP address: {}", e))?;

        builder.set_liquidity_source_lsps2(lsp_pubkey, lsp_address.clone(), None);
        builder.set_liquidity_source_lsps1(lsp_pubkey, lsp_address, None);

        // Determine mnemonic for wallet
        let seed_phrase_path = data_dir.join("seed_phrase");
        let keys_seed_path = data_dir.join("keys_seed");
        let mut mnemonic_words: Option<String> = None;
        let mut node_entropy: Option<ldk_node::entropy::NodeEntropy> = None;

        if seed_phrase_path.exists() {
            // Existing wallet — re-read saved mnemonic
            if let Ok(saved) = std::fs::read_to_string(&seed_phrase_path) {
                let trimmed = saved.trim().to_string();
                if !trimmed.is_empty() {
                    let mnemonic = ldk_node::bip39::Mnemonic::from_str(&trimmed)
                        .expect("Invalid saved mnemonic");
                    let entropy =
                        ldk_node::entropy::NodeEntropy::from_bip39_mnemonic(mnemonic, None);
                    node_entropy = Some(entropy);
                    mnemonic_words = Some(trimmed);
                }
            }
        } else if !keys_seed_path.exists() {
            // Truly new wallet — no seed_phrase, no keys_seed
            for name in [
                "ldk_node_data.sqlite",
                "ldk_node_data.sqlite-wal",
                "ldk_node_data.sqlite-shm",
            ] {
                std::fs::remove_file(data_dir.join(name)).ok();
            }
            let mnemonic = ldk_node::entropy::generate_entropy_mnemonic(None);
            let words = mnemonic.to_string();
            std::fs::write(&seed_phrase_path, &words).expect("Failed to save seed phrase");
            let entropy = ldk_node::entropy::NodeEntropy::from_bip39_mnemonic(mnemonic, None);
            node_entropy = Some(entropy);
            mnemonic_words = Some(words);
        }
        // Only remaining case: keys_seed exists but no seed_phrase → pre-upgrade wallet

        // Use provided entropy or fall back to seed file
        let entropy = node_entropy.unwrap_or_else(|| {
            ldk_node::entropy::NodeEntropy::from_seed_path(
                keys_seed_path.to_string_lossy().to_string(),
            )
            .expect("Failed to load seed")
        });
        let node = Arc::new(builder.build(entropy).expect("Failed to build node"));
        node.start().expect("Failed to start node");

        println!("User node started: {}", node.node_id());

        // We try to connect to the "GATEWAY NODE" ... a well-connected Lightning node
        if let (Ok(gateway_pubkey), Ok(gateway_address)) = (
            PublicKey::from_str(DEFAULT_GATEWAY_PUBKEY),
            SocketAddress::from_str(DEFAULT_GATEWAY_ADDRESS),
        ) {
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
            user_channel_id: 0,
            counterparty: lsp_pubkey,
            is_stable_receiver: true,
            expected_usd: USD::from_f64(0.0),
            expected_btc: Bitcoin::from_sats(0),
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
            native_channel_btc: Bitcoin::from_sats(0),
            backing_sats: 0,
            native_sats: 0,
            last_stability_payment: 0,
        };
        let stable_channel = Arc::new(Mutex::new(sc_init));

        // Show onboarding only if no channels AND no funds at all (pending or on-chain)
        let balances = node.list_balances();
        let has_any_funds = balances.total_lightning_balance_sats > 0
            || !balances.pending_balances_from_channel_closures.is_empty()
            || balances.total_onchain_balance_sats > 0;
        let show_onboarding = node.list_channels().is_empty() && !has_any_funds;

        // Initialize SQLite database
        let db =
            Database::open(&data_dir).map_err(|e| format!("Failed to open database: {}", e))?;

        let mut app = Self {
            node: Arc::clone(&node),
            status_message: String::new(),
            invoice_result: String::new(),
            show_onboarding,
            qr_texture: None,
            onchain_qr_texture: None,
            waiting_for_payment: false,
            trigger_fund_wallet: false,
            fund_tab: FundTab::Lightning,
            stable_channel: Arc::clone(&stable_channel),
            background_started: false,
            btc_price,
            invoice_amount: "0".to_string(),
            invoice_to_pay: String::new(),
            on_chain_address: String::new(),
            on_chain_amount: "0".to_string(),
            onchain_send_address: String::new(),
            onchain_send_amount: String::new(),
            pending_splice: Arc::new(std::sync::Mutex::new(None)),
            auto_sweep_in_progress: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            auto_sweep_onchain_at_start: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            show_onchain_receive: false,
            show_onchain_send: false,
            lightning_balance_btc: 0.0,
            onchain_balance_btc: 0.0,
            lightning_balance_usd: 0.0,
            onchain_balance_usd: 0.0,
            total_balance_btc: 0.0,
            total_balance_usd: 0.0,
            cached_fee_rate: None,
            fee_rate_receiver: None,
            show_log_window: false,
            log_contents: String::new(),
            log_last_read: std::time::Instant::now(),
            audit_log_path,
            show_advanced: false,
            settings_show_sats: true,
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
            current_tab: Tab::Home,
            show_transfer_modal: false,
            show_buy_modal: false,
            show_sell_modal: false,
            modal_opened_at: std::time::Instant::now() - Duration::from_secs(10),
            transfer_tab: TransferTab::Send,
            send_input: String::new(),
            send_all: false,
            send_amount: String::new(),
            send_amount_usd: String::new(),
            send_amount_editing: true,
            send_error: String::new(),
            bolt12_offer: String::new(),
            show_lightning_receive: false,
            lightning_receive_amount: String::new(),
            lightning_receive_amount_usd: String::new(),
            lightning_receive_editing_btc: true,
            lightning_receive_invoice: String::new(),
            lightning_receive_qr: None,
            lightning_receive_error: String::new(),
            chart_period: ChartPeriod::Month1,
            chart_prices: Vec::new(),
            chart_last_update: std::time::Instant::now() - std::time::Duration::from_secs(3600),
            intraday_prices: Vec::new(),
            last_price_record: std::time::Instant::now() - std::time::Duration::from_secs(300),
            is_syncing: true, // Always start syncing until we have price AND balance data
            pending_trade_payments: HashMap::new(),
            pending_payments: HashMap::new(),
            daily_prices_receiver: None,
            bar_chart_show_btc: false,
            bar_chart_anim: 0.0,
            history_show_btc: false,
            selected_payment: None,
            selected_trade: None,
            show_seed_words: false,
            saved_mnemonic: mnemonic_words,
        };

        // Seed historical price data if needed
        app.seed_historical_prices();

        // Backfill intraday data from Kraken for the 1D chart
        app.backfill_intraday_prices();

        // Load initial chart data
        app.load_chart_data();

        {
            let mut sc = app.stable_channel.lock().unwrap();
            if let Some(payment_info) = stable::check_stability(&app.node, &mut sc, btc_price) {
                // Record sent stability payment as pending (confirmed on PaymentSuccessful)
                let amount_usd = (payment_info.amount_msat as f64 / 1000.0 / 100_000_000.0)
                    * payment_info.btc_price;
                let _ = app.db.record_payment(
                    Some(&payment_info.payment_id),
                    "stability",
                    "sent",
                    payment_info.amount_msat,
                    Some(amount_usd),
                    Some(payment_info.btc_price),
                    Some(&payment_info.counterparty),
                    "pending",
                    None,
                    None,
                );
                // Persist updated backing_sats
                app.save_channel_settings();
            }
            update_balances(&app.node, &mut sc);
        }

        // Load persisted channel settings from database
        app.load_channel_settings();

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
        let sweep_flag = Arc::clone(&self.auto_sweep_in_progress);
        let sweep_onchain_start = Arc::clone(&self.auto_sweep_onchain_at_start);
        let pending_splice_arc = Arc::clone(&self.pending_splice);

        std::thread::spawn(move || {
            fn current_unix_time() -> i64 {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    .try_into()
                    .unwrap_or(0)
            }

            let mut prev_onchain_sats: u64 = node_arc.list_balances().total_onchain_balance_sats;

            loop {
                // Fetch price (network call) OUTSIDE of lock
                let price = match stable_channels::price_feeds::get_latest_price(&ureq::Agent::new())
                {
                    Ok(p) if p > 0.0 => p,
                    _ => stable_channels::price_feeds::get_cached_price(),
                };

                // Only proceed if we have a valid price
                if price > 0.0 {
                    let mut payment_sent = false;

                    // Brief lock to update values
                    if let Ok(mut sc) = sc_arc.lock() {
                        if !node_arc.list_channels().is_empty() {
                            if let Some(payment_info) =
                                stable_channels::stable::check_stability(&node_arc, &mut sc, price)
                            {
                                // Record sent stability payment as pending (confirmed on PaymentSuccessful)
                                let amount_usd =
                                    (payment_info.amount_msat as f64 / 1000.0 / 100_000_000.0)
                                        * payment_info.btc_price;
                                let _ = db.record_payment(
                                    Some(&payment_info.payment_id),
                                    "stability",
                                    "sent",
                                    payment_info.amount_msat,
                                    Some(amount_usd),
                                    Some(payment_info.btc_price),
                                    Some(&payment_info.counterparty),
                                    "pending",
                                    None,
                                    None,
                                );
                                payment_sent = true;

                                // Persist updated backing_sats to DB
                                let ch_id = sc.channel_id.to_string();
                                let uch_id = format!("{}", sc.user_channel_id);
                                if sc.expected_usd.0 > 0.0 {
                                    let _ = db.save_channel(
                                        &ch_id,
                                        &uch_id,
                                        sc.expected_usd.0,
                                        sc.backing_sats,
                                        sc.native_sats,
                                        sc.note.as_deref(),
                                    );
                                }
                            }
                            stable_channels::stable::update_balances(&node_arc, &mut sc);
                        }
                        sc.latest_price = price;
                        sc.timestamp = current_unix_time();
                    }

                    // After a stability payment, drop the lock and wait for
                    // LDK's background processor to persist the ChannelManager.
                    // Without this, a crash/restart can find the ChannelMonitor
                    // ahead of the ChannelManager, causing a force close.
                    if payment_sent {
                        std::thread::sleep(Duration::from_secs(2));
                    }

                    // Detect new on-chain deposits
                    {
                        let current_onchain = node_arc.list_balances().total_onchain_balance_sats;
                        let is_sweeping = sweep_flag.load(std::sync::atomic::Ordering::Relaxed);
                        if current_onchain > prev_onchain_sats && !is_sweeping {
                            let deposit_sats = current_onchain - prev_onchain_sats;
                            let amount_usd = if price > 0.0 {
                                Some(deposit_sats as f64 / 100_000_000.0 * price)
                            } else {
                                None
                            };
                            // Try to find the txid from LDK's payment list
                            let deposit_txid: Option<String> =
                                node_arc.list_payments().iter().rev().find_map(|p| {
                                    if p.direction == PaymentDirection::Inbound {
                                        if let PaymentKind::Onchain { ref txid, .. } = p.kind {
                                            return Some(txid.to_string());
                                        }
                                    }
                                    None
                                });
                            let _ = db.record_payment(
                                None,
                                "onchain",
                                "received",
                                deposit_sats * 1000,
                                amount_usd,
                                if price > 0.0 { Some(price) } else { None },
                                None,
                                "completed",
                                deposit_txid.as_deref(),
                                None,
                            );
                            audit_event(
                                "ONCHAIN_DEPOSIT_DETECTED",
                                json!({
                                    "amount_sats": deposit_sats,
                                    "prev_onchain": prev_onchain_sats,
                                    "new_onchain": current_onchain,
                                }),
                            );
                        }
                        prev_onchain_sats = current_onchain;
                    }

                    // Auto-sweep: if on-chain balance exists and channel is ready,
                    // splice it into the channel automatically.
                    // The sweep_flag stays true until the on-chain balance drops
                    // (confirming the splice tx landed), preventing duplicate sweeps.
                    if sweep_flag.load(std::sync::atomic::Ordering::Relaxed) {
                        // Check if the on-chain balance dropped since we started the sweep
                        let prev = sweep_onchain_start.load(std::sync::atomic::Ordering::Relaxed);
                        let current = node_arc.list_balances().total_onchain_balance_sats;
                        if prev > 0 && current < prev {
                            sweep_flag.store(false, std::sync::atomic::Ordering::Relaxed);
                            sweep_onchain_start.store(0, std::sync::atomic::Ordering::Relaxed);
                            audit_event(
                                "AUTO_SWEEP_CONFIRMED",
                                json!({
                                    "prev_onchain": prev,
                                    "new_onchain": current,
                                }),
                            );
                        }
                    }

                    if !sweep_flag.load(std::sync::atomic::Ordering::Relaxed) {
                        if let Some(ch) = node_arc
                            .list_channels()
                            .into_iter()
                            .find(|c| c.is_channel_ready)
                        {
                            let balances = node_arc.list_balances();
                            println!(
                                "[sweep] total_onchain={} spendable={} threshold={}",
                                balances.total_onchain_balance_sats,
                                balances.spendable_onchain_balance_sats,
                                AUTO_SWEEP_MIN_SATS
                            );
                            if balances.total_onchain_balance_sats > AUTO_SWEEP_MIN_SATS {
                                // Fetch prevailing fee rate (sat/vB) for 6-block target
                                let fee_rate_sat_vb: u64 = ureq::Agent::new()
                                    .get(&format!("{}/fee-estimates", DEFAULT_CHAIN_URL))
                                    .call()
                                    .ok()
                                    .and_then(|r| r.into_json::<serde_json::Value>().ok())
                                    .and_then(|j| j.get("6").and_then(|v| v.as_f64()))
                                    .map(|f| f.ceil() as u64)
                                    .unwrap_or(2); // conservative fallback

                                // Splice tx ~170 vbytes; reserve exactly enough for fees
                                let fee_reserve = fee_rate_sat_vb * 170;
                                let sweep_amount = balances
                                    .spendable_onchain_balance_sats
                                    .saturating_sub(fee_reserve);

                                println!(
                                    "[sweep] fee_rate={} sat/vB, reserve={}, sweep_amount={}",
                                    fee_rate_sat_vb, fee_reserve, sweep_amount
                                );

                                if sweep_amount > 0 {
                                    audit_event(
                                        "AUTO_SWEEP_ATTEMPT",
                                        json!({
                                            "onchain_sats": balances.total_onchain_balance_sats,
                                            "spendable_sats": balances.spendable_onchain_balance_sats,
                                            "fee_rate_sat_vb": fee_rate_sat_vb,
                                            "fee_reserve": fee_reserve,
                                            "sweep_amount": sweep_amount,
                                        }),
                                    );

                                    match node_arc.splice_in(
                                        &ch.user_channel_id,
                                        ch.counterparty_node_id,
                                        sweep_amount,
                                    ) {
                                        Ok(()) => {
                                            sweep_flag
                                                .store(true, std::sync::atomic::Ordering::Relaxed);
                                            sweep_onchain_start.store(
                                                balances.total_onchain_balance_sats,
                                                std::sync::atomic::Ordering::Relaxed,
                                            );
                                            *pending_splice_arc.lock().unwrap() =
                                                Some(PendingSplice {
                                                    direction: "in".to_string(),
                                                    amount_sats: sweep_amount,
                                                    address: None,
                                                });
                                            // Record pending deposit in payment history immediately
                                            let _ = db.record_payment(
                                                None,
                                                "splice_in",
                                                "received",
                                                sweep_amount * 1000,
                                                None,
                                                None,
                                                None,
                                                "pending",
                                                None,
                                                None,
                                            );
                                            audit_event(
                                                "AUTO_SWEEP_INITIATED",
                                                json!({
                                                    "amount_sats": sweep_amount,
                                                }),
                                            );
                                        }
                                        Err(e) => {
                                            println!("[sweep] splice_in FAILED: {}", e);
                                            audit_event(
                                                "AUTO_SWEEP_FAILED",
                                                json!({
                                                    "error": format!("{e}"),
                                                }),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                std::thread::sleep(Duration::from_secs(BALANCE_UPDATE_INTERVAL_SECS));
            }
        });

        self.background_started = true;
    }

    fn get_jit_invoice(&mut self, ctx: &egui::Context) {
        println!("[DEBUG] get_jit_invoice called");
        let description = ldk_node::lightning_invoice::Bolt11InvoiceDescription::Direct(
            ldk_node::lightning_invoice::Description::new(
                "Stable Channel Wallet onboarding".to_string(),
            )
            .unwrap(),
        );

        // Variable amount invoice - user pays any amount they want
        println!("[DEBUG] Calling receive_variable_amount_via_jit_channel");
        let result = self
            .node
            .bolt11_payment()
            .receive_variable_amount_via_jit_channel(
                &description,
                INVOICE_EXPIRY_SECS,
                Some(MAX_PROPORTIONAL_LSP_FEE_LIMIT_PPM_MSAT),
            );
        println!("[DEBUG] JIT invoice result: {:?}", result.is_ok());

        audit_event(
            "JIT_INVOICE_ATTEMPT",
            json!({
                "type": "variable_amount"
            }),
        );

        match result {
            Ok(invoice) => {
                self.invoice_result = invoice.to_string();
                audit_event(
                    "JIT_INVOICE_GENERATED",
                    json!({
                        "invoice": self.invoice_result,
                        "type": "variable_amount"
                    }),
                );
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
                println!("[DEBUG] JIT invoice ERROR: {:?}", e);
                audit_event(
                    "JIT_INVOICE_FAILED",
                    json!({
                        "error": format!("{e}")
                    }),
                );
                self.invoice_result = format!("Error: {e:?}");
                self.status_message = format!("Failed to generate invoice: {}", e);
                self.show_toast("Invoice failed", "!");
            }
        }
    }

    pub fn generate_invoice(&mut self) -> bool {
        if let Ok(amount) = self.invoice_amount.parse::<u64>() {
            let msats = amount * 1000;
            match self.node.bolt11_payment().receive(
                msats,
                &ldk_node::lightning_invoice::Bolt11InvoiceDescription::Direct(
                    ldk_node::lightning_invoice::Description::new("Invoice".to_string()).unwrap(),
                ),
                INVOICE_EXPIRY_SECS,
            ) {
                Ok(invoice) => {
                    self.invoice_result = invoice.to_string();
                    self.status_message = "Invoice generated".to_string();
                    audit_event(
                        "INVOICE_GENERATED",
                        json!({
                            "amount_msats": msats,
                            "invoice": self.invoice_result
                        }),
                    );
                    true
                }
                Err(e) => {
                    self.status_message = format!("Error: {}", e);
                    audit_event(
                        "INVOICE_GENERATION_FAILED",
                        json!({
                            "amount_msats": msats,
                            "error": format!("{e}")
                        }),
                    );
                    false
                }
            }
        } else {
            self.status_message = "Invalid amount".to_string();
            audit_event(
                "INVOICE_INPUT_INVALID",
                json!({
                    "raw_input": self.invoice_amount
                }),
            );
            false
        }
    }

    /// Generate a Lightning invoice with QR code for the transfer modal
    fn generate_lightning_receive_invoice(&mut self, ctx: &egui::Context) {
        let btc_val = self.lightning_receive_amount.parse::<f64>().unwrap_or(0.0);
        if btc_val <= 0.0 {
            self.status_message = "Enter an amount in BTC".to_string();
            return;
        }
        let amount_sats = (btc_val * 100_000_000.0) as u64;

        let msats = amount_sats * 1000;
        match self.node.bolt11_payment().receive(
            msats,
            &ldk_node::lightning_invoice::Bolt11InvoiceDescription::Direct(
                ldk_node::lightning_invoice::Description::new("Lightning payment".to_string())
                    .unwrap(),
            ),
            INVOICE_EXPIRY_SECS,
        ) {
            Ok(invoice) => {
                self.lightning_receive_invoice = invoice.to_string();
                audit_event(
                    "LIGHTNING_RECEIVE_INVOICE",
                    json!({
                        "amount_sats": amount_sats,
                        "invoice": self.lightning_receive_invoice
                    }),
                );

                // Generate QR code
                let code = QrCode::new(&self.lightning_receive_invoice).unwrap();
                let bits = code.to_colors();
                let width = code.width();
                let scale = 4;
                let border = scale * 2;
                let img_size = (width * scale) as u32;
                let bordered_size = img_size + (border * 2) as u32;

                let mut imgbuf = GrayImage::from_pixel(bordered_size, bordered_size, Luma([255]));

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
                    "lightning_receive_qr",
                    egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba),
                    TextureOptions::LINEAR,
                );
                self.lightning_receive_qr = Some(tex);
                self.show_lightning_receive = true;
            }
            Err(e) => {
                self.status_message = format!("Failed to generate invoice: {}", e);
                audit_event(
                    "LIGHTNING_RECEIVE_FAILED",
                    json!({
                        "error": format!("{e}")
                    }),
                );
            }
        }
    }

    pub fn pay_invoice(&mut self) -> bool {
        match Bolt11Invoice::from_str(&self.invoice_to_pay) {
            Ok(invoice) => match self.node.bolt11_payment().send(&invoice, None) {
                Ok(payment_id) => {
                    self.status_message = format!("Payment sent, ID: {}", payment_id);
                    self.invoice_to_pay.clear();
                    self.update_balances();
                    true
                }
                Err(e) => {
                    self.status_message = format!("Payment error: {}", e);
                    false
                }
            },
            Err(e) => {
                self.status_message = format!("Invalid invoice: {}", e);
                false
            }
        }
    }

    /// Unified send: auto-detects bolt11, bolt12, or on-chain address from self.send_input
    fn send_unified(&mut self) -> bool {
        let input = self.send_input.trim().to_string();
        if input.is_empty() {
            self.send_error = "Paste an address, invoice, or offer".to_string();
            return false;
        }

        // Auto-fix double-paste
        let input = {
            let len = input.len();
            if len > 60 && len.is_multiple_of(2) {
                let (first, second) = input.split_at(len / 2);
                if first == second {
                    first.to_string()
                } else {
                    input
                }
            } else {
                input
            }
        };

        let lower = input.to_lowercase();

        // Try Bolt11 invoice
        if lower.starts_with("lnbc") || lower.starts_with("lntb") || lower.starts_with("lightning:")
        {
            let invoice_str = if lower.starts_with("lightning:") {
                &input[10..]
            } else {
                &input
            };
            match Bolt11Invoice::from_str(invoice_str) {
                Ok(invoice) => {
                    // Check if we have an active channel before attempting payment
                    let has_channel = self.node.list_channels().iter().any(|c| c.is_channel_ready);
                    if !has_channel {
                        self.send_error =
                            "No Lightning channel open. Fund your wallet first.".to_string();
                        return false;
                    }
                    let result = if invoice.amount_milli_satoshis().is_none() {
                        // Variable-amount invoice — use amount from input
                        match self.send_amount.trim().parse::<f64>() {
                            Ok(btc) if btc > 0.0 => {
                                let msat = (btc * 100_000_000_000.0) as u64;
                                self.node
                                    .bolt11_payment()
                                    .send_using_amount(&invoice, msat, None)
                            }
                            _ => {
                                self.send_error = "Enter an amount in BTC".to_string();
                                return false;
                            }
                        }
                    } else {
                        self.node.bolt11_payment().send(&invoice, None)
                    };
                    match result {
                        Ok(payment_id) => {
                            // Determine amount_msat for the pending record
                            let amount_msat = if let Some(amt) = invoice.amount_milli_satoshis() {
                                amt
                            } else {
                                self.send_amount.trim().parse::<u64>().unwrap_or(0) * 1000
                            };
                            let (amount_usd, btc_price_opt) = {
                                let sc = self.stable_channel.lock().unwrap();
                                let price = sc.latest_price;
                                let usd = if price > 0.0 && amount_msat > 0 {
                                    Some(amount_msat as f64 / 1000.0 / SATS_IN_BTC as f64 * price)
                                } else {
                                    None
                                };
                                (usd, if price > 0.0 { Some(price) } else { None })
                            };
                            let payment_id_str = format!("{payment_id}");
                            if let Ok(db_id) = self.db.record_payment(
                                Some(&payment_id_str),
                                "lightning",
                                "sent",
                                amount_msat,
                                amount_usd,
                                btc_price_opt,
                                None,
                                "pending",
                                None,
                                None,
                            ) {
                                self.pending_payments.insert(
                                    payment_id,
                                    PendingPayment {
                                        payment_db_id: db_id,
                                    },
                                );
                            }
                            self.status_message = "Payment pending...".to_string();
                            self.send_input.clear();
                            self.send_amount.clear();
                            self.send_error.clear();
                            self.update_balances();
                            return true;
                        }
                        Err(e) => {
                            self.send_error = format!("Payment failed: {}", e);
                            return false;
                        }
                    }
                }
                Err(e) => {
                    self.send_error = format!("Invalid invoice: {}", e);
                    return false;
                }
            }
        }

        // Try Bolt12 offer
        if lower.starts_with("lno1") {
            let has_channel = self.node.list_channels().iter().any(|c| c.is_channel_ready);
            if !has_channel {
                self.send_error = "No Lightning channel open. Fund your wallet first.".to_string();
                return false;
            }
            match Offer::from_str(&input) {
                Ok(offer) => {
                    let amount_msat = match self.send_amount.trim().parse::<f64>() {
                        Ok(btc) if btc > 0.0 => (btc * 100_000_000_000.0) as u64,
                        _ => {
                            self.send_error = "Enter an amount in BTC".to_string();
                            return false;
                        }
                    };
                    match self.node.bolt12_payment().send_using_amount(
                        &offer,
                        amount_msat,
                        None,
                        None,
                        None,
                    ) {
                        Ok(payment_id) => {
                            let (amount_usd, btc_price_opt) = {
                                let sc = self.stable_channel.lock().unwrap();
                                let price = sc.latest_price;
                                let usd = if price > 0.0 && amount_msat > 0 {
                                    Some(amount_msat as f64 / 1000.0 / SATS_IN_BTC as f64 * price)
                                } else {
                                    None
                                };
                                (usd, if price > 0.0 { Some(price) } else { None })
                            };
                            let payment_id_str = format!("{payment_id}");
                            if let Ok(db_id) = self.db.record_payment(
                                Some(&payment_id_str),
                                "lightning",
                                "sent",
                                amount_msat,
                                amount_usd,
                                btc_price_opt,
                                None,
                                "pending",
                                None,
                                None,
                            ) {
                                self.pending_payments.insert(
                                    payment_id,
                                    PendingPayment {
                                        payment_db_id: db_id,
                                    },
                                );
                            }
                            self.status_message = "Payment pending...".to_string();
                            self.send_input.clear();
                            self.send_amount.clear();
                            self.send_error.clear();
                            self.update_balances();
                            return true;
                        }
                        Err(e) => {
                            self.send_error = format!("Bolt12 payment failed: {}", e);
                            return false;
                        }
                    }
                }
                Err(e) => {
                    self.send_error = format!("Invalid offer: {:?}", e);
                    return false;
                }
            }
        }

        // Try on-chain address — route through splice_out when a channel exists
        if lower.starts_with("bc1")
            || lower.starts_with("tb1")
            || lower.starts_with("1")
            || lower.starts_with("3")
            || lower.starts_with("bcrt1")
        {
            match ldk_node::bitcoin::Address::from_str(&input) {
                Ok(addr) => match addr.require_network(self.network) {
                    Ok(valid_addr) => {
                        // Check if we have a ready channel for splice_out
                        let ready_channel = self
                            .node
                            .list_channels()
                            .into_iter()
                            .find(|c| c.is_channel_ready);

                        if let Some(ch) = ready_channel {
                            // Block if a splice is already in flight (auto-sweep or prior splice)
                            if self
                                .auto_sweep_in_progress
                                .load(std::sync::atomic::Ordering::Relaxed)
                            {
                                self.send_error =
                                    "A splice is already in progress — try again shortly"
                                        .to_string();
                                return false;
                            }

                            // Splice-first: withdraw from channel via splice_out
                            let amount_sats = match self.send_amount.trim().parse::<f64>() {
                                Ok(btc) if btc > 0.0 => (btc * 100_000_000.0) as u64,
                                _ => {
                                    self.send_error = "Enter a valid amount in BTC".to_string();
                                    return false;
                                }
                            };

                            match self.node.splice_out(
                                &ch.user_channel_id,
                                ch.counterparty_node_id,
                                &valid_addr,
                                amount_sats,
                            ) {
                                Ok(()) => {
                                    // Block auto-sweep while this splice is in flight
                                    self.auto_sweep_in_progress
                                        .store(true, std::sync::atomic::Ordering::Relaxed);
                                    self.auto_sweep_onchain_at_start.store(
                                        self.node.list_balances().total_onchain_balance_sats,
                                        std::sync::atomic::Ordering::Relaxed,
                                    );

                                    *self.pending_splice.lock().unwrap() = Some(PendingSplice {
                                        direction: "out".to_string(),
                                        amount_sats,
                                        address: Some(input.clone()),
                                    });
                                    // Record pending withdrawal in payment history immediately
                                    let amount_msat = amount_sats * 1000;
                                    let (amount_usd, btc_price_opt) = {
                                        let sc = self.stable_channel.lock().unwrap();
                                        let price = sc.latest_price;
                                        let usd = if price > 0.0 {
                                            Some(amount_sats as f64 / SATS_IN_BTC as f64 * price)
                                        } else {
                                            None
                                        };
                                        (usd, if price > 0.0 { Some(price) } else { None })
                                    };
                                    let _ = self.db.record_payment(
                                        None,
                                        "splice_out",
                                        "sent",
                                        amount_msat,
                                        amount_usd,
                                        btc_price_opt,
                                        None,
                                        "pending",
                                        None,
                                        Some(&input),
                                    );

                                    self.show_toast("Withdrawal started", "-");
                                    self.status_message = format!(
                                        "Splice-out: {:.8} BTC",
                                        amount_sats as f64 / 100_000_000.0
                                    );
                                    self.send_input.clear();
                                    self.send_amount.clear();
                                    self.send_error.clear();
                                    self.update_balances();
                                    return true;
                                }
                                Err(e) => {
                                    self.send_error = format!("Splice-out failed: {}", e);
                                    return false;
                                }
                            }
                        } else {
                            // No channel — fallback to regular on-chain send
                            let amount_sats = match self.send_amount.trim().parse::<f64>() {
                                Ok(btc) if btc > 0.0 => (btc * 100_000_000.0) as u64,
                                _ if self.send_all => {
                                    self.node.list_balances().spendable_onchain_balance_sats
                                }
                                _ => {
                                    self.send_error = "Enter a valid amount in BTC".to_string();
                                    return false;
                                }
                            };
                            let result = if self.send_all {
                                self.node.onchain_payment().send_all_to_address(
                                    &valid_addr,
                                    false,
                                    None,
                                )
                            } else if amount_sats > 0 {
                                self.node.onchain_payment().send_to_address(
                                    &valid_addr,
                                    amount_sats,
                                    None,
                                )
                            } else {
                                self.send_error = "Enter a valid amount in BTC".to_string();
                                return false;
                            };
                            match result {
                                Ok(txid) => {
                                    // Record on-chain send in payment history
                                    let amount_msat = amount_sats * 1000;
                                    let (amount_usd, btc_price_opt) = {
                                        let sc = self.stable_channel.lock().unwrap();
                                        let price = sc.latest_price;
                                        let usd = if price > 0.0 && amount_msat > 0 {
                                            Some(
                                                amount_msat as f64 / 1000.0 / SATS_IN_BTC as f64
                                                    * price,
                                            )
                                        } else {
                                            None
                                        };
                                        (usd, if price > 0.0 { Some(price) } else { None })
                                    };
                                    let txid_str = txid.to_string();
                                    let _ = self.db.record_payment(
                                        Some(&txid_str),
                                        "onchain",
                                        "sent",
                                        amount_msat,
                                        amount_usd,
                                        btc_price_opt,
                                        None,
                                        "completed",
                                        Some(&txid_str),
                                        None,
                                    );
                                    self.show_toast("Sent!", "OK");
                                    self.status_message = format!("On-chain TX: {}", txid);
                                    self.send_input.clear();
                                    self.send_amount.clear();
                                    self.send_error.clear();
                                    let node_clone = Arc::clone(&self.node);
                                    std::thread::spawn(move || {
                                        let _ = node_clone.sync_wallets();
                                    });
                                    self.update_balances();
                                    return true;
                                }
                                Err(e) => {
                                    self.send_error = format!("Send failed: {}", e);
                                    return false;
                                }
                            }
                        }
                    }
                    Err(_) => {
                        self.send_error = "Wrong network for this address".to_string();
                        return false;
                    }
                },
                Err(_) => {
                    self.send_error = "Invalid address".to_string();
                    return false;
                }
            }
        }

        self.send_error =
            "Unrecognized format. Paste a bitcoin address, bolt11 invoice, or bolt12 offer."
                .to_string();
        false
    }

    pub fn update_balances(&mut self) {
        // Use non-blocking price fetch to avoid UI lag
        let current_price = get_cached_price_no_fetch();
        if current_price > 0.0 {
            self.btc_price = current_price;
            // Record price for 1D chart (every 30s for granular intraday data)
            if self.last_price_record.elapsed() >= Duration::from_secs(30) {
                let _ = self.db.record_price(current_price, Some("cached"));
                self.last_price_record = std::time::Instant::now();
            }
        }

        let balances = self.node.list_balances();

        self.lightning_balance_btc = balances.total_lightning_balance_sats as f64 / 100_000_000.0;
        self.onchain_balance_btc = balances.total_onchain_balance_sats as f64 / 100_000_000.0;

        self.lightning_balance_usd = self.lightning_balance_btc * self.btc_price;
        self.onchain_balance_usd = self.onchain_balance_btc * self.btc_price;

        self.total_balance_btc = self.lightning_balance_btc + self.onchain_balance_btc;
        self.total_balance_usd = self.lightning_balance_usd + self.onchain_balance_usd;

        // Clear syncing state once we have valid price AND stable_channel has been updated
        if self.is_syncing && current_price > 0.0 {
            let channels = self.node.list_channels();
            let sc = self.stable_channel.lock().unwrap();
            let sc_timestamp = sc.timestamp;
            drop(sc);

            // Ready to show main screen when:
            // 1. No channels (new user) - just need price
            // 2. Has channels AND stable_channel has been updated by background thread (timestamp > 0)
            if channels.is_empty() || sc_timestamp > 0 {
                self.is_syncing = false;
            }
        }
    }

    fn close_active_channel(&mut self) {
        let channels = self.node.list_channels();
        if let Some(ch) = channels.first() {
            if !ch.is_usable {
                self.show_toast("Close failed. Please try again in a few minutes.", "-");
                return;
            }
            match self
                .node
                .close_channel(&ch.user_channel_id, ch.counterparty_node_id)
            {
                Ok(_) => self.status_message = format!("Closing channel {}", ch.channel_id),
                Err(e) => {
                    self.show_toast("Close failed. Please try again in a few minutes.", "-");
                    self.status_message = format!("Error closing channel: {}", e);
                }
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
            }
            Err(e) => {
                self.status_message = format!("Error: {}", e);
                false
            }
        }
    }

    fn generate_qr_texture(
        data: &str,
        ctx: &egui::Context,
        name: &str,
    ) -> Option<egui::TextureHandle> {
        let code = QrCode::new(data).ok()?;
        let bits = code.to_colors();
        let width = code.width();
        let scale = 4usize;
        let border = scale * 2;
        let img_size = (width * scale) as u32;
        let bordered_size = img_size + (border * 2) as u32;
        let mut imgbuf = GrayImage::from_pixel(bordered_size, bordered_size, Luma([255]));
        for y in 0..width {
            for x in 0..width {
                let color = if bits[y * width + x] == qrcode::Color::Dark {
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
        Some(ctx.load_texture(
            name,
            egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba),
            TextureOptions::LINEAR,
        ))
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

    /// Send a trade message to the LSP with the new stabilized USD amount.
    /// The fee is sent as the keysend payment amount.
    /// Returns the PaymentId on success so the caller can track it.
    fn send_trade(
        &mut self,
        new_expected_usd: f64,
        fee_usd: f64,
        trade_action: &str,
    ) -> Option<PaymentId> {
        // Grab channel identifiers, counterparty, price from StableChannel
        let (channel_id_str, user_channel_id_str, counterparty, price, old_expected_usd) = {
            let sc = self.stable_channel.lock().unwrap();
            (
                sc.channel_id.to_string(),
                format!("{}", sc.user_channel_id),
                sc.counterparty,
                sc.latest_price,
                sc.expected_usd.0,
            )
        };

        // Build payload with channel_id (shared between both nodes) for LSP lookup
        let payload = json!({
            "type": TRADE_MESSAGE_TYPE,
            "channel_id": channel_id_str,
            "user_channel_id": user_channel_id_str,
            "expected_usd": new_expected_usd,
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

        // Calculate fee in msats (sent to LSP as keysend amount)
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
                // Don't apply_trade yet — wait for PaymentSuccessful event.
                // Store pending info so we can finalize or revert later.
                let payment_id_str = format!("{payment_id}");

                self.status_message = format!("Trade pending (fee: ${:.2})", fee_usd,);
                audit_event(
                    "TRADE_MESSAGE_SENT",
                    json!({
                        "payment_id": payment_id_str,
                        "user_channel_id": user_channel_id_str,
                        "action": trade_action,
                        "old_expected_usd": old_expected_usd,
                        "new_expected_usd": new_expected_usd,
                        "fee_usd": fee_usd,
                        "fee_msats": amt_msat,
                        "status": "pending",
                    }),
                );

                Some(payment_id)
            }
            Err(e) => {
                self.status_message = format!("Failed to send trade order: {}", e);
                audit_event(
                    "TRADE_MESSAGE_FAILED",
                    json!({
                        "user_channel_id": user_channel_id_str,
                        "action": trade_action,
                        "new_expected_usd": new_expected_usd,
                        "error": format!("{e}"),
                    }),
                );
                None
            }
        }
    }

    /// Save user's stable channel to database
    fn save_channel_settings(&self) {
        let sc = self.stable_channel.lock().unwrap();

        // Only save if we have a valid channel
        if sc.user_channel_id == 0 {
            return;
        }

        let channel_id_str = sc.channel_id.to_string();
        let user_channel_id_str = format!("{}", sc.user_channel_id);
        let note_ref = sc.note.as_deref();

        if let Err(e) = self.db.save_channel(
            &channel_id_str,
            &user_channel_id_str,
            sc.expected_usd.0,
            sc.backing_sats,
            sc.native_sats,
            note_ref,
        ) {
            eprintln!("Failed to save channel: {}", e);
        }
    }

    /// Load user's stable channel from database
    fn load_channel_settings(&mut self) {
        let user_channel_id_str = {
            let sc = self.stable_channel.lock().unwrap();
            format!("{}", sc.user_channel_id)
        };

        // Try to load from database by user_channel_id (stable across splices)
        if let Ok(Some(record)) = self.db.load_channel(&user_channel_id_str) {
            let mut sc = self.stable_channel.lock().unwrap();
            sc.expected_usd = USD::from_f64(record.expected_usd);
            sc.backing_sats = record.backing_sats;
            sc.native_sats = record.native_sats;
            if record.note.is_some() {
                sc.note = record.note;
            }
            return;
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
        if let Some(entry) = entries
            .iter()
            .find(|e| e.get("channel_id").and_then(|v| v.as_str()) == Some(&channel_id_str))
        {
            let mut expected_usd = 0.0;
            let note: Option<String> = entry
                .get("note")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            if let Some(exp) = entry.get("expected_usd").and_then(|v| v.as_f64()) {
                expected_usd = exp;
            }

            // Apply to current state
            sc.expected_usd = USD::from_f64(expected_usd);
            if note.is_some() {
                sc.note = note.clone();
            }

            // For migrated channels, calculate backing_sats if we have price
            // Otherwise default to 0 (will be set on next trade)
            let backing_sats = if sc.latest_price > 0.0 && expected_usd > 0.0 {
                let btc_amount = expected_usd / sc.latest_price;
                (btc_amount * 100_000_000.0) as u64
            } else {
                0
            };
            sc.backing_sats = backing_sats;

            // Save to database
            let uch_id = format!("{}", sc.user_channel_id);
            let _ = self.db.save_channel(
                &channel_id_str,
                &uch_id,
                expected_usd,
                backing_sats,
                0, // native_sats unknown during migration; derived on first stability check
                note.as_deref(),
            );

            println!("Migrated channel {} from JSON to SQLite", channel_id_str);
        }
    }

    /// Record a trade in the database. Returns the DB row id (0 on error).
    fn record_trade(
        &self,
        action: &str,
        amount_usd: f64,
        amount_btc: f64,
        fee_usd: f64,
        payment_id: Option<&str>,
        status: &str,
    ) -> i64 {
        let user_channel_id_str = {
            let sc = self.stable_channel.lock().unwrap();
            format!("{}", sc.user_channel_id)
        };

        match self.db.record_trade(
            &user_channel_id_str,
            action,
            amount_usd,
            amount_btc,
            self.btc_price,
            fee_usd,
            payment_id,
            status,
        ) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("Failed to record trade: {}", e);
                0
            }
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
                Event::ChannelReady {
                    channel_id,
                    user_channel_id,
                    ..
                } => {
                    let txid_str = self
                        .node
                        .list_channels()
                        .iter()
                        .find(|ch| ch.channel_id == channel_id)
                        .and_then(|ch| ch.funding_txo.as_ref())
                        .map(|outpoint| outpoint.txid.to_string())
                        .unwrap_or_else(|| "unknown".to_string());

                    {
                        let mut sc = self.stable_channel.lock().unwrap();
                        let is_splice =
                            sc.user_channel_id == user_channel_id.0 && sc.channel_id != channel_id;
                        // Update channel_id in case this is a splice (channel_id changes, user_channel_id doesn't)
                        if is_splice {
                            audit_event(
                                "CHANNEL_ID_UPDATED_SPLICE",
                                json!({
                                    "old_channel_id": sc.channel_id.to_string(),
                                    "new_channel_id": channel_id.to_string(),
                                    "user_channel_id": format!("{}", user_channel_id.0),
                                }),
                            );
                            sc.channel_id = channel_id;
                        }
                        update_balances(&self.node, &mut sc);

                        // After splice confirms, reconcile: if splice-out exceeded
                        // native BTC, the overflow eats into the stable position.
                        if is_splice {
                            let price = sc.latest_price;
                            if let Some(usd_deducted) = stable::reconcile_outgoing(&mut sc, price) {
                                audit_event(
                                    "SPLICE_OUT_STABLE_DEDUCTED",
                                    json!({
                                        "usd_deducted": usd_deducted,
                                        "new_expected_usd": sc.expected_usd.0,
                                        "btc_price": price,
                                    }),
                                );
                            }
                        }
                    }
                    self.save_channel_settings();
                    self.update_balances(); // Update UI immediately

                    // Mark any pending splice payment with this txid as confirmed
                    if txid_str != "unknown" {
                        let _ = self
                            .db
                            .update_payment_confirmations(&txid_str, 1, "completed");
                    }

                    audit_event(
                        "CHANNEL_READY",
                        json!({
                            "channel_id": channel_id.to_string(),
                            "user_channel_id": format!("{}", user_channel_id.0)
                        }),
                    );
                    self.status_message =
                        format!("Channel {channel_id} is now ready\nTXID: {txid_str}");
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

                    let funding_str = funding_txo.txid.to_string();

                    {
                        let mut sc = self.stable_channel.lock().unwrap();
                        sc.user_channel_id = user_channel_id.0;
                        update_balances(&self.node, &mut sc);
                    }
                    self.update_balances(); // Update UI immediately

                    audit_event(
                        "CHANNEL_PENDING",
                        json!({
                            "channel_id":            channel_id.to_string(),
                            "user_channel_id":       format!("{}", user_channel_id.0),
                            "temp_channel_id":       temp_id_str,
                            "counterparty_node_id":  counterparty_node_id.to_string(),
                            "funding_txo":           funding_str,
                        }),
                    );

                    self.status_message =
                        format!("Channel {channel_id} is now ready\nTXID: {funding_str}");
                }

                Event::PaymentReceived {
                    amount_msat,
                    payment_hash,
                    custom_records,
                    ..
                } => {
                    let payment_hash_str = format!("{payment_hash}");

                    // Check for SYNC_V1 message from LSP (expected_usd synchronization)
                    let handled_sync = 'sync: {
                        for tlv in &custom_records {
                            if tlv.type_num != STABLE_CHANNEL_TLV_TYPE {
                                continue;
                            }
                            let raw = match String::from_utf8(tlv.value.clone()) {
                                Ok(s) => s,
                                Err(_) => continue,
                            };
                            let envelope: serde_json::Value = match serde_json::from_str(&raw) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };
                            let payload_str = match envelope.get("payload").and_then(|v| v.as_str())
                            {
                                Some(s) => s,
                                None => continue,
                            };
                            let signature = match envelope.get("signature").and_then(|v| v.as_str())
                            {
                                Some(s) => s,
                                None => continue,
                            };
                            let payload: serde_json::Value = match serde_json::from_str(payload_str)
                            {
                                Ok(v) => v,
                                Err(_) => continue,
                            };
                            if payload.get("type").and_then(|v| v.as_str())
                                != Some(SYNC_MESSAGE_TYPE)
                            {
                                continue;
                            }
                            // Verify signature against LSP (counterparty)
                            let counterparty = {
                                let sc = self.stable_channel.lock().unwrap();
                                sc.counterparty
                            };
                            let sig_ok = self.node.verify_signature(
                                payload_str.as_bytes(),
                                signature,
                                &counterparty,
                            );
                            if !sig_ok {
                                audit_event(
                                    "SYNC_V1_SIGNATURE_INVALID",
                                    json!({
                                        "payment_hash": &payment_hash_str,
                                    }),
                                );
                                continue;
                            }
                            if let Some(expected_usd) =
                                payload.get("expected_usd").and_then(|v| v.as_f64())
                            {
                                let mut sc = self.stable_channel.lock().unwrap();
                                let price = sc.latest_price;
                                let old_expected = sc.expected_usd.0;
                                stable::apply_trade(&mut sc, expected_usd, price);
                                drop(sc);
                                self.save_channel_settings();

                                audit_event(
                                    "SYNC_V1_APPLIED",
                                    json!({
                                        "old_expected_usd": old_expected,
                                        "new_expected_usd": expected_usd,
                                        "btc_price": price,
                                        "payment_hash": &payment_hash_str,
                                    }),
                                );
                                break 'sync true;
                            }
                        }
                        false
                    };

                    if handled_sync {
                        // Sync message: update balances but don't record as a normal payment
                        {
                            let mut sc = self.stable_channel.lock().unwrap();
                            update_balances(&self.node, &mut sc);
                        }
                        self.update_balances();
                    } else if self.db.payment_exists(&payment_hash_str).unwrap_or(false) {
                        // Already recorded, just update balances silently
                        {
                            let mut sc = self.stable_channel.lock().unwrap();
                            update_balances(&self.node, &mut sc);
                        }
                        self.update_balances();
                    } else {
                        audit_event(
                            "PAYMENT_RECEIVED",
                            json!({
                                "amount_msat": amount_msat,
                                "payment_hash": payment_hash_str
                            }),
                        );

                        // Record payment in database
                        let (amount_usd, btc_price, payment_type) = {
                            let sc = self.stable_channel.lock().unwrap();
                            let price = sc.latest_price;
                            let usd = if price > 0.0 {
                                Some((amount_msat as f64 / 1000.0 / 100_000_000.0) * price)
                            } else {
                                None
                            };
                            let ptype = if sc.expected_usd.0 > 0.0 {
                                "stability"
                            } else {
                                "lightning"
                            };
                            (usd, if price > 0.0 { Some(price) } else { None }, ptype)
                        };
                        let _ = self.db.record_payment(
                            Some(&payment_hash_str),
                            payment_type,
                            "received",
                            amount_msat,
                            amount_usd,
                            btc_price,
                            None,
                            "completed",
                            None,
                            None,
                        );

                        self.status_message = format!("Received payment of {} msats", amount_msat);
                        {
                            let mut sc = self.stable_channel.lock().unwrap();
                            update_balances(&self.node, &mut sc);
                            stable::reconcile_incoming(&mut sc);
                        }
                        self.save_channel_settings();
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
                }

                Event::PaymentSuccessful {
                    payment_id,
                    payment_hash,
                    payment_preimage: _,
                    fee_paid_msat,
                    ..
                } => {
                    let payment_hash_str = format!("{payment_hash}");

                    // Check if this is a pending trade payment
                    let pending_trade =
                        payment_id.and_then(|pid| self.pending_trade_payments.remove(&pid));

                    if let Some(trade) = pending_trade {
                        // Trade payment confirmed — now apply the trade
                        {
                            let mut sc = self.stable_channel.lock().unwrap();
                            stable::apply_trade(&mut sc, trade.new_expected_usd, trade.price);
                        }
                        self.save_channel_settings();
                        let _ = self.db.update_trade_status(trade.trade_db_id, "completed");

                        audit_event(
                            "TRADE_CONFIRMED",
                            json!({
                                "payment_hash": payment_hash_str,
                                "action": trade.action,
                                "new_expected_usd": trade.new_expected_usd,
                                "fee_paid_msat": fee_paid_msat,
                            }),
                        );

                        self.update_balances();
                        let verb = if trade.action == "buy" { "Buy" } else { "Sell" };
                        self.show_toast(&format!("{} confirmed", verb), "OK");
                    } else {
                        // Normal (non-trade) outgoing payment
                        let pending = payment_id.and_then(|pid| self.pending_payments.remove(&pid));

                        // Refresh channel balances from LDK
                        {
                            let mut sc = self.stable_channel.lock().unwrap();
                            update_balances(&self.node, &mut sc);
                        }

                        let price = {
                            let sc = self.stable_channel.lock().unwrap();
                            sc.latest_price
                        };

                        audit_event(
                            "PAYMENT_SUCCESSFUL",
                            json!({
                                "payment_hash": payment_hash_str,
                                "fee_paid_msat": fee_paid_msat,
                            }),
                        );

                        // Reconcile stable balance
                        {
                            let mut sc = self.stable_channel.lock().unwrap();
                            let old_expected = sc.expected_usd.0;
                            if let Some(usd_deducted) = stable::reconcile_outgoing(&mut sc, price) {
                                audit_event(
                                    "OUTGOING_STABLE_DEDUCTED",
                                    json!({
                                        "payment_hash": payment_hash_str,
                                        "usd_deducted": usd_deducted,
                                        "old_expected_usd": old_expected,
                                        "new_expected_usd": sc.expected_usd.0,
                                        "btc_price": price,
                                    }),
                                );
                            }
                        }

                        if let Some(p) = pending {
                            // Update existing pending record to completed + fee
                            let _ = self.db.update_payment_status(
                                p.payment_db_id,
                                "completed",
                                fee_paid_msat,
                            );
                        } else {
                            // Try to confirm a pending stability payment by payment_hash
                            let updated = self
                                .db
                                .update_payment_status_by_pid(
                                    &payment_hash_str,
                                    "completed",
                                    fee_paid_msat,
                                )
                                .unwrap_or(0);
                            if updated == 0
                                && !self.db.payment_exists(&payment_hash_str).unwrap_or(false)
                            {
                                // Completely unknown payment — record as completed
                                let _ = self.db.record_payment(
                                    Some(&payment_hash_str),
                                    "lightning",
                                    "sent",
                                    0,
                                    None,
                                    if price > 0.0 { Some(price) } else { None },
                                    None,
                                    "completed",
                                    None,
                                    None,
                                );
                            }
                        }

                        self.save_channel_settings();
                        self.update_balances();
                        self.status_message = "Payment confirmed".to_string();
                        self.show_toast("Payment confirmed", "-");
                    }
                }

                Event::PaymentFailed {
                    payment_id,
                    payment_hash,
                    reason,
                } => {
                    // Check if this is a pending trade payment
                    let pending_trade =
                        payment_id.and_then(|pid| self.pending_trade_payments.remove(&pid));

                    if let Some(trade) = pending_trade {
                        // Trade payment failed — mark as failed, don't apply trade
                        let _ = self.db.update_trade_status(trade.trade_db_id, "failed");

                        audit_event(
                            "TRADE_FAILED",
                            json!({
                                "payment_hash": payment_hash.map(|h| format!("{h}")),
                                "action": trade.action,
                                "new_expected_usd": trade.new_expected_usd,
                                "reason": format!("{:?}", reason),
                            }),
                        );

                        let verb = if trade.action == "buy" { "Buy" } else { "Sell" };
                        self.status_message = format!("{} trade failed: {:?}", verb, reason);
                        self.show_toast(&format!("{} failed", verb), "X");
                    } else {
                        // Check if this is a pending outgoing payment
                        let pending = payment_id.and_then(|pid| self.pending_payments.remove(&pid));
                        if let Some(p) = pending {
                            let _ = self
                                .db
                                .update_payment_status(p.payment_db_id, "failed", None);
                        }

                        audit_event(
                            "PAYMENT_FAILED",
                            json!({
                                "payment_hash": payment_hash.map(|h| format!("{h}")),
                                "reason": format!("{:?}", reason),
                            }),
                        );

                        self.status_message = format!("Payment failed: {:?}", reason);
                        self.show_toast("Payment failed", "X");
                    }
                }

                Event::ChannelClosed {
                    channel_id,
                    user_channel_id,
                    reason,
                    ..
                } => {
                    audit_event(
                        "CHANNEL_CLOSED",
                        json!({
                            "channel_id": format!("{channel_id}"),
                            "user_channel_id": format!("{}", user_channel_id.0),
                            "reason": format!("{:?}", reason)
                        }),
                    );
                    self.status_message = format!("Channel {channel_id} has been closed");
                    self.show_toast("Channel closed", "-");

                    // Clear stable state so a new channel starts fresh
                    {
                        let mut sc = self.stable_channel.lock().unwrap();
                        // Always clear if this is our channel OR if no channels remain
                        if sc.user_channel_id == user_channel_id.0
                            || self.node.list_channels().is_empty()
                        {
                            let _ = self.db.delete_channel(&format!("{}", sc.user_channel_id));
                            sc.expected_usd = USD::from_f64(0.0);
                            sc.backing_sats = 0;
                            sc.native_sats = 0;
                            sc.native_channel_btc = Bitcoin::from_sats(0);
                            sc.stable_receiver_btc = Bitcoin::from_sats(0);
                            sc.stable_receiver_usd = USD::from_f64(0.0);
                            sc.channel_id =
                                ldk_node::lightning::ln::types::ChannelId::from_bytes([0; 32]);
                            sc.user_channel_id = 0;
                        }
                    }

                    // Don't show onboarding after close — the update() loop
                    // already checks for on-chain/pending funds before deciding.
                    // Setting it here would flash the onboarding screen before
                    // the closed channel's funds appear on-chain.
                    self.waiting_for_payment = false;
                }

                Event::SplicePending {
                    channel_id,
                    user_channel_id,
                    new_funding_txo,
                    ..
                } => {
                    // NOTE: Do NOT clear auto_sweep_in_progress here!
                    // The on-chain balance doesn't update until the splice tx confirms.
                    // Clearing the flag here allows the auto-sweep to fire again on
                    // the same unspent UTXO, creating duplicate splice_in calls.

                    audit_event(
                        "SPLICE_PENDING",
                        json!({
                            "channel_id": format!("{channel_id}"),
                            "user_channel_id": format!("{}", user_channel_id.0),
                            "funding_txo": format!("{new_funding_txo}")
                        }),
                    );

                    // Record/update splice payment
                    if let Some(splice) = self.pending_splice.lock().unwrap().take() {
                        let txid_str = new_funding_txo.txid.to_string();
                        if splice.direction == "in" {
                            // Auto-sweep splice_in was already recorded in background thread
                            // — just update it with the txid now that we know it
                            let _ = self.db.set_pending_splice_txid(&txid_str);
                        } else {
                            let btc_price = {
                                let sc = self.stable_channel.lock().unwrap();
                                if sc.latest_price > 0.0 {
                                    Some(sc.latest_price)
                                } else {
                                    None
                                }
                            };
                            let amount_msat = splice.amount_sats * 1000;
                            let amount_usd =
                                btc_price.map(|p| splice.amount_sats as f64 / 100_000_000.0 * p);
                            let txid_str = new_funding_txo.txid.to_string();
                            let _ = self.db.record_payment(
                                Some(&txid_str),
                                "splice_out",
                                "sent",
                                amount_msat,
                                amount_usd,
                                btc_price,
                                None,
                                "pending",
                                Some(&txid_str),
                                splice.address.as_deref(),
                            );
                        }
                    }

                    // Deduct stable balance if splice-out exceeded native BTC.
                    // Uses the same capacity-delta approach as the LSP (via bitcoind)
                    // so both sides compute identical expected_usd.
                    let txid_str = new_funding_txo.txid.to_string();
                    let vout = new_funding_txo.vout;

                    let old_channel_value_sats = self
                        .node
                        .list_channels()
                        .iter()
                        .find(|c| c.user_channel_id.0 == user_channel_id.0)
                        .map(|c| c.channel_value_sats);

                    if let Some(old_value) = old_channel_value_sats {
                        match Self::lookup_funding_output_sats_esplora(&txid_str, vout) {
                            Some(new_value) => {
                                audit_event(
                                    "SPLICE_PENDING_LOOKUP",
                                    json!({
                                        "old_channel_sats": old_value,
                                        "new_channel_sats": new_value,
                                        "delta_sats": (new_value as i64) - (old_value as i64),
                                    }),
                                );

                                if new_value < old_value {
                                    let splice_out_sats = old_value - new_value;
                                    let mut sc = self.stable_channel.lock().unwrap();
                                    let price = sc.latest_price;
                                    if let Some(usd_deducted) =
                                        stable::deduct_outgoing(&mut sc, splice_out_sats, price)
                                    {
                                        audit_event(
                                            "SPLICE_OUT_STABLE_DEDUCTED",
                                            json!({
                                                "splice_out_sats": splice_out_sats,
                                                "usd_deducted": usd_deducted,
                                                "new_expected_usd": sc.expected_usd.0,
                                                "btc_price": price,
                                            }),
                                        );
                                    }
                                    drop(sc);
                                    self.save_channel_settings();
                                }
                            }
                            None => {
                                println!(
                                    "[SplicePending] Could not look up funding output {}:{}",
                                    txid_str, vout
                                );
                                audit_event(
                                    "SPLICE_PENDING_LOOKUP_FAILED",
                                    json!({
                                        "txid": txid_str,
                                        "vout": vout,
                                    }),
                                );
                            }
                        }
                    }

                    self.status_message = format!("Splice pending - tx: {}", new_funding_txo.txid);
                    self.show_toast("Splice pending", "~");
                    self.update_balances();
                }

                Event::SpliceFailed {
                    channel_id,
                    user_channel_id,
                    ..
                } => {
                    self.auto_sweep_in_progress
                        .store(false, std::sync::atomic::Ordering::Relaxed);
                    self.auto_sweep_onchain_at_start
                        .store(0, std::sync::atomic::Ordering::Relaxed);
                    audit_event(
                        "SPLICE_FAILED",
                        json!({
                            "channel_id": format!("{channel_id}"),
                            "user_channel_id": format!("{:?}", user_channel_id)
                        }),
                    );
                    *self.pending_splice.lock().unwrap() = None; // Clear pending splice on failure
                    self.status_message = "Splice failed".to_string();
                    self.show_toast("Splice failed", "!");
                }

                _ => {
                    audit_event(
                        "EVENT_IGNORED",
                        json!({
                            "event_type": format!("{:?}", event)
                        }),
                    );
                }
            }

            let _ = self.node.event_handled();
        }
    }

    /// Look up the value (in sats) of a specific transaction output via esplora.
    ///
    /// Queries `{esplora_url}/tx/{txid}` and extracts the output value at the given vout.
    /// Works for both confirmed and mempool transactions.
    ///
    /// Returns None if the lookup fails (tx not found, network error, etc.)
    fn lookup_funding_output_sats_esplora(txid: &str, vout: u32) -> Option<u64> {
        let url = format!("{}/tx/{}", DEFAULT_CHAIN_URL, txid);

        let response = ureq::Agent::new().get(&url).call().ok()?;

        let json: serde_json::Value = response.into_json().ok()?;
        let vouts = json["vout"].as_array()?;
        let output = vouts.get(vout as usize)?;
        let value_sats = output["value"].as_u64()?;

        Some(value_sats)
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

    /// Short timestamp for compact tables (MM-DD HH:MM)
    fn format_short_timestamp(timestamp: i64) -> String {
        match Utc.timestamp_opt(timestamp, 0) {
            chrono::LocalResult::Single(dt) => dt.format("%m-%d %H:%M").to_string(),
            _ => "—".to_string(),
        }
    }

    /// Format a price with comma separators and 2 decimal places (e.g., 100000.50 -> "$100,000.50")
    fn format_price(price: f64) -> String {
        let price_int = price as i64;
        let decimal_part = ((price - price_int as f64) * 100.0).round() as i64;
        let formatted = price_int
            .to_string()
            .as_bytes()
            .rchunks(3)
            .rev()
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect::<Vec<_>>()
            .join(",");
        format!("${}.{:02}", formatted, decimal_part.abs())
    }

    fn format_chart_price(price: f64) -> String {
        let price_int = price.round() as i64;
        let formatted = price_int
            .to_string()
            .as_bytes()
            .rchunks(3)
            .rev()
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect::<Vec<_>>()
            .join(",");
        format!("${}", formatted)
    }

    /// Seed historical price data into the database if not already present
    fn seed_historical_prices(&mut self) {
        // Check if we already have historical data going back to 2013
        let needs_seed = match self.db.get_oldest_daily_price_date() {
            Ok(Some(oldest)) => {
                // Re-seed if oldest data is not from 2013
                !oldest.starts_with("2013")
            }
            Ok(None) => true, // No data at all
            Err(_) => true,   // Error, try to seed
        };

        if !needs_seed {
            if let Ok(count) = self.db.get_daily_price_count() {
                println!(
                    "[Chart] Historical prices already seeded ({} records back to 2013)",
                    count
                );
            }
            // Still check if we need to fetch recent Kraken data
            self.maybe_fetch_recent_data();
            return;
        }

        println!("[Chart] Seeding historical price data (2013-present)...");
        let seed_data = get_seed_prices();
        let data: Vec<(String, f64, f64, f64, f64, Option<f64>)> = seed_data
            .into_iter()
            .map(|(date, o, h, l, c, v)| (date.to_string(), o, h, l, c, v))
            .collect();

        match self.db.bulk_insert_daily_prices(&data) {
            Ok(count) => println!("[Chart] Seeded {} historical price records", count),
            Err(e) => eprintln!("[Chart] Failed to seed historical prices: {}", e),
        }

        // Immediately fetch recent Kraken data to fill in gaps
        self.fetch_kraken_daily_data();
    }

    /// Check if we need to fetch recent Kraken data and do so if needed
    fn maybe_fetch_recent_data(&mut self) {
        // Check if latest data is older than 3 days
        let needs_update = match self.db.get_latest_daily_price_date() {
            Ok(Some(latest)) => {
                let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
                if let (Ok(latest_date), Ok(today_date)) = (
                    chrono::NaiveDate::parse_from_str(&latest, "%Y-%m-%d"),
                    chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d"),
                ) {
                    let days_old = (today_date - latest_date).num_days();
                    println!("[Chart] Latest data is {} days old ({})", days_old, latest);
                    days_old > 3
                } else {
                    true
                }
            }
            _ => true,
        };

        if needs_update {
            println!("[Chart] Data is stale, fetching from Kraken...");
            self.fetch_kraken_daily_data();
        }
    }

    /// Fetch daily OHLC data from Kraken API (up to 720 days)
    fn fetch_kraken_daily_data(&mut self) {
        println!("[Chart] Fetching Kraken OHLC data...");
        let agent = ureq::Agent::new();

        // Fetch without 'since' to get the most recent 720 days
        match stable_channels::price_feeds::fetch_kraken_ohlc(&agent, None) {
            Ok(prices) => {
                let count = prices.len();
                for (date, open, high, low, close, volume) in prices {
                    let _ = self.db.record_daily_price(
                        &date,
                        open,
                        high,
                        low,
                        close,
                        volume,
                        Some("kraken"),
                    );
                }
                println!("[Chart] Fetched {} daily prices from Kraken", count);
            }
            Err(e) => {
                eprintln!("[Chart] Failed to fetch Kraken OHLC data: {}", e);
            }
        }
    }

    /// Backfill intraday price data from Kraken (15-min candles) if we have gaps
    fn backfill_intraday_prices(&self) {
        // Check how many points we have in the last 24 hours
        let existing = self.db.get_price_history(24).unwrap_or_default();
        if existing.len() >= 80 {
            println!(
                "[Chart] Intraday data sufficient ({} points), skipping backfill",
                existing.len()
            );
            return;
        }

        println!(
            "[Chart] Backfilling intraday prices from Kraken (have {} points)...",
            existing.len()
        );
        let agent = ureq::Agent::new();
        match stable_channels::price_feeds::fetch_kraken_intraday(&agent) {
            Ok(prices) => {
                // Build a set of existing timestamps (rounded to nearest minute) to avoid dupes
                let existing_ts: std::collections::HashSet<i64> = existing
                    .iter()
                    .map(|p| p.timestamp / 60 * 60) // round to minute
                    .collect();

                let mut inserted = 0;
                for (ts, price) in &prices {
                    let rounded = ts / 60 * 60;
                    if !existing_ts.contains(&rounded) {
                        let _ = self.db.record_price_at(*price, *ts, Some("kraken"));
                        inserted += 1;
                    }
                }
                println!(
                    "[Chart] Backfilled {} intraday price points from Kraken",
                    inserted
                );
            }
            Err(e) => {
                eprintln!("[Chart] Failed to backfill intraday prices: {}", e);
            }
        }
    }

    /// Load chart data based on current period selection
    fn load_chart_data(&mut self) {
        let days = self.chart_period.days();

        // For 1D chart, use intraday price_history table
        if self.chart_period == ChartPeriod::Day1 {
            if let Ok(prices) = self.db.get_price_history(24) {
                self.intraday_prices = prices.into_iter().map(|p| (p.timestamp, p.price)).collect();
                println!(
                    "[Chart] Loaded {} intraday prices for 1D",
                    self.intraday_prices.len()
                );
            }
        } else {
            // For longer periods, use daily_prices table
            match self.db.get_daily_prices(days) {
                Ok(prices) => {
                    println!(
                        "[Chart] Loaded {} daily prices for {} (days={})",
                        prices.len(),
                        self.chart_period.label(),
                        days
                    );
                    if let (Some(first), Some(last)) = (prices.first(), prices.last()) {
                        println!("[Chart] Date range: {} to {}", first.date, last.date);
                    }
                    self.chart_prices = prices;
                }
                Err(e) => {
                    eprintln!("[Chart] Failed to load daily prices: {}", e);
                }
            }
        }
        self.chart_last_update = std::time::Instant::now();
    }

    /// Update daily prices from API (called periodically, rate-limited).
    /// Network fetch runs on a background thread to avoid blocking the UI.
    fn update_daily_prices(&mut self) {
        // Already fetching — nothing to do
        if self.daily_prices_receiver.is_some() {
            return;
        }

        // Only update once per hour to avoid rate limits
        let now = std::time::Instant::now();
        if now.duration_since(self.chart_last_update).as_secs() < 3600 {
            return;
        }

        // Prune old intraday price history (keep last 2 days) — fast local DB op
        let _ = self.db.prune_price_history(2);

        // Get the latest date we have — fast local DB op
        let latest_date = self.db.get_latest_daily_price_date().ok().flatten();
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

        // If we already have today's data, skip
        if let Some(ref date) = latest_date {
            if date == &today {
                return;
            }
        }

        let since = latest_date
            .as_ref()
            .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp());

        let (tx, rx) = std::sync::mpsc::channel();
        self.daily_prices_receiver = Some(rx);
        let db = self.db.clone();

        std::thread::spawn(move || {
            let agent = ureq::Agent::new();
            match stable_channels::price_feeds::fetch_kraken_ohlc(&agent, since) {
                Ok(prices) => {
                    for (date, open, high, low, close, volume) in prices {
                        let _ = db.record_daily_price(
                            &date,
                            open,
                            high,
                            low,
                            close,
                            volume,
                            Some("kraken"),
                        );
                    }
                    println!("[Chart] Updated daily prices");
                    let _ = tx.send(true);
                }
                Err(e) => {
                    eprintln!("[Chart] Failed to fetch OHLC data: {}", e);
                    let _ = tx.send(false);
                }
            }
        });
    }

    /// Check if user clicked outside the modal area. Returns true if so.
    fn check_click_outside_modal(&self, ctx: &egui::Context) -> bool {
        // Debounce: ignore the click that opened the modal
        if self.modal_opened_at.elapsed() < Duration::from_millis(300) {
            return false;
        }
        // Check if clicked outside modal (modal windows are ~340px wide, centered)
        let screen_rect = ctx.screen_rect();
        let modal_rect =
            egui::Rect::from_center_size(screen_rect.center(), egui::vec2(360.0, 550.0));
        ctx.input(|i| {
            if let Some(pos) = i.pointer.interact_pos() {
                i.pointer.any_released() && !modal_rect.contains(pos)
            } else {
                false
            }
        })
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

            let bg_color =
                egui::Color32::from_rgba_unmultiplied(16, 150, 105, (220.0 * alpha) as u8);
            let text_color =
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, (255.0 * alpha) as u8);

            egui::Area::new(egui::Id::new(format!("toast_{}", i)))
                .fixed_pos(egui::pos2(
                    screen_rect.right() - toast_width - margin,
                    screen_rect.top() + y_offset,
                ))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::NONE
                        .fill(bg_color)
                        .corner_radius(12.0)
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
                                    ui.label(
                                        egui::RichText::new(&toast.message)
                                            .size(14.0)
                                            .color(text_color)
                                            .strong(),
                                    );
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
                        .color(egui::Color32::DARK_GRAY),
                );

                if timestamp > 0 {
                    let secs = SystemTime::now()
                        .duration_since(
                            UNIX_EPOCH + std::time::Duration::from_secs(timestamp as u64),
                        )
                        .map(|d| d.as_secs())
                        .unwrap_or(0);

                    ui.label(
                        egui::RichText::new(Self::format_time_ago(secs))
                            .size(11.0)
                            .color(egui::Color32::GRAY),
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

    fn show_waiting_for_payment_modal(&mut self, ctx: &egui::Context) {
        egui::Window::new("Fund Your Wallet")
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(Color32::WHITE)
                    .corner_radius(16.0),
            )
            .show(ctx, |ui| {
                ui.set_min_width(320.0);
                ui.add_space(15.0);

                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("Fund Your Wallet")
                            .size(22.0)
                            .strong()
                            .color(egui::Color32::BLACK),
                    );
                    ui.add_space(3.0);
                    ui.label(
                        egui::RichText::new("Get started for free")
                            .size(14.0)
                            .color(Color32::from_rgb(16, 185, 129)),
                    );
                    ui.add_space(12.0);

                    // Tab bar
                    ui.columns(2, |cols| {
                        let lightning_selected = self.fund_tab == FundTab::Lightning;
                        let onchain_selected = self.fund_tab == FundTab::Onchain;

                        cols[0].vertical_centered(|ui| {
                            let lightning_btn = egui::Button::new(
                                egui::RichText::new("Lightning").size(14.0).color(
                                    if lightning_selected {
                                        Color32::WHITE
                                    } else {
                                        Color32::BLACK
                                    },
                                ),
                            )
                            .fill(if lightning_selected {
                                Color32::from_rgb(30, 41, 59)
                            } else {
                                Color32::from_rgb(226, 232, 240)
                            })
                            .corner_radius(8.0)
                            .min_size(egui::vec2(130.0, 34.0));

                            if ui.add(lightning_btn).clicked() {
                                self.fund_tab = FundTab::Lightning;
                            }
                        });

                        cols[1].vertical_centered(|ui| {
                            let onchain_btn = egui::Button::new(
                                egui::RichText::new("On-chain").size(14.0).color(
                                    if onchain_selected {
                                        Color32::WHITE
                                    } else {
                                        Color32::BLACK
                                    },
                                ),
                            )
                            .fill(if onchain_selected {
                                Color32::from_rgb(30, 41, 59)
                            } else {
                                Color32::from_rgb(226, 232, 240)
                            })
                            .corner_radius(8.0)
                            .min_size(egui::vec2(130.0, 34.0));

                            if ui.add(onchain_btn).clicked() {
                                self.fund_tab = FundTab::Onchain;
                                if self.on_chain_address.is_empty() {
                                    self.get_address();
                                }
                                if self.onchain_qr_texture.is_none()
                                    && !self.on_chain_address.is_empty()
                                {
                                    self.onchain_qr_texture = Self::generate_qr_texture(
                                        &self.on_chain_address,
                                        ctx,
                                        "onchain_qr",
                                    );
                                }
                            }
                        });
                    });

                    ui.add_space(12.0);

                    match self.fund_tab {
                        FundTab::Lightning => {
                            ui.label(
                                egui::RichText::new("Send any amount of bitcoin via Lightning")
                                    .size(12.0)
                                    .color(egui::Color32::DARK_GRAY),
                            );
                            ui.add_space(10.0);

                            if let Some(ref qr) = self.qr_texture {
                                let img = egui::Image::from_texture(qr)
                                    .max_size(egui::vec2(200.0, 200.0));
                                ui.add(img);
                            } else {
                                ui.label("Loading QR...");
                            }

                            ui.add_space(10.0);

                            let invoice_display = if self.invoice_result.len() > 40 {
                                format!("{}...", &self.invoice_result[..40])
                            } else {
                                self.invoice_result.clone()
                            };
                            ui.label(
                                egui::RichText::new(invoice_display)
                                    .monospace()
                                    .size(10.0)
                                    .color(egui::Color32::DARK_GRAY),
                            );

                            ui.add_space(15.0);

                            let copy_btn = egui::Button::new(
                                egui::RichText::new("Copy Invoice")
                                    .color(egui::Color32::WHITE)
                                    .size(15.0),
                            )
                            .min_size(egui::vec2(200.0, 42.0))
                            .fill(egui::Color32::BLACK)
                            .corner_radius(21.0);

                            if ui.add(copy_btn).clicked() {
                                ui.ctx().copy_text(self.invoice_result.clone());
                                self.show_toast("Copied!", "OK");
                            }
                        }
                        FundTab::Onchain => {
                            let has_channel =
                                self.node.list_channels().iter().any(|c| c.is_channel_ready);
                            let label = if has_channel {
                                "Send bitcoin here — it will be deposited automatically"
                            } else {
                                "Send bitcoin to this on-chain address"
                            };
                            ui.label(
                                egui::RichText::new(label)
                                    .size(12.0)
                                    .color(egui::Color32::DARK_GRAY),
                            );
                            ui.add_space(10.0);

                            if !self.on_chain_address.is_empty() {
                                // Generate QR if not yet created
                                if self.onchain_qr_texture.is_none() {
                                    self.onchain_qr_texture = Self::generate_qr_texture(
                                        &self.on_chain_address,
                                        ctx,
                                        "onchain_qr",
                                    );
                                }

                                if let Some(ref qr) = self.onchain_qr_texture {
                                    let img = egui::Image::from_texture(qr)
                                        .max_size(egui::vec2(200.0, 200.0));
                                    ui.add(img);
                                }

                                ui.add_space(10.0);

                                let addr_display = if self.on_chain_address.len() > 40 {
                                    format!("{}...", &self.on_chain_address[..40])
                                } else {
                                    self.on_chain_address.clone()
                                };
                                ui.label(
                                    egui::RichText::new(addr_display)
                                        .monospace()
                                        .size(10.0)
                                        .color(egui::Color32::DARK_GRAY),
                                );

                                ui.add_space(15.0);

                                let copy_btn = egui::Button::new(
                                    egui::RichText::new("Copy Address")
                                        .color(egui::Color32::WHITE)
                                        .size(15.0),
                                )
                                .min_size(egui::vec2(200.0, 42.0))
                                .fill(egui::Color32::BLACK)
                                .corner_radius(21.0);

                                if ui.add(copy_btn).clicked() {
                                    ui.ctx().copy_text(self.on_chain_address.clone());
                                    self.show_toast("Copied!", "OK");
                                }
                            } else {
                                ui.spinner();
                            }
                        }
                    }

                    ui.add_space(8.0);

                    if ui
                        .link(
                            egui::RichText::new("Cancel")
                                .size(14.0)
                                .color(Color32::DARK_GRAY),
                        )
                        .clicked()
                    {
                        self.waiting_for_payment = false;
                    }

                    ui.add_space(10.0);
                });
            });

        // Close if clicked outside
        if self.check_click_outside_modal(ctx) {
            self.waiting_for_payment = false;
        }
    }

    fn show_syncing_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::WHITE))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() / 3.0);

                    ui.label(
                        RichText::new("Syncing...")
                            .size(28.0)
                            .color(Color32::BLACK)
                            .strong(),
                    );
                    ui.add_space(15.0);
                    ui.label(
                        RichText::new("Fetching latest prices")
                            .size(16.0)
                            .color(Color32::DARK_GRAY),
                    );
                    ui.add_space(20.0);
                    ui.spinner();
                });
            });

        // Request repaint to keep checking for sync completion
        ctx.request_repaint();
    }

    fn show_onboarding_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::WHITE))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);

                    // Main headline
                    ui.label(
                        egui::RichText::new("Everything you need.")
                            .size(28.0)
                            .strong()
                            .color(egui::Color32::BLACK),
                    );
                    ui.label(
                        egui::RichText::new("Bitcoin only.")
                            .size(28.0)
                            .strong()
                            .color(egui::Color32::BLACK),
                    );

                    ui.add_space(15.0);

                    // One-liner explanation
                    ui.label(
                        egui::RichText::new("Send, receive, save, and stabilize")
                            .size(16.0)
                            .color(egui::Color32::DARK_GRAY),
                    );
                    ui.label(
                        egui::RichText::new("your bitcoin in one place.")
                            .size(16.0)
                            .color(egui::Color32::DARK_GRAY),
                    );

                    ui.add_space(40.0);

                    // Simple steps
                    ui.label(
                        egui::RichText::new("1. Fund your wallet with bitcoin")
                            .size(18.0)
                            .color(egui::Color32::BLACK),
                    );
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new("2. Trade into a stable dollar balance")
                            .size(18.0)
                            .color(egui::Color32::BLACK),
                    );
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new("3. Trade out and withdraw anytime")
                            .size(18.0)
                            .color(egui::Color32::BLACK),
                    );

                    ui.add_space(40.0);

                    let btn = egui::Button::new(
                        egui::RichText::new("Add Bitcoin")
                            .color(egui::Color32::WHITE)
                            .strong()
                            .size(18.0),
                    )
                    .min_size(egui::vec2(200.0, 55.0))
                    .fill(egui::Color32::BLACK)
                    .corner_radius(25.0);

                    ui.add_space(50.0);

                    if ui.add(btn).clicked() {
                        self.status_message = "Creating your wallet...".to_string();
                        self.get_jit_invoice(ctx);
                    }

                    // Show transfer option if user has onchain funds
                    let onchain_sats = self.node.list_balances().total_onchain_balance_sats;
                    if onchain_sats > 0 {
                        ui.add_space(20.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "You have {} sats available",
                                onchain_sats
                            ))
                            .size(14.0)
                            .color(egui::Color32::DARK_GRAY),
                        );
                        ui.add_space(8.0);
                        let transfer_btn = egui::Button::new(
                            egui::RichText::new("Send Bitcoin")
                                .color(egui::Color32::BLACK)
                                .size(14.0),
                        )
                        .min_size(egui::vec2(120.0, 35.0))
                        .fill(egui::Color32::from_rgb(241, 245, 249))
                        .corner_radius(8.0);

                        if ui.add(transfer_btn).clicked() {
                            self.send_input.clear();
                            self.send_error.clear();
                            self.show_transfer_modal = true;
                            self.modal_opened_at = std::time::Instant::now();
                        }
                    }

                    ui.add_space(20.0);
                    self.show_price_info(ui);
                    ui.add_space(30.0);

                    ui.label(
                        egui::RichText::new("100% self-custody bitcoin.")
                            .size(14.0)
                            .italics()
                            .color(egui::Color32::DARK_GRAY),
                    );

                    ui.add_space(5.0);

                    let resp = ui
                        .add(
                            egui::Label::new(
                                egui::RichText::new("Learn more")
                                    .underline()
                                    .color(egui::Color32::from_rgb(59, 130, 246)),
                            )
                            .sense(Sense::click()),
                        )
                        .on_hover_cursor(CursorIcon::PointingHand);

                    if resp.clicked() {
                        ui.ctx().open_url(OpenUrl {
                            url: "https://www.stablechannels.com".to_owned(),
                            new_tab: true,
                        });
                    }

                    ui.add_space(30.0);

                    ui.horizontal(|ui| {
                        ui.label("Wallet ID: ");
                        let wallet_id = self.node.node_id().to_string();
                        ui.monospace(&wallet_id[..7.min(wallet_id.len())]);
                        if ui.small_button("Copy").clicked() {
                            ui.ctx().copy_text(wallet_id);
                        }
                    });

                    // Show "History" link if there are any payment records
                    if let Ok(payments) = self.db.get_recent_payments(1) {
                        if !payments.is_empty() {
                            ui.add_space(12.0);
                            let resp = ui
                                .add(
                                    egui::Label::new(
                                        RichText::new("View history")
                                            .underline()
                                            .size(13.0)
                                            .color(Color32::from_rgb(59, 130, 246)),
                                    )
                                    .sense(Sense::click()),
                                )
                                .on_hover_cursor(CursorIcon::PointingHand);
                            if resp.clicked() {
                                self.current_tab = Tab::History;
                                self.show_onboarding = false;
                            }
                        }
                    }
                });
            });
    }

    fn show_main_screen(&mut self, ctx: &egui::Context) {
        // Bottom navigation bar
        egui::TopBottomPanel::bottom("bottom_nav")
            .exact_height(60.0)
            .frame(
                egui::Frame::new()
                    .fill(Color32::WHITE)
                    .inner_margin(egui::Margin::symmetric(0, 8)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let available_width = ui.available_width();
                    let button_width = available_width / 3.0;

                    // Home tab
                    let home_selected = self.current_tab == Tab::Home;
                    let home_color = if home_selected {
                        Color32::BLACK
                    } else {
                        Color32::GRAY
                    };
                    ui.allocate_ui_with_layout(
                        egui::vec2(button_width, 44.0),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Home").size(14.0).color(home_color),
                                    )
                                    .frame(false),
                                )
                                .clicked()
                            {
                                self.current_tab = Tab::Home;
                            }
                        },
                    );

                    // Settings tab
                    let settings_selected = self.current_tab == Tab::Settings;
                    let settings_color = if settings_selected {
                        Color32::BLACK
                    } else {
                        Color32::GRAY
                    };
                    ui.allocate_ui_with_layout(
                        egui::vec2(button_width, 44.0),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Settings").size(14.0).color(settings_color),
                                    )
                                    .frame(false),
                                )
                                .clicked()
                            {
                                self.current_tab = Tab::Settings;
                            }
                        },
                    );

                    // History tab
                    let history_selected = self.current_tab == Tab::History;
                    let history_color = if history_selected {
                        Color32::BLACK
                    } else {
                        Color32::GRAY
                    };
                    ui.allocate_ui_with_layout(
                        egui::vec2(button_width, 44.0),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("History").size(14.0).color(history_color),
                                    )
                                    .frame(false),
                                )
                                .clicked()
                            {
                                self.current_tab = Tab::History;
                            }
                        },
                    );
                });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(Color32::WHITE)
                    .inner_margin(egui::Margin::same(20)),
            )
            .show(ctx, |ui| match self.current_tab {
                Tab::Home => self.show_home_tab(ui),
                Tab::Settings => self.show_settings_tab(ui),
                Tab::History => self.show_history_tab(ui),
            });

        // Modals are rendered in `update` so they draw on top of all screens

        // Close channel confirmation popup
        if self.confirm_close_popup {
            self.show_close_channel_popup(ctx);
        }
    }

    fn show_home_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            // Check if we have an active, ready channel (not just pending)
            let has_active_channel = self.node.list_channels().iter().any(|c| c.is_channel_ready);

            // Get fresh balances
            let balances = self.node.list_balances();

            // Pending sweep: count PendingBroadcast and BroadcastAwaitingConfirmation
            // These funds are NOT yet in total_onchain_balance_sats
            // AwaitingThresholdConfirmations IS included in total_onchain_balance_sats
            let pending_sweep_sats: u64 = balances
                .pending_balances_from_channel_closures
                .iter()
                .map(|p| match p {
                    ldk_node::PendingSweepBalance::PendingBroadcast {
                        amount_satoshis, ..
                    } => *amount_satoshis,
                    ldk_node::PendingSweepBalance::BroadcastAwaitingConfirmation {
                        amount_satoshis,
                        ..
                    } => *amount_satoshis,
                    _ => 0, // AwaitingThresholdConfirmations: already in onchain balance
                })
                .sum();
            let pending_sweep_btc = pending_sweep_sats as f64 / 100_000_000.0;

            // NOTE: lightning_balances (claimable) intentionally not displayed —
            // LDK reports stale entries long after sweep confirms and funds are spent.

            // Total onchain balance (includes confirmed + AwaitingThresholdConfirmations)
            let total_onchain_sats = balances.total_onchain_balance_sats;
            let spendable_onchain_btc = total_onchain_sats as f64 / 100_000_000.0;

            // Get balance info
            let (btc_price, last_update, expected_usd) = {
                let sc = self.stable_channel.lock().unwrap();
                (sc.latest_price, sc.timestamp, sc.expected_usd.0)
            };

            // If no active channel, stability is gone - show $0 stabilized
            // Claimable lightning balances are NOT added to total because they overlap
            // with pending_sweep or onchain once the sweep tx is broadcast/confirmed.
            let stabilized_usd = if has_active_channel {
                expected_usd
            } else {
                0.0
            };
            // Total balance calculation:
            // With active channel: lightning + onchain (no overlap)
            // Without active channel: pending_sweep + onchain only
            //   (lightning_balances overlaps with pending_sweep/onchain after close)
            let total_sats = if has_active_channel {
                balances.total_lightning_balance_sats + total_onchain_sats
            } else {
                pending_sweep_sats + total_onchain_sats
            };
            let total_btc = total_sats as f64 / 100_000_000.0;
            let total_usd = total_btc * btc_price;

            // Header: "Total Balance"
            ui.label(
                RichText::new("Total Balance")
                    .size(24.0)
                    .color(Color32::BLACK)
                    .strong(),
            );
            ui.add_space(4.0);

            // Large total balance (USD)
            ui.label(
                RichText::new(Self::format_price(total_usd))
                    .size(42.0)
                    .color(Color32::BLACK)
                    .strong(),
            );
            ui.label(
                RichText::new(format!("{:.8} BTC", total_btc))
                    .size(14.0)
                    .color(Color32::DARK_GRAY),
            );

            ui.add_space(12.0);

            // Bar chart: | Stable $XX | BTC $XX |
            let synth_color = Color32::from_rgb(16, 185, 129); // Emerald
            let btc_color = Color32::from_rgb(245, 158, 11); // Amber

            if has_active_channel && total_usd > 0.01 {
                let native_usd = total_usd - stabilized_usd;
                let synth_ratio = (stabilized_usd / total_usd).clamp(0.0, 1.0) as f32;

                let bar_height = 36.0;
                let bar_width = ui.available_width() - 16.0;
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(bar_width, bar_height), Sense::click());
                if response.clicked() {
                    self.bar_chart_show_btc = !self.bar_chart_show_btc;
                    self.bar_chart_anim = 1.0;
                }

                // Animate: decay from 1.0 → 0.0
                if self.bar_chart_anim > 0.0 {
                    self.bar_chart_anim =
                        (self.bar_chart_anim - ui.input(|i| i.unstable_dt) * 3.0).max(0.0);
                    ui.ctx().request_repaint();
                }
                let painter = ui.painter();

                let synth_w = rect.width() * synth_ratio;
                let btc_w = rect.width() - synth_w;

                // Draw colored bar segments
                if synth_w > 0.5 {
                    let synth_rect =
                        egui::Rect::from_min_size(rect.min, egui::vec2(synth_w, bar_height));
                    let rounding = if btc_w < 0.5 {
                        egui::CornerRadius::same(6)
                    } else {
                        egui::CornerRadius {
                            nw: 6,
                            sw: 6,
                            ne: 0,
                            se: 0,
                        }
                    };
                    painter.rect_filled(synth_rect, rounding, synth_color);
                }
                if btc_w > 0.5 {
                    let btc_rect = egui::Rect::from_min_size(
                        egui::pos2(rect.min.x + synth_w, rect.min.y),
                        egui::vec2(btc_w, bar_height),
                    );
                    let rounding = if synth_w < 0.5 {
                        egui::CornerRadius::same(6)
                    } else {
                        egui::CornerRadius {
                            nw: 0,
                            sw: 0,
                            ne: 6,
                            se: 6,
                        }
                    };
                    painter.rect_filled(btc_rect, rounding, btc_color);
                }

                // Text labels embedded inside the bar segments
                // Dark text on colored backgrounds for maximum contrast
                let anim_scale = 1.0 + self.bar_chart_anim * 0.15; // 1.0 → 1.15 → 1.0
                let font_size = 14.0 * anim_scale;
                let font = egui::FontId::new(font_size, egui::FontFamily::Proportional);
                let alpha = ((1.0 - self.bar_chart_anim * 0.3) * 255.0) as u8;
                let label_color = Color32::from_rgba_premultiplied(0, 0, 0, alpha);

                // Default: Stable side = USD, BTC side = BTC
                // Toggled: Stable side = BTC, BTC side = USD
                if synth_w > 40.0 {
                    let synth_label = if self.bar_chart_show_btc {
                        let synth_btc = stabilized_usd / btc_price;
                        format!("Stable {:.6} BTC", synth_btc)
                    } else {
                        format!("Stable {}", Self::format_price(stabilized_usd))
                    };
                    let synth_center = egui::pos2(rect.min.x + synth_w / 2.0, rect.center().y);
                    painter.text(
                        synth_center,
                        egui::Align2::CENTER_CENTER,
                        synth_label,
                        font.clone(),
                        label_color,
                    );
                }

                if btc_w > 40.0 {
                    let btc_label = if self.bar_chart_show_btc {
                        format!("BTC {}", Self::format_price(native_usd))
                    } else {
                        let native_btc = native_usd / btc_price;
                        format!("BTC {:.6} BTC", native_btc)
                    };
                    let btc_center =
                        egui::pos2(rect.min.x + synth_w + btc_w / 2.0, rect.center().y);
                    painter.text(
                        btc_center,
                        egui::Align2::CENTER_CENTER,
                        btc_label,
                        font,
                        label_color,
                    );
                }
            }

            // Bitcoin (pending) - sweep tx broadcast, awaiting confirmation
            if pending_sweep_btc > 0.000000001 {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Bitcoin (pending)")
                            .size(14.0)
                            .color(Color32::from_rgb(217, 119, 6)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{:.8} BTC", pending_sweep_btc))
                                .size(14.0)
                                .color(Color32::BLACK)
                                .strong(),
                        );
                    });
                });
            }

            // Bitcoin (onchain) - confirmed spendable, only show if > 0
            if spendable_onchain_btc > 0.000000001 {
                let has_ready_channel =
                    self.node.list_channels().iter().any(|c| c.is_channel_ready);
                let sweeping = self
                    .auto_sweep_in_progress
                    .load(std::sync::atomic::Ordering::Relaxed);
                let onchain_sats = self.node.list_balances().total_onchain_balance_sats;
                let above_threshold = onchain_sats > AUTO_SWEEP_MIN_SATS;
                let onchain_label = if sweeping {
                    "Processing deposit..."
                } else if has_ready_channel && above_threshold {
                    "Deposit queued"
                } else if has_ready_channel && onchain_sats > 0 && !above_threshold {
                    "Below $10 minimum"
                } else {
                    "Bitcoin (onchain)"
                };
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let label_resp = ui.label(
                        RichText::new(onchain_label)
                            .size(14.0)
                            .color(Color32::DARK_GRAY),
                    );
                    if sweeping {
                        label_resp.on_hover_text("Your deposit is being added to your channel");
                        if ui
                            .small_button("?")
                            .on_hover_text("View deposit details")
                            .clicked()
                        {
                            if let Ok(payments) = self.db.get_recent_payments(10) {
                                if let Some(p) = payments.into_iter().find(|p| {
                                    p.payment_type == "splice_in" && p.status == "pending"
                                }) {
                                    self.selected_payment = Some(p);
                                    self.modal_opened_at = std::time::Instant::now();
                                }
                            }
                        }
                    } else if has_ready_channel && above_threshold {
                        label_resp.on_hover_text("Will be deposited into your channel shortly");
                    } else if has_ready_channel && onchain_sats > 0 && !above_threshold {
                        label_resp.on_hover_text("Minimum ~$10 needed to auto-deposit");
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{:.8} BTC", spendable_onchain_btc))
                                .size(14.0)
                                .color(Color32::BLACK)
                                .strong(),
                        );
                    });
                });
            }

            ui.add_space(12.0);

            // BTC Price
            ui.label(
                RichText::new(format!("BTC Price: {}", Self::format_price(btc_price)))
                    .size(13.0)
                    .color(Color32::DARK_GRAY),
            );

            // Last updated
            if last_update > 0 {
                let secs = std::time::SystemTime::now()
                    .duration_since(
                        std::time::UNIX_EPOCH + std::time::Duration::from_secs(last_update as u64),
                    )
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                ui.label(
                    RichText::new(format!("Updated {}", Self::format_time_ago(secs)))
                        .size(11.0)
                        .color(Color32::GRAY),
                );
            }

            // Check if we have an active channel (needed for layout decisions)
            let has_channel = !self.node.list_channels().is_empty();
            let balances_for_check = self.node.list_balances();
            // Only consider truly pending funds (not AwaitingThresholdConfirmations which are already confirmed)
            let has_pending_funds = balances_for_check
                .pending_balances_from_channel_closures
                .iter()
                .any(|p| {
                    matches!(
                        p,
                        ldk_node::PendingSweepBalance::PendingBroadcast { .. }
                            | ldk_node::PendingSweepBalance::BroadcastAwaitingConfirmation { .. }
                    )
                })
                || balances_for_check.lightning_balances.iter().any(|b| {
                    !matches!(
                        b,
                        ldk_node::LightningBalance::ClaimableOnChannelClose { .. }
                    )
                });

            // If no channel but has any funds (pending, awaiting confirmations, or onchain), show compact layout
            let has_onchain_balance = balances_for_check.total_onchain_balance_sats > 0;
            let has_any_pending = !balances_for_check
                .pending_balances_from_channel_closures
                .is_empty()
                || !balances_for_check.lightning_balances.is_empty();
            if !has_channel && (has_pending_funds || has_onchain_balance || has_any_pending) {
                ui.add_space(15.0);

                if has_pending_funds || has_any_pending {
                    ui.label(
                        RichText::new("Please wait while your funds are confirmed on-chain.")
                            .size(14.0)
                            .color(Color32::from_rgb(217, 119, 6)),
                    );
                    ui.add_space(10.0);
                }

                // Compact pending funds summary
                // Only count truly pending sweeps (not yet broadcast, or awaiting first confirmation)
                // AwaitingThresholdConfirmations means the sweep tx IS confirmed - those funds
                // should already be in the on-chain balance (or were already spent)
                let total_pending: u64 = balances_for_check
                    .pending_balances_from_channel_closures
                    .iter()
                    .map(|p| match p {
                        ldk_node::PendingSweepBalance::PendingBroadcast {
                            amount_satoshis, ..
                        } => *amount_satoshis,
                        ldk_node::PendingSweepBalance::BroadcastAwaitingConfirmation {
                            amount_satoshis,
                            ..
                        } => *amount_satoshis,
                        ldk_node::PendingSweepBalance::AwaitingThresholdConfirmations {
                            ..
                        } => 0, // Already confirmed, skip
                    })
                    .sum();

                let mut total_claimable: u64 = 0;
                let mut claimable_height: Option<u32> = None;
                for b in &balances_for_check.lightning_balances {
                    match b {
                        ldk_node::LightningBalance::ClaimableAwaitingConfirmations {
                            amount_satoshis,
                            confirmation_height,
                            ..
                        } => {
                            total_claimable += amount_satoshis;
                            let h = *confirmation_height;
                            claimable_height =
                                Some(claimable_height.map_or(h, |prev: u32| prev.min(h)));
                        }
                        ldk_node::LightningBalance::ContentiousClaimable {
                            amount_satoshis,
                            timeout_height,
                            ..
                        } => {
                            total_claimable += amount_satoshis;
                            let h = *timeout_height;
                            claimable_height =
                                Some(claimable_height.map_or(h, |prev: u32| prev.min(h)));
                        }
                        ldk_node::LightningBalance::MaybeTimeoutClaimableHTLC {
                            amount_satoshis,
                            claimable_height: ch,
                            ..
                        } => {
                            total_claimable += amount_satoshis;
                            let h = *ch;
                            claimable_height =
                                Some(claimable_height.map_or(h, |prev: u32| prev.min(h)));
                        }
                        ldk_node::LightningBalance::MaybePreimageClaimableHTLC {
                            amount_satoshis,
                            ..
                        } => {
                            total_claimable += amount_satoshis;
                        }
                        ldk_node::LightningBalance::CounterpartyRevokedOutputClaimable {
                            amount_satoshis,
                            ..
                        } => {
                            total_claimable += amount_satoshis;
                        }
                        _ => {}
                    }
                }

                // Also check pending sweep confirmation heights
                let mut sweep_height: Option<u32> = None;
                for p in &balances_for_check.pending_balances_from_channel_closures {
                    if let ldk_node::PendingSweepBalance::AwaitingThresholdConfirmations {
                        confirmation_height,
                        ..
                    } = p
                    {
                        let h = *confirmation_height;
                        sweep_height = Some(sweep_height.map_or(h, |prev: u32| prev.min(h)));
                    }
                }

                // Earliest block at which any funds become available
                let earliest_height = match (claimable_height, sweep_height) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (Some(a), None) => Some(a),
                    (None, Some(b)) => Some(b),
                    (None, None) => None,
                };

                if total_pending > 0 {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("⏳").size(14.0));
                            ui.label(
                                RichText::new("Funds recovering from channel closure")
                                    .size(13.0)
                                    .color(Color32::from_rgb(217, 119, 6)),
                            );
                        });
                        ui.add_space(4.0);
                        if total_pending > 0 {
                            ui.label(
                                RichText::new(format!("{} sats pending sweep", total_pending))
                                    .size(12.0)
                                    .color(Color32::DARK_GRAY),
                            );
                        }
                        if total_claimable > 0 {
                            ui.label(
                                RichText::new(format!(
                                    "{} sats awaiting confirmation",
                                    total_claimable
                                ))
                                .size(12.0)
                                .color(Color32::DARK_GRAY),
                            );
                        }
                        if let Some(height) = earliest_height {
                            ui.label(
                                RichText::new(format!("Recoverable at block {}", height))
                                    .size(12.0)
                                    .color(Color32::from_rgb(59, 130, 246)),
                            );
                        }
                    });
                }

                ui.add_space(20.0);

                // Big "Fund Your Wallet" button
                ui.vertical_centered(|ui| {
                    let fund_btn = egui::Button::new(
                        RichText::new("Fund Your Wallet")
                            .size(18.0)
                            .color(Color32::WHITE)
                            .strong(),
                    )
                    .fill(Color32::from_rgb(16, 185, 129))
                    .corner_radius(25.0)
                    .min_size(egui::vec2(280.0, 55.0));
                    if ui.add(fund_btn).clicked() {
                        println!("[DEBUG] Fund Your Wallet button clicked");
                        self.trigger_fund_wallet = true;
                    }

                    // Show "Send Bitcoin" if there's onchain balance
                    let onchain_sats = balances_for_check.total_onchain_balance_sats;
                    if onchain_sats > 0 {
                        ui.add_space(15.0);
                        let send_btn = egui::Button::new(
                            RichText::new("Send Bitcoin")
                                .size(14.0)
                                .color(Color32::BLACK),
                        )
                        .fill(Color32::from_rgb(241, 245, 249))
                        .corner_radius(8.0)
                        .min_size(egui::vec2(140.0, 40.0));
                        if ui.add(send_btn).clicked() {
                            self.send_input.clear();
                            self.send_error.clear();
                            self.show_transfer_modal = true;
                            self.modal_opened_at = std::time::Instant::now();
                        }
                    }
                });

                ui.add_space(20.0);

                // Smaller chart when no channel
                let chart_height = 100.0;
                let chart_width = ui.available_width();
                let (rect, _response) =
                    ui.allocate_exact_size(egui::vec2(chart_width, chart_height), Sense::hover());

                let painter = ui.painter();

                if self.chart_period == ChartPeriod::Day1 {
                    // Time-based 1D mini chart
                    let now_ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64;
                    let t_start = now_ts - 24 * 3600;
                    let pts: Vec<(f64, f64)> = self
                        .intraday_prices
                        .iter()
                        .filter(|(ts, _)| *ts >= t_start)
                        .map(|(ts, p)| (*ts as f64, *p))
                        .collect();
                    if pts.len() >= 2 {
                        let min_p = pts.iter().map(|(_, p)| *p).fold(f64::INFINITY, f64::min);
                        let max_p = pts
                            .iter()
                            .map(|(_, p)| *p)
                            .fold(f64::NEG_INFINITY, f64::max);
                        let range = max_p - min_p;
                        let is_up = pts.last().unwrap().1 >= pts.first().unwrap().1;
                        let color = if is_up {
                            Color32::from_rgb(34, 197, 94)
                        } else {
                            Color32::from_rgb(239, 68, 68)
                        };
                        let points: Vec<egui::Pos2> = pts
                            .iter()
                            .map(|(ts, p)| {
                                let x_frac = ((*ts - t_start as f64) / (24.0 * 3600.0))
                                    .clamp(0.0, 1.0)
                                    as f32;
                                let y_frac = if range > 0.0 {
                                    ((*p - min_p) / range) as f32
                                } else {
                                    0.5
                                };
                                egui::Pos2::new(
                                    rect.left() + x_frac * rect.width(),
                                    rect.bottom() - y_frac * (chart_height - 20.0) - 10.0,
                                )
                            })
                            .collect();
                        for i in 0..points.len().saturating_sub(1) {
                            painter.line_segment(
                                [points[i], points[i + 1]],
                                egui::Stroke::new(2.0, color),
                            );
                        }
                    }
                } else {
                    let prices: Vec<f64> = self.chart_prices.iter().map(|p| p.close).collect();
                    if prices.len() >= 2 {
                        let min_price = prices.iter().cloned().fold(f64::INFINITY, f64::min);
                        let max_price = prices.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                        let price_range = max_price - min_price;
                        let is_up = prices.last().unwrap_or(&0.0) >= prices.first().unwrap_or(&0.0);
                        let chart_color = if is_up {
                            Color32::from_rgb(34, 197, 94)
                        } else {
                            Color32::from_rgb(239, 68, 68)
                        };
                        let points: Vec<egui::Pos2> = prices
                            .iter()
                            .enumerate()
                            .map(|(i, price)| {
                                let x = rect.left()
                                    + (i as f32 / (prices.len() - 1).max(1) as f32) * rect.width();
                                let normalized = if price_range > 0.0 {
                                    (price - min_price) / price_range
                                } else {
                                    0.5
                                };
                                let y = rect.bottom()
                                    - (normalized as f32 * (chart_height - 20.0))
                                    - 10.0;
                                egui::Pos2::new(x, y)
                            })
                            .collect();
                        for i in 0..points.len().saturating_sub(1) {
                            painter.line_segment(
                                [points[i], points[i + 1]],
                                egui::Stroke::new(2.0, chart_color),
                            );
                        }
                    }
                }

                return; // Skip the rest of the normal layout
            }

            ui.add_space(30.0);

            // Chart with real data
            let chart_height = 200.0;
            let chart_width = ui.available_width();
            let (rect, _response) =
                ui.allocate_exact_size(egui::vec2(chart_width, chart_height), Sense::hover());

            let painter = ui.painter();

            // Grey border around the chart
            painter.rect_stroke(
                rect,
                egui::CornerRadius::same(8),
                egui::Stroke::new(1.0, Color32::from_rgb(200, 200, 200)),
                egui::StrokeKind::Outside,
            );

            // Inner drawing area with padding
            let inner = rect.shrink2(egui::vec2(12.0, 14.0));

            if self.chart_period == ChartPeriod::Day1 {
                // ── 1D chart: time-based x-axis ──────────────────────────
                let now_ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;
                let window_secs: i64 = 24 * 3600;
                let t_start = now_ts - window_secs;

                let pts: Vec<(f64, f64)> = self
                    .intraday_prices
                    .iter()
                    .filter(|(ts, _)| *ts >= t_start)
                    .map(|(ts, p)| (*ts as f64, *p))
                    .collect();

                if pts.len() >= 2 {
                    let min_price = pts.iter().map(|(_, p)| *p).fold(f64::INFINITY, f64::min);
                    let max_price = pts
                        .iter()
                        .map(|(_, p)| *p)
                        .fold(f64::NEG_INFINITY, f64::max);
                    let price_range = max_price - min_price;

                    let is_up = pts.last().unwrap().1 >= pts.first().unwrap().1;
                    let chart_color = if is_up {
                        Color32::from_rgb(34, 197, 94)
                    } else {
                        Color32::from_rgb(239, 68, 68)
                    };

                    let chart_top = inner.top();
                    let chart_bottom = inner.bottom() - 14.0; // room for time labels
                    let chart_h = chart_bottom - chart_top;

                    let to_pos = |ts: f64, price: f64| -> egui::Pos2 {
                        let x_frac =
                            ((ts - t_start as f64) / window_secs as f64).clamp(0.0, 1.0) as f32;
                        let x = inner.left() + x_frac * inner.width();
                        let y_frac = if price_range > 0.0 {
                            ((price - min_price) / price_range) as f32
                        } else {
                            0.5
                        };
                        let y = chart_bottom - y_frac * chart_h;
                        egui::Pos2::new(x, y)
                    };

                    let points: Vec<egui::Pos2> =
                        pts.iter().map(|(ts, p)| to_pos(*ts, *p)).collect();

                    for i in 0..points.len().saturating_sub(1) {
                        painter.line_segment(
                            [points[i], points[i + 1]],
                            egui::Stroke::new(2.5, chart_color),
                        );
                    }

                    // Price labels
                    let label_color = Color32::GRAY;
                    painter.text(
                        egui::pos2(inner.left(), chart_top),
                        egui::Align2::LEFT_TOP,
                        Self::format_chart_price(max_price),
                        egui::FontId::proportional(10.0),
                        label_color,
                    );
                    painter.text(
                        egui::pos2(inner.left(), chart_bottom + 1.0),
                        egui::Align2::LEFT_TOP,
                        Self::format_chart_price(min_price),
                        egui::FontId::proportional(10.0),
                        label_color,
                    );

                    // Time labels along x-axis (every 6 hours)
                    for h_offset in &[0, 6, 12, 18, 24] {
                        let label_ts = t_start + (*h_offset as i64) * 3600;
                        let x_frac = (*h_offset as f32) / 24.0;
                        let x = inner.left() + x_frac * inner.width();

                        // Format as local hour
                        let dt = chrono::DateTime::from_timestamp(label_ts, 0);
                        let label = if let Some(dt) = dt {
                            let local = dt.with_timezone(&chrono::Local);
                            local.format("%-I%P").to_string()
                        } else {
                            format!("{}h", h_offset)
                        };

                        let align = if *h_offset == 0 {
                            egui::Align2::LEFT_TOP
                        } else if *h_offset == 24 {
                            egui::Align2::RIGHT_TOP
                        } else {
                            egui::Align2::CENTER_TOP
                        };
                        painter.text(
                            egui::pos2(x, inner.bottom() - 2.0),
                            align,
                            label,
                            egui::FontId::proportional(9.0),
                            Color32::GRAY,
                        );
                    }
                } else {
                    painter.text(
                        inner.center(),
                        egui::Align2::CENTER_CENTER,
                        "Collecting price data...",
                        egui::FontId::proportional(14.0),
                        Color32::GRAY,
                    );
                }
            } else {
                // ── Longer periods: index-based x-axis ───────────────────
                let prices: Vec<f64> = self.chart_prices.iter().map(|p| p.close).collect();

                if prices.len() >= 2 {
                    let min_price = prices.iter().cloned().fold(f64::INFINITY, f64::min);
                    let max_price = prices.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                    let price_range = max_price - min_price;

                    let is_up = prices.last().unwrap_or(&0.0) >= prices.first().unwrap_or(&0.0);
                    let chart_color = if is_up {
                        Color32::from_rgb(34, 197, 94)
                    } else {
                        Color32::from_rgb(239, 68, 68)
                    };

                    let points: Vec<egui::Pos2> = prices
                        .iter()
                        .enumerate()
                        .map(|(i, price)| {
                            let x = inner.left()
                                + (i as f32 / (prices.len() - 1).max(1) as f32) * inner.width();
                            let normalized = if price_range > 0.0 {
                                (price - min_price) / price_range
                            } else {
                                0.5
                            };
                            let y = inner.bottom() - normalized as f32 * inner.height();
                            egui::Pos2::new(x, y)
                        })
                        .collect();

                    for i in 0..points.len().saturating_sub(1) {
                        painter.line_segment(
                            [points[i], points[i + 1]],
                            egui::Stroke::new(2.5, chart_color),
                        );
                    }

                    let label_color = Color32::GRAY;
                    painter.text(
                        egui::pos2(inner.left(), inner.top()),
                        egui::Align2::LEFT_TOP,
                        Self::format_chart_price(max_price),
                        egui::FontId::proportional(10.0),
                        label_color,
                    );
                    painter.text(
                        egui::pos2(inner.left(), inner.bottom() - 12.0),
                        egui::Align2::LEFT_TOP,
                        Self::format_chart_price(min_price),
                        egui::FontId::proportional(10.0),
                        label_color,
                    );
                } else {
                    painter.text(
                        inner.center(),
                        egui::Align2::CENTER_CENTER,
                        "No price data available",
                        egui::FontId::proportional(14.0),
                        Color32::GRAY,
                    );
                }
            }

            ui.add_space(20.0);

            // Time period buttons
            let periods = [
                ChartPeriod::Day1,
                ChartPeriod::Week1,
                ChartPeriod::Month1,
                ChartPeriod::Year1,
                ChartPeriod::Year3,
                ChartPeriod::All,
            ];
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 336.0) / 2.0);
                for period in &periods {
                    let is_selected = *period == self.chart_period;
                    let bg_color = if is_selected {
                        Color32::from_rgb(226, 232, 240)
                    } else {
                        Color32::from_rgb(241, 245, 249)
                    };
                    let btn = egui::Button::new(
                        RichText::new(period.label())
                            .size(13.0)
                            .color(Color32::BLACK),
                    )
                    .fill(bg_color)
                    .corner_radius(16.0)
                    .min_size(egui::vec2(48.0, 32.0));
                    if ui.add(btn).clicked() {
                        self.chart_period = period.clone();
                        self.load_chart_data();
                    }
                }
            });

            ui.add_space(30.0);

            // Normal action buttons when channel is active
            ui.horizontal(|ui| {
                let btn_width = (ui.available_width() - 20.0) / 3.0;
                let btn_height = 50.0;

                // Buy button
                let btn_dark = Color32::from_rgb(30, 41, 59);
                let buy_btn = egui::Button::new(
                    RichText::new("Buy")
                        .size(16.0)
                        .color(Color32::WHITE)
                        .strong(),
                )
                .fill(btn_dark)
                .corner_radius(25.0)
                .min_size(egui::vec2(btn_width, btn_height));
                if ui.add(buy_btn).clicked() {
                    self.show_buy_modal = true;
                    self.modal_opened_at = std::time::Instant::now();
                }

                ui.add_space(10.0);

                // Sell button
                let sell_btn = egui::Button::new(
                    RichText::new("Sell")
                        .size(16.0)
                        .color(Color32::WHITE)
                        .strong(),
                )
                .fill(btn_dark)
                .corner_radius(25.0)
                .min_size(egui::vec2(btn_width, btn_height));
                if ui.add(sell_btn).clicked() {
                    self.show_sell_modal = true;
                    self.modal_opened_at = std::time::Instant::now();
                }

                ui.add_space(10.0);

                // Transfer button
                let transfer_btn = egui::Button::new(
                    RichText::new("Transfer")
                        .size(18.0)
                        .color(Color32::WHITE)
                        .strong(),
                )
                .fill(btn_dark)
                .corner_radius(25.0)
                .min_size(egui::vec2(btn_width, btn_height));
                if ui.add(transfer_btn).clicked() {
                    self.send_input.clear();
                    self.send_error.clear();
                    self.show_transfer_modal = true;
                    self.modal_opened_at = std::time::Instant::now();
                }
            });

            ui.add_space(20.0);
        });
    }

    fn show_settings_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading(RichText::new("Settings").color(Color32::BLACK));
                ui.add_space(20.0);

                // Channel Balance Distribution
                if let Some(ch) = self.node.list_channels().first() {
                    let btc_price = {
                        let sc = self.stable_channel.lock().unwrap();
                        sc.latest_price
                    };

                    ui.group(|ui| {
                        ui.label(RichText::new("Wallet Details").size(16.0).strong().color(Color32::BLACK));
                        ui.add_space(10.0);

                        let total_sats = ch.channel_value_sats;
                        let our_sats = ch.outbound_capacity_msat / 1000 + ch.unspendable_punishment_reserve.unwrap_or(0);
                        let lsp_sats = total_sats.saturating_sub(our_sats);

                        let our_ratio = if total_sats > 0 { our_sats as f32 / total_sats as f32 } else { 0.5 };

                        // Draw the bar
                        let bar_height = 24.0;
                        let available_width = ui.available_width() - 20.0;
                        let (rect, _response) = ui.allocate_exact_size(egui::vec2(available_width, bar_height), egui::Sense::hover());

                        let our_width = rect.width() * our_ratio;
                        let our_rect = egui::Rect::from_min_size(rect.min, egui::vec2(our_width, bar_height));
                        let lsp_rect = egui::Rect::from_min_size(
                            egui::pos2(rect.min.x + our_width, rect.min.y),
                            egui::vec2(rect.width() - our_width, bar_height),
                        );

                        // Colors: blue for user, orange for LSP
                        let our_color = Color32::from_rgb(59, 130, 246); // Blue
                        let lsp_color = Color32::from_rgb(251, 146, 60); // Orange

                        ui.painter().rect_filled(our_rect, 4.0, our_color);
                        ui.painter().rect_filled(lsp_rect, 4.0, lsp_color);

                        ui.add_space(8.0);

                        // Labels below the bar — click to toggle sats/USD
                        let mut toggled = false;
                        let (our_label, lsp_label) = if self.settings_show_sats {
                            let our_btc = our_sats as f64 / 100_000_000.0;
                            let lsp_btc = lsp_sats as f64 / 100_000_000.0;
                            (
                                format!("Yours: {:.8} BTC", our_btc),
                                format!("LSP: {:.8} BTC", lsp_btc),
                            )
                        } else if btc_price > 0.0 {
                            let our_usd = our_sats as f64 / 100_000_000.0 * btc_price;
                            let lsp_usd = lsp_sats as f64 / 100_000_000.0 * btc_price;
                            (
                                format!("Yours: ${:.2}", our_usd),
                                format!("LSP: ${:.2}", lsp_usd),
                            )
                        } else {
                            let our_btc = our_sats as f64 / 100_000_000.0;
                            let lsp_btc = lsp_sats as f64 / 100_000_000.0;
                            (
                                format!("Yours: {:.8} BTC", our_btc),
                                format!("LSP: {:.8} BTC", lsp_btc),
                            )
                        };

                        ui.horizontal(|ui| {
                            let our_resp = ui.add(egui::Label::new(
                                RichText::new(&our_label).color(our_color)
                            ).sense(egui::Sense::click()));
                            if our_resp.clicked() { toggled = true; }

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let lsp_resp = ui.add(egui::Label::new(
                                    RichText::new(&lsp_label).color(lsp_color)
                                ).sense(egui::Sense::click()));
                                if lsp_resp.clicked() { toggled = true; }
                            });
                        });
                        if toggled { self.settings_show_sats = !self.settings_show_sats; }

                        ui.add_space(5.0);
                        let unit_hint = if self.settings_show_sats { "Tap to show USD" } else { "Tap to show BTC" };
                        ui.label(RichText::new(unit_hint).size(10.0).color(Color32::GRAY));
                    });

                    ui.add_space(20.0);
                }

                // Wallet Info section
                ui.group(|ui| {
                    ui.label(RichText::new("Wallet Information").size(16.0).strong().color(Color32::BLACK));
                    ui.add_space(10.0);

                    let wallet_id = self.node.node_id().to_string();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Wallet ID:").color(Color32::DARK_GRAY));
                        ui.label(RichText::new(&wallet_id[..7.min(wallet_id.len())]).monospace().size(12.0).color(Color32::BLACK));
                        if ui.small_button("Copy").clicked() {
                            ui.ctx().copy_text(wallet_id.clone());
                            self.show_toast("Copied!", "OK");
                        }
                    });

                    ui.add_space(10.0);

                    if let Some(ch) = self.node.list_channels().first() {
                        let user_ch_id = format!("{}", ch.user_channel_id.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Channel ID:").color(Color32::DARK_GRAY));
                            ui.label(RichText::new(&user_ch_id[..7.min(user_ch_id.len())]).monospace().size(12.0).color(Color32::BLACK));
                            if ui.small_button("Copy").clicked() {
                                ui.ctx().copy_text(user_ch_id.clone());
                                self.show_toast("Copied!", "OK");
                            }
                        });

                        if let Some(funding_txo) = &ch.funding_txo {
                            // Txid::to_string() already displays in reversed byte order (block explorer format)
                            let txid_display = funding_txo.txid.to_string();
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Funding Tx:").color(Color32::DARK_GRAY));
                                ui.label(RichText::new(&txid_display[..7.min(txid_display.len())]).monospace().size(12.0).color(Color32::BLACK));
                                if ui.small_button("Copy").clicked() {
                                    ui.ctx().copy_text(txid_display.clone());
                                    self.show_toast("Copied!", "OK");
                                }
                                let mempool_base = match self.network {
                                    Network::Signet => "https://mempool.space/signet",
                                    Network::Testnet => "https://mempool.space/testnet",
                                    _ => "https://mempool.space",
                                };
                                let url = format!("{}/tx/{}", mempool_base, txid_display);
                                let link_btn = egui::Button::new(RichText::new("View").size(11.0).color(Color32::from_rgb(59, 130, 246)))
                                    .frame(false);
                                if ui.add(link_btn).clicked() {
                                    ui.ctx().open_url(egui::OpenUrl::new_tab(&url));
                                }
                            });
                        }
                    }
                });

                ui.add_space(20.0);

                // Close Wallet
                ui.group(|ui| {
                    ui.label(RichText::new("Close Wallet").size(16.0).strong().color(Color32::from_rgb(225, 29, 72)));
                    ui.add_space(6.0);
                    ui.label(RichText::new("After you close your wallet, all funds will be sent to an on-chain Bitcoin address you control. Your trade and payment history will be preserved.").size(12.0).color(Color32::from_rgb(100, 116, 139)));
                    ui.add_space(8.0);

                    let close_btn = egui::Button::new(RichText::new("Close Wallet").color(Color32::from_rgb(225, 29, 72)))
                        .fill(Color32::from_rgb(255, 241, 242));
                    if ui.add(close_btn).clicked() {
                        self.confirm_close_popup = true;
                    }
                    ui.add_space(5.0);
                    ui.label(RichText::new("Confirm on the following screen.").size(11.0).color(Color32::GRAY));
                });

                ui.add_space(20.0);

                // Send Custom Message section
                ui.group(|ui| {
                    ui.label(RichText::new("Send Message").size(16.0).strong().color(Color32::BLACK));
                    ui.add_space(10.0);

                    ui.label(RichText::new("Send a message to the wallet devs.").color(Color32::DARK_GRAY).size(12.0));
                    ui.add_space(5.0);

                    ui.horizontal(|ui| {
                        let msg_edit = egui::TextEdit::singleline(&mut self.stable_message)
                            .hint_text("Say hello")
                            .desired_width(180.0);
                        ui.add(msg_edit);

                        if ui.button("Send").clicked() && !self.stable_message.is_empty() {
                            self.send_stable_message();
                            self.show_toast("Message sent!", "OK");
                        }
                    });
                });

                ui.add_space(20.0);

                // Backup section
                ui.group(|ui| {
                    ui.label(RichText::new("Backup").size(16.0).strong().color(Color32::BLACK));
                    ui.add_space(10.0);

                    ui.label(RichText::new("Download a backup of your wallet data").color(Color32::DARK_GRAY).size(12.0));
                    ui.add_space(5.0);

                    let backup_btn = egui::Button::new(RichText::new("Download Backup").color(Color32::WHITE))
                        .fill(Color32::from_rgb(59, 130, 246));
                    if ui.add(backup_btn).clicked() {
                        match self.create_backup() {
                            Ok(_path) => {
                                self.show_toast("Backup saved!", "OK");
                            }
                            Err(e) => {
                                self.show_toast("Backup failed", "!");
                                self.status_message = format!("Backup failed: {}", e);
                            }
                        }
                    }
                    ui.add_space(5.0);
                    ui.label(RichText::new("Your backup will be saved to your Downloads folder.").color(Color32::GRAY).size(11.0));

                    ui.add_space(15.0);

                    let seed_btn = egui::Button::new(RichText::new("Backup Seed Words").color(Color32::WHITE))
                        .fill(Color32::from_rgb(59, 130, 246));
                    if ui.add(seed_btn).clicked() {
                        self.show_seed_words = !self.show_seed_words;
                    }

                    if self.show_seed_words {
                        ui.add_space(10.0);
                        if let Some(ref words) = self.saved_mnemonic {
                            ui.label(RichText::new("Write these words down on paper and store them in a safe place. Never share them. Anyone with these words can access your funds.").color(Color32::from_rgb(217, 119, 6)).size(12.0));
                            ui.add_space(8.0);
                            for (i, word) in words.split_whitespace().enumerate() {
                                ui.label(RichText::new(format!("{}. {}", i + 1, word)).size(13.0).color(Color32::BLACK).monospace());
                            }
                        } else {
                            ui.label(RichText::new("Seed phrase not available for this wallet.").color(Color32::GRAY).size(12.0));
                        }
                    }
                });

                ui.add_space(20.0);

                // Pending Balances (from channel closures)
                let balances = self.node.list_balances();
                if !balances.pending_balances_from_channel_closures.is_empty() {
                    ui.group(|ui| {
                        ui.label(RichText::new("Pending from Channel Closures").strong().color(Color32::from_rgb(217, 119, 6)));
                        ui.add_space(8.0);

                        let mut total_pending: u64 = 0;
                        for pending in &balances.pending_balances_from_channel_closures {
                            let (status, amount) = match pending {
                                ldk_node::PendingSweepBalance::PendingBroadcast { amount_satoshis, .. } => {
                                    ("Pending broadcast", *amount_satoshis)
                                },
                                ldk_node::PendingSweepBalance::BroadcastAwaitingConfirmation { amount_satoshis, latest_spending_txid, .. } => {
                                    let status = format!("Awaiting confirmation ({}...)", &latest_spending_txid.to_string()[..8]);
                                    (status.leak() as &str, *amount_satoshis)
                                },
                                ldk_node::PendingSweepBalance::AwaitingThresholdConfirmations { amount_satoshis, confirmation_height, .. } => {
                                    let status = format!("Confirming (height {})", confirmation_height);
                                    (status.leak() as &str, *amount_satoshis)
                                },
                            };
                            total_pending += amount;
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("{} sats", amount)).size(12.0).color(Color32::BLACK).strong());
                                ui.label(RichText::new(format!("- {}", status)).size(11.0).color(Color32::DARK_GRAY));
                            });
                        }

                        if balances.pending_balances_from_channel_closures.len() > 1 {
                            ui.add_space(5.0);
                            ui.separator();
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Total pending:").size(12.0).color(Color32::DARK_GRAY));
                                ui.label(RichText::new(format!("{} sats", total_pending)).size(12.0).color(Color32::BLACK).strong());
                            });
                        }
                    });

                    ui.add_space(20.0);
                }

                // Claimable lightning balances
                let has_claimable = balances.lightning_balances.iter().any(|b| {
                    !matches!(b, ldk_node::LightningBalance::ClaimableOnChannelClose { .. })
                });
                if has_claimable {
                    ui.group(|ui| {
                        ui.label(RichText::new("Claimable Balances").strong().color(Color32::from_rgb(59, 130, 246)));
                        ui.add_space(8.0);

                        for balance in &balances.lightning_balances {
                            match balance {
                                ldk_node::LightningBalance::ClaimableAwaitingConfirmations { amount_satoshis, confirmation_height, .. } => {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(format!("{} sats", amount_satoshis)).size(12.0).color(Color32::BLACK).strong());
                                        ui.label(RichText::new(format!("- claimable at block {}", confirmation_height)).size(11.0).color(Color32::DARK_GRAY));
                                    });
                                },
                                ldk_node::LightningBalance::ContentiousClaimable { amount_satoshis, timeout_height, .. } => {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(format!("{} sats", amount_satoshis)).size(12.0).color(Color32::BLACK).strong());
                                        ui.label(RichText::new(format!("- claimable at height {}", timeout_height)).size(11.0).color(Color32::DARK_GRAY));
                                    });
                                },
                                ldk_node::LightningBalance::MaybeTimeoutClaimableHTLC { amount_satoshis, claimable_height, .. } => {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(format!("{} sats", amount_satoshis)).size(12.0).color(Color32::BLACK).strong());
                                        ui.label(RichText::new(format!("- HTLC claimable at {}", claimable_height)).size(11.0).color(Color32::DARK_GRAY));
                                    });
                                },
                                ldk_node::LightningBalance::MaybePreimageClaimableHTLC { amount_satoshis, expiry_height, .. } => {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(format!("{} sats", amount_satoshis)).size(12.0).color(Color32::BLACK).strong());
                                        ui.label(RichText::new(format!("- needs preimage, expires {}", expiry_height)).size(11.0).color(Color32::DARK_GRAY));
                                    });
                                },
                                ldk_node::LightningBalance::CounterpartyRevokedOutputClaimable { amount_satoshis, .. } => {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(format!("{} sats", amount_satoshis)).size(12.0).color(Color32::BLACK).strong());
                                        ui.label(RichText::new("- revoked output (justice tx)").size(11.0).color(Color32::from_rgb(225, 29, 72)));
                                    });
                                },
                                _ => {}
                            }
                        }
                    });

                    ui.add_space(20.0);
                }

                // Debug
                ui.group(|ui| {
                    ui.label(RichText::new("Debug").size(16.0).strong().color(Color32::BLACK));
                    ui.add_space(10.0);

                    if ui.button("View Logs").clicked() {
                        self.show_log_window = true;
                    }
                });
            });
    }

    fn show_history_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading(RichText::new("History").color(Color32::BLACK));
            ui.add_space(20.0);

            // Trade History
            ui.label(
                RichText::new("Trade History")
                    .size(16.0)
                    .strong()
                    .color(Color32::BLACK),
            );
            ui.add_space(10.0);

            match self.db.get_recent_trades(100) {
                Ok(trades) if trades.is_empty() => {
                    ui.label(RichText::new("No trades yet").color(Color32::GRAY));
                }
                Ok(trades) => {
                    let show_btc = self.history_show_btc;
                    let amt_header = if show_btc {
                        "Amt (BTC) <>"
                    } else {
                        "Amt (USD) <>"
                    };

                    // Header row — click Amt header to toggle BTC/USD
                    let mut toggled = false;
                    ui.horizontal(|ui| {
                        ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::hover()); // match "i" button width
                        ui.add_sized(
                            [30.0, 18.0],
                            egui::Label::new(
                                RichText::new("Type")
                                    .size(11.0)
                                    .color(Color32::DARK_GRAY)
                                    .strong(),
                            ),
                        );
                        let amt_resp = ui.add_sized(
                            [72.0, 18.0],
                            egui::Label::new(
                                RichText::new(amt_header)
                                    .size(11.0)
                                    .color(Color32::from_rgb(79, 70, 229))
                                    .strong(),
                            )
                            .sense(Sense::click()),
                        );
                        if amt_resp.clicked() {
                            toggled = true;
                        }
                        ui.add_sized(
                            [58.0, 18.0],
                            egui::Label::new(
                                RichText::new("Status")
                                    .size(11.0)
                                    .color(Color32::DARK_GRAY)
                                    .strong(),
                            ),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new("Date")
                                    .size(11.0)
                                    .color(Color32::DARK_GRAY)
                                    .strong(),
                            );
                        });
                    });
                    if toggled {
                        self.history_show_btc = !self.history_show_btc;
                    }
                    ui.separator();

                    let mut clicked_trade: Option<TradeRecord> = None;
                    egui::ScrollArea::vertical()
                        .id_salt("trade_history_scroll")
                        .max_height(200.0)
                        .show(ui, |ui| {
                            for trade in trades.iter() {
                                ui.horizontal(|ui| {
                                    // Circle "i" info button
                                    let (info_rect, info_resp) = ui.allocate_exact_size(
                                        egui::vec2(18.0, 18.0),
                                        Sense::click(),
                                    );
                                    let painter = ui.painter();
                                    let center = info_rect.center();
                                    painter.circle_stroke(
                                        center,
                                        7.0,
                                        egui::Stroke::new(1.0, Color32::GRAY),
                                    );
                                    painter.text(
                                        center,
                                        egui::Align2::CENTER_CENTER,
                                        "i",
                                        egui::FontId::proportional(11.0),
                                        Color32::GRAY,
                                    );
                                    if info_resp.clicked() {
                                        clicked_trade = Some(trade.clone());
                                    }

                                    let (color, label) = if trade.action == "buy" {
                                        (Color32::from_rgb(16, 185, 129), "Buy")
                                    } else {
                                        (Color32::from_rgb(244, 63, 94), "Sell")
                                    };
                                    ui.add_sized(
                                        [30.0, 18.0],
                                        egui::Label::new(
                                            RichText::new(label).color(color).strong(),
                                        ),
                                    );

                                    let amt_str = if show_btc {
                                        format!("{:.6}", trade.amount_btc)
                                    } else {
                                        format!("${:.2}", trade.amount_usd)
                                    };
                                    ui.add_sized(
                                        [72.0, 18.0],
                                        egui::Label::new(
                                            RichText::new(amt_str).color(Color32::BLACK),
                                        ),
                                    );

                                    let (status_label, status_color) = match trade.status.as_str() {
                                        "completed" => {
                                            ("Confirmed", Color32::from_rgb(16, 185, 129))
                                        }
                                        "pending" => ("Pending", Color32::from_rgb(234, 179, 8)),
                                        "failed" => ("Failed", Color32::from_rgb(225, 29, 72)),
                                        _ => (&*trade.status, Color32::DARK_GRAY),
                                    };
                                    ui.add_sized(
                                        [58.0, 18.0],
                                        egui::Label::new(
                                            RichText::new(status_label)
                                                .size(11.0)
                                                .color(status_color),
                                        ),
                                    );

                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new(Self::format_short_timestamp(
                                                    trade.created_at,
                                                ))
                                                .size(11.0)
                                                .color(Color32::GRAY),
                                            );
                                        },
                                    );
                                });
                                ui.add_space(6.0);
                            }
                        });
                    if let Some(t) = clicked_trade {
                        self.selected_trade = Some(t);
                        self.modal_opened_at = std::time::Instant::now();
                    }
                }
                Err(_) => {
                    ui.label(
                        RichText::new("Error loading trades").color(Color32::from_rgb(225, 29, 72)),
                    );
                }
            }

            ui.add_space(20.0);

            // Payment History
            ui.label(
                RichText::new("Payment History")
                    .size(16.0)
                    .strong()
                    .color(Color32::BLACK),
            );
            ui.add_space(10.0);

            match self.db.get_recent_payments(100) {
                Ok(payments) if payments.is_empty() => {
                    ui.label(RichText::new("No payments yet").color(Color32::GRAY));
                }
                Ok(payments) => {
                    let show_btc = self.history_show_btc;
                    let amt_header = if show_btc {
                        "Amt (BTC) <>"
                    } else {
                        "Amt (USD) <>"
                    };

                    // Header row — click Amt header to toggle BTC/USD
                    let mut toggled = false;
                    ui.horizontal(|ui| {
                        ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::hover()); // match "i" button width
                        ui.add_sized(
                            [24.0, 18.0],
                            egui::Label::new(
                                RichText::new("Dir")
                                    .size(11.0)
                                    .color(Color32::DARK_GRAY)
                                    .strong(),
                            ),
                        );
                        ui.add_sized(
                            [56.0, 18.0],
                            egui::Label::new(
                                RichText::new("Type")
                                    .size(11.0)
                                    .color(Color32::DARK_GRAY)
                                    .strong(),
                            ),
                        );
                        let amt_resp = ui.add_sized(
                            [72.0, 18.0],
                            egui::Label::new(
                                RichText::new(amt_header)
                                    .size(11.0)
                                    .color(Color32::from_rgb(79, 70, 229))
                                    .strong(),
                            )
                            .sense(Sense::click()),
                        );
                        if amt_resp.clicked() {
                            toggled = true;
                        }
                        ui.add_sized(
                            [58.0, 18.0],
                            egui::Label::new(
                                RichText::new("Status")
                                    .size(11.0)
                                    .color(Color32::DARK_GRAY)
                                    .strong(),
                            ),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new("Date")
                                    .size(11.0)
                                    .color(Color32::DARK_GRAY)
                                    .strong(),
                            );
                        });
                    });
                    if toggled {
                        self.history_show_btc = !self.history_show_btc;
                    }
                    ui.separator();

                    let mut clicked_payment: Option<PaymentRecord> = None;
                    egui::ScrollArea::vertical()
                        .id_salt("payment_history_scroll")
                        .max_height(200.0)
                        .show(ui, |ui| {
                            for payment in payments.iter() {
                                ui.horizontal(|ui| {
                                    // Circle "i" info button
                                    let (info_rect, info_resp) = ui.allocate_exact_size(
                                        egui::vec2(18.0, 18.0),
                                        Sense::click(),
                                    );
                                    let painter = ui.painter();
                                    let center = info_rect.center();
                                    painter.circle_stroke(
                                        center,
                                        7.0,
                                        egui::Stroke::new(1.0, Color32::GRAY),
                                    );
                                    painter.text(
                                        center,
                                        egui::Align2::CENTER_CENTER,
                                        "i",
                                        egui::FontId::proportional(11.0),
                                        Color32::GRAY,
                                    );
                                    if info_resp.clicked() {
                                        clicked_payment = Some(payment.clone());
                                    }

                                    let (color, label) = if payment.direction == "received" {
                                        (Color32::from_rgb(16, 185, 129), "In")
                                    } else {
                                        (Color32::from_rgb(217, 119, 6), "Out")
                                    };
                                    ui.add_sized(
                                        [24.0, 18.0],
                                        egui::Label::new(RichText::new(label).color(color)),
                                    );

                                    let (type_label, type_color) = match payment
                                        .payment_type
                                        .as_str()
                                    {
                                        "stability" => {
                                            ("Settlement", Color32::from_rgb(96, 165, 250))
                                        }
                                        "splice_in" => ("Deposit", Color32::from_rgb(16, 185, 129)),
                                        "splice_out" => {
                                            ("Withdraw", Color32::from_rgb(217, 119, 6))
                                        }
                                        "onchain" => ("On-chain", Color32::from_rgb(16, 185, 129)),
                                        "lightning" => {
                                            ("Lightning", Color32::from_rgb(234, 179, 8))
                                        }
                                        _ => ("Manual", Color32::DARK_GRAY),
                                    };
                                    ui.add_sized(
                                        [56.0, 18.0],
                                        egui::Label::new(
                                            RichText::new(type_label).size(11.0).color(type_color),
                                        ),
                                    );

                                    let amt_str = if show_btc {
                                        format!(
                                            "{:.6}",
                                            payment.amount_msat as f64 / 100_000_000_000.0
                                        )
                                    } else {
                                        payment
                                            .amount_usd
                                            .map(|u| format!("${:.2}", u))
                                            .unwrap_or("—".to_string())
                                    };
                                    ui.add_sized(
                                        [72.0, 18.0],
                                        egui::Label::new(
                                            RichText::new(amt_str).color(Color32::BLACK),
                                        ),
                                    );

                                    let (status_label, status_color) = match payment.status.as_str()
                                    {
                                        "completed" => {
                                            ("Confirmed", Color32::from_rgb(16, 185, 129))
                                        }
                                        "pending" => ("Pending", Color32::from_rgb(234, 179, 8)),
                                        "failed" => ("Failed", Color32::from_rgb(225, 29, 72)),
                                        _ => (&*payment.status, Color32::DARK_GRAY),
                                    };
                                    ui.add_sized(
                                        [58.0, 18.0],
                                        egui::Label::new(
                                            RichText::new(status_label)
                                                .size(11.0)
                                                .color(status_color),
                                        ),
                                    );

                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new(Self::format_short_timestamp(
                                                    payment.created_at,
                                                ))
                                                .size(11.0)
                                                .color(Color32::GRAY),
                                            );
                                        },
                                    );
                                });
                                if payment.payment_type == "onchain" {
                                    if let Some(ref txid) = payment.txid {
                                        let short = if txid.len() > 16 {
                                            format!("{}...{}", &txid[..8], &txid[txid.len() - 8..])
                                        } else {
                                            txid.clone()
                                        };
                                        ui.horizontal(|ui| {
                                            ui.add_space(20.0);
                                            let resp = ui.add(
                                                egui::Label::new(
                                                    RichText::new(format!("txid: {}", short))
                                                        .size(10.0)
                                                        .color(Color32::GRAY)
                                                        .monospace(),
                                                )
                                                .sense(Sense::click()),
                                            );
                                            if resp.clicked() {
                                                ui.ctx().copy_text(txid.clone());
                                            }
                                            if resp.hovered() {
                                                resp.on_hover_text("Click to copy full txid");
                                            }
                                        });
                                    }
                                }
                                ui.add_space(6.0);
                            }
                        });
                    if let Some(p) = clicked_payment {
                        self.selected_payment = Some(p);
                        self.modal_opened_at = std::time::Instant::now();
                    }
                }
                Err(_) => {
                    ui.label(
                        RichText::new("Error loading payments")
                            .color(Color32::from_rgb(225, 29, 72)),
                    );
                }
            }
        });
    }

    fn show_trade_detail_modal(&mut self, ctx: &egui::Context) {
        let trade = match &self.selected_trade {
            Some(t) => t.clone(),
            None => return,
        };
        egui::Window::new("Trade Details")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(Color32::WHITE)
                    .corner_radius(12.0),
            )
            .show(ctx, |ui| {
                ui.set_min_width(320.0);
                ui.add_space(8.0);

                let row = |ui: &mut egui::Ui, label: &str, value: &str| {
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [90.0, 18.0],
                            egui::Label::new(RichText::new(label).size(12.0).color(Color32::GRAY)),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(value).size(12.0).color(Color32::BLACK));
                        });
                    });
                    ui.add_space(2.0);
                };

                let action = if trade.action == "buy" { "Buy" } else { "Sell" };
                row(ui, "Type", action);
                row(ui, "Amount (USD)", &format!("${:.2}", trade.amount_usd));
                row(ui, "Amount (BTC)", &format!("{:.8}", trade.amount_btc));
                row(ui, "Fee (USD)", &format!("${:.2}", trade.fee_usd));
                if trade.btc_price > 0.0 {
                    let fee_btc = trade.fee_usd / trade.btc_price;
                    row(ui, "Fee (BTC)", &format!("{:.8}", fee_btc));
                }
                row(ui, "BTC Price", &Self::format_price(trade.btc_price));

                let (status_label, status_color) = match trade.status.as_str() {
                    "completed" => ("Confirmed", Color32::from_rgb(16, 185, 129)),
                    "pending" => ("Pending", Color32::from_rgb(234, 179, 8)),
                    "failed" => ("Failed", Color32::from_rgb(225, 29, 72)),
                    _ => (&*trade.status, Color32::DARK_GRAY),
                };
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [90.0, 18.0],
                        egui::Label::new(RichText::new("Status").size(12.0).color(Color32::GRAY)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(status_label)
                                .size(12.0)
                                .color(status_color)
                                .strong(),
                        );
                    });
                });
                ui.add_space(2.0);

                row(ui, "Date", &Self::format_timestamp(trade.created_at));

                if let Some(ref pid) = trade.payment_id {
                    ui.add_space(4.0);
                    ui.label(RichText::new("Trade ID").size(11.0).color(Color32::GRAY));
                    ui.label(
                        RichText::new(pid)
                            .size(10.0)
                            .color(Color32::DARK_GRAY)
                            .monospace(),
                    );
                }

                ui.add_space(10.0);
                ui.vertical_centered(|ui| {
                    if ui.button("Close").clicked() {
                        self.selected_trade = None;
                    }
                });
                ui.add_space(4.0);
            });
    }

    fn show_payment_detail_modal(&mut self, ctx: &egui::Context) {
        let payment = match &self.selected_payment {
            Some(p) => p.clone(),
            None => return,
        };
        egui::Window::new("Payment Details")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(Color32::WHITE)
                    .corner_radius(12.0),
            )
            .show(ctx, |ui| {
                ui.set_min_width(320.0);
                ui.add_space(8.0);

                let row = |ui: &mut egui::Ui, label: &str, value: &str| {
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [90.0, 18.0],
                            egui::Label::new(RichText::new(label).size(12.0).color(Color32::GRAY)),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(value).size(12.0).color(Color32::BLACK));
                        });
                    });
                    ui.add_space(2.0);
                };

                let dir = if payment.direction == "received" {
                    "Received"
                } else {
                    "Sent"
                };
                row(ui, "Direction", dir);

                let type_label = match payment.payment_type.as_str() {
                    "stability" => "Settlement",
                    "lightning" => "Lightning",
                    "splice_in" => "Deposit",
                    "splice_out" => "Withdraw",
                    "onchain" => "On-chain",
                    _ => "Manual",
                };
                row(ui, "Type", type_label);

                let btc_val = payment.amount_msat as f64 / 100_000_000_000.0;
                row(ui, "Amount (BTC)", &format!("{:.8}", btc_val));
                if let Some(usd) = payment.amount_usd {
                    row(ui, "Amount (USD)", &format!("${:.2}", usd));
                }

                if payment.fee_msat > 0 {
                    let fee_btc = payment.fee_msat as f64 / 100_000_000_000.0;
                    row(ui, "Fee (BTC)", &format!("{:.8}", fee_btc));
                    if let Some(price) = payment.btc_price {
                        row(ui, "Fee (USD)", &format!("${:.4}", fee_btc * price));
                    }
                }

                if let Some(price) = payment.btc_price {
                    row(ui, "BTC Price", &Self::format_price(price));
                }

                let (status_label, status_color) = match payment.status.as_str() {
                    "completed" => ("Confirmed", Color32::from_rgb(16, 185, 129)),
                    "pending" => ("Pending", Color32::from_rgb(234, 179, 8)),
                    "failed" => ("Failed", Color32::from_rgb(225, 29, 72)),
                    _ => (&*payment.status, Color32::DARK_GRAY),
                };
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [90.0, 18.0],
                        egui::Label::new(RichText::new("Status").size(12.0).color(Color32::GRAY)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(status_label)
                                .size(12.0)
                                .color(status_color)
                                .strong(),
                        );
                    });
                });
                ui.add_space(2.0);

                row(ui, "Date", &Self::format_timestamp(payment.created_at));

                if let Some(ref pid) = payment.payment_id {
                    ui.add_space(4.0);
                    ui.label(RichText::new("Payment ID").size(11.0).color(Color32::GRAY));
                    ui.label(
                        RichText::new(pid)
                            .size(10.0)
                            .color(Color32::DARK_GRAY)
                            .monospace(),
                    );
                }
                if let Some(ref cp) = payment.counterparty {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Counterparty")
                            .size(11.0)
                            .color(Color32::GRAY),
                    );
                    ui.label(
                        RichText::new(cp)
                            .size(10.0)
                            .color(Color32::DARK_GRAY)
                            .monospace(),
                    );
                }

                if let Some(ref txid) = payment.txid {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Transaction ID")
                            .size(11.0)
                            .color(Color32::GRAY),
                    );
                    ui.label(
                        RichText::new(txid)
                            .size(10.0)
                            .color(Color32::DARK_GRAY)
                            .monospace(),
                    );
                }
                if let Some(ref addr) = payment.address {
                    ui.add_space(4.0);
                    ui.label(RichText::new("Address").size(11.0).color(Color32::GRAY));
                    ui.label(
                        RichText::new(addr)
                            .size(10.0)
                            .color(Color32::DARK_GRAY)
                            .monospace(),
                    );
                }

                ui.add_space(10.0);
                ui.vertical_centered(|ui| {
                    if ui.button("Close").clicked() {
                        self.selected_payment = None;
                    }
                });
                ui.add_space(4.0);
            });
    }

    fn show_transfer_modal_ui(&mut self, ctx: &egui::Context) {
        egui::Window::new("Transfer")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(egui::Frame::window(&ctx.style()).fill(Color32::WHITE))
            .show(ctx, |ui| {
                ui.set_min_width(320.0);
                ui.add_space(10.0);

                // Tab bar
                ui.horizontal(|ui| {
                    let tabs = [
                        (TransferTab::Send, "Send"),
                        (TransferTab::Receive, "Receive"),
                    ];
                    for (tab, label) in &tabs {
                        let is_selected = self.transfer_tab == *tab;
                        let btn = if is_selected {
                            egui::Button::new(
                                RichText::new(*label)
                                    .size(15.0)
                                    .color(Color32::WHITE)
                                    .strong(),
                            )
                            .fill(Color32::from_rgb(30, 41, 59))
                            .corner_radius(8.0)
                        } else {
                            egui::Button::new(
                                RichText::new(*label)
                                    .size(15.0)
                                    .color(Color32::from_rgb(100, 116, 139)),
                            )
                            .fill(Color32::from_rgb(241, 245, 249))
                            .corner_radius(8.0)
                        };
                        if ui.add(btn).clicked() {
                            self.transfer_tab = tab.clone();
                        }
                    }
                });

                ui.add_space(15.0);

                match self.transfer_tab {
                    TransferTab::Send => self.show_send_tab(ui),
                    TransferTab::Receive => self.show_receive_tab(ui, ctx),
                }

                ui.add_space(15.0);

                ui.vertical_centered(|ui| {
                    if ui.button("Close").clicked() {
                        self.show_transfer_modal = false;
                        self.show_lightning_receive = false;
                        self.lightning_receive_invoice.clear();
                        self.lightning_receive_qr = None;
                        self.send_error.clear();
                    }
                });

                ui.add_space(10.0);
            });
    }

    fn show_send_tab(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("Paste an on-chain address, bolt11 invoice, or bolt12 offer")
                .size(12.0)
                .italics()
                .color(Color32::from_rgb(100, 116, 139)),
        );
        ui.add_space(10.0);

        let input_edit = egui::TextEdit::multiline(&mut self.send_input)
            .hint_text("bc1..., lnbc..., or lno1...")
            .desired_width(300.0)
            .desired_rows(3);
        ui.add(input_edit);

        ui.add_space(10.0);

        // Auto-detect indicator
        let input_lower = self.send_input.trim().to_lowercase();
        if !input_lower.is_empty() {
            let detected = if input_lower.starts_with("lnbc")
                || input_lower.starts_with("lntb")
                || input_lower.starts_with("lightning:")
            {
                "Bolt11 Invoice"
            } else if input_lower.starts_with("lno1") {
                "Bolt12 Offer"
            } else if input_lower.starts_with("bc1")
                || input_lower.starts_with("tb1")
                || input_lower.starts_with("1")
                || input_lower.starts_with("3")
                || input_lower.starts_with("bcrt1")
            {
                "On-chain Address"
            } else {
                "Unknown"
            };
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Detected:")
                        .size(11.0)
                        .color(Color32::DARK_GRAY),
                );
                let color = if detected == "Unknown" {
                    Color32::from_rgb(244, 63, 94)
                } else {
                    Color32::from_rgb(16, 185, 129)
                };
                ui.label(RichText::new(detected).size(11.0).color(color).strong());
            });
            ui.add_space(8.0);
        }

        // Show "Send all" checkbox for on-chain addresses
        let is_onchain = {
            let l = self.send_input.trim().to_lowercase();
            l.starts_with("bc1")
                || l.starts_with("tb1")
                || l.starts_with("1")
                || l.starts_with("3")
                || l.starts_with("bcrt1")
        };
        let is_bolt11 = input_lower.starts_with("lnbc")
            || input_lower.starts_with("lntb")
            || input_lower.starts_with("lightning:");
        let is_bolt12 = input_lower.starts_with("lno1");

        if is_onchain {
            let price = self.stable_channel.lock().unwrap().latest_price;
            ui.add_space(6.0);

            // USD amount field
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Amount (USD):")
                        .size(12.0)
                        .color(Color32::DARK_GRAY),
                );
                let prev_usd = self.send_amount_usd.clone();
                let usd_edit = egui::TextEdit::singleline(&mut self.send_amount_usd)
                    .hint_text("e.g. 10.00")
                    .desired_width(120.0);
                let usd_resp = ui.add(usd_edit);
                if usd_resp.has_focus() {
                    self.send_amount_editing = false;
                }
                if !self.send_amount_editing && self.send_amount_usd != prev_usd {
                    self.send_all = false;
                    if let Ok(usd_val) = self.send_amount_usd.trim().parse::<f64>() {
                        if price > 0.0 && usd_val >= 0.0 {
                            self.send_amount = format!("{:.8}", usd_val / price);
                        }
                    }
                }
            });

            // BTC amount field
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Amount (BTC):")
                        .size(12.0)
                        .color(Color32::DARK_GRAY),
                );
                let prev_amount = self.send_amount.clone();
                let amount_edit = egui::TextEdit::singleline(&mut self.send_amount)
                    .hint_text("e.g. 0.0005")
                    .desired_width(120.0);
                let btc_resp = ui.add(amount_edit);
                if btc_resp.has_focus() {
                    self.send_amount_editing = true;
                }
                if self.send_amount_editing && self.send_amount != prev_amount {
                    self.send_all = false;
                    if let Ok(btc_val) = self.send_amount.trim().parse::<f64>() {
                        if price > 0.0 && btc_val >= 0.0 {
                            self.send_amount_usd = format!("{:.2}", btc_val * price);
                        }
                    }
                }
                let max_btn = egui::Button::new(
                    RichText::new("Max")
                        .size(12.0)
                        .color(Color32::from_rgb(79, 70, 229)),
                )
                .fill(Color32::from_rgb(238, 242, 255))
                .corner_radius(6.0);
                if ui.add(max_btn).clicked() {
                    let balances = self.node.list_balances();
                    let max_sats = if let Some(ch) = self
                        .node
                        .list_channels()
                        .iter()
                        .find(|c| c.is_channel_ready)
                    {
                        (ch.outbound_capacity_msat / 1000).max(balances.total_onchain_balance_sats)
                    } else {
                        balances.total_onchain_balance_sats
                    };
                    let max_btc = max_sats as f64 / 100_000_000.0;
                    self.send_amount = format!("{:.8}", max_btc);
                    if price > 0.0 {
                        self.send_amount_usd = format!("{:.2}", max_btc * price);
                    }
                    self.send_all = true;
                }
            });
            ui.add_space(8.0);
        }

        // Amount field for Lightning (Bolt12 always needs it; Bolt11 for variable-amount invoices)
        let needs_ln_amount = if is_bolt12 {
            true
        } else if is_bolt11 {
            let inv_str = if input_lower.starts_with("lightning:") {
                &self.send_input.trim()[10..]
            } else {
                self.send_input.trim()
            };
            Bolt11Invoice::from_str(inv_str)
                .map(|inv| inv.amount_milli_satoshis().is_none())
                .unwrap_or(false)
        } else {
            false
        };

        if needs_ln_amount {
            let price = self.stable_channel.lock().unwrap().latest_price;
            ui.add_space(4.0);

            // USD amount field
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Amount (USD):")
                        .size(12.0)
                        .color(Color32::DARK_GRAY),
                );
                let prev_usd = self.send_amount_usd.clone();
                let usd_edit = egui::TextEdit::singleline(&mut self.send_amount_usd)
                    .hint_text("e.g. 10.00")
                    .desired_width(140.0);
                let usd_resp = ui.add(usd_edit);
                if usd_resp.has_focus() {
                    self.send_amount_editing = false;
                }
                if !self.send_amount_editing && self.send_amount_usd != prev_usd {
                    if let Ok(usd_val) = self.send_amount_usd.trim().parse::<f64>() {
                        if price > 0.0 && usd_val >= 0.0 {
                            self.send_amount = format!("{:.8}", usd_val / price);
                        }
                    }
                }
            });

            // BTC amount field
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Amount (BTC):")
                        .size(12.0)
                        .color(Color32::DARK_GRAY),
                );
                let prev_btc = self.send_amount.clone();
                let amount_edit = egui::TextEdit::singleline(&mut self.send_amount)
                    .hint_text("e.g. 0.0005")
                    .desired_width(140.0);
                let btc_resp = ui.add(amount_edit);
                if btc_resp.has_focus() {
                    self.send_amount_editing = true;
                }
                if self.send_amount_editing && self.send_amount != prev_btc {
                    if let Ok(btc_val) = self.send_amount.trim().parse::<f64>() {
                        if price > 0.0 && btc_val >= 0.0 {
                            self.send_amount_usd = format!("{:.2}", btc_val * price);
                        }
                    }
                }
            });
            ui.add_space(8.0);
        }

        // Fee estimation
        if !input_lower.is_empty() && (is_onchain || is_bolt11 || is_bolt12) {
            if self.cached_fee_rate.is_none() {
                self.fetch_fee_rate();
            }

            let fee_text = if is_onchain {
                // On-chain: ~140 vB for simple send, ~250 vB for send-all (more inputs)
                let vbytes: u64 = if self.send_all { 250 } else { 140 };
                match self.cached_fee_rate {
                    Some(rate) => {
                        let estimated_fee = rate * vbytes;
                        let fee_btc = estimated_fee as f64 / 100_000_000.0;
                        format!("Estimated fee: ~{:.8} BTC ({} sat/vB)", fee_btc, rate)
                    }
                    None => "Estimating fee...".to_string(),
                }
            } else if is_bolt11 {
                // Try to parse invoice and show amount + routing fee estimate
                let invoice_str = if input_lower.starts_with("lightning:") {
                    &self.send_input.trim()[10..]
                } else {
                    self.send_input.trim()
                };
                match Bolt11Invoice::from_str(invoice_str) {
                    Ok(invoice) => {
                        if let Some(amount_msat) = invoice.amount_milli_satoshis() {
                            let amount_btc = amount_msat as f64 / 1000.0 / 100_000_000.0;
                            let fee_btc = amount_btc / 100.0;
                            let price = self.stable_channel.lock().unwrap().latest_price;
                            let usd_str = if price > 0.0 {
                                format!(" (${:.2})", amount_btc * price)
                            } else {
                                String::new()
                            };
                            format!(
                                "Amount: {:.8} BTC{} + ~{:.8} BTC routing fee",
                                amount_btc,
                                usd_str,
                                fee_btc.max(0.00000001)
                            )
                        } else {
                            "Variable amount invoice (no fee estimate)".to_string()
                        }
                    }
                    Err(_) => String::new(),
                }
            } else {
                "Routing fee: typically < 1%".to_string()
            };

            if !fee_text.is_empty() {
                ui.label(
                    RichText::new(&fee_text)
                        .size(11.0)
                        .color(Color32::DARK_GRAY),
                );
                ui.add_space(8.0);
            }
        }

        // Warn if funds are pending and not yet spendable
        let balances_check = self.node.list_balances();
        let has_pending = !balances_check
            .pending_balances_from_channel_closures
            .is_empty();
        let spendable = balances_check.spendable_onchain_balance_sats;
        let has_channel = self.node.list_channels().iter().any(|c| c.is_channel_ready);
        if has_pending && spendable == 0 && !has_channel {
            ui.label(
                RichText::new("Funds are pending on-chain confirmation and can't be sent yet.")
                    .size(12.0)
                    .color(Color32::from_rgb(217, 119, 6)),
            );
            ui.add_space(8.0);
        }

        ui.vertical_centered(|ui| {
            let send_btn = egui::Button::new(
                RichText::new("Send")
                    .size(14.0)
                    .color(Color32::WHITE)
                    .strong(),
            )
            .fill(Color32::from_rgb(30, 41, 59))
            .corner_radius(10.0)
            .min_size(egui::vec2(200.0, 38.0));
            if ui.add(send_btn).clicked() && self.send_unified() {
                self.show_toast("Payment sent!", "OK");
                self.show_transfer_modal = false;
            }
        });

        if !self.send_error.is_empty() {
            ui.add_space(8.0);
            ui.label(
                RichText::new(&self.send_error)
                    .size(12.0)
                    .color(Color32::from_rgb(225, 29, 72)),
            );
        }
    }

    fn show_receive_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // On-chain address
        ui.group(|ui| {
            ui.label(
                RichText::new("On-chain Address")
                    .strong()
                    .color(Color32::BLACK),
            );
            ui.add_space(6.0);

            if ui.button("Generate Address").clicked() {
                self.get_address();
                self.show_toast("Address ready!", "OK");
            }

            if !self.on_chain_address.is_empty() {
                ui.add_space(5.0);
                ui.label(
                    RichText::new(&self.on_chain_address)
                        .monospace()
                        .size(10.0)
                        .color(Color32::DARK_GRAY),
                );
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(self.on_chain_address.clone());
                    self.show_toast("Copied!", "OK");
                }
            }
        });

        ui.add_space(10.0);

        // Bolt11 invoice
        ui.group(|ui| {
            ui.label(
                RichText::new("Lightning Invoice (Bolt11)")
                    .strong()
                    .color(Color32::BLACK),
            );
            ui.add_space(6.0);

            if self.show_lightning_receive && !self.lightning_receive_invoice.is_empty() {
                ui.vertical_centered(|ui| {
                    if let Some(ref qr) = self.lightning_receive_qr {
                        let img = egui::Image::from_texture(qr).max_size(egui::vec2(180.0, 180.0));
                        ui.add(img);
                    }
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(
                            &self.lightning_receive_invoice
                                [..40.min(self.lightning_receive_invoice.len())],
                        )
                        .monospace()
                        .size(10.0)
                        .color(Color32::DARK_GRAY),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Copy Invoice").clicked() {
                            ui.ctx().copy_text(self.lightning_receive_invoice.clone());
                            self.show_toast("Copied!", "OK");
                        }
                        if ui.button("Done").clicked() {
                            self.show_lightning_receive = false;
                            self.lightning_receive_invoice.clear();
                            self.lightning_receive_qr = None;
                            self.lightning_receive_error.clear();
                        }
                    });
                });
            } else {
                let price = self.btc_price;

                // USD amount field
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Amount (USD):").color(Color32::DARK_GRAY));
                    let prev_usd = self.lightning_receive_amount_usd.clone();
                    let usd_edit =
                        egui::TextEdit::singleline(&mut self.lightning_receive_amount_usd)
                            .hint_text("e.g. 10.00")
                            .desired_width(100.0);
                    let usd_resp = ui.add(usd_edit);
                    if usd_resp.has_focus() {
                        self.lightning_receive_editing_btc = false;
                    }
                    if !self.lightning_receive_editing_btc
                        && self.lightning_receive_amount_usd != prev_usd
                    {
                        if let Ok(usd_val) = self.lightning_receive_amount_usd.trim().parse::<f64>()
                        {
                            if price > 0.0 && usd_val >= 0.0 {
                                self.lightning_receive_amount = format!("{:.8}", usd_val / price);
                            }
                        }
                    }
                });

                // BTC amount field
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Amount (BTC):").color(Color32::DARK_GRAY));
                    let prev_btc = self.lightning_receive_amount.clone();
                    let btc_edit = egui::TextEdit::singleline(&mut self.lightning_receive_amount)
                        .hint_text("e.g. 0.0001")
                        .desired_width(100.0);
                    let btc_resp = ui.add(btc_edit);
                    if btc_resp.has_focus() {
                        self.lightning_receive_editing_btc = true;
                    }
                    if self.lightning_receive_editing_btc
                        && self.lightning_receive_amount != prev_btc
                    {
                        if let Ok(btc_val) = self.lightning_receive_amount.trim().parse::<f64>() {
                            if price > 0.0 && btc_val >= 0.0 {
                                self.lightning_receive_amount_usd =
                                    format!("{:.2}", btc_val * price);
                            }
                        }
                    }
                });

                ui.add_space(8.0);
                if ui.button("Generate Invoice").clicked() {
                    let amount = self.lightning_receive_amount.trim();
                    if amount.is_empty()
                        || amount == "0"
                        || amount.parse::<f64>().unwrap_or(0.0) <= 0.0
                    {
                        self.lightning_receive_error = "Please enter an amount.".to_string();
                    } else {
                        self.lightning_receive_error.clear();
                        self.generate_lightning_receive_invoice(ctx);
                    }
                }
                if !self.lightning_receive_error.is_empty() {
                    ui.add_space(5.0);
                    ui.label(
                        RichText::new(&self.lightning_receive_error)
                            .color(Color32::from_rgb(225, 29, 72)),
                    );
                }
            }
        });

        ui.add_space(10.0);

        // Bolt12 offer
        ui.group(|ui| {
            ui.label(
                RichText::new("Lightning Offer (Bolt12)")
                    .strong()
                    .color(Color32::BLACK),
            );
            ui.add_space(6.0);

            if !self.bolt12_offer.is_empty() {
                ui.label(
                    RichText::new(&self.bolt12_offer[..60.min(self.bolt12_offer.len())])
                        .monospace()
                        .size(10.0)
                        .color(Color32::DARK_GRAY),
                );
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    if ui.button("Copy Offer").clicked() {
                        ui.ctx().copy_text(self.bolt12_offer.clone());
                        self.show_toast("Copied!", "OK");
                    }
                    if ui.button("Done").clicked() {
                        self.bolt12_offer.clear();
                    }
                });
            } else {
                let has_ready_channel =
                    self.node.list_channels().iter().any(|c| c.is_channel_ready);
                if !has_ready_channel {
                    ui.label(
                        RichText::new(
                            "Channel not ready — offer creation requires an active channel",
                        )
                        .size(11.0)
                        .color(Color32::from_rgb(217, 119, 6)),
                    );
                } else {
                    ui.label(
                        RichText::new("Reusable, no amount required from sender")
                            .size(11.0)
                            .italics()
                            .color(Color32::from_rgb(100, 116, 139)),
                    );
                    ui.add_space(6.0);
                    if ui.button("Generate Offer").clicked() {
                        match self
                            .node
                            .bolt12_payment()
                            .receive_variable_amount("Stable Channels", None)
                        {
                            Ok(offer) => {
                                self.bolt12_offer = offer.to_string();
                                self.show_toast("Offer ready!", "OK");
                            }
                            Err(e) => {
                                self.show_toast("Offer failed", "!");
                                self.status_message = format!("Bolt12 error: {}", e);
                            }
                        }
                    }
                }
            }
        });
    }

    fn show_buy_modal_ui(&mut self, ctx: &egui::Context) {
        egui::Window::new("Buy BTC")
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(Color32::WHITE)
                    .corner_radius(16.0),
            )
            .show(ctx, |ui| {
                ui.set_min_width(300.0);
                ui.add_space(20.0);

                // Check if we're on confirmation screen
                if self.show_confirm_trade && self.pending_trade.is_some() {
                    self.show_buy_confirm_screen(ui);
                } else {
                    self.show_buy_amount_screen(ui);
                }

                ui.add_space(10.0);
            });
    }

    fn show_buy_amount_screen(&mut self, ui: &mut egui::Ui) {
        // Header
        ui.label(
            RichText::new("Buy bitcoin")
                .size(28.0)
                .color(Color32::BLACK)
                .strong(),
        );
        ui.add_space(5.0);

        // Show available USD balance (the stabilized amount)
        let available_usd = {
            let sc = self.stable_channel.lock().unwrap();
            sc.expected_usd.0
        };
        ui.label(
            RichText::new(format!("Available: {}", Self::format_price(available_usd)))
                .size(14.0)
                .color(Color32::DARK_GRAY),
        );
        ui.add_space(20.0);

        // Preset amount buttons in a 2x3 grid
        let preset_amounts = [20.0, 50.0, 100.0, 150.0, 200.0];
        let btn_size = egui::vec2(85.0, 55.0);

        // Row 1: $20, $50, $100
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for &amount in &preset_amounts[0..3] {
                let selected = self.trade_amount_input == format!("{:.0}", amount);
                let btn = egui::Button::new(
                    RichText::new(format!("${:.0}", amount))
                        .size(18.0)
                        .color(Color32::BLACK),
                )
                .fill(if selected {
                    Color32::from_rgb(226, 232, 240)
                } else {
                    Color32::WHITE
                })
                .stroke(egui::Stroke::new(1.5, Color32::from_rgb(203, 213, 225)))
                .corner_radius(12.0)
                .min_size(btn_size);
                if ui.add(btn).clicked() {
                    self.trade_amount_input = format!("{:.0}", amount);
                }
            }
        });

        ui.add_space(10.0);

        // Row 2: $150, $200, ...
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for &amount in &preset_amounts[3..5] {
                let selected = self.trade_amount_input == format!("{:.0}", amount);
                let btn = egui::Button::new(
                    RichText::new(format!("${:.0}", amount))
                        .size(18.0)
                        .color(Color32::BLACK),
                )
                .fill(if selected {
                    Color32::from_rgb(226, 232, 240)
                } else {
                    Color32::WHITE
                })
                .stroke(egui::Stroke::new(1.5, Color32::from_rgb(203, 213, 225)))
                .corner_radius(12.0)
                .min_size(btn_size);
                if ui.add(btn).clicked() {
                    self.trade_amount_input = format!("{:.0}", amount);
                }
            }
            // Custom amount button (...)
            let btn = egui::Button::new(RichText::new("...").size(18.0).color(Color32::BLACK))
                .fill(Color32::WHITE)
                .stroke(egui::Stroke::new(1.5, Color32::from_rgb(203, 213, 225)))
                .corner_radius(12.0)
                .min_size(btn_size);
            if ui.add(btn).clicked() {
                self.trade_amount_input.clear();
            }
        });

        // Custom input field (shown when ... is selected or input is custom)
        let is_custom = !preset_amounts
            .iter()
            .any(|&a| self.trade_amount_input == format!("{:.0}", a));
        if is_custom && !self.trade_amount_input.is_empty() || self.trade_amount_input.is_empty() {
            ui.add_space(15.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Custom:").color(Color32::DARK_GRAY));
                let amount_edit = egui::TextEdit::singleline(&mut self.trade_amount_input)
                    .hint_text("Enter amount")
                    .desired_width(100.0);
                ui.add(amount_edit);
            });
        }

        if !self.trade_error.is_empty() {
            ui.add_space(5.0);
            ui.label(
                RichText::new(&self.trade_error)
                    .color(Color32::from_rgb(225, 29, 72))
                    .size(12.0),
            );
        }

        ui.add_space(20.0);

        // Next button (disabled if no amount selected)
        let has_amount = !self.trade_amount_input.is_empty();
        let btn_color = if has_amount {
            Color32::from_rgb(30, 41, 59)
        } else {
            Color32::from_rgb(203, 213, 225)
        };
        let text_color = if has_amount {
            Color32::WHITE
        } else {
            Color32::from_rgb(148, 163, 184)
        };

        let mut should_process = false;
        ui.vertical_centered(|ui| {
            let next_btn =
                egui::Button::new(RichText::new("Next").size(16.0).color(text_color).strong())
                    .fill(btn_color)
                    .corner_radius(25.0)
                    .min_size(egui::vec2(280.0, 50.0));
            if ui.add(next_btn).clicked() && has_amount {
                should_process = true;
            }
        });

        if should_process {
            if let Ok(amount) = self.trade_amount_input.parse::<f64>() {
                // Validate user has enough USD
                if amount > available_usd {
                    self.trade_error = format!(
                        "Insufficient balance. You have {} available.",
                        Self::format_price(available_usd)
                    );
                } else {
                    // Calculate trade details (use non-blocking cached price)
                    let btc_price = get_cached_price_no_fetch();
                    let fee_usd = amount * 0.01; // 1% fee
                    let net_amount = amount - fee_usd;
                    let btc_amount = net_amount / btc_price;

                    self.pending_trade = Some(PendingTrade {
                        action: TradeAction::BuyBtc,
                        amount_usd: amount,
                        btc_price,
                        fee_usd,
                        btc_amount,
                        net_amount_usd: net_amount,
                    });
                    self.show_confirm_trade = true;
                    self.trade_error.clear();
                }
            } else {
                self.trade_error = "Invalid amount".to_string();
            }
        }

        ui.add_space(8.0);

        // Disclaimer
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("Exchange rate may differ slightly at execution.")
                    .size(11.0)
                    .color(Color32::GRAY),
            );
        });

        ui.add_space(8.0);

        // Cancel link
        ui.vertical_centered(|ui| {
            if ui
                .add(egui::Button::new(RichText::new("Cancel").color(Color32::GRAY)).frame(false))
                .clicked()
            {
                self.show_buy_modal = false;
                self.trade_amount_input.clear();
                self.trade_error.clear();
                self.pending_trade = None;
                self.show_confirm_trade = false;
            }
        });
    }

    fn show_buy_confirm_screen(&mut self, ui: &mut egui::Ui) {
        let trade = self.pending_trade.as_ref().unwrap();
        let amount_usd = trade.amount_usd;
        let btc_price = trade.btc_price;
        let fee_usd = trade.fee_usd;
        let btc_amount = trade.btc_amount;
        let net_amount = trade.net_amount_usd;

        // Header
        ui.label(
            RichText::new("Confirm purchase")
                .size(28.0)
                .color(Color32::BLACK)
                .strong(),
        );
        ui.add_space(20.0);

        // Order summary
        ui.group(|ui| {
            ui.set_min_width(260.0);

            // Amount
            ui.horizontal(|ui| {
                ui.label(RichText::new("Amount").color(Color32::DARK_GRAY));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("${:.2}", amount_usd))
                            .color(Color32::BLACK)
                            .strong(),
                    );
                });
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            // BTC Price
            ui.horizontal(|ui| {
                ui.label(RichText::new("BTC Price").color(Color32::DARK_GRAY));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("${:.2}", btc_price)).color(Color32::BLACK));
                });
            });

            ui.add_space(8.0);

            // Fee (1%)
            ui.horizontal(|ui| {
                ui.label(RichText::new("Fee (1%)").color(Color32::DARK_GRAY));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("-${:.2}", fee_usd))
                            .color(Color32::from_rgb(244, 63, 94)),
                    );
                });
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            // You receive
            ui.horizontal(|ui| {
                ui.label(RichText::new("You receive").color(Color32::DARK_GRAY));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{:.8} BTC", btc_amount))
                            .color(Color32::BLACK)
                            .strong(),
                    );
                });
            });

            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label(RichText::new("").color(Color32::DARK_GRAY));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("(${:.2})", net_amount))
                            .color(Color32::GRAY)
                            .size(12.0),
                    );
                });
            });
        });

        ui.add_space(20.0);

        // Confirm button
        let mut should_confirm = false;
        ui.vertical_centered(|ui| {
            let confirm_btn = egui::Button::new(
                RichText::new("Confirm Buy")
                    .size(16.0)
                    .color(Color32::WHITE)
                    .strong(),
            )
            .fill(Color32::from_rgb(16, 185, 129))
            .corner_radius(25.0)
            .min_size(egui::vec2(280.0, 50.0));
            if ui.add(confirm_btn).clicked() {
                should_confirm = true;
            }
        });
        if should_confirm {
            self.execute_buy(amount_usd);
            self.show_buy_modal = false;
            self.trade_amount_input.clear();
            self.pending_trade = None;
            self.show_confirm_trade = false;
        }

        ui.add_space(10.0);

        // Back button
        ui.vertical_centered(|ui| {
            if ui
                .add(egui::Button::new(RichText::new("Back").color(Color32::GRAY)).frame(false))
                .clicked()
            {
                self.show_confirm_trade = false;
            }
        });
    }

    fn show_sell_modal_ui(&mut self, ctx: &egui::Context) {
        egui::Window::new("Sell BTC")
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(Color32::WHITE)
                    .corner_radius(16.0),
            )
            .show(ctx, |ui| {
                ui.set_min_width(300.0);
                ui.add_space(20.0);

                // Check if we're on confirmation screen
                if self.show_confirm_trade && self.pending_trade.is_some() {
                    self.show_sell_confirm_screen(ui);
                } else {
                    self.show_sell_amount_screen(ui);
                }

                ui.add_space(10.0);
            });
    }

    fn show_sell_amount_screen(&mut self, ui: &mut egui::Ui) {
        // Header
        ui.label(
            RichText::new("Sell bitcoin")
                .size(28.0)
                .color(Color32::BLACK)
                .strong(),
        );
        ui.add_space(5.0);

        // Show available BTC balance (in USD terms) = total - stabilized
        let btc_price = get_cached_price_no_fetch();
        let available_btc_usd = {
            let sc = self.stable_channel.lock().unwrap();
            let total_usd = if sc.is_stable_receiver {
                sc.stable_receiver_usd.0
            } else {
                sc.stable_provider_usd.0
            };
            // BTC portion is total minus the stabilized USD amount
            (total_usd - sc.expected_usd.0).max(0.0)
        };
        ui.label(
            RichText::new(format!(
                "Available: {}",
                Self::format_price(available_btc_usd)
            ))
            .size(14.0)
            .color(Color32::DARK_GRAY),
        );
        ui.add_space(20.0);

        // Preset amount buttons in a 2x3 grid
        let preset_amounts = [20.0, 50.0, 100.0, 150.0, 200.0];
        let btn_size = egui::vec2(85.0, 55.0);

        // Row 1: $20, $50, $100
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for &amount in &preset_amounts[0..3] {
                let selected = self.trade_amount_input == format!("{:.0}", amount);
                let btn = egui::Button::new(
                    RichText::new(format!("${:.0}", amount))
                        .size(18.0)
                        .color(Color32::BLACK),
                )
                .fill(if selected {
                    Color32::from_rgb(226, 232, 240)
                } else {
                    Color32::WHITE
                })
                .stroke(egui::Stroke::new(1.5, Color32::from_rgb(203, 213, 225)))
                .corner_radius(12.0)
                .min_size(btn_size);
                if ui.add(btn).clicked() {
                    self.trade_amount_input = format!("{:.0}", amount);
                }
            }
        });

        ui.add_space(10.0);

        // Row 2: $150, $200, ...
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for &amount in &preset_amounts[3..5] {
                let selected = self.trade_amount_input == format!("{:.0}", amount);
                let btn = egui::Button::new(
                    RichText::new(format!("${:.0}", amount))
                        .size(18.0)
                        .color(Color32::BLACK),
                )
                .fill(if selected {
                    Color32::from_rgb(226, 232, 240)
                } else {
                    Color32::WHITE
                })
                .stroke(egui::Stroke::new(1.5, Color32::from_rgb(203, 213, 225)))
                .corner_radius(12.0)
                .min_size(btn_size);
                if ui.add(btn).clicked() {
                    self.trade_amount_input = format!("{:.0}", amount);
                }
            }
            // Custom amount button (...)
            let btn = egui::Button::new(RichText::new("...").size(18.0).color(Color32::BLACK))
                .fill(Color32::WHITE)
                .stroke(egui::Stroke::new(1.5, Color32::from_rgb(203, 213, 225)))
                .corner_radius(12.0)
                .min_size(btn_size);
            if ui.add(btn).clicked() {
                self.trade_amount_input.clear();
            }
        });

        // Custom input field (shown when ... is selected or input is custom)
        let is_custom = !preset_amounts
            .iter()
            .any(|&a| self.trade_amount_input == format!("{:.0}", a));
        if is_custom && !self.trade_amount_input.is_empty() || self.trade_amount_input.is_empty() {
            ui.add_space(15.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Custom:").color(Color32::DARK_GRAY));
                let amount_edit = egui::TextEdit::singleline(&mut self.trade_amount_input)
                    .hint_text("Enter amount")
                    .desired_width(100.0);
                ui.add(amount_edit);
            });
        }

        if !self.trade_error.is_empty() {
            ui.add_space(5.0);
            ui.label(
                RichText::new(&self.trade_error)
                    .color(Color32::from_rgb(225, 29, 72))
                    .size(12.0),
            );
        }

        ui.add_space(20.0);

        // Next button (disabled if no amount selected)
        let has_amount = !self.trade_amount_input.is_empty();
        let btn_color = if has_amount {
            Color32::from_rgb(30, 41, 59)
        } else {
            Color32::from_rgb(203, 213, 225)
        };
        let text_color = if has_amount {
            Color32::WHITE
        } else {
            Color32::from_rgb(148, 163, 184)
        };

        let mut should_process = false;
        ui.vertical_centered(|ui| {
            let next_btn =
                egui::Button::new(RichText::new("Next").size(16.0).color(text_color).strong())
                    .fill(btn_color)
                    .corner_radius(25.0)
                    .min_size(egui::vec2(280.0, 50.0));
            if ui.add(next_btn).clicked() && has_amount {
                should_process = true;
            }
        });

        if should_process {
            if let Ok(amount) = self.trade_amount_input.parse::<f64>() {
                // Validate user has enough BTC to sell
                if amount > available_btc_usd {
                    self.trade_error = format!(
                        "Insufficient BTC. You have {} available.",
                        Self::format_price(available_btc_usd)
                    );
                } else {
                    // Calculate trade details
                    let fee_usd = amount * 0.01; // 1% fee
                    let net_amount = amount - fee_usd;
                    let btc_amount = amount / btc_price; // BTC being sold

                    self.pending_trade = Some(PendingTrade {
                        action: TradeAction::SellBtc,
                        amount_usd: amount,
                        btc_price,
                        fee_usd,
                        btc_amount,
                        net_amount_usd: net_amount,
                    });
                    self.show_confirm_trade = true;
                    self.trade_error.clear();
                }
            } else {
                self.trade_error = "Invalid amount".to_string();
            }
        }

        ui.add_space(8.0);

        // Disclaimer
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("Exchange rate may differ slightly at execution.")
                    .size(11.0)
                    .color(Color32::GRAY),
            );
        });

        ui.add_space(8.0);

        // Cancel link
        ui.vertical_centered(|ui| {
            if ui
                .add(egui::Button::new(RichText::new("Cancel").color(Color32::GRAY)).frame(false))
                .clicked()
            {
                self.show_sell_modal = false;
                self.trade_amount_input.clear();
                self.trade_error.clear();
                self.pending_trade = None;
                self.show_confirm_trade = false;
            }
        });
    }

    fn show_sell_confirm_screen(&mut self, ui: &mut egui::Ui) {
        let trade = self.pending_trade.as_ref().unwrap();
        let amount_usd = trade.amount_usd;
        let btc_price = trade.btc_price;
        let fee_usd = trade.fee_usd;
        let btc_amount = trade.btc_amount;
        let net_amount = trade.net_amount_usd;

        // Header
        ui.label(
            RichText::new("Confirm sale")
                .size(28.0)
                .color(Color32::BLACK)
                .strong(),
        );
        ui.add_space(20.0);

        // Order summary
        ui.group(|ui| {
            ui.set_min_width(260.0);

            // You're selling
            ui.horizontal(|ui| {
                ui.label(RichText::new("You're selling").color(Color32::DARK_GRAY));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{:.8} BTC", btc_amount))
                            .color(Color32::BLACK)
                            .strong(),
                    );
                });
            });

            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label(RichText::new("").color(Color32::DARK_GRAY));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("(${:.2})", amount_usd))
                            .color(Color32::GRAY)
                            .size(12.0),
                    );
                });
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            // BTC Price
            ui.horizontal(|ui| {
                ui.label(RichText::new("BTC Price").color(Color32::DARK_GRAY));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("${:.2}", btc_price)).color(Color32::BLACK));
                });
            });

            ui.add_space(8.0);

            // Fee (1%)
            ui.horizontal(|ui| {
                ui.label(RichText::new("Fee (1%)").color(Color32::DARK_GRAY));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("-${:.2}", fee_usd))
                            .color(Color32::from_rgb(244, 63, 94)),
                    );
                });
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            // You receive
            ui.horizontal(|ui| {
                ui.label(RichText::new("You receive").color(Color32::DARK_GRAY));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("${:.2}", net_amount))
                            .color(Color32::BLACK)
                            .strong(),
                    );
                });
            });
        });

        ui.add_space(20.0);

        // Confirm button
        let mut should_confirm = false;
        ui.vertical_centered(|ui| {
            let confirm_btn = egui::Button::new(
                RichText::new("Confirm Sell")
                    .size(16.0)
                    .color(Color32::WHITE)
                    .strong(),
            )
            .fill(Color32::from_rgb(244, 63, 94))
            .corner_radius(25.0)
            .min_size(egui::vec2(280.0, 50.0));
            if ui.add(confirm_btn).clicked() {
                should_confirm = true;
            }
        });
        if should_confirm {
            self.execute_sell(amount_usd);
            self.show_sell_modal = false;
            self.trade_amount_input.clear();
            self.pending_trade = None;
            self.show_confirm_trade = false;
        }

        ui.add_space(10.0);

        // Back button
        ui.vertical_centered(|ui| {
            if ui
                .add(egui::Button::new(RichText::new("Back").color(Color32::GRAY)).frame(false))
                .clicked()
            {
                self.show_confirm_trade = false;
            }
        });
    }

    fn show_close_channel_popup(&mut self, ctx: &egui::Context) {
        let mut clicked_yes = false;
        let mut clicked_cancel = false;

        // Fetch fee rate if not cached
        if self.cached_fee_rate.is_none() {
            self.fetch_fee_rate();
        }

        egui::Window::new("Close Wallet")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .frame(egui::Frame::window(&ctx.style()).fill(Color32::WHITE).corner_radius(16.0))
                .show(ctx, |ui| {
                    ui.label(RichText::new("Are you sure you want to close your wallet?").size(13.0).color(Color32::BLACK));
                    ui.add_space(5.0);
                    ui.label(RichText::new("All funds will be sent to an on-chain address you control. Your trade and payment history will be preserved.").size(12.0).color(Color32::DARK_GRAY));
                    ui.add_space(10.0);

                    // Show balance that will be received
                    let total_sats = self.node.list_balances().total_lightning_balance_sats;
                    if total_sats > 0 {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Balance:").size(12.0).color(Color32::DARK_GRAY));
                            ui.label(RichText::new(format!("{} sats", total_sats)).size(12.0).color(Color32::BLACK).strong());
                        });
                        ui.add_space(5.0);
                    }

                    // Show fee rate
                    let fee_text = match self.cached_fee_rate {
                        Some(rate) => format!("Fee rate: {} sat/vB", rate),
                        None => "Fetching fee rate...".to_string(),
                    };
                    ui.label(RichText::new(fee_text).size(12.0).color(Color32::from_rgb(100, 116, 139)));

                    ui.add_space(15.0);
                    ui.horizontal(|ui| {
                        let yes_btn = egui::Button::new(RichText::new("Close Wallet").color(Color32::WHITE))
                            .fill(Color32::from_rgb(225, 29, 72))
                            .corner_radius(8.0);
                        if ui.add(yes_btn).clicked() {
                            clicked_yes = true;
                        }
                        ui.add_space(8.0);
                        if ui.button("Cancel").clicked() {
                            clicked_cancel = true;
                        }
                    });
                });

        if clicked_yes {
            self.close_active_channel();
            self.confirm_close_popup = false;
            self.cached_fee_rate = None;
            self.fee_rate_receiver = None;
        } else if clicked_cancel {
            self.confirm_close_popup = false;
            self.cached_fee_rate = None;
            self.fee_rate_receiver = None;
        }
    }

    fn fetch_fee_rate(&mut self) {
        // Already cached or already fetching — nothing to do
        if self.cached_fee_rate.is_some() || self.fee_rate_receiver.is_some() {
            return;
        }

        let (tx, rx) = std::sync::mpsc::channel();
        self.fee_rate_receiver = Some(rx);

        std::thread::spawn(move || {
            let agent = ureq::Agent::new();
            let url = format!("{}/fee-estimates", DEFAULT_CHAIN_URL);
            if let Ok(response) = agent.get(&url).call() {
                if let Ok(json) = response.into_json::<serde_json::Value>() {
                    if let Some(fee) = json.get("6").and_then(|v| v.as_f64()) {
                        let _ = tx.send(fee.round() as u64);
                    }
                }
            }
            // If anything fails, tx is dropped and try_recv returns Disconnected
        });
    }

    fn execute_buy(&mut self, amount_usd: f64) {
        let fee_usd = amount_usd * 0.01; // 1% fee
        let net_amount = amount_usd - fee_usd;

        let (current_expected_usd, btc_price) = {
            let sc = self.stable_channel.lock().unwrap();
            (sc.expected_usd.0, sc.latest_price)
        };

        if btc_price < 1.0 {
            self.trade_error = "Invalid price".to_string();
            return;
        }

        // Buying BTC means reducing the stabilized USD amount
        // Subtract full amount (fee comes out of the trade)
        let new_expected_usd = (current_expected_usd - amount_usd).max(0.0);

        if amount_usd > current_expected_usd {
            self.trade_error = "Amount exceeds available USD".to_string();
            return;
        }

        let btc_amount = if btc_price > 0.0 {
            net_amount / btc_price
        } else {
            0.0
        };
        if let Some(payment_id) = self.send_trade(new_expected_usd, fee_usd, "buy") {
            let payment_id_str = format!("{payment_id}");
            let trade_db_id = self.record_trade(
                "buy",
                amount_usd,
                btc_amount,
                fee_usd,
                Some(&payment_id_str),
                "pending",
            );
            self.pending_trade_payments.insert(
                payment_id,
                PendingTradePayment {
                    new_expected_usd,
                    price: btc_price,
                    trade_db_id,
                    action: "buy".to_string(),
                },
            );
            self.show_toast(&format!("Buy ${:.2} BTC pending...", net_amount), "...");
        }
        self.trade_error.clear();
    }

    fn execute_sell(&mut self, amount_usd: f64) {
        let fee_usd = amount_usd * 0.01; // 1% fee
        let net_amount = amount_usd - fee_usd;

        let (current_expected_usd, total_usd, btc_price) = {
            let sc = self.stable_channel.lock().unwrap();
            let total = if sc.is_stable_receiver {
                sc.stable_receiver_usd.0
            } else {
                sc.stable_provider_usd.0
            };
            (sc.expected_usd.0, total, sc.latest_price)
        };

        if btc_price < 1.0 {
            self.trade_error = "Invalid price".to_string();
            return;
        }

        // Can't sell more BTC than you have
        let available_btc_usd = total_usd - current_expected_usd;
        if amount_usd > available_btc_usd {
            self.trade_error = format!(
                "Amount exceeds BTC holdings (${:.2} available)",
                available_btc_usd
            );
            return;
        }

        // Selling BTC means increasing the stabilized USD amount
        // Add net amount (after fee) to stable
        let new_expected_usd = current_expected_usd + net_amount;

        let btc_amount = net_amount / btc_price;
        if let Some(payment_id) = self.send_trade(new_expected_usd, fee_usd, "sell") {
            let payment_id_str = format!("{payment_id}");
            let trade_db_id = self.record_trade(
                "sell",
                amount_usd,
                btc_amount,
                fee_usd,
                Some(&payment_id_str),
                "pending",
            );
            self.pending_trade_payments.insert(
                payment_id,
                PendingTradePayment {
                    new_expected_usd,
                    price: btc_price,
                    trade_db_id,
                    action: "sell".to_string(),
                },
            );
            self.show_toast(&format!("Sell ${:.2} BTC pending...", net_amount), "...");
        }
        self.trade_error.clear();
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

    /// Create a backup zip of the data directory and save to Downloads
    fn create_backup(&mut self) -> Result<String, String> {
        let data_dir = get_user_data_dir();

        // Get downloads directory
        let downloads_dir =
            dirs::download_dir().ok_or_else(|| "Could not find Downloads folder".to_string())?;

        // Create timestamped filename
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let backup_filename = format!("stable_channels_backup_{}.zip", timestamp);
        let backup_path = downloads_dir.join(&backup_filename);

        // Create the zip file
        let file = File::create(&backup_path)
            .map_err(|e| format!("Failed to create backup file: {}", e))?;
        let mut zip = zip::ZipWriter::new(file);

        let options =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        // Walk through data directory and add files
        Self::add_dir_to_zip(&mut zip, &data_dir, &data_dir, &options)?;

        zip.finish()
            .map_err(|e| format!("Failed to finalize zip: {}", e))?;

        Ok(backup_path.to_string_lossy().to_string())
    }

    fn add_dir_to_zip(
        zip: &mut zip::ZipWriter<File>,
        dir: &Path,
        base: &Path,
        options: &zip::write::FileOptions,
    ) -> Result<(), String> {
        if !dir.exists() {
            return Ok(());
        }

        let entries =
            std::fs::read_dir(dir).map_err(|e| format!("Failed to read directory: {}", e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let path = entry.path();
            let name = path
                .strip_prefix(base)
                .map_err(|e| format!("Failed to strip prefix: {}", e))?
                .to_string_lossy()
                .to_string();

            if path.is_dir() {
                // Add directory entry
                zip.add_directory(&name, *options)
                    .map_err(|e| format!("Failed to add directory: {}", e))?;
                // Recursively add contents
                Self::add_dir_to_zip(zip, &path, base, options)?;
            } else {
                // Add file
                zip.start_file(&name, *options)
                    .map_err(|e| format!("Failed to start file: {}", e))?;

                let mut file =
                    File::open(&path).map_err(|e| format!("Failed to open file: {}", e))?;
                let mut buffer = Vec::new();
                file.read_to_end(&mut buffer)
                    .map_err(|e| format!("Failed to read file: {}", e))?;
                zip.write_all(&buffer)
                    .map_err(|e| format!("Failed to write file: {}", e))?;
            }
        }

        Ok(())
    }
}

impl App for UserApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        // Set light/white background
        let mut visuals = egui::Visuals::light();
        visuals.window_fill = egui::Color32::WHITE;
        visuals.panel_fill = egui::Color32::WHITE;
        visuals.widgets.noninteractive.bg_fill = egui::Color32::WHITE;
        ctx.set_visuals(visuals);

        self.process_events();

        // Poll background fee rate fetch
        if let Some(ref rx) = self.fee_rate_receiver {
            match rx.try_recv() {
                Ok(rate) => {
                    self.cached_fee_rate = Some(rate);
                    self.fee_rate_receiver = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // Thread finished without sending (fetch failed)
                    self.fee_rate_receiver = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {} // Still fetching
            }
        }

        // Poll background daily prices fetch
        if let Some(ref rx) = self.daily_prices_receiver {
            match rx.try_recv() {
                Ok(true) => {
                    self.load_chart_data();
                    self.daily_prices_receiver = None;
                }
                Ok(false) => {
                    // Fetch failed
                    self.daily_prices_receiver = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.daily_prices_receiver = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {} // Still fetching
            }
        }

        // Show onboarding only if no channels AND no funds at all (pending or on-chain)
        // Don't override if user navigated to History tab
        if self.current_tab == Tab::Home {
            let balances = self.node.list_balances();
            let has_any_funds = balances.total_lightning_balance_sats > 0
                || !balances.pending_balances_from_channel_closures.is_empty()
                || balances.total_onchain_balance_sats > 0;
            self.show_onboarding =
                self.node.list_channels().is_empty() && !self.waiting_for_payment && !has_any_funds;
        }

        self.start_background_if_needed();

        if self.balance_last_update.elapsed() >= Duration::from_secs(2) {
            self.update_balances();
            self.balance_last_update = std::time::Instant::now();
        }

        // Auto-refresh 1D chart data every 30s so new price points appear in real-time
        if self.chart_period == ChartPeriod::Day1
            && self.chart_last_update.elapsed() >= Duration::from_secs(30)
        {
            self.load_chart_data();
        }

        // Update daily price data periodically (rate-limited internally)
        self.update_daily_prices();

        // Handle trigger_fund_wallet flag from "Fund Your Wallet" button
        if self.trigger_fund_wallet {
            self.trigger_fund_wallet = false;
            let has_channel = self.node.list_channels().iter().any(|c| c.is_channel_ready);
            if has_channel {
                // Channel exists — show on-chain deposit address (auto-sweep handles the rest)
                self.fund_tab = FundTab::Onchain;
                if self.on_chain_address.is_empty() {
                    self.get_address();
                }
                if self.onchain_qr_texture.is_none() && !self.on_chain_address.is_empty() {
                    self.onchain_qr_texture =
                        Self::generate_qr_texture(&self.on_chain_address, ctx, "onchain_qr");
                }
                self.waiting_for_payment = true;
            } else {
                // No channel — JIT invoice to open one
                println!("[DEBUG] Fund wallet triggered, calling get_jit_invoice");
                self.status_message = "Getting JIT channel invoice...".to_string();
                self.get_jit_invoice(ctx);
            }
        }

        // Show the appropriate base screen
        if self.show_onboarding {
            self.show_onboarding_screen(ctx);
        } else if self.is_syncing {
            self.show_syncing_screen(ctx);
        } else {
            self.show_main_screen(ctx);
        }

        // Modals - show on top of any screen
        if self.show_transfer_modal {
            self.show_transfer_modal_ui(ctx);
            if self.check_click_outside_modal(ctx) {
                self.show_transfer_modal = false;
            }
        }
        if self.show_buy_modal {
            self.show_buy_modal_ui(ctx);
            if self.check_click_outside_modal(ctx) {
                self.show_buy_modal = false;
                self.show_confirm_trade = false;
                self.pending_trade = None;
                self.trade_amount_input.clear();
                self.trade_error.clear();
            }
        }
        if self.show_sell_modal {
            self.show_sell_modal_ui(ctx);
            if self.check_click_outside_modal(ctx) {
                self.show_sell_modal = false;
                self.show_confirm_trade = false;
                self.pending_trade = None;
                self.trade_amount_input.clear();
                self.trade_error.clear();
            }
        }

        // Detail modals for history rows
        if self.selected_trade.is_some() {
            self.show_trade_detail_modal(ctx);
            if self.check_click_outside_modal(ctx) {
                self.selected_trade = None;
            }
        }
        if self.selected_payment.is_some() {
            self.show_payment_detail_modal(ctx);
            if self.check_click_outside_modal(ctx) {
                self.selected_payment = None;
            }
        }

        // Show payment modal on top if waiting for payment
        if self.waiting_for_payment {
            self.show_waiting_for_payment_modal(ctx);
        }
        self.show_log_window_if_open(ctx);

        // Render toast notifications on top
        self.render_toasts(ctx);

        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

pub fn run() {
    println!("Starting User Interface...");
    // Load app icon from embedded PNG
    let icon_data = {
        let icon_bytes = include_bytes!("../photos/sc-icon-egui.png");
        let img = image::load_from_memory(icon_bytes).expect("Failed to load icon");
        let rgba = img.into_rgba8();
        let (w, h) = rgba.dimensions();
        egui::IconData {
            rgba: rgba.into_raw(),
            width: w,
            height: h,
        }
    };

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([460.0, 700.0])
            .with_decorations(true)
            .with_transparent(false)
            .with_icon(std::sync::Arc::new(icon_data)),
        ..Default::default()
    };

    let app_result = UserApp::new();
    match app_result {
        Ok(app) => {
            eframe::run_native(
                "Stable Channels Wallet",
                native_options,
                Box::new(|cc| {
                    // Set light theme with white background
                    let mut visuals = egui::Visuals::light();
                    visuals.window_fill = egui::Color32::WHITE;
                    visuals.panel_fill = egui::Color32::WHITE;
                    cc.egui_ctx.set_visuals(visuals);

                    Ok(Box::new(app))
                }),
            )
            .unwrap();
        }
        Err(e) => {
            eprintln!("Failed to initialize app: {}", e);
            std::process::exit(1);
        }
    }
}
