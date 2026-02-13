#![allow(dead_code)]

use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::time::Duration;

use electrsd::corepc_node::{Client as BitcoindClient, Node as BitcoinD};
use electrsd::ElectrsD;
use electrum_client::ElectrumApi;

use ldk_node::bitcoin::{Address, Amount, Network};
use ldk_node::config::{AnchorChannelsConfig, Config, EsploraSyncConfig};
use ldk_node::lightning::ln::msgs::SocketAddress;
use ldk_node::lightning::routing::gossip::NodeAlias;
use ldk_node::liquidity::LSPS2ServiceConfig;
use ldk_node::{Builder, Node};

use rand::distr::Alphanumeric;
use rand::Rng;
use serde_json::{json, Value};

use stable_channels::types::{StableChannel, USD, Bitcoin};

// ================================================================
// Event macros (adapted from ldk-node test suite)
// ================================================================

macro_rules! expect_event {
    ($node:expr, $event_type:ident) => {{
        match $node.next_event_async().await {
            ref e @ ldk_node::Event::$event_type { .. } => {
                println!("{} got event {:?}", $node.node_id(), e);
                $node.event_handled().unwrap();
            },
            ref e => {
                panic!("{} got unexpected event: {:?}", stringify!($node), e);
            },
        }
    }};
}
pub(crate) use expect_event;

macro_rules! expect_channel_pending_event {
    ($node:expr, $counterparty_node_id:expr) => {{
        match $node.next_event_async().await {
            ref e @ ldk_node::Event::ChannelPending { funding_txo, counterparty_node_id, .. } => {
                println!("{} got event {:?}", $node.node_id(), e);
                assert_eq!(counterparty_node_id, $counterparty_node_id);
                $node.event_handled().unwrap();
                funding_txo
            },
            ref e => {
                panic!("{} got unexpected event: {:?}", stringify!($node), e);
            },
        }
    }};
}
pub(crate) use expect_channel_pending_event;

macro_rules! expect_channel_ready_event {
    ($node:expr, $counterparty_node_id:expr) => {{
        match $node.next_event_async().await {
            ref e @ ldk_node::Event::ChannelReady { user_channel_id, counterparty_node_id, .. } => {
                println!("{} got event {:?}", $node.node_id(), e);
                assert_eq!(counterparty_node_id, Some($counterparty_node_id));
                $node.event_handled().unwrap();
                user_channel_id
            },
            ref e => {
                panic!("{} got unexpected event: {:?}", stringify!($node), e);
            },
        }
    }};
}
pub(crate) use expect_channel_ready_event;

macro_rules! expect_payment_received_event {
    ($node:expr) => {{
        match $node.next_event_async().await {
            ref e @ ldk_node::Event::PaymentReceived { payment_id, amount_msat, .. } => {
                println!("{} got event {:?}", $node.node_id(), e);
                $node.event_handled().unwrap();
                (payment_id, amount_msat)
            },
            ref e => {
                panic!("{} got unexpected event: {:?}", stringify!($node), e);
            },
        }
    }};
}
pub(crate) use expect_payment_received_event;

macro_rules! expect_payment_successful_event {
    ($node:expr) => {{
        match $node.next_event_async().await {
            ref e @ ldk_node::Event::PaymentSuccessful { payment_id, fee_paid_msat, .. } => {
                println!("{} got event {:?}", $node.node_id(), e);
                $node.event_handled().unwrap();
                (payment_id, fee_paid_msat)
            },
            ref e => {
                panic!("{} got unexpected event: {:?}", stringify!($node), e);
            },
        }
    }};
}
pub(crate) use expect_payment_successful_event;

// ================================================================
// Infrastructure setup
// ================================================================

pub fn setup_bitcoind_and_electrsd() -> (BitcoinD, ElectrsD) {
    let bitcoind_exe = env::var("BITCOIND_EXE")
        .ok()
        .or_else(|| electrsd::corepc_node::downloaded_exe_path().ok())
        .expect("provide BITCOIND_EXE env var or use corepc-node_27_2 feature");

    let mut bitcoind_conf = electrsd::corepc_node::Conf::default();
    bitcoind_conf.network = "regtest";
    bitcoind_conf.args.push("-rest");
    let bitcoind = BitcoinD::with_conf(bitcoind_exe, &bitcoind_conf).unwrap();

    let electrs_exe = env::var("ELECTRS_EXE")
        .ok()
        .or_else(electrsd::downloaded_exe_path)
        .expect("provide ELECTRS_EXE env var or use esplora_a33e97e1 feature");

    let mut electrsd_conf = electrsd::Conf::default();
    electrsd_conf.http_enabled = true;
    electrsd_conf.network = "regtest";
    let electrsd = ElectrsD::with_conf(electrs_exe, &bitcoind, &electrsd_conf).unwrap();

    (bitcoind, electrsd)
}

// ================================================================
// Helpers
// ================================================================

pub fn random_storage_path() -> PathBuf {
    let mut temp_path = std::env::temp_dir();
    let mut rng = rand::rng();
    let rand_dir: String = (0..7).map(|_| rng.sample(Alphanumeric) as char).collect();
    temp_path.push(format!("sc_regtest_{}", rand_dir));
    temp_path
}

pub fn random_port() -> u16 {
    let mut rng = rand::rng();
    rng.random_range(5000..32768)
}

pub fn random_listening_addresses() -> Vec<SocketAddress> {
    let mut addrs = Vec::with_capacity(1);
    let port = random_port();
    let address: SocketAddress = format!("127.0.0.1:{}", port).parse().unwrap();
    addrs.push(address);
    addrs
}

pub fn random_node_alias(prefix: &str) -> Option<NodeAlias> {
    let mut rng = rand::rng();
    let rand_val: u16 = rng.random_range(0..1000);
    let alias = format!("{}-{}", prefix, rand_val);
    let mut bytes = [0u8; 32];
    let len = alias.len().min(32);
    bytes[..len].copy_from_slice(&alias.as_bytes()[..len]);
    Some(NodeAlias(bytes))
}

// ================================================================
// Node builders
// ================================================================

/// Build a generic node on regtest with Esplora chain source
pub fn setup_node(electrsd: &ElectrsD, alias_prefix: &str, anchor_channels: bool) -> Node {
    let mut config = Config::default();
    config.network = Network::Regtest;
    config.storage_dir_path = random_storage_path().to_str().unwrap().to_owned();
    config.listening_addresses = Some(random_listening_addresses());
    config.node_alias = random_node_alias(alias_prefix);

    if !anchor_channels {
        config.anchor_channels_config = None;
    }

    let mut builder = Builder::from_config(config);
    let esplora_url = format!("http://{}", electrsd.esplora_url.as_ref().unwrap());
    let sync_config = EsploraSyncConfig { background_sync_config: None };
    builder.set_chain_source_esplora(esplora_url, Some(sync_config));

    let node = builder.build().unwrap();
    node.start().unwrap();
    println!("[setup] {} node started: {}", alias_prefix, node.node_id());
    node
}

/// Build an LSP node with LSPS2 service provider configured
pub fn setup_lsp_node(electrsd: &ElectrsD) -> Node {
    let mut config = Config::default();
    config.network = Network::Regtest;
    config.storage_dir_path = random_storage_path().to_str().unwrap().to_owned();
    config.listening_addresses = Some(random_listening_addresses());
    config.node_alias = random_node_alias("lsp");

    let mut builder = Builder::from_config(config);
    let esplora_url = format!("http://{}", electrsd.esplora_url.as_ref().unwrap());
    let sync_config = EsploraSyncConfig { background_sync_config: None };
    builder.set_chain_source_esplora(esplora_url, Some(sync_config));

    // Configure as LSPS2 service provider
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
        client_trusts_lsp: true,
    };
    builder.set_liquidity_provider_lsps2(service_config);

    let node = builder.build().unwrap();
    node.start().unwrap();
    println!("[setup] LSP node started: {}", node.node_id());
    node
}

/// Build a User node with LSPS2 client + trusted peer no reserve
pub fn setup_user_node(
    electrsd: &ElectrsD,
    lsp_pubkey: ldk_node::bitcoin::secp256k1::PublicKey,
    lsp_address: SocketAddress,
) -> Node {
    let mut config = Config::default();
    config.network = Network::Regtest;
    config.storage_dir_path = random_storage_path().to_str().unwrap().to_owned();
    config.listening_addresses = Some(random_listening_addresses());
    config.node_alias = random_node_alias("user");
    config.anchor_channels_config = Some(AnchorChannelsConfig {
        trusted_peers_no_reserve: vec![lsp_pubkey],
        per_channel_reserve_sats: 25_000,
    });

    let mut builder = Builder::from_config(config);
    let esplora_url = format!("http://{}", electrsd.esplora_url.as_ref().unwrap());
    let sync_config = EsploraSyncConfig { background_sync_config: None };
    builder.set_chain_source_esplora(esplora_url, Some(sync_config));

    // LSPS2 client
    builder.set_liquidity_source_lsps2(lsp_pubkey, lsp_address, None);

    let node = builder.build().unwrap();
    node.start().unwrap();
    println!("[setup] User node started: {}", node.node_id());
    node
}

// ================================================================
// Block generation and funding
// ================================================================

pub async fn generate_blocks_and_wait<E: ElectrumApi>(
    bitcoind: &BitcoindClient, electrs: &E, num: usize,
) {
    let _ = bitcoind.create_wallet("sc_regtest");
    let _ = bitcoind.load_wallet("sc_regtest");
    print!("Generating {} blocks...", num);
    let blockchain_info = bitcoind.get_blockchain_info().expect("failed to get blockchain info");
    let cur_height = blockchain_info.blocks;
    let address = bitcoind.new_address().expect("failed to get new address");
    let _block_hashes_res = bitcoind.generate_to_address(num, &address);
    wait_for_block(electrs, cur_height as usize + num).await;
    println!(" Done! (height={})", cur_height as usize + num);
}

pub async fn wait_for_block<E: ElectrumApi>(electrs: &E, min_height: usize) {
    let mut header = match electrs.block_headers_subscribe() {
        Ok(header) => header,
        Err(_) => {
            tokio::time::sleep(Duration::from_secs(3)).await;
            electrs.block_headers_subscribe().expect("failed to subscribe to block headers")
        },
    };
    loop {
        if header.height >= min_height {
            break;
        }
        header = exponential_backoff_poll(|| {
            electrs.ping().expect("failed to ping electrs");
            electrs.block_headers_pop().expect("failed to pop block header")
        })
        .await;
    }
}

pub async fn premine_blocks<E: ElectrumApi>(bitcoind: &BitcoindClient, electrs: &E) {
    let _ = bitcoind.create_wallet("sc_regtest");
    let _ = bitcoind.load_wallet("sc_regtest");
    generate_blocks_and_wait(bitcoind, electrs, 101).await;
}

pub async fn premine_and_distribute_funds<E: ElectrumApi>(
    bitcoind: &BitcoindClient, electrs: &E, addrs: Vec<Address>, amount: Amount,
) {
    premine_blocks(bitcoind, electrs).await;

    let mut amounts = HashMap::<String, f64>::new();
    for addr in &addrs {
        amounts.insert(addr.to_string(), amount.to_btc());
    }

    let empty_account = json!("");
    let amounts_json = json!(amounts);
    let txid: ldk_node::bitcoin::Txid = bitcoind
        .call::<Value>("sendmany", &[empty_account, amounts_json])
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    wait_for_tx(electrs, txid).await;
    generate_blocks_and_wait(bitcoind, electrs, 1).await;
}

pub async fn wait_for_tx<E: ElectrumApi>(electrs: &E, txid: ldk_node::bitcoin::Txid) {
    if electrs.transaction_get(&txid).is_ok() {
        return;
    }

    exponential_backoff_poll(|| {
        electrs.ping().unwrap();
        electrs.transaction_get(&txid).ok()
    })
    .await;
}

pub async fn exponential_backoff_poll<T, F>(mut poll: F) -> T
where
    F: FnMut() -> Option<T>,
{
    let mut delay = Duration::from_millis(64);
    let mut tries = 0;
    loop {
        match poll() {
            Some(data) => break data,
            None if delay.as_millis() < 512 => {
                delay = delay.mul_f32(2.0);
            },
            None => {},
        }
        assert!(tries < 20, "Reached max tries.");
        tries += 1;
        tokio::time::sleep(delay).await;
    }
}

// ================================================================
// Channel helpers
// ================================================================

pub async fn open_channel(
    node_a: &Node, node_b: &Node, funding_amount_sat: u64, push_msat: Option<u64>,
    electrsd: &ElectrsD,
) -> ldk_node::bitcoin::OutPoint {
    node_a
        .open_channel(
            node_b.node_id(),
            node_b.listening_addresses().unwrap().first().unwrap().clone(),
            funding_amount_sat,
            push_msat,
            None,
        )
        .unwrap();

    let funding_txo_a = expect_channel_pending_event!(node_a, node_b.node_id());
    let funding_txo_b = expect_channel_pending_event!(node_b, node_a.node_id());
    assert_eq!(funding_txo_a, funding_txo_b);

    wait_for_tx(&electrsd.client, funding_txo_a.txid).await;
    funding_txo_a
}

/// Open a channel and wait for it to be fully ready (confirmed)
pub async fn open_channel_and_confirm(
    node_a: &Node, node_b: &Node, funding_amount_sat: u64, push_msat: Option<u64>,
    bitcoind: &BitcoindClient, electrsd: &ElectrsD,
) {
    let _funding_txo = open_channel(node_a, node_b, funding_amount_sat, push_msat, electrsd).await;

    // Mine 6 blocks to confirm
    generate_blocks_and_wait(bitcoind, &electrsd.client, 6).await;
    node_a.sync_wallets().unwrap();
    node_b.sync_wallets().unwrap();

    // Wait for channel ready on both sides
    let _user_channel_id_a = expect_channel_ready_event!(node_a, node_b.node_id());
    let _user_channel_id_b = expect_channel_ready_event!(node_b, node_a.node_id());

    println!(
        "[channel] {}↔{} ready ({}sat, push={}msat)",
        node_a.node_id(), node_b.node_id(), funding_amount_sat, push_msat.unwrap_or(0)
    );
}

// ================================================================
// StableChannel helpers
// ================================================================

/// Create a StableChannel from live node state.
/// Call after channel is ready.
pub fn create_stable_channel(
    node: &Node,
    counterparty: ldk_node::bitcoin::secp256k1::PublicKey,
    is_stable_receiver: bool,
    expected_usd: f64,
    price: f64,
) -> StableChannel {
    let channels = node.list_channels();
    let ch = channels.iter()
        .find(|c| c.counterparty_node_id == counterparty)
        .expect("no channel found with counterparty");

    let btc_amount = expected_usd / price;
    let backing_sats = (btc_amount * 100_000_000.0) as u64;

    StableChannel {
        channel_id: ch.channel_id,
        is_stable_receiver,
        counterparty,
        expected_usd: USD::from_f64(expected_usd),
        backing_sats,
        latest_price: price,
        ..StableChannel::default()
    }
}

/// Set the mock BTC/USD price for stability testing
pub fn set_mock_price(price: f64) {
    stable_channels::price_feeds::set_cached_price(price);
    println!("[price] Set mock BTC price to ${:.2}", price);
}

/// Print a summary of channel balances for debugging
pub fn print_channel_balances(label: &str, node: &Node) {
    let channels = node.list_channels();
    let balances = node.list_balances();
    println!("\n=== {} ({}) ===", label, node.node_id());
    println!("  total_lightning_balance_sats: {}", balances.total_lightning_balance_sats);
    println!("  spendable_onchain_balance_sats: {}", balances.spendable_onchain_balance_sats);
    for ch in &channels {
        println!("  channel {} (ready={}):", ch.channel_id, ch.is_channel_ready);
        println!("    channel_value_sats: {}", ch.channel_value_sats);
        println!("    outbound_capacity_msat: {}", ch.outbound_capacity_msat);
        println!("    inbound_capacity_msat: {}", ch.inbound_capacity_msat);
        println!("    reserve: {:?}", ch.unspendable_punishment_reserve);
    }
}

/// Print StableChannel state for debugging
pub fn print_stable_channel(label: &str, sc: &StableChannel) {
    println!("\n=== StableChannel: {} ===", label);
    println!("  is_stable_receiver: {}", sc.is_stable_receiver);
    println!("  expected_usd: {}", sc.expected_usd);
    println!("  backing_sats: {}", sc.backing_sats);
    println!("  stable_receiver_btc: {}", sc.stable_receiver_btc);
    println!("  stable_provider_btc: {}", sc.stable_provider_btc);
    println!("  stable_receiver_usd: {}", sc.stable_receiver_usd);
    println!("  stable_provider_usd: {}", sc.stable_provider_usd);
    println!("  latest_price: {:.2}", sc.latest_price);
}
