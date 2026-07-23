//! Mac desktop E2E test + demo harness (debug builds only).
//!
//! Extracted verbatim from `user.rs`. This is a *child* module of `user`, so it
//! reaches `UserApp`'s private fields and methods directly through
//! `use super::*;` — the same visibility rule that lets `#[cfg(test)] mod tests`
//! touch a module's privates. No `pub(crate)` widening of the production
//! `UserApp` struct is required.
//!
//! `user.rs` gates `mod mac_e2e;` on `#[cfg(debug_assertions)]`, so this whole
//! module is absent from release builds; the per-item `#[cfg(debug_assertions)]`
//! attributes carried over from `user.rs` are therefore redundant but harmless.

use super::*;

#[cfg(debug_assertions)]
pub fn run_mac_flows() -> Result<(), String> {
    let config = load_desktop_runtime_config()
        .map_err(|err| format!("Invalid Mac flow config: {err}"))?;
    if config.network != "regtest" {
        return Err(format!("Mac flows expected regtest, got {}", config.network));
    }

    let harness = MacFlowHarness::new();
    harness.set_price(100_000.0)?;
    stable_channels::price_feeds::set_cached_price(100_000.0);

    let app = UserApp::new()?;
    let mut runner = MacFlowRunner {
        app,
        ctx: egui::Context::default(),
        harness,
        price_usd: 100_000.0,
    };
    runner.run_all()?;
    let _ = runner.app.node.stop();
    Ok(())
}

#[cfg(debug_assertions)]
struct MacFlowHarness {
    api: String,
    agent: ureq::Agent,
}

#[cfg(debug_assertions)]
impl MacFlowHarness {
    fn new() -> Self {
        let api = std::env::var("SC_HARNESS_API")
            .or_else(|_| std::env::var("HARNESS_API"))
            .unwrap_or_else(|_| "http://localhost:9737".to_string())
            .trim_end_matches('/')
            .to_string();
        Self::with_api(api)
    }

    fn with_api(api: String) -> Self {
        Self {
            api,
            agent: ureq::Agent::new(),
        }
    }

    fn get(&self, path: &str) -> Result<serde_json::Value, String> {
        let url = format!("{}{}", self.api, path);
        self.agent
            .get(&url)
            .call()
            .map_err(|err| format!("GET {url} failed: {err}"))?
            .into_json::<serde_json::Value>()
            .map_err(|err| format!("GET {url} returned invalid JSON: {err}"))
    }

    fn post(&self, path: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
        let url = format!("{}{}", self.api, path);
        self.agent
            .post(&url)
            .send_json(body)
            .map_err(|err| format!("POST {url} failed: {err}"))?
            .into_json::<serde_json::Value>()
            .map_err(|err| format!("POST {url} returned invalid JSON: {err}"))
    }

    fn set_price(&self, price: f64) -> Result<(), String> {
        self.post("/price", json!({ "price": price }))?;
        Ok(())
    }

    fn pay_invoice(&self, invoice: &str) -> Result<(), String> {
        self.post("/pay", json!({ "invoice": invoice }))?;
        Ok(())
    }

    fn invoice(&self, amount_msat: u64) -> Result<String, String> {
        let response = self.post("/invoice", json!({ "amount_msat": amount_msat }))?;
        response["invoice"]
            .as_str()
            .map(ToOwned::to_owned)
            .ok_or_else(|| "harness /invoice response did not include invoice".to_string())
    }

    fn address(&self) -> Result<String, String> {
        let response = self.post("/address", json!({}))?;
        response["address"]
            .as_str()
            .map(ToOwned::to_owned)
            .ok_or_else(|| "harness /address response did not include address".to_string())
    }

    fn send_onchain(&self, address: &str, amount_sats: u64) -> Result<(), String> {
        self.post(
            "/send",
            json!({
                "address": address,
                "amount_sats": amount_sats,
            }),
        )?;
        Ok(())
    }

    fn mine(&self, blocks: u64) -> Result<(), String> {
        self.post("/mine", json!({ "blocks": blocks }))?;
        Ok(())
    }

    fn audit_tail_contains_after(&self, after: &str, events: &[&str]) -> Result<bool, String> {
        let response = self.get("/audit-tail?n=100")?;
        let Some(lines) = response["lines"].as_array() else {
            return Ok(false);
        };

        for line in lines {
            let Some(line) = line.as_str() else {
                continue;
            };
            let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(name) = event["event"].as_str() else {
                continue;
            };
            let Some(ts) = event["ts"].as_str() else {
                continue;
            };
            if ts > after && events.iter().any(|candidate| candidate == &name) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(debug_assertions)]
struct MacFlowRunner {
    app: UserApp,
    ctx: egui::Context,
    harness: MacFlowHarness,
    price_usd: f64,
}

#[cfg(debug_assertions)]
impl MacFlowRunner {
    fn run_all(&mut self) -> Result<(), String> {
        self.set_price(100_000.0)?;
        self.pump()?;

        self.flow("01_onboard_lightning", "onboard over Lightning", |runner| {
            runner.flow_onboard_lightning()
        })?;
        self.flow("02_btc_to_usd", "convert BTC exposure to USD", |runner| {
            runner.flow_btc_to_usd()
        })?;
        self.flow("03_usd_stability", "settle after a price move", |runner| {
            runner.flow_usd_stability()
        })?;
        self.flow("04_lightning_receive", "receive a Lightning invoice", |runner| {
            runner.flow_lightning_receive()
        })?;
        self.flow("05_onchain_receive", "receive onchain and splice in", |runner| {
            runner.flow_onchain_receive()
        })?;
        self.flow("06_lightning_send", "send a regtest Lightning invoice", |runner| {
            runner.flow_lightning_send()
        })?;
        self.flow("07_onchain_send", "send a regtest onchain address", |runner| {
            runner.flow_onchain_send()
        })?;
        self.flow("08_usd_to_btc", "convert USD exposure to BTC", |runner| {
            runner.flow_usd_to_btc()
        })?;
        self.flow("09_close_channel", "close the channel", |runner| {
            runner.flow_close_channel()
        })?;
        self.flow("10_backup_keys", "reveal backup seed words", |runner| {
            runner.flow_backup_keys()
        })?;
        self.flow("11_import_keys", "restart from saved seed state", |runner| {
            runner.flow_import_keys()
        })?;
        self.flow("12_offboard_onchain", "offboard remaining onchain funds", |runner| {
            runner.flow_offboard_onchain()
        })?;

        println!("\nMac flows passed");
        Ok(())
    }

    fn flow<F>(&mut self, id: &str, title: &str, f: F) -> Result<(), String>
    where
        F: FnOnce(&mut Self) -> Result<(), String>,
    {
        let started = std::time::Instant::now();
        println!("\n▶ {id} — {title}");
        f(self).map_err(|err| format!("{id} failed: {err}"))?;
        println!("✓ {id} ({:.1}s)", started.elapsed().as_secs_f64());
        Ok(())
    }

    fn flow_onboard_lightning(&mut self) -> Result<(), String> {
        let amount_sats = self.sats_for_usd(85.0);
        self.app
            .generate_jit_ln_invoice(&self.ctx, Some(amount_sats));
        if !self.app.lightning_receive_error.is_empty() {
            return Err(self.app.lightning_receive_error.clone());
        }
        let invoice = self.require_invoice(&self.app.lightning_receive_invoice)?;
        self.harness.pay_invoice(&invoice)?;
        self.wait_until("ready channel after JIT payment", Duration::from_secs(180), |runner| {
            Ok(runner.ready_channel_count() > 0 && runner.lightning_sats() > 0)
        })?;
        self.ensure_stable_channel_has_price();
        Ok(())
    }

    fn flow_btc_to_usd(&mut self) -> Result<(), String> {
        let before = self.expected_usd();
        let pending_before = self.app.pending_trade_payments.len();
        self.app.execute_sell(75.0);
        if self.app.pending_trade_payments.len() == pending_before {
            return Err(format!(
                "sell order was not submitted: status='{}' error='{}'",
                self.app.status_message, self.app.trade_error
            ));
        }
        self.wait_until("sell order confirmed", Duration::from_secs(60), |runner| {
            Ok(runner.expected_usd() > before + 70.0)
        })
    }

    fn flow_usd_stability(&mut self) -> Result<(), String> {
        let marker = chrono::Utc::now().to_rfc3339();
        self.harness.set_price(101_000.0)?;
        self.set_price(101_000.0)?;

        let payment_info = {
            let mut sc = self.app.stable_channel.lock().unwrap();
            stable::check_stability(&self.app.node, &mut sc, self.price_usd)
        };
        let Some(payment_info) = payment_info else {
            return Err("stability check did not send a settlement payment".to_string());
        };

        let amount_usd = (payment_info.amount_msat as f64 / 1000.0 / 100_000_000.0)
            * payment_info.btc_price;
        let _ = self.app.db.record_payment(
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
        self.app.save_channel_settings();

        self.wait_until("LSP observed settlement", Duration::from_secs(200), |runner| {
            runner
                .harness
                .audit_tail_contains_after(&marker, &["PAYMENT_RECEIVED", "MESSAGE_RECEIVED"])
        })?;

        self.harness.set_price(100_000.0)?;
        self.set_price(100_000.0)?;
        Ok(())
    }

    fn flow_lightning_receive(&mut self) -> Result<(), String> {
        let before = self.lightning_sats();
        self.app.lightning_receive_amount = self.btc_string_for_usd(10.0);
        self.app.generate_lightning_receive_invoice(&self.ctx);
        let invoice = self.require_invoice(&self.app.lightning_receive_invoice)?;
        self.harness.pay_invoice(&invoice)?;
        self.wait_until("Lightning receive settled", Duration::from_secs(90), |runner| {
            Ok(runner.lightning_sats() > before)
        })
    }

    fn flow_onchain_receive(&mut self) -> Result<(), String> {
        let before_lightning = self.lightning_sats();
        if !self.app.get_address() {
            return Err(format!("address generation failed: {}", self.app.status_message));
        }
        let address = self.app.on_chain_address.clone();
        if !address.starts_with("bcrt1") {
            return Err(format!("expected regtest address, got {address}"));
        }
        self.harness.send_onchain(&address, 100_000)?;
        self.harness.mine(6)?;
        self.wait_until("onchain deposit spendable", Duration::from_secs(120), |runner| {
            Ok(runner.spendable_onchain_sats() > 0)
        })?;
        self.app.splice_to_channel();
        self.wait_until("splice-in negotiated", Duration::from_secs(90), |runner| {
            Ok(runner
                .app
                .auto_splice_in_progress
                .load(std::sync::atomic::Ordering::Relaxed))
        })?;
        self.harness.mine(6)?;
        self.wait_until("splice-in confirmed", Duration::from_secs(180), |runner| {
            Ok(runner.lightning_sats() > before_lightning
                && !runner
                    .app
                    .auto_splice_in_progress
                    .load(std::sync::atomic::Ordering::Relaxed))
        })
    }

    fn flow_lightning_send(&mut self) -> Result<(), String> {
        let invoice = self.harness.invoice(5_000_000)?;
        if !invoice.starts_with("lnbcrt") {
            return Err(format!("expected lnbcrt invoice, got {invoice}"));
        }
        let before = self.outbound_success_count();
        self.app.send_input = invoice;
        self.app.send_amount.clear();
        if !self.app.send_unified() {
            return Err(format!("Lightning send failed to start: {}", self.app.send_error));
        }
        self.wait_until("Lightning send confirmed", Duration::from_secs(90), |runner| {
            Ok(runner.outbound_success_count() > before)
        })
    }

    fn flow_onchain_send(&mut self) -> Result<(), String> {
        let address = self.harness.address()?;
        if !address.starts_with("bcrt1") {
            return Err(format!("expected bcrt1 address, got {address}"));
        }
        self.app.send_input = address;
        self.app.send_amount = self.btc_string_for_usd(5.0);
        self.app.send_all = false;
        if !self.app.send_unified() {
            return Err(format!("onchain send failed to start: {}", self.app.send_error));
        }
        self.wait_until("splice-out negotiated", Duration::from_secs(90), |runner| {
            Ok(runner
                .app
                .auto_splice_in_progress
                .load(std::sync::atomic::Ordering::Relaxed))
        })?;
        self.harness.mine(6)?;
        self.wait_until("splice-out confirmed", Duration::from_secs(180), |runner| {
            Ok(!runner
                .app
                .auto_splice_in_progress
                .load(std::sync::atomic::Ordering::Relaxed))
        })
    }

    fn flow_usd_to_btc(&mut self) -> Result<(), String> {
        let before = self.expected_usd();
        let pending_before = self.app.pending_trade_payments.len();
        self.app.execute_buy(20.0);
        if self.app.pending_trade_payments.len() == pending_before {
            return Err(format!(
                "buy order was not submitted: status='{}' error='{}'",
                self.app.status_message, self.app.trade_error
            ));
        }
        self.wait_until("buy order confirmed", Duration::from_secs(60), |runner| {
            Ok(runner.expected_usd() < before - 15.0)
        })
    }

    fn flow_close_channel(&mut self) -> Result<(), String> {
        self.app.close_active_channel();
        if !self.app.status_message.contains("Closing") {
            return Err(format!("close did not start: {}", self.app.status_message));
        }
        self.wait_until("channel close started", Duration::from_secs(60), |runner| {
            Ok(runner.ready_channel_count() == 0)
        })?;
        self.harness.mine(6)?;
        self.wait_until("closed channel funds spendable", Duration::from_secs(180), |runner| {
            Ok(runner.ready_channel_count() == 0 && runner.spendable_onchain_sats() > 0)
        })
    }

    fn flow_backup_keys(&mut self) -> Result<(), String> {
        let words = self.seed_words()?;
        let word_count = words.split_whitespace().count();
        if ![12, 15, 18, 21, 24].contains(&word_count) {
            return Err(format!("unexpected BIP39 seed word count: {word_count}"));
        }
        ldk_node::bip39::Mnemonic::from_str(&words)
            .map_err(|err| format!("saved seed phrase is invalid: {err}"))?;
        Ok(())
    }

    fn flow_import_keys(&mut self) -> Result<(), String> {
        let seed_words = self.seed_words()?;
        let node_id_before = self.app.node.node_id().to_string();
        self.app.node.stop().map_err(|err| format!("node stop failed: {err}"))?;
        self.app = UserApp::new()?;
        self.set_price(self.price_usd)?;
        self.pump()?;
        let restored_words = self.seed_words()?;
        if restored_words != seed_words {
            return Err("restored wallet did not load the same seed words".to_string());
        }
        let node_id_after = self.app.node.node_id().to_string();
        if node_id_after != node_id_before {
            return Err(format!(
                "restored node id changed: before={node_id_before} after={node_id_after}"
            ));
        }
        Ok(())
    }

    fn flow_offboard_onchain(&mut self) -> Result<(), String> {
        self.harness.mine(6)?;
        self.wait_until("offboard funds spendable", Duration::from_secs(120), |runner| {
            Ok(runner.spendable_onchain_sats() > 0)
        })?;
        let before = self.spendable_onchain_sats();
        let address = self.harness.address()?;
        self.app.send_input = address;
        self.app.send_all = true;
        self.app.send_amount.clear();
        if !self.app.send_unified() {
            return Err(format!("send-all failed to start: {}", self.app.send_error));
        }
        self.harness.mine(6)?;
        self.wait_until("offboard spend confirmed", Duration::from_secs(120), |runner| {
            Ok(runner.spendable_onchain_sats() < before.saturating_div(2))
        })
    }

    fn set_price(&mut self, price: f64) -> Result<(), String> {
        self.price_usd = price;
        stable_channels::price_feeds::set_cached_price(price);
        {
            let mut sc = self.app.stable_channel.lock().unwrap();
            sc.latest_price = price;
            sc.timestamp = Self::unix_time();
        }
        self.app.btc_price = price;
        self.pump()
    }

    fn pump(&mut self) -> Result<(), String> {
        self.app
            .node
            .sync_wallets()
            .map_err(|err| format!("wallet sync failed: {err}"))?;
        self.app.process_events();
        {
            let mut sc = self.app.stable_channel.lock().unwrap();
            stable::update_balances(&self.app.node, &mut sc);
            sc.latest_price = self.price_usd;
            sc.timestamp = Self::unix_time();
        }
        self.app.update_balances();
        Ok(())
    }

    fn wait_until<F>(&mut self, label: &str, timeout: Duration, mut f: F) -> Result<(), String>
    where
        F: FnMut(&mut Self) -> Result<bool, String>,
    {
        let started = std::time::Instant::now();
        let mut last_error: Option<String> = None;
        while started.elapsed() < timeout {
            if let Err(err) = self.pump() {
                last_error = Some(err);
            } else {
                match f(self) {
                    Ok(true) => return Ok(()),
                    Ok(false) => {}
                    Err(err) => last_error = Some(err),
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }

        let balances = self.app.node.list_balances();
        let channels = self.app.node.list_channels();
        Err(format!(
            "timed out waiting for {label}; status='{}' send_error='{}' lightning_sats={} spendable_onchain_sats={} ready_channels={} last_error={}",
            self.app.status_message,
            self.app.send_error,
            balances.total_lightning_balance_sats,
            balances.spendable_onchain_balance_sats,
            channels.iter().filter(|channel| channel.is_channel_ready).count(),
            last_error.unwrap_or_else(|| "none".to_string())
        ))
    }

    fn ensure_stable_channel_has_price(&mut self) {
        let mut sc = self.app.stable_channel.lock().unwrap();
        sc.latest_price = self.price_usd;
        if sc.user_channel_id == 0 {
            if let Some(channel) = self.app.node.list_channels().first() {
                sc.user_channel_id = channel.user_channel_id.0;
                sc.channel_id = channel.channel_id;
            }
        }
        stable::update_balances(&self.app.node, &mut sc);
        sc.latest_price = self.price_usd;
        sc.timestamp = Self::unix_time();
    }

    fn require_invoice(&self, invoice: &str) -> Result<String, String> {
        if invoice.starts_with("lnbcrt") {
            Ok(invoice.to_string())
        } else if invoice.is_empty() {
            Err("invoice was empty".to_string())
        } else {
            Err(format!("expected lnbcrt invoice, got {invoice}"))
        }
    }

    fn seed_words(&self) -> Result<String, String> {
        self.app
            .saved_mnemonic
            .clone()
            .filter(|words| !words.trim().is_empty())
            .ok_or_else(|| "no saved seed words available".to_string())
    }

    fn expected_usd(&self) -> f64 {
        self.app.stable_channel.lock().unwrap().expected_usd.0
    }

    fn lightning_sats(&self) -> u64 {
        self.app.node.list_balances().total_lightning_balance_sats
    }

    fn spendable_onchain_sats(&self) -> u64 {
        self.app.node.list_balances().spendable_onchain_balance_sats
    }

    fn ready_channel_count(&self) -> usize {
        self.app
            .node
            .list_channels()
            .iter()
            .filter(|channel| channel.is_channel_ready)
            .count()
    }

    fn outbound_success_count(&self) -> usize {
        self.app
            .node
            .list_payments()
            .iter()
            .filter(|payment| {
                payment.direction == PaymentDirection::Outbound
                    && payment.status == PaymentStatus::Succeeded
            })
            .count()
    }

    fn sats_for_usd(&self, usd: f64) -> u64 {
        ((usd / self.price_usd) * 100_000_000.0).round() as u64
    }

    fn btc_string_for_usd(&self, usd: f64) -> String {
        format!("{:.8}", self.sats_for_usd(usd) as f64 / 100_000_000.0)
    }

    fn unix_time() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0)
    }
}

#[cfg(debug_assertions)]
pub(crate) fn mac_demo_enabled() -> bool {
    std::env::var("SC_MAC_DEMO").is_ok_and(|value| value == "1")
}

#[cfg(debug_assertions)]
fn mac_demo_pause_ms() -> u64 {
    std::env::var("SC_MAC_DEMO_PAUSE_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value.clamp(250, 10_000))
        .unwrap_or(1_800)
}

#[cfg(debug_assertions)]
struct MacDemoTask {
    label: String,
    receiver: std::sync::mpsc::Receiver<Result<serde_json::Value, String>>,
}

#[cfg(debug_assertions)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MacDemoStep {
    Prepare,
    PrepareWait,
    OnboardGenerate,
    OnboardPayWait,
    OnboardReadyWait,
    SellStart,
    SellWait,
    StabilitySetPrice,
    StabilitySetPriceWait,
    StabilityAuditWait,
    StabilityResetPriceWait,
    LightningReceiveGenerate,
    LightningReceivePayWait,
    LightningReceiveSettleWait,
    OnchainReceiveStart,
    OnchainReceiveFundWait,
    OnchainReceiveDepositWait,
    OnchainReceiveSpliceWait,
    OnchainReceiveMineWait,
    OnchainReceiveConfirmWait,
    LightningSendInvoice,
    LightningSendInvoiceWait,
    LightningSendStart,
    LightningSendWait,
    OnchainSendAddress,
    OnchainSendAddressWait,
    OnchainSendStart,
    OnchainSendSpliceWait,
    OnchainSendMineWait,
    OnchainSendConfirmWait,
    BuyStart,
    BuyWait,
    CloseStart,
    CloseStartWait,
    CloseMineWait,
    CloseConfirmWait,
    BackupCheck,
    ImportRestart,
    OffboardMineWait,
    OffboardFundsWait,
    OffboardAddressWait,
    OffboardSend,
    OffboardMineConfirmWait,
    OffboardConfirmWait,
    Done,
}

#[cfg(debug_assertions)]
pub(crate) struct MacDemoController {
    harness: MacFlowHarness,
    step: MacDemoStep,
    current_flow: String,
    action: String,
    detail: String,
    completed: Vec<&'static str>,
    started_at: std::time::Instant,
    step_started: std::time::Instant,
    pause_until: Option<std::time::Instant>,
    pause_ms: u64,
    pending_task: Option<MacDemoTask>,
    failed: Option<String>,
    finished: bool,
    price_usd: f64,
    last_pump: std::time::Instant,
    last_poll: std::time::Instant,
    before_lightning_sats: u64,
    before_spendable_sats: u64,
    before_expected_usd: f64,
    before_outbound_success: usize,
    invoice: String,
    address: String,
    settlement_marker: String,
    seed_words: String,
    node_id_before: String,
}

#[cfg(debug_assertions)]
impl MacDemoController {
    const FLOW_COUNT: usize = 12;

    pub(crate) fn new() -> Self {
        let now = std::time::Instant::now();
        Self {
            harness: MacFlowHarness::new(),
            step: MacDemoStep::Prepare,
            current_flow: "Preparing".to_string(),
            action: "Starting regtest demo".to_string(),
            detail: "Waiting for the local harness".to_string(),
            completed: Vec::new(),
            started_at: now,
            step_started: now,
            pause_until: None,
            pause_ms: mac_demo_pause_ms(),
            pending_task: None,
            failed: None,
            finished: false,
            price_usd: 100_000.0,
            last_pump: now
                .checked_sub(Duration::from_secs(10))
                .unwrap_or(now),
            last_poll: now
                .checked_sub(Duration::from_secs(10))
                .unwrap_or(now),
            before_lightning_sats: 0,
            before_spendable_sats: 0,
            before_expected_usd: 0.0,
            before_outbound_success: 0,
            invoice: String::new(),
            address: String::new(),
            settlement_marker: String::new(),
            seed_words: String::new(),
            node_id_before: String::new(),
        }
    }

    pub(crate) fn tick(&mut self, app: &mut UserApp, ctx: &egui::Context) {
        if self.failed.is_some() || self.finished {
            return;
        }

        if self.last_pump.elapsed() >= Duration::from_millis(350) {
            if let Err(err) = self.pump(app) {
                self.fail(format!("wallet sync failed: {err}"));
                return;
            }
        }

        if self.pause_until.is_some_and(|until| std::time::Instant::now() < until) {
            return;
        }
        self.pause_until = None;

        match self.step {
            MacDemoStep::Prepare => {
                self.current_flow = "Prepare".to_string();
                self.action = "Pinning regtest price".to_string();
                self.start_task("Set mock BTC/USD price to $100,000", |harness| {
                    harness.set_price(100_000.0)?;
                    Ok(json!({}))
                });
                self.enter(MacDemoStep::PrepareWait);
            }
            MacDemoStep::PrepareWait => {
                if self.take_task_result().is_some() {
                    self.set_app_price(app, 100_000.0);
                    self.enter_flow(
                        MacDemoStep::OnboardGenerate,
                        "01 Onboard over Lightning",
                        "Generating a JIT invoice",
                    );
                }
            }
            MacDemoStep::OnboardGenerate => {
                self.before_lightning_sats = self.lightning_sats(app);
                let amount_sats = self.sats_for_usd(85.0);
                app.generate_jit_ln_invoice(ctx, Some(amount_sats));
                if !app.lightning_receive_error.is_empty() {
                    self.fail(app.lightning_receive_error.clone());
                    return;
                }
                self.invoice = match self.require_invoice(&app.lightning_receive_invoice) {
                    Ok(invoice) => invoice,
                    Err(err) => {
                        self.fail(err);
                        return;
                    }
                };
                let invoice = self.invoice.clone();
                self.start_task("Harness is paying the JIT invoice", move |harness| {
                    harness.pay_invoice(&invoice)?;
                    Ok(json!({}))
                });
                self.enter(MacDemoStep::OnboardPayWait);
            }
            MacDemoStep::OnboardPayWait => {
                if self.take_task_result().is_some() {
                    self.action = "Waiting for the channel to become ready".to_string();
                    self.enter(MacDemoStep::OnboardReadyWait);
                }
            }
            MacDemoStep::OnboardReadyWait => {
                if self.wait_for(app, "ready JIT channel", Duration::from_secs(180), |demo, app| {
                    demo.ready_channel_count(app) > 0 && demo.lightning_sats(app) > 0
                }) {
                    self.ensure_stable_channel_has_price(app);
                    self.complete_flow("01_onboard_lightning");
                    self.enter_flow(
                        MacDemoStep::SellStart,
                        "02 BTC to USD",
                        "Submitting BTC to USD order",
                    );
                }
            }
            MacDemoStep::SellStart => {
                self.before_expected_usd = self.expected_usd(app);
                let pending_before = app.pending_trade_payments.len();
                app.execute_sell(75.0);
                if app.pending_trade_payments.len() == pending_before {
                    self.fail(format!(
                        "sell order was not submitted: status='{}' error='{}'",
                        app.status_message, app.trade_error
                    ));
                    return;
                }
                self.enter(MacDemoStep::SellWait);
            }
            MacDemoStep::SellWait => {
                if self.wait_for(app, "sell order confirmation", Duration::from_secs(60), |demo, app| {
                    demo.expected_usd(app) > demo.before_expected_usd + 70.0
                }) {
                    self.complete_flow("02_btc_to_usd");
                    self.enter_flow(
                        MacDemoStep::StabilitySetPrice,
                        "03 USD Stability",
                        "Moving price up 1%",
                    );
                }
            }
            MacDemoStep::StabilitySetPrice => {
                self.settlement_marker = chrono::Utc::now().to_rfc3339();
                self.start_task("Set mock BTC/USD price to $101,000", |harness| {
                    harness.set_price(101_000.0)?;
                    Ok(json!({}))
                });
                self.enter(MacDemoStep::StabilitySetPriceWait);
            }
            MacDemoStep::StabilitySetPriceWait => {
                if self.take_task_result().is_some() {
                    self.set_app_price(app, 101_000.0);
                    let payment_info = {
                        let mut sc = app.stable_channel.lock().unwrap();
                        stable::check_stability(&app.node, &mut sc, self.price_usd)
                    };
                    let Some(payment_info) = payment_info else {
                        self.fail("stability check did not send a settlement payment".to_string());
                        return;
                    };
                    let amount_usd =
                        (payment_info.amount_msat as f64 / 1000.0 / 100_000_000.0)
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
                    app.save_channel_settings();
                    self.action = "Waiting for LSP settlement audit".to_string();
                    self.last_poll = std::time::Instant::now()
                        .checked_sub(Duration::from_secs(10))
                        .unwrap_or_else(std::time::Instant::now);
                    self.enter(MacDemoStep::StabilityAuditWait);
                }
            }
            MacDemoStep::StabilityAuditWait => {
                if self.step_started.elapsed() > Duration::from_secs(200) {
                    self.fail("timed out waiting for LSP settlement audit".to_string());
                    return;
                }
                if self.last_poll.elapsed() >= Duration::from_secs(2) {
                    self.last_poll = std::time::Instant::now();
                    match self.harness.audit_tail_contains_after(
                        &self.settlement_marker,
                        &["PAYMENT_RECEIVED", "MESSAGE_RECEIVED"],
                    ) {
                        Ok(true) => {
                            self.start_task("Reset mock BTC/USD price to $100,000", |harness| {
                                harness.set_price(100_000.0)?;
                                Ok(json!({}))
                            });
                            self.enter(MacDemoStep::StabilityResetPriceWait);
                        }
                        Ok(false) => {
                            self.detail = "Polling LSP audit log".to_string();
                        }
                        Err(err) => self.fail(err),
                    }
                }
            }
            MacDemoStep::StabilityResetPriceWait => {
                if self.take_task_result().is_some() {
                    self.set_app_price(app, 100_000.0);
                    self.complete_flow("03_usd_stability");
                    self.enter_flow(
                        MacDemoStep::LightningReceiveGenerate,
                        "04 Lightning Receive",
                        "Generating a Lightning invoice",
                    );
                }
            }
            MacDemoStep::LightningReceiveGenerate => {
                self.before_lightning_sats = self.lightning_sats(app);
                app.lightning_receive_amount = self.btc_string_for_usd(10.0);
                app.generate_lightning_receive_invoice(ctx);
                self.invoice = match self.require_invoice(&app.lightning_receive_invoice) {
                    Ok(invoice) => invoice,
                    Err(err) => {
                        self.fail(err);
                        return;
                    }
                };
                let invoice = self.invoice.clone();
                self.start_task("Harness is paying the Lightning invoice", move |harness| {
                    harness.pay_invoice(&invoice)?;
                    Ok(json!({}))
                });
                self.enter(MacDemoStep::LightningReceivePayWait);
            }
            MacDemoStep::LightningReceivePayWait => {
                if self.take_task_result().is_some() {
                    self.action = "Waiting for received balance".to_string();
                    self.enter(MacDemoStep::LightningReceiveSettleWait);
                }
            }
            MacDemoStep::LightningReceiveSettleWait => {
                if self.wait_for(app, "Lightning receive settlement", Duration::from_secs(90), |demo, app| {
                    demo.lightning_sats(app) > demo.before_lightning_sats
                }) {
                    self.complete_flow("04_lightning_receive");
                    self.enter_flow(
                        MacDemoStep::OnchainReceiveStart,
                        "05 Onchain Receive",
                        "Generating an onchain address",
                    );
                }
            }
            MacDemoStep::OnchainReceiveStart => {
                self.before_lightning_sats = self.lightning_sats(app);
                if !app.get_address() {
                    self.fail(format!("address generation failed: {}", app.status_message));
                    return;
                }
                self.address = app.on_chain_address.clone();
                if !self.address.starts_with("bcrt1") {
                    self.fail(format!("expected regtest address, got {}", self.address));
                    return;
                }
                let address = self.address.clone();
                self.start_task("Harness sends 100,000 sats and mines 6 blocks", move |harness| {
                    harness.send_onchain(&address, 100_000)?;
                    harness.mine(6)?;
                    Ok(json!({}))
                });
                self.enter(MacDemoStep::OnchainReceiveFundWait);
            }
            MacDemoStep::OnchainReceiveFundWait => {
                if self.take_task_result().is_some() {
                    self.action = "Waiting for spendable onchain balance".to_string();
                    self.enter(MacDemoStep::OnchainReceiveDepositWait);
                }
            }
            MacDemoStep::OnchainReceiveDepositWait => {
                if self.wait_for(app, "onchain deposit", Duration::from_secs(120), |demo, app| {
                    demo.spendable_onchain_sats(app) > 0
                }) {
                    app.splice_to_channel();
                    self.action = "Waiting for splice-in negotiation".to_string();
                    self.enter(MacDemoStep::OnchainReceiveSpliceWait);
                }
            }
            MacDemoStep::OnchainReceiveSpliceWait => {
                if self.wait_for(app, "splice-in negotiation", Duration::from_secs(90), |_demo, app| {
                    app.auto_splice_in_progress
                        .load(std::sync::atomic::Ordering::Relaxed)
                }) {
                    self.start_task("Mining splice-in confirmation", |harness| {
                        harness.mine(6)?;
                        Ok(json!({}))
                    });
                    self.enter(MacDemoStep::OnchainReceiveMineWait);
                }
            }
            MacDemoStep::OnchainReceiveMineWait => {
                if self.take_task_result().is_some() {
                    self.action = "Waiting for splice-in to clear".to_string();
                    self.enter(MacDemoStep::OnchainReceiveConfirmWait);
                }
            }
            MacDemoStep::OnchainReceiveConfirmWait => {
                if self.wait_for(app, "splice-in confirmation", Duration::from_secs(180), |demo, app| {
                    demo.lightning_sats(app) > demo.before_lightning_sats
                        && !app
                            .auto_splice_in_progress
                            .load(std::sync::atomic::Ordering::Relaxed)
                }) {
                    self.complete_flow("05_onchain_receive");
                    self.enter_flow(
                        MacDemoStep::LightningSendInvoice,
                        "06 Lightning Send",
                        "Requesting a regtest invoice",
                    );
                }
            }
            MacDemoStep::LightningSendInvoice => {
                self.start_task("Harness creates a 5,000,000 msat invoice", |harness| {
                    let invoice = harness.invoice(5_000_000)?;
                    Ok(json!({ "invoice": invoice }))
                });
                self.enter(MacDemoStep::LightningSendInvoiceWait);
            }
            MacDemoStep::LightningSendInvoiceWait => {
                if let Some(value) = self.take_task_result() {
                    let Some(invoice) = value["invoice"].as_str() else {
                        self.fail("harness invoice response missing invoice".to_string());
                        return;
                    };
                    self.invoice = invoice.to_string();
                    self.enter(MacDemoStep::LightningSendStart);
                }
            }
            MacDemoStep::LightningSendStart => {
                if !self.invoice.starts_with("lnbcrt") {
                    self.fail(format!("expected lnbcrt invoice, got {}", self.invoice));
                    return;
                }
                self.before_outbound_success = self.outbound_success_count(app);
                app.send_input = self.invoice.clone();
                app.send_amount.clear();
                if !app.send_unified() {
                    self.fail(format!("Lightning send failed to start: {}", app.send_error));
                    return;
                }
                self.action = "Waiting for Lightning send confirmation".to_string();
                self.enter(MacDemoStep::LightningSendWait);
            }
            MacDemoStep::LightningSendWait => {
                if self.wait_for(app, "Lightning send", Duration::from_secs(90), |demo, app| {
                    demo.outbound_success_count(app) > demo.before_outbound_success
                }) {
                    self.complete_flow("06_lightning_send");
                    self.enter_flow(
                        MacDemoStep::OnchainSendAddress,
                        "07 Onchain Send",
                        "Requesting a regtest address",
                    );
                }
            }
            MacDemoStep::OnchainSendAddress => {
                self.start_task("Harness creates a bcrt1 address", |harness| {
                    let address = harness.address()?;
                    Ok(json!({ "address": address }))
                });
                self.enter(MacDemoStep::OnchainSendAddressWait);
            }
            MacDemoStep::OnchainSendAddressWait => {
                if let Some(value) = self.take_task_result() {
                    let Some(address) = value["address"].as_str() else {
                        self.fail("harness address response missing address".to_string());
                        return;
                    };
                    self.address = address.to_string();
                    self.enter(MacDemoStep::OnchainSendStart);
                }
            }
            MacDemoStep::OnchainSendStart => {
                if !self.address.starts_with("bcrt1") {
                    self.fail(format!("expected bcrt1 address, got {}", self.address));
                    return;
                }
                app.send_input = self.address.clone();
                app.send_amount = self.btc_string_for_usd(5.0);
                app.send_all = false;
                if !app.send_unified() {
                    self.fail(format!("onchain send failed to start: {}", app.send_error));
                    return;
                }
                self.action = "Waiting for splice-out negotiation".to_string();
                self.enter(MacDemoStep::OnchainSendSpliceWait);
            }
            MacDemoStep::OnchainSendSpliceWait => {
                if self.wait_for(app, "splice-out negotiation", Duration::from_secs(90), |_demo, app| {
                    app.auto_splice_in_progress
                        .load(std::sync::atomic::Ordering::Relaxed)
                }) {
                    self.start_task("Mining splice-out confirmation", |harness| {
                        harness.mine(6)?;
                        Ok(json!({}))
                    });
                    self.enter(MacDemoStep::OnchainSendMineWait);
                }
            }
            MacDemoStep::OnchainSendMineWait => {
                if self.take_task_result().is_some() {
                    self.action = "Waiting for splice-out to clear".to_string();
                    self.enter(MacDemoStep::OnchainSendConfirmWait);
                }
            }
            MacDemoStep::OnchainSendConfirmWait => {
                if self.wait_for(app, "splice-out confirmation", Duration::from_secs(180), |_demo, app| {
                    !app.auto_splice_in_progress
                        .load(std::sync::atomic::Ordering::Relaxed)
                }) {
                    self.complete_flow("07_onchain_send");
                    self.enter_flow(
                        MacDemoStep::BuyStart,
                        "08 USD to BTC",
                        "Submitting USD to BTC order",
                    );
                }
            }
            MacDemoStep::BuyStart => {
                self.before_expected_usd = self.expected_usd(app);
                let pending_before = app.pending_trade_payments.len();
                app.execute_buy(20.0);
                if app.pending_trade_payments.len() == pending_before {
                    self.fail(format!(
                        "buy order was not submitted: status='{}' error='{}'",
                        app.status_message, app.trade_error
                    ));
                    return;
                }
                self.enter(MacDemoStep::BuyWait);
            }
            MacDemoStep::BuyWait => {
                if self.wait_for(app, "buy order confirmation", Duration::from_secs(60), |demo, app| {
                    demo.expected_usd(app) < demo.before_expected_usd - 15.0
                }) {
                    self.complete_flow("08_usd_to_btc");
                    self.enter_flow(
                        MacDemoStep::CloseStart,
                        "09 Close Channel",
                        "Requesting cooperative close",
                    );
                }
            }
            MacDemoStep::CloseStart => {
                app.close_active_channel();
                if !app.status_message.contains("Closing") {
                    self.fail(format!("close did not start: {}", app.status_message));
                    return;
                }
                self.enter(MacDemoStep::CloseStartWait);
            }
            MacDemoStep::CloseStartWait => {
                if self.wait_for(app, "channel close start", Duration::from_secs(60), |demo, app| {
                    demo.ready_channel_count(app) == 0
                }) {
                    self.start_task("Mining close confirmation", |harness| {
                        harness.mine(6)?;
                        Ok(json!({}))
                    });
                    self.enter(MacDemoStep::CloseMineWait);
                }
            }
            MacDemoStep::CloseMineWait => {
                if self.take_task_result().is_some() {
                    self.action = "Waiting for closed funds onchain".to_string();
                    self.enter(MacDemoStep::CloseConfirmWait);
                }
            }
            MacDemoStep::CloseConfirmWait => {
                if self.wait_for(app, "closed channel funds", Duration::from_secs(180), |demo, app| {
                    demo.ready_channel_count(app) == 0 && demo.spendable_onchain_sats(app) > 0
                }) {
                    self.complete_flow("09_close_channel");
                    self.enter_flow(
                        MacDemoStep::BackupCheck,
                        "10 Backup Keys",
                        "Validating saved seed words",
                    );
                }
            }
            MacDemoStep::BackupCheck => {
                let words = match self.seed_words(app) {
                    Ok(words) => words,
                    Err(err) => {
                        self.fail(err);
                        return;
                    }
                };
                let word_count = words.split_whitespace().count();
                if ![12, 15, 18, 21, 24].contains(&word_count) {
                    self.fail(format!("unexpected BIP39 seed word count: {word_count}"));
                    return;
                }
                if let Err(err) = ldk_node::bip39::Mnemonic::from_str(&words) {
                    self.fail(format!("saved seed phrase is invalid: {err}"));
                    return;
                }
                self.seed_words = words;
                self.complete_flow("10_backup_keys");
                self.enter_flow(
                    MacDemoStep::ImportRestart,
                    "11 Import Keys",
                    "Restarting from saved seed state",
                );
            }
            MacDemoStep::ImportRestart => {
                self.node_id_before = app.node.node_id().to_string();
                if let Err(err) = app.node.stop() {
                    self.fail(format!("node stop failed: {err}"));
                    return;
                }
                let mut new_app = match UserApp::new() {
                    Ok(app) => app,
                    Err(err) => {
                        self.fail(err);
                        return;
                    }
                };
                new_app.mac_demo = None;
                *app = new_app;
                self.set_app_price(app, self.price_usd);
                if let Err(err) = self.pump(app) {
                    self.fail(err);
                    return;
                }
                match self.seed_words(app) {
                    Ok(words) if words == self.seed_words => {}
                    Ok(_) => {
                        self.fail("restored wallet did not load the same seed words".to_string());
                        return;
                    }
                    Err(err) => {
                        self.fail(err);
                        return;
                    }
                }
                let node_id_after = app.node.node_id().to_string();
                if node_id_after != self.node_id_before {
                    self.fail(format!(
                        "restored node id changed: before={} after={node_id_after}",
                        self.node_id_before
                    ));
                    return;
                }
                self.complete_flow("11_import_keys");
                self.enter_flow(
                    MacDemoStep::OffboardMineWait,
                    "12 Offboard Onchain",
                    "Mining final close outputs",
                );
                self.start_task("Mining final close outputs", |harness| {
                    harness.mine(6)?;
                    Ok(json!({}))
                });
            }
            MacDemoStep::OffboardMineWait => {
                if self.take_task_result().is_some() {
                    self.action = "Waiting for spendable offboard funds".to_string();
                    self.enter(MacDemoStep::OffboardFundsWait);
                }
            }
            MacDemoStep::OffboardFundsWait => {
                if self.wait_for(app, "offboard funds", Duration::from_secs(120), |demo, app| {
                    demo.spendable_onchain_sats(app) > 0
                }) {
                    self.before_spendable_sats = self.spendable_onchain_sats(app);
                    self.start_task("Harness creates final onchain address", |harness| {
                        let address = harness.address()?;
                        Ok(json!({ "address": address }))
                    });
                    self.enter(MacDemoStep::OffboardAddressWait);
                }
            }
            MacDemoStep::OffboardAddressWait => {
                if let Some(value) = self.take_task_result() {
                    let Some(address) = value["address"].as_str() else {
                        self.fail("harness address response missing address".to_string());
                        return;
                    };
                    self.address = address.to_string();
                    self.enter(MacDemoStep::OffboardSend);
                }
            }
            MacDemoStep::OffboardSend => {
                app.send_input = self.address.clone();
                app.send_all = true;
                app.send_amount.clear();
                if !app.send_unified() {
                    self.fail(format!("send-all failed to start: {}", app.send_error));
                    return;
                }
                self.start_task("Mining final offboard transaction", |harness| {
                    harness.mine(6)?;
                    Ok(json!({}))
                });
                self.enter(MacDemoStep::OffboardMineConfirmWait);
            }
            MacDemoStep::OffboardMineConfirmWait => {
                if self.take_task_result().is_some() {
                    self.action = "Waiting for offboard spend confirmation".to_string();
                    self.enter(MacDemoStep::OffboardConfirmWait);
                }
            }
            MacDemoStep::OffboardConfirmWait => {
                if self.wait_for(app, "offboard spend", Duration::from_secs(120), |demo, app| {
                    demo.spendable_onchain_sats(app) < demo.before_spendable_sats.saturating_div(2)
                }) {
                    self.complete_flow("12_offboard_onchain");
                    self.current_flow = "Demo complete".to_string();
                    self.action = "All Mac demo flows passed".to_string();
                    self.detail = format!(
                        "{} flows passed in {:.1}s",
                        Self::FLOW_COUNT,
                        self.started_at.elapsed().as_secs_f32()
                    );
                    self.finished = true;
                    self.step = MacDemoStep::Done;
                }
            }
            MacDemoStep::Done => {}
        }
    }

    pub(crate) fn render(&self, ctx: &egui::Context) {
        egui::Window::new("Mac Demo")
            .anchor(egui::Align2::RIGHT_TOP, [-16.0, 16.0])
            .collapsible(false)
            .resizable(false)
            .default_width(310.0)
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    let progress = self.completed.len() as f32 / Self::FLOW_COUNT as f32;
                    ui.add(
                        egui::ProgressBar::new(progress)
                            .show_percentage()
                            .desired_width(280.0),
                    );
                    ui.add_space(4.0);
                    ui.label(RichText::new(&self.current_flow).strong().size(15.0));
                    ui.label(&self.action);
                    if !self.detail.is_empty() {
                        ui.label(RichText::new(&self.detail).size(12.0).color(theme::MUTED));
                    }
                    ui.add_space(6.0);
                    ui.label(format!(
                        "{}/{} flows passed",
                        self.completed.len(),
                        Self::FLOW_COUNT
                    ));
                    if let Some(task) = self.pending_task.as_ref() {
                        ui.label(RichText::new(format!("Running: {}", task.label)).size(12.0));
                    }
                    if let Some(err) = self.failed.as_ref() {
                        ui.separator();
                        ui.colored_label(theme::DANGER, "Demo failed");
                        ui.label(RichText::new(err).size(12.0));
                    } else if self.finished {
                        ui.separator();
                        ui.colored_label(theme::SUCCESS, "Demo passed");
                    }
                });
            });
    }

    pub(crate) fn needs_fast_repaint(&self) -> bool {
        !self.finished && self.failed.is_none()
    }

    fn start_task<F>(&mut self, label: &str, f: F)
    where
        F: FnOnce(MacFlowHarness) -> Result<serde_json::Value, String> + Send + 'static,
    {
        let api = self.harness.api.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let harness = MacFlowHarness::with_api(api);
            let _ = tx.send(f(harness));
        });
        self.detail = label.to_string();
        self.pending_task = Some(MacDemoTask {
            label: label.to_string(),
            receiver: rx,
        });
    }

    fn take_task_result(&mut self) -> Option<serde_json::Value> {
        let task = self.pending_task.as_ref()?;
        match task.receiver.try_recv() {
            Ok(Ok(value)) => {
                self.pending_task = None;
                Some(value)
            }
            Ok(Err(err)) => {
                self.pending_task = None;
                self.fail(err);
                None
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                self.detail = format!("Waiting: {}", task.label);
                None
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending_task = None;
                self.fail("background demo task disconnected".to_string());
                None
            }
        }
    }

    fn enter(&mut self, step: MacDemoStep) {
        self.step = step;
        self.step_started = std::time::Instant::now();
    }

    fn enter_flow(&mut self, step: MacDemoStep, flow: &str, action: &str) {
        self.current_flow = flow.to_string();
        self.action = action.to_string();
        self.detail.clear();
        self.enter(step);
        self.pause_until =
            Some(std::time::Instant::now() + Duration::from_millis(self.pause_ms));
    }

    fn complete_flow(&mut self, id: &'static str) {
        if !self.completed.contains(&id) {
            self.completed.push(id);
        }
        self.detail = format!("{id} passed");
        self.pause_until =
            Some(std::time::Instant::now() + Duration::from_millis(self.pause_ms));
    }

    fn fail(&mut self, err: String) {
        self.failed = Some(err);
    }

    fn wait_for<F>(
        &mut self,
        app: &mut UserApp,
        label: &str,
        timeout: Duration,
        predicate: F,
    ) -> bool
    where
        F: FnOnce(&Self, &UserApp) -> bool,
    {
        if predicate(self, app) {
            return true;
        }
        if self.step_started.elapsed() > timeout {
            self.fail(format!("timed out waiting for {label}"));
            return false;
        }
        self.detail = format!(
            "Waiting for {label} ({:.1}s)",
            self.step_started.elapsed().as_secs_f32()
        );
        false
    }

    fn pump(&mut self, app: &mut UserApp) -> Result<(), String> {
        app.node
            .sync_wallets()
            .map_err(|err| format!("wallet sync failed: {err}"))?;
        app.process_events();
        {
            let mut sc = app.stable_channel.lock().unwrap();
            stable::update_balances(&app.node, &mut sc);
            sc.latest_price = self.price_usd;
            sc.timestamp = MacFlowRunner::unix_time();
        }
        app.update_balances();
        self.last_pump = std::time::Instant::now();
        Ok(())
    }

    fn set_app_price(&mut self, app: &mut UserApp, price: f64) {
        self.price_usd = price;
        stable_channels::price_feeds::set_cached_price(price);
        {
            let mut sc = app.stable_channel.lock().unwrap();
            sc.latest_price = price;
            sc.timestamp = MacFlowRunner::unix_time();
        }
        app.btc_price = price;
    }

    fn ensure_stable_channel_has_price(&mut self, app: &mut UserApp) {
        let mut sc = app.stable_channel.lock().unwrap();
        sc.latest_price = self.price_usd;
        if sc.user_channel_id == 0 {
            if let Some(channel) = app.node.list_channels().first() {
                sc.user_channel_id = channel.user_channel_id.0;
                sc.channel_id = channel.channel_id;
            }
        }
        stable::update_balances(&app.node, &mut sc);
        sc.latest_price = self.price_usd;
        sc.timestamp = MacFlowRunner::unix_time();
    }

    fn require_invoice(&self, invoice: &str) -> Result<String, String> {
        if invoice.starts_with("lnbcrt") {
            Ok(invoice.to_string())
        } else if invoice.is_empty() {
            Err("invoice was empty".to_string())
        } else {
            Err(format!("expected lnbcrt invoice, got {invoice}"))
        }
    }

    fn seed_words(&self, app: &UserApp) -> Result<String, String> {
        app.saved_mnemonic
            .clone()
            .filter(|words| !words.trim().is_empty())
            .ok_or_else(|| "no saved seed words available".to_string())
    }

    fn expected_usd(&self, app: &UserApp) -> f64 {
        app.stable_channel.lock().unwrap().expected_usd.0
    }

    fn lightning_sats(&self, app: &UserApp) -> u64 {
        app.node.list_balances().total_lightning_balance_sats
    }

    fn spendable_onchain_sats(&self, app: &UserApp) -> u64 {
        app.node.list_balances().spendable_onchain_balance_sats
    }

    fn ready_channel_count(&self, app: &UserApp) -> usize {
        app.node
            .list_channels()
            .iter()
            .filter(|channel| channel.is_channel_ready)
            .count()
    }

    fn outbound_success_count(&self, app: &UserApp) -> usize {
        app.node
            .list_payments()
            .iter()
            .filter(|payment| {
                payment.direction == PaymentDirection::Outbound
                    && payment.status == PaymentStatus::Succeeded
            })
            .count()
    }

    fn sats_for_usd(&self, usd: f64) -> u64 {
        ((usd / self.price_usd) * 100_000_000.0).round() as u64
    }

    fn btc_string_for_usd(&self, usd: f64) -> String {
        format!("{:.8}", self.sats_for_usd(usd) as f64 / 100_000_000.0)
    }
}
