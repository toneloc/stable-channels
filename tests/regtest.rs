mod common;

use ldk_node::bitcoin::Amount;
use stable_channels::stable::{
    check_stability, update_balances,
    reconcile_outgoing, reconcile_incoming, reconcile_forwarded, apply_trade,
};

use common::*;

// ==================================================================
// Test 1: Price drop — LSP pays user to maintain stability
// ==================================================================

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires bitcoind + electrs (run with --ignored)"]
async fn test_stability_price_drop_lsp_pays() {
    println!("\n========================================");
    println!("TEST: Price drop — LSP should pay user");
    println!("========================================\n");

    // --- Setup infrastructure ---
    let (bitcoind, electrsd) = setup_bitcoind_and_electrsd();

    // Build LSP first (we need its pubkey for user config)
    let lsp_node = setup_lsp_node(&electrsd);
    let lsp_pubkey = lsp_node.node_id();
    let lsp_addr = lsp_node.listening_addresses().unwrap().first().unwrap().clone();

    // Build User node pointing to LSP
    let user_node = setup_user_node(&electrsd, lsp_pubkey, lsp_addr);

    // --- Fund both nodes on-chain ---
    let addr_lsp = lsp_node.onchain_payment().new_address().unwrap();
    let addr_user = user_node.onchain_payment().new_address().unwrap();

    premine_and_distribute_funds(
        &bitcoind.client,
        &electrsd.client,
        vec![addr_lsp, addr_user],
        Amount::from_sat(2_125_000),
    )
    .await;

    lsp_node.sync_wallets().unwrap();
    user_node.sync_wallets().unwrap();

    println!(
        "LSP onchain: {} sats",
        lsp_node.list_balances().spendable_onchain_balance_sats
    );
    println!(
        "User onchain: {} sats",
        user_node.list_balances().spendable_onchain_balance_sats
    );

    // --- Open channel: LSP -> User with balanced push ---
    let funding_sats = 2_000_000;
    let push_msat = (funding_sats / 2) * 1000; // 1M sats each side

    open_channel_and_confirm(
        &lsp_node,
        &user_node,
        funding_sats,
        Some(push_msat),
        &bitcoind.client,
        &electrsd,
    )
    .await;

    print_channel_balances("LSP", &lsp_node);
    print_channel_balances("User", &user_node);

    // --- Set initial mock price and create StableChannels ---
    let initial_price = 100_000.0; // $100k/BTC
    set_mock_price(initial_price);

    // User has ~1M sats = $1000 at $100k. Stabilize $500.
    let expected_usd = 500.0;

    // User-side StableChannel (is_stable_receiver = true)
    let mut user_sc = create_stable_channel(
        &user_node,
        lsp_pubkey,
        true, // user is stable receiver
        expected_usd,
        initial_price,
    );

    // LSP-side StableChannel (is_stable_receiver = false)
    let mut lsp_sc = create_stable_channel(
        &lsp_node,
        user_node.node_id(),
        false, // LSP is stable provider
        expected_usd,
        initial_price,
    );

    // --- Verify initial state: at equilibrium, no payments needed ---
    let (ok, _) = update_balances(&user_node, &mut user_sc);
    assert!(ok, "update_balances should succeed");
    print_stable_channel("User (initial)", &user_sc);

    let result = check_stability(&user_node, &mut user_sc, initial_price);
    assert!(result.is_none(), "No payment needed at initial price");
    println!("[check] User-side at initial price: no payment needed (correct)");

    let (ok, _) = update_balances(&lsp_node, &mut lsp_sc);
    assert!(ok, "update_balances should succeed for LSP");
    print_stable_channel("LSP (initial)", &lsp_sc);

    let result = check_stability(&lsp_node, &mut lsp_sc, initial_price);
    assert!(result.is_none(), "No payment needed from LSP at initial price");
    println!("[check] LSP-side at initial price: no payment needed (correct)");

    // --- Simulate price DROP: $100k -> $90k ---
    let drop_price = 90_000.0;
    set_mock_price(drop_price);

    // User-side: receiver is below expected, should be CHECK_ONLY
    let result = check_stability(&user_node, &mut user_sc, drop_price);
    assert!(
        result.is_none(),
        "User should NOT send payment when receiver is below par"
    );
    println!("[check] User-side after price drop: CHECK_ONLY (correct — LSP should pay)");

    // LSP-side: provider sees receiver below expected, should PAY
    let result = check_stability(&lsp_node, &mut lsp_sc, drop_price);
    assert!(
        result.is_some(),
        "LSP SHOULD make stability payment when price drops"
    );

    let payment_info = result.unwrap();
    println!(
        "[check] LSP sent stability payment: {} msats (payment_id: {})",
        payment_info.amount_msat, payment_info.payment_id
    );
    assert!(payment_info.amount_msat > 0);

    // Handle events on both sides
    let (_pid, _fee) = expect_payment_successful_event!(lsp_node);
    println!("[event] LSP: PaymentSuccessful");

    let (_pid, received_msat) = expect_payment_received_event!(user_node);
    println!("[event] User: PaymentReceived {} msats", received_msat);

    // --- Verify balance conservation ---
    user_node.sync_wallets().unwrap();
    lsp_node.sync_wallets().unwrap();

    print_channel_balances("LSP (after payment)", &lsp_node);
    print_channel_balances("User (after payment)", &user_node);

    let user_channels = user_node.list_channels();
    let ch = user_channels.first().unwrap();
    let user_sats =
        (ch.outbound_capacity_msat / 1000) + ch.unspendable_punishment_reserve.unwrap_or(0);
    let counterparty_sats = ch.channel_value_sats - user_sats;
    println!(
        "\n[balance] User: {} sats, LSP: {} sats, Total: {} sats (capacity: {} sats)",
        user_sats,
        counterparty_sats,
        user_sats + counterparty_sats,
        ch.channel_value_sats
    );

    // Total should equal channel capacity (conservation law)
    assert_eq!(
        user_sats + counterparty_sats,
        ch.channel_value_sats,
        "Balance conservation violated!"
    );

    println!("\n[PASS] test_stability_price_drop_lsp_pays");

    // Cleanup
    user_node.stop().unwrap();
    lsp_node.stop().unwrap();
}

// ==================================================================
// Test 2: Price rise — user pays LSP
// ==================================================================

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires bitcoind + electrs (run with --ignored)"]
async fn test_stability_price_rise_user_pays() {
    println!("\n========================================");
    println!("TEST: Price rise — User should pay LSP");
    println!("========================================\n");

    let (bitcoind, electrsd) = setup_bitcoind_and_electrsd();

    let lsp_node = setup_lsp_node(&electrsd);
    let lsp_pubkey = lsp_node.node_id();
    let lsp_addr = lsp_node.listening_addresses().unwrap().first().unwrap().clone();
    let user_node = setup_user_node(&electrsd, lsp_pubkey, lsp_addr);

    // Fund and open channel
    let addr_lsp = lsp_node.onchain_payment().new_address().unwrap();
    let addr_user = user_node.onchain_payment().new_address().unwrap();
    premine_and_distribute_funds(
        &bitcoind.client,
        &electrsd.client,
        vec![addr_lsp, addr_user],
        Amount::from_sat(2_125_000),
    )
    .await;
    lsp_node.sync_wallets().unwrap();
    user_node.sync_wallets().unwrap();

    let funding_sats = 2_000_000;
    let push_msat = (funding_sats / 2) * 1000;
    open_channel_and_confirm(
        &lsp_node,
        &user_node,
        funding_sats,
        Some(push_msat),
        &bitcoind.client,
        &electrsd,
    )
    .await;

    // Set initial price and stabilize $500
    let initial_price = 100_000.0;
    set_mock_price(initial_price);
    let expected_usd = 500.0;

    let mut user_sc = create_stable_channel(
        &user_node,
        lsp_pubkey,
        true,
        expected_usd,
        initial_price,
    );

    // Verify initial equilibrium
    let (ok, _) = update_balances(&user_node, &mut user_sc);
    assert!(ok);
    let result = check_stability(&user_node, &mut user_sc, initial_price);
    assert!(result.is_none(), "No payment at initial price");

    // --- Simulate price RISE: $100k -> $110k ---
    let rise_price = 110_000.0;
    set_mock_price(rise_price);

    // User-side: receiver is ABOVE expected (BTC worth more), should PAY
    let result = check_stability(&user_node, &mut user_sc, rise_price);
    assert!(
        result.is_some(),
        "User SHOULD pay when price rises (above expected)"
    );

    let payment_info = result.unwrap();
    println!(
        "[check] User sent stability payment: {} msats",
        payment_info.amount_msat
    );
    assert!(payment_info.amount_msat > 0);

    // Handle events
    let (_pid, _fee) = expect_payment_successful_event!(user_node);
    println!("[event] User: PaymentSuccessful");

    let (_pid, received_msat) = expect_payment_received_event!(lsp_node);
    println!("[event] LSP: PaymentReceived {} msats", received_msat);

    // Verify balance conservation
    let user_channels = user_node.list_channels();
    let ch = user_channels.first().unwrap();
    let user_sats =
        (ch.outbound_capacity_msat / 1000) + ch.unspendable_punishment_reserve.unwrap_or(0);
    let counterparty_sats = ch.channel_value_sats - user_sats;
    assert_eq!(
        user_sats + counterparty_sats,
        ch.channel_value_sats,
        "Balance conservation violated!"
    );

    println!("\n[PASS] test_stability_price_rise_user_pays");

    user_node.stop().unwrap();
    lsp_node.stop().unwrap();
}

// ==================================================================
// Test 3: Multiple stability cycles
// ==================================================================

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires bitcoind + electrs (run with --ignored)"]
async fn test_multiple_stability_cycles() {
    println!("\n========================================");
    println!("TEST: Multiple stability cycles");
    println!("========================================\n");

    let (bitcoind, electrsd) = setup_bitcoind_and_electrsd();

    let lsp_node = setup_lsp_node(&electrsd);
    let lsp_pubkey = lsp_node.node_id();
    let lsp_addr = lsp_node.listening_addresses().unwrap().first().unwrap().clone();
    let user_node = setup_user_node(&electrsd, lsp_pubkey, lsp_addr);

    // Fund and open channel
    let addr_lsp = lsp_node.onchain_payment().new_address().unwrap();
    let addr_user = user_node.onchain_payment().new_address().unwrap();
    premine_and_distribute_funds(
        &bitcoind.client,
        &electrsd.client,
        vec![addr_lsp, addr_user],
        Amount::from_sat(2_125_000),
    )
    .await;
    lsp_node.sync_wallets().unwrap();
    user_node.sync_wallets().unwrap();

    let funding_sats = 2_000_000;
    let push_msat = (funding_sats / 2) * 1000;
    open_channel_and_confirm(
        &lsp_node,
        &user_node,
        funding_sats,
        Some(push_msat),
        &bitcoind.client,
        &electrsd,
    )
    .await;

    let initial_price = 100_000.0;
    set_mock_price(initial_price);
    let expected_usd = 500.0;

    let mut user_sc = create_stable_channel(&user_node, lsp_pubkey, true, expected_usd, initial_price);
    let mut lsp_sc = create_stable_channel(&lsp_node, user_node.node_id(), false, expected_usd, initial_price);

    // --- Cycle 1: Price drops 5% ($100k -> $95k) — LSP pays ---
    println!("\n--- Cycle 1: Price drop 5% ---");
    let price1 = 95_000.0;
    set_mock_price(price1);

    let user_result = check_stability(&user_node, &mut user_sc, price1);
    assert!(user_result.is_none(), "User should not pay on price drop");

    let lsp_result = check_stability(&lsp_node, &mut lsp_sc, price1);
    assert!(lsp_result.is_some(), "LSP should pay on price drop");
    let info = lsp_result.unwrap();
    println!("[cycle1] LSP paid {} msats", info.amount_msat);

    expect_payment_successful_event!(lsp_node);
    expect_payment_received_event!(user_node);

    // Update balances after payment
    update_balances(&user_node, &mut user_sc);
    update_balances(&lsp_node, &mut lsp_sc);
    // Reset backing_sats to new equilibrium (as the real app does)
    user_sc.backing_sats = (expected_usd / price1 * 100_000_000.0) as u64;
    lsp_sc.backing_sats = user_sc.backing_sats;

    print_stable_channel("User (after cycle 1)", &user_sc);

    // --- Cycle 2: Price recovers 3% ($95k -> $97.85k) — User pays ---
    println!("\n--- Cycle 2: Price rise 3% ---");
    let price2 = 97_850.0;
    set_mock_price(price2);

    let user_result = check_stability(&user_node, &mut user_sc, price2);
    assert!(user_result.is_some(), "User should pay on price rise");
    let info = user_result.unwrap();
    println!("[cycle2] User paid {} msats", info.amount_msat);

    expect_payment_successful_event!(user_node);
    expect_payment_received_event!(lsp_node);

    update_balances(&user_node, &mut user_sc);
    update_balances(&lsp_node, &mut lsp_sc);
    user_sc.backing_sats = (expected_usd / price2 * 100_000_000.0) as u64;
    lsp_sc.backing_sats = user_sc.backing_sats;

    print_stable_channel("User (after cycle 2)", &user_sc);

    // --- Cycle 3: Tiny price change (within 0.1% threshold) — No payment ---
    println!("\n--- Cycle 3: Tiny price change (within threshold) ---");
    let price3 = 97_900.0; // ~0.05% change from $97,850
    set_mock_price(price3);

    let user_result = check_stability(&user_node, &mut user_sc, price3);
    assert!(user_result.is_none(), "No payment for tiny price change");
    let lsp_result = check_stability(&lsp_node, &mut lsp_sc, price3);
    assert!(lsp_result.is_none(), "No payment for tiny price change");
    println!("[cycle3] Both sides: STABLE (correct)");

    // --- Verify final balance conservation ---
    let user_channels = user_node.list_channels();
    let ch = user_channels.first().unwrap();
    let user_sats =
        (ch.outbound_capacity_msat / 1000) + ch.unspendable_punishment_reserve.unwrap_or(0);
    let counterparty_sats = ch.channel_value_sats - user_sats;
    assert_eq!(user_sats + counterparty_sats, ch.channel_value_sats);

    println!("\n[PASS] test_multiple_stability_cycles");

    user_node.stop().unwrap();
    lsp_node.stop().unwrap();
}

// ==================================================================
// Test 4: JIT Onboarding (Funder -> User via LSP)
// ==================================================================

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires bitcoind + electrs (run with --ignored)"]
async fn test_jit_onboarding() {
    println!("\n========================================");
    println!("TEST: JIT Onboarding");
    println!("========================================\n");

    let (bitcoind, electrsd) = setup_bitcoind_and_electrsd();

    // Create all 3 nodes
    let lsp_node = setup_lsp_node(&electrsd);
    let lsp_pubkey = lsp_node.node_id();
    let lsp_addr = lsp_node.listening_addresses().unwrap().first().unwrap().clone();

    let user_node = setup_user_node(&electrsd, lsp_pubkey, lsp_addr.clone());
    let funder_node = setup_node(&electrsd, "funder", true);

    // Fund LSP and Funder
    let addr_lsp = lsp_node.onchain_payment().new_address().unwrap();
    let addr_funder = funder_node.onchain_payment().new_address().unwrap();
    premine_and_distribute_funds(
        &bitcoind.client,
        &electrsd.client,
        vec![addr_lsp, addr_funder],
        Amount::from_sat(2_125_000),
    )
    .await;
    lsp_node.sync_wallets().unwrap();
    funder_node.sync_wallets().unwrap();

    // Open channel: Funder <-> LSP (so funder can route payments through LSP)
    let funder_funding = 2_000_000;
    let funder_push = (funder_funding / 2) * 1000;
    open_channel_and_confirm(
        &funder_node,
        &lsp_node,
        funder_funding,
        Some(funder_push),
        &bitcoind.client,
        &electrsd,
    )
    .await;

    println!("\n--- Funder↔LSP channel ready ---");
    print_channel_balances("Funder", &funder_node);
    print_channel_balances("LSP", &lsp_node);

    // User creates a JIT invoice
    let description = ldk_node::lightning_invoice::Bolt11InvoiceDescription::Direct(
        ldk_node::lightning_invoice::Description::new("Regtest onboarding".to_string()).unwrap(),
    );
    let invoice = user_node
        .bolt11_payment()
        .receive_variable_amount_via_jit_channel(&description, 3600, Some(10_000_000))
        .expect("Failed to create JIT invoice");

    println!("[jit] User created JIT invoice");

    // Funder pays the invoice with 500k sats
    let payment_amount_msat = 500_000 * 1000; // 500k sats in msats
    funder_node
        .bolt11_payment()
        .send_using_amount(&invoice, payment_amount_msat, None)
        .expect("Funder failed to pay JIT invoice");

    println!("[jit] Funder sent payment of {} msats", payment_amount_msat);

    // Wait for events — this may take a bit as LSP opens the JIT channel
    // LSP should get channel pending + payment forwarded
    // User should get channel pending + channel ready + payment received

    // Funder gets PaymentSuccessful
    expect_payment_successful_event!(funder_node);
    println!("[event] Funder: PaymentSuccessful");

    // Drain user events until we see PaymentReceived
    let mut got_payment = false;
    for _ in 0..10 {
        match user_node.next_event() {
            Some(ldk_node::Event::PaymentReceived { amount_msat, .. }) => {
                println!("[event] User: PaymentReceived {} msats", amount_msat);
                user_node.event_handled().unwrap();
                got_payment = true;
                break;
            }
            Some(other) => {
                println!("[event] User: {:?}", other);
                user_node.event_handled().unwrap();
            }
            None => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    }
    assert!(got_payment, "User should have received the JIT payment");

    // Verify user now has a channel
    user_node.sync_wallets().unwrap();
    let user_channels = user_node.list_channels();
    assert!(!user_channels.is_empty(), "User should have a channel after JIT");

    print_channel_balances("User (after JIT)", &user_node);

    let user_lightning_sats = user_node.list_balances().total_lightning_balance_sats;
    println!("[jit] User lightning balance: {} sats", user_lightning_sats);
    assert!(
        user_lightning_sats > 0,
        "User should have lightning balance after JIT onboarding"
    );

    println!("\n[PASS] test_jit_onboarding");

    user_node.stop().unwrap();
    lsp_node.stop().unwrap();
    funder_node.stop().unwrap();
}

// ==================================================================
// Test 5: Outgoing payment from stabilized channel
// ==================================================================

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires bitcoind + electrs (run with --ignored)"]
async fn test_outgoing_payment_deducts_from_stable() {
    println!("\n=====================================================");
    println!("TEST: Outgoing payment deducts from stable balance");
    println!("=====================================================\n");

    let (bitcoind, electrsd) = setup_bitcoind_and_electrsd();

    // Create 3 nodes
    let lsp_node = setup_lsp_node(&electrsd);
    let lsp_pubkey = lsp_node.node_id();
    let lsp_addr = lsp_node.listening_addresses().unwrap().first().unwrap().clone();

    let user_node = setup_user_node(&electrsd, lsp_pubkey, lsp_addr);
    let funder_node = setup_node(&electrsd, "funder", true);

    // Fund all 3 — LSP needs extra for opening 2 channels
    let addr_lsp = lsp_node.onchain_payment().new_address().unwrap();
    let addr_user = user_node.onchain_payment().new_address().unwrap();
    let addr_funder = funder_node.onchain_payment().new_address().unwrap();
    premine_and_distribute_funds(
        &bitcoind.client,
        &electrsd.client,
        vec![addr_lsp, addr_user, addr_funder],
        Amount::from_sat(4_250_000),
    )
    .await;
    lsp_node.sync_wallets().unwrap();
    user_node.sync_wallets().unwrap();
    funder_node.sync_wallets().unwrap();

    // Channel 1: LSP -> User (stable channel, balanced with push)
    let stable_funding = 2_000_000;
    let stable_push = (stable_funding / 2) * 1000; // 1M sats each side
    open_channel_and_confirm(
        &lsp_node,
        &user_node,
        stable_funding,
        Some(stable_push),
        &bitcoind.client,
        &electrsd,
    )
    .await;

    // Channel 2: LSP -> Funder (LSP has all balance = outbound to funder)
    let funder_funding = 2_000_000;
    open_channel_and_confirm(
        &lsp_node,
        &funder_node,
        funder_funding,
        None, // no push — LSP keeps all balance (outbound capacity to Funder)
        &bitcoind.client,
        &electrsd,
    )
    .await;

    println!("\n--- Both channels ready ---");
    print_channel_balances("User", &user_node);
    print_channel_balances("LSP", &lsp_node);
    print_channel_balances("Funder", &funder_node);

    // --- Set up stable position ---
    let price = 100_000.0; // $100k/BTC
    set_mock_price(price);

    // User has ~1M sats = $1000 at $100k. Stabilize ALL of it ($1000).
    // This means user has no native BTC — everything is stable.
    let expected_usd = 1000.0;
    let mut user_sc = create_stable_channel(
        &user_node,
        lsp_pubkey,
        true,
        expected_usd,
        price,
    );

    let (ok, _) = update_balances(&user_node, &mut user_sc);
    assert!(ok);
    print_stable_channel("User (before payment)", &user_sc);

    let user_sats_before = user_sc.stable_receiver_btc.sats;
    let backing_sats_before = user_sc.backing_sats;
    println!("\n[balance] User sats before: {}", user_sats_before);
    println!("[balance] Backing sats before: {}", backing_sats_before);
    println!("[balance] Expected USD before: {}", user_sc.expected_usd);

    // --- Funder creates bolt11 invoice ---
    let payment_sats: u64 = 100_000; // 100k sats = $100 at $100k
    let payment_msat = payment_sats * 1000;
    let description = ldk_node::lightning_invoice::Bolt11InvoiceDescription::Direct(
        ldk_node::lightning_invoice::Description::new("test payment".to_string()).unwrap(),
    );
    let invoice = funder_node
        .bolt11_payment()
        .receive(payment_msat, &description.into(), 3600)
        .expect("Funder failed to create invoice");

    println!("\n[send] User sending {} sats to Funder...", payment_sats);

    // --- User pays the invoice (routes: User -> LSP -> Funder) ---
    let payment_id = user_node
        .bolt11_payment()
        .send(&invoice, None)
        .expect("User failed to send payment");

    // Wait for payment events
    expect_payment_successful_event!(user_node);
    println!("[event] User: PaymentSuccessful");

    expect_payment_received_event!(funder_node);
    println!("[event] Funder: PaymentReceived");

    // --- Refresh balances and check deduction ---
    let (ok, _) = update_balances(&user_node, &mut user_sc);
    assert!(ok);
    print_stable_channel("User (after payment)", &user_sc);

    let user_sats_after = user_sc.stable_receiver_btc.sats;
    let sats_spent = user_sats_before.saturating_sub(user_sats_after);
    println!("\n[balance] User sats after: {}", user_sats_after);
    println!("[balance] Sats spent: {} (expected ~{})", sats_spent, payment_sats);

    // Sats spent should be approximately the payment amount (may differ slightly due to fees)
    assert!(
        sats_spent >= payment_sats,
        "User should have spent at least {} sats, but only spent {}",
        payment_sats, sats_spent
    );
    assert!(
        sats_spent <= payment_sats + 5000, // allow up to 5k sats for routing fees
        "User spent way too much: {} sats (expected ~{})",
        sats_spent, payment_sats
    );

    // --- Reconcile using the shared function (same code as user.rs handler) ---
    let old_expected = user_sc.expected_usd.0;
    if let Some(usd_deducted) = reconcile_outgoing(&mut user_sc, price) {
        println!("\n[reconcile] USD deducted: ${:.2}", usd_deducted);
        println!("[reconcile] Expected USD: ${:.2} -> ${:.2}", old_expected, user_sc.expected_usd.0);
    }

    // --- Verify the deduction ---
    let expected_usd_after = user_sc.expected_usd.0;
    let expected_deduction_usd = payment_sats as f64 / 100_000_000.0 * price; // $100

    println!("\n[verify] Expected USD after reconciliation: ${:.2}", expected_usd_after);
    println!("[verify] Expected deduction: ~${:.2}", expected_deduction_usd);

    // Since user had no native BTC, the full payment should come from stable
    // expected_usd should decrease by approximately $100 (100k sats at $100k)
    let actual_deduction = expected_usd - expected_usd_after;
    println!("[verify] Actual deduction from stable: ${:.2}", actual_deduction);

    assert!(
        actual_deduction > expected_deduction_usd * 0.95,
        "Stable deduction ${:.2} should be close to ${:.2}",
        actual_deduction, expected_deduction_usd
    );
    assert!(
        actual_deduction < expected_deduction_usd * 1.10, // allow 10% for fees
        "Stable deduction ${:.2} is too large vs expected ${:.2}",
        actual_deduction, expected_deduction_usd
    );

    // After reconciliation, backing_sats should match the new expected_usd
    let expected_backing = (expected_usd_after / price * 100_000_000.0) as u64;
    assert_eq!(
        user_sc.backing_sats, expected_backing,
        "Backing sats should match new expected_usd / price"
    );

    // Verify the stable position is still approximately right
    println!("\n[final] Stable position: ${:.2} (was ${:.2})", expected_usd_after, expected_usd);
    println!("[final] Backing sats: {} (was {})", user_sc.backing_sats, backing_sats_before);

    // Now run check_stability — should be close to equilibrium
    let result = check_stability(&user_node, &mut user_sc, price);
    println!("[final] User check_stability: {:?}", result.as_ref().map(|i| i.amount_msat));

    // =================================================================
    // LSP side: verify the LSP also reconciles correctly
    // =================================================================
    println!("\n--- LSP Side ---");

    // LSP gets a PaymentForwarded event (User -> LSP -> Funder)
    // Drain LSP events to find it
    let mut forwarded_msat: u64 = 0;
    for _ in 0..5 {
        match lsp_node.next_event() {
            Some(ldk_node::Event::PaymentForwarded {
                prev_channel_id,
                next_channel_id,
                outbound_amount_forwarded_msat,
                total_fee_earned_msat,
                ..
            }) => {
                forwarded_msat = outbound_amount_forwarded_msat.unwrap_or(0);
                let fee_msat = total_fee_earned_msat.unwrap_or(0);
                println!(
                    "[event] LSP: PaymentForwarded {} msats (fee {} msats) prev={} next={}",
                    forwarded_msat, fee_msat, prev_channel_id, next_channel_id
                );
                lsp_node.event_handled().unwrap();
                break;
            }
            Some(other) => {
                println!("[event] LSP: {:?}", other);
                lsp_node.event_handled().unwrap();
            }
            None => {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }
    assert!(forwarded_msat > 0, "LSP should have received PaymentForwarded event");

    // Create LSP-side StableChannel and update balances
    let mut lsp_sc = create_stable_channel(
        &lsp_node,
        user_node.node_id(),
        false, // LSP is stable provider
        expected_usd, // original expected_usd (before user's reconciliation)
        price,
    );

    let (ok, _) = update_balances(&lsp_node, &mut lsp_sc);
    assert!(ok);
    print_stable_channel("LSP (after user payment)", &lsp_sc);

    // LSP sees user's balance (stable_receiver_btc from LSP perspective = user's sats)
    let user_sats_from_lsp_view = lsp_sc.stable_receiver_btc.sats;
    println!("\n[lsp] User's balance as seen by LSP: {} sats", user_sats_from_lsp_view);
    println!("[lsp] LSP's balance (provider): {} sats", lsp_sc.stable_provider_btc.sats);

    // The LSP should see the user's balance decreased by ~100k sats
    // LSP's backing_sats (1M) > user's actual sats (~900k) -> overflow
    assert!(
        lsp_sc.backing_sats > user_sats_from_lsp_view,
        "LSP backing_sats ({}) should exceed user's actual sats ({})",
        lsp_sc.backing_sats, user_sats_from_lsp_view
    );

    // LSP reconciliation using the shared function (same code as lsp_backend.rs handler)
    let total_forwarded_sats = forwarded_msat / 1000;
    let old_lsp_expected = lsp_sc.expected_usd.0;
    if let Some(usd_deducted) = reconcile_forwarded(&mut lsp_sc, user_sats_from_lsp_view, total_forwarded_sats, price) {
        println!("\n[lsp reconcile] USD deducted: ${:.2}", usd_deducted);
        println!("[lsp reconcile] Expected USD: ${:.2} -> ${:.2}", old_lsp_expected, lsp_sc.expected_usd.0);
    }

    // Both sides should now agree on expected_usd (approximately)
    let user_expected = user_sc.expected_usd.0;
    let lsp_expected = lsp_sc.expected_usd.0;
    let diff = (user_expected - lsp_expected).abs();
    println!("\n[verify] User expected_usd: ${:.2}", user_expected);
    println!("[verify] LSP expected_usd:  ${:.2}", lsp_expected);
    println!("[verify] Difference:        ${:.4}", diff);

    assert!(
        diff < 1.0,
        "User and LSP should agree on expected_usd (diff=${:.2})",
        diff
    );

    // LSP check_stability should also be near equilibrium
    let result = check_stability(&lsp_node, &mut lsp_sc, price);
    println!("[final] LSP check_stability: {:?}", result.as_ref().map(|i| i.amount_msat));

    println!("\n[PASS] test_outgoing_payment_deducts_from_stable");

    user_node.stop().unwrap();
    lsp_node.stop().unwrap();
    funder_node.stop().unwrap();
}

// ==================================================================
// Test 6: Buy BTC — reduces stable position, stability still works
// ==================================================================

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires bitcoind + electrs (run with --ignored)"]
async fn test_buy_btc_reduces_stable_position() {
    println!("\n=====================================================");
    println!("TEST: Buy BTC reduces stable position");
    println!("=====================================================\n");

    let (bitcoind, electrsd) = setup_bitcoind_and_electrsd();

    let lsp_node = setup_lsp_node(&electrsd);
    let lsp_pubkey = lsp_node.node_id();
    let lsp_addr = lsp_node.listening_addresses().unwrap().first().unwrap().clone();
    let user_node = setup_user_node(&electrsd, lsp_pubkey, lsp_addr);

    // Fund and open channel
    let addr_lsp = lsp_node.onchain_payment().new_address().unwrap();
    let addr_user = user_node.onchain_payment().new_address().unwrap();
    premine_and_distribute_funds(
        &bitcoind.client,
        &electrsd.client,
        vec![addr_lsp, addr_user],
        Amount::from_sat(2_125_000),
    )
    .await;
    lsp_node.sync_wallets().unwrap();
    user_node.sync_wallets().unwrap();

    let funding_sats = 2_000_000;
    let push_msat = (funding_sats / 2) * 1000; // 1M sats each
    open_channel_and_confirm(
        &lsp_node,
        &user_node,
        funding_sats,
        Some(push_msat),
        &bitcoind.client,
        &electrsd,
    )
    .await;

    // --- Set up stable position: $500 out of ~$1000 total ---
    let price = 100_000.0; // $100k/BTC
    set_mock_price(price);
    let initial_expected_usd = 500.0;

    let mut user_sc = create_stable_channel(
        &user_node, lsp_pubkey, true, initial_expected_usd, price,
    );
    let mut lsp_sc = create_stable_channel(
        &lsp_node, user_node.node_id(), false, initial_expected_usd, price,
    );

    let (ok, _) = update_balances(&user_node, &mut user_sc);
    assert!(ok);
    let (ok, _) = update_balances(&lsp_node, &mut lsp_sc);
    assert!(ok);

    print_stable_channel("User (before buy)", &user_sc);
    println!("[state] User native BTC: ~${:.2}", user_sc.stable_receiver_usd.0 - initial_expected_usd);

    // Verify equilibrium before trade
    let result = check_stability(&user_node, &mut user_sc, price);
    assert!(result.is_none(), "Should be stable before trade");

    // ===== EXECUTE BUY: User buys $200 worth of BTC =====
    let buy_amount_usd = 200.0;
    let fee_usd = buy_amount_usd * 0.01; // 1% fee = $2
    let _net_btc_usd = buy_amount_usd - fee_usd; // $198 of BTC gained

    // Buying BTC means reducing expected_usd by full amount
    let new_expected_usd = initial_expected_usd - buy_amount_usd; // $500 -> $300
    println!("\n[buy] Buying ${:.2} BTC (fee: ${:.2})", buy_amount_usd, fee_usd);
    println!("[buy] expected_usd: ${:.2} -> ${:.2}", initial_expected_usd, new_expected_usd);

    // Send the fee as keysend to LSP (this is what send_trade does)
    let fee_btc = fee_usd / price;
    let fee_sats = (fee_btc * 100_000_000.0) as u64;
    let fee_msats = (fee_sats * 1000).max(1);

    user_node
        .spontaneous_payment()
        .send(fee_msats, lsp_pubkey, None)
        .expect("Failed to send trade fee");

    // Handle payment events
    expect_payment_successful_event!(user_node);
    println!("[event] User: PaymentSuccessful (trade fee)");
    expect_payment_received_event!(lsp_node);
    println!("[event] LSP: PaymentReceived (trade fee)");

    // Apply trade on both sides (same shared code as app handlers)
    apply_trade(&mut user_sc, new_expected_usd, price);
    apply_trade(&mut lsp_sc, new_expected_usd, price);

    // Refresh balances after payment
    let (ok, _) = update_balances(&user_node, &mut user_sc);
    assert!(ok);
    let (ok, _) = update_balances(&lsp_node, &mut lsp_sc);
    assert!(ok);

    print_stable_channel("User (after buy)", &user_sc);
    print_stable_channel("LSP (after buy)", &lsp_sc);

    // --- Verify the buy ---
    assert!(
        (user_sc.expected_usd.0 - 300.0).abs() < 0.01,
        "Expected USD should be $300 after buying $200, got ${:.2}",
        user_sc.expected_usd.0
    );
    assert_eq!(
        user_sc.backing_sats,
        (300.0 / price * 100_000_000.0) as u64,
        "Backing sats should match $300 at $100k"
    );

    // Both sides should agree
    assert!(
        (user_sc.expected_usd.0 - lsp_sc.expected_usd.0).abs() < 0.01,
        "Both sides should agree on expected_usd"
    );

    // --- Verify stability works at same price (should be STABLE) ---
    let result = check_stability(&user_node, &mut user_sc, price);
    assert!(result.is_none(), "Should be stable right after buy at same price");
    let result = check_stability(&lsp_node, &mut lsp_sc, price);
    assert!(result.is_none(), "LSP should also be stable right after buy");
    println!("\n[check] Both sides STABLE at same price after buy (correct)");

    // --- Verify stability works after price change ---
    // Drop price 10%: $100k -> $90k. The $300 stable position should trigger LSP payment.
    let drop_price = 90_000.0;
    set_mock_price(drop_price);

    let user_result = check_stability(&user_node, &mut user_sc, drop_price);
    assert!(user_result.is_none(), "User should not pay on price drop (CHECK_ONLY)");

    let lsp_result = check_stability(&lsp_node, &mut lsp_sc, drop_price);
    assert!(lsp_result.is_some(), "LSP should pay to stabilize $300 after price drop");

    let payment_info = lsp_result.unwrap();
    println!(
        "[check] LSP sent stability payment: {} msats for $300 position at $90k",
        payment_info.amount_msat
    );

    // The payment should be proportional to $300 position (not the old $500)
    // At $100k, 300k backing sats. At $90k, those sats are worth $270. Drift = $30.
    // Expected payment: $30 / $90k * 1e8 * 1000 = ~33,333,333 msats
    let expected_payment_msats = (30.0 / drop_price * 100_000_000.0 * 1000.0) as u64;
    let tolerance = expected_payment_msats / 10; // 10% tolerance
    assert!(
        payment_info.amount_msat > expected_payment_msats - tolerance
            && payment_info.amount_msat < expected_payment_msats + tolerance,
        "Payment {} msats should be ~{} msats (for $300 position, not $500)",
        payment_info.amount_msat, expected_payment_msats
    );

    // Drain events
    expect_payment_successful_event!(lsp_node);
    expect_payment_received_event!(user_node);

    println!("\n[PASS] test_buy_btc_reduces_stable_position");

    user_node.stop().unwrap();
    lsp_node.stop().unwrap();
}

// ==================================================================
// Test 7: Sell BTC — increases stable position, stability still works
// ==================================================================

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires bitcoind + electrs (run with --ignored)"]
async fn test_sell_btc_increases_stable_position() {
    println!("\n=====================================================");
    println!("TEST: Sell BTC increases stable position");
    println!("=====================================================\n");

    let (bitcoind, electrsd) = setup_bitcoind_and_electrsd();

    let lsp_node = setup_lsp_node(&electrsd);
    let lsp_pubkey = lsp_node.node_id();
    let lsp_addr = lsp_node.listening_addresses().unwrap().first().unwrap().clone();
    let user_node = setup_user_node(&electrsd, lsp_pubkey, lsp_addr);

    // Fund and open channel
    let addr_lsp = lsp_node.onchain_payment().new_address().unwrap();
    let addr_user = user_node.onchain_payment().new_address().unwrap();
    premine_and_distribute_funds(
        &bitcoind.client,
        &electrsd.client,
        vec![addr_lsp, addr_user],
        Amount::from_sat(2_125_000),
    )
    .await;
    lsp_node.sync_wallets().unwrap();
    user_node.sync_wallets().unwrap();

    let funding_sats = 2_000_000;
    let push_msat = (funding_sats / 2) * 1000;
    open_channel_and_confirm(
        &lsp_node,
        &user_node,
        funding_sats,
        Some(push_msat),
        &bitcoind.client,
        &electrsd,
    )
    .await;

    // --- Set up stable position: $300 out of ~$1000 total ---
    // User has ~$700 native BTC + $300 stable
    let price = 100_000.0;
    set_mock_price(price);
    let initial_expected_usd = 300.0;

    let mut user_sc = create_stable_channel(
        &user_node, lsp_pubkey, true, initial_expected_usd, price,
    );
    let mut lsp_sc = create_stable_channel(
        &lsp_node, user_node.node_id(), false, initial_expected_usd, price,
    );

    let (ok, _) = update_balances(&user_node, &mut user_sc);
    assert!(ok);
    let (ok, _) = update_balances(&lsp_node, &mut lsp_sc);
    assert!(ok);

    let native_btc_usd = user_sc.stable_receiver_usd.0 - initial_expected_usd;
    print_stable_channel("User (before sell)", &user_sc);
    println!("[state] User native BTC: ~${:.2}", native_btc_usd);

    // Verify equilibrium before trade
    let result = check_stability(&user_node, &mut user_sc, price);
    assert!(result.is_none(), "Should be stable before trade");

    // ===== EXECUTE SELL: User sells $200 worth of BTC into stable =====
    let sell_amount_usd = 200.0;
    let fee_usd = sell_amount_usd * 0.01; // 1% fee = $2
    let net_amount = sell_amount_usd - fee_usd; // $198 added to stable

    // Selling BTC means increasing expected_usd by net amount
    let new_expected_usd = initial_expected_usd + net_amount; // $300 -> $498
    println!("\n[sell] Selling ${:.2} BTC into stable (fee: ${:.2})", sell_amount_usd, fee_usd);
    println!("[sell] expected_usd: ${:.2} -> ${:.2}", initial_expected_usd, new_expected_usd);

    // Verify user has enough native BTC to sell
    assert!(
        sell_amount_usd <= native_btc_usd,
        "Can't sell ${:.2} — only ${:.2} native BTC available",
        sell_amount_usd, native_btc_usd
    );

    // Send the fee as keysend to LSP
    let fee_btc = fee_usd / price;
    let fee_sats = (fee_btc * 100_000_000.0) as u64;
    let fee_msats = (fee_sats * 1000).max(1);

    user_node
        .spontaneous_payment()
        .send(fee_msats, lsp_pubkey, None)
        .expect("Failed to send trade fee");

    expect_payment_successful_event!(user_node);
    println!("[event] User: PaymentSuccessful (trade fee)");
    expect_payment_received_event!(lsp_node);
    println!("[event] LSP: PaymentReceived (trade fee)");

    // Apply trade on both sides (same shared code as app handlers)
    apply_trade(&mut user_sc, new_expected_usd, price);
    apply_trade(&mut lsp_sc, new_expected_usd, price);

    // Refresh balances
    let (ok, _) = update_balances(&user_node, &mut user_sc);
    assert!(ok);
    let (ok, _) = update_balances(&lsp_node, &mut lsp_sc);
    assert!(ok);

    print_stable_channel("User (after sell)", &user_sc);
    print_stable_channel("LSP (after sell)", &lsp_sc);

    // --- Verify the sell ---
    assert!(
        (user_sc.expected_usd.0 - 498.0).abs() < 0.01,
        "Expected USD should be $498 after selling $200 (net $198), got ${:.2}",
        user_sc.expected_usd.0
    );
    assert_eq!(
        user_sc.backing_sats,
        (498.0 / price * 100_000_000.0) as u64,
        "Backing sats should match $498 at $100k"
    );

    // Both sides agree
    assert!(
        (user_sc.expected_usd.0 - lsp_sc.expected_usd.0).abs() < 0.01,
        "Both sides should agree on expected_usd"
    );

    // Native BTC should have decreased
    let new_native_btc_usd = user_sc.stable_receiver_usd.0 - user_sc.expected_usd.0;
    println!("\n[verify] Native BTC: ${:.2} -> ${:.2}", native_btc_usd, new_native_btc_usd);
    assert!(
        new_native_btc_usd < native_btc_usd,
        "Native BTC should decrease after selling into stable"
    );

    // --- Verify stability at same price (STABLE) ---
    let result = check_stability(&user_node, &mut user_sc, price);
    assert!(result.is_none(), "Should be stable right after sell at same price");
    let result = check_stability(&lsp_node, &mut lsp_sc, price);
    assert!(result.is_none(), "LSP should also be stable right after sell");
    println!("[check] Both sides STABLE at same price after sell (correct)");

    // --- Verify stability after price rise ---
    // Rise 10%: $100k -> $110k. The $498 stable position should trigger user payment.
    let rise_price = 110_000.0;
    set_mock_price(rise_price);

    // User-side: receiver is above expected, should PAY
    let user_result = check_stability(&user_node, &mut user_sc, rise_price);
    assert!(user_result.is_some(), "User should pay when price rises after sell");

    let payment_info = user_result.unwrap();
    println!(
        "[check] User sent stability payment: {} msats for $498 position at $110k",
        payment_info.amount_msat
    );

    // At $100k backing had 498k sats. At $110k, those sats worth $547.80. Drift = $49.80.
    // Expected payment: $49.80 / $110k * 1e8 * 1000 = ~45,272,727 msats
    let drift_usd = (498_000.0 / 100_000_000.0 * rise_price) - 498.0;
    let expected_payment_msats = (drift_usd / rise_price * 100_000_000.0 * 1000.0) as u64;
    let tolerance = expected_payment_msats / 5; // 20% tolerance (fees may vary)
    assert!(
        payment_info.amount_msat > expected_payment_msats.saturating_sub(tolerance)
            && payment_info.amount_msat < expected_payment_msats + tolerance,
        "Payment {} msats should be ~{} msats (for $498 position, not $300)",
        payment_info.amount_msat, expected_payment_msats
    );

    expect_payment_successful_event!(user_node);
    expect_payment_received_event!(lsp_node);

    // --- Now do a sell AFTER a price change to verify combined flow ---
    // Price is $110k. User sells another $100 BTC into stable.
    println!("\n--- Second sell at new price ---");
    let sell2_amount = 100.0;
    let fee2 = sell2_amount * 0.01; // $1
    let net2 = sell2_amount - fee2; // $99
    let pre_sell2_expected = user_sc.expected_usd.0;
    let new_expected_usd2 = pre_sell2_expected + net2;
    println!("[sell2] expected_usd: ${:.2} -> ${:.2} at price ${:.2}",
        pre_sell2_expected, new_expected_usd2, rise_price);

    // Send fee
    let fee2_btc = fee2 / rise_price;
    let fee2_sats = (fee2_btc * 100_000_000.0) as u64;
    let fee2_msats = (fee2_sats * 1000).max(1);

    user_node
        .spontaneous_payment()
        .send(fee2_msats, lsp_pubkey, None)
        .expect("Failed to send trade fee");
    expect_payment_successful_event!(user_node);
    expect_payment_received_event!(lsp_node);

    // Apply trade on both sides at the NEW price
    apply_trade(&mut user_sc, new_expected_usd2, rise_price);
    apply_trade(&mut lsp_sc, new_expected_usd2, rise_price);

    update_balances(&user_node, &mut user_sc);
    update_balances(&lsp_node, &mut lsp_sc);

    print_stable_channel("User (after sell2)", &user_sc);

    // Should be stable at current price
    let result = check_stability(&user_node, &mut user_sc, rise_price);
    assert!(result.is_none(), "Should be stable after second sell at same price");
    let result = check_stability(&lsp_node, &mut lsp_sc, rise_price);
    assert!(result.is_none(), "LSP should be stable after second sell");
    println!("[check] Both sides STABLE after second sell (correct)");

    println!("\n[PASS] test_sell_btc_increases_stable_position");

    user_node.stop().unwrap();
    lsp_node.stop().unwrap();
}

// ==================================================================
// Test 8: Bolt11 receive — stable position preserved, native increases
// ==================================================================

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires bitcoind + electrs (run with --ignored)"]
async fn test_bolt11_receive_preserves_stable() {
    println!("\n=====================================================");
    println!("TEST: Bolt11 receive preserves stable position");
    println!("=====================================================\n");

    let (bitcoind, electrsd) = setup_bitcoind_and_electrsd();

    // 3-node setup: Funder → LSP → User
    let lsp_node = setup_lsp_node(&electrsd);
    let lsp_pubkey = lsp_node.node_id();
    let lsp_addr = lsp_node.listening_addresses().unwrap().first().unwrap().clone();
    let user_node = setup_user_node(&electrsd, lsp_pubkey, lsp_addr);
    let funder_node = setup_node(&electrsd, "funder", true);

    // Fund all nodes
    let addr_lsp = lsp_node.onchain_payment().new_address().unwrap();
    let addr_user = user_node.onchain_payment().new_address().unwrap();
    let addr_funder = funder_node.onchain_payment().new_address().unwrap();
    premine_and_distribute_funds(
        &bitcoind.client,
        &electrsd.client,
        vec![addr_lsp, addr_user, addr_funder],
        Amount::from_sat(4_250_000),
    )
    .await;
    lsp_node.sync_wallets().unwrap();
    user_node.sync_wallets().unwrap();
    funder_node.sync_wallets().unwrap();

    // Channel 1: LSP → User (balanced, LSP has outbound to route to User)
    let stable_funding = 2_000_000;
    let stable_push = (stable_funding / 2) * 1000;
    open_channel_and_confirm(
        &lsp_node, &user_node, stable_funding, Some(stable_push),
        &bitcoind.client, &electrsd,
    ).await;

    // Channel 2: Funder → LSP (Funder has outbound to send through LSP)
    let funder_funding = 2_000_000;
    open_channel_and_confirm(
        &funder_node, &lsp_node, funder_funding, None,
        &bitcoind.client, &electrsd,
    ).await;

    // --- Set up stable position: $500 stable out of ~$1000 ---
    let price = 100_000.0;
    set_mock_price(price);
    let expected_usd = 500.0;

    let mut user_sc = create_stable_channel(
        &user_node, lsp_pubkey, true, expected_usd, price,
    );
    let mut lsp_sc = create_stable_channel(
        &lsp_node, user_node.node_id(), false, expected_usd, price,
    );

    let (ok, _) = update_balances(&user_node, &mut user_sc);
    assert!(ok);
    print_stable_channel("User (before receive)", &user_sc);

    let user_sats_before = user_sc.stable_receiver_btc.sats;
    let backing_before = user_sc.backing_sats;
    println!("[state] User sats: {}, backing: {}, expected_usd: ${:.2}",
        user_sats_before, backing_before, user_sc.expected_usd.0);

    // --- User creates bolt11 invoice ---
    let receive_sats: u64 = 50_000;
    let receive_msat = receive_sats * 1000;
    let description = ldk_node::lightning_invoice::Bolt11InvoiceDescription::Direct(
        ldk_node::lightning_invoice::Description::new("test receive".to_string()).unwrap(),
    );
    let invoice = user_node
        .bolt11_payment()
        .receive(receive_msat, &description.into(), 3600)
        .expect("User failed to create invoice");

    println!("\n[receive] User invoice for {} sats", receive_sats);

    // --- Funder pays the invoice (Funder → LSP → User) ---
    funder_node
        .bolt11_payment()
        .send(&invoice, None)
        .expect("Funder failed to pay invoice");

    // Handle events
    expect_payment_successful_event!(funder_node);
    println!("[event] Funder: PaymentSuccessful");
    expect_payment_received_event!(user_node);
    println!("[event] User: PaymentReceived");

    // Allow channel state to settle after payment
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    user_node.sync_wallets().unwrap();

    // --- Simulate PaymentReceived handler: recalculate backing_sats ---
    {
        let (ok, _) = update_balances(&user_node, &mut user_sc);
        assert!(ok);
        reconcile_incoming(&mut user_sc);
    }

    print_stable_channel("User (after receive)", &user_sc);

    let user_sats_after = user_sc.stable_receiver_btc.sats;
    let sats_gained = user_sats_after.saturating_sub(user_sats_before);

    println!("\n[verify] User sats: {} -> {} (gained {})", user_sats_before, user_sats_after, sats_gained);
    println!("[verify] expected_usd: ${:.2} (unchanged)", user_sc.expected_usd.0);
    println!("[verify] backing_sats: {} (was {})", user_sc.backing_sats, backing_before);

    // Stable position should be UNCHANGED
    assert!(
        (user_sc.expected_usd.0 - expected_usd).abs() < 0.01,
        "expected_usd should stay at ${:.2}, got ${:.2}",
        expected_usd, user_sc.expected_usd.0
    );

    // backing_sats should be the same (same expected_usd, same price)
    assert_eq!(
        user_sc.backing_sats, backing_before,
        "backing_sats should be unchanged after receive"
    );

    // User should have gained sats
    assert!(
        sats_gained >= receive_sats - 100, // small tolerance for fees
        "User should have gained ~{} sats, got {}",
        receive_sats, sats_gained
    );

    // Native BTC should have increased
    let native_usd_after = user_sc.stable_receiver_usd.0 - user_sc.expected_usd.0;
    let native_usd_before = user_sats_before as f64 / 100_000_000.0 * price - expected_usd;
    println!("[verify] Native BTC: ${:.2} -> ${:.2}", native_usd_before, native_usd_after);
    assert!(
        native_usd_after > native_usd_before,
        "Native BTC USD should increase after receive"
    );

    // Stability check: should be at equilibrium
    let result = check_stability(&user_node, &mut user_sc, price);
    assert!(result.is_none(), "Should be stable after receiving payment");
    println!("[check] User STABLE after bolt11 receive (correct)");

    // LSP side: drain forwarded event, verify stable unchanged
    for _ in 0..5 {
        match lsp_node.next_event() {
            Some(ldk_node::Event::PaymentForwarded { .. }) => {
                println!("[event] LSP: PaymentForwarded");
                lsp_node.event_handled().unwrap();
                break;
            }
            Some(other) => {
                println!("[event] LSP: {:?}", other);
                lsp_node.event_handled().unwrap();
            }
            None => {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }

    update_balances(&lsp_node, &mut lsp_sc);
    let result = check_stability(&lsp_node, &mut lsp_sc, price);
    assert!(result.is_none(), "LSP should be stable after forwarding");
    println!("[check] LSP STABLE after forwarding (correct)");

    println!("\n[PASS] test_bolt11_receive_preserves_stable");

    user_node.stop().unwrap();
    lsp_node.stop().unwrap();
    funder_node.stop().unwrap();
}

// ==================================================================
// Test 9: Keysend send — deducts from stable balance
//         (single-hop: User → LSP, same reconciliation logic)
// ==================================================================

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires bitcoind + electrs (run with --ignored)"]
async fn test_keysend_send_deducts_from_stable() {
    println!("\n=====================================================");
    println!("TEST: Keysend send deducts from stable balance");
    println!("=====================================================\n");

    let (bitcoind, electrsd) = setup_bitcoind_and_electrsd();

    let lsp_node = setup_lsp_node(&electrsd);
    let lsp_pubkey = lsp_node.node_id();
    let lsp_addr = lsp_node.listening_addresses().unwrap().first().unwrap().clone();
    let user_node = setup_user_node(&electrsd, lsp_pubkey, lsp_addr);

    // Fund and open channel
    let addr_lsp = lsp_node.onchain_payment().new_address().unwrap();
    let addr_user = user_node.onchain_payment().new_address().unwrap();
    premine_and_distribute_funds(
        &bitcoind.client,
        &electrsd.client,
        vec![addr_lsp, addr_user],
        Amount::from_sat(2_125_000),
    )
    .await;
    lsp_node.sync_wallets().unwrap();
    user_node.sync_wallets().unwrap();

    let stable_funding = 2_000_000;
    let stable_push = (stable_funding / 2) * 1000;
    open_channel_and_confirm(
        &lsp_node, &user_node, stable_funding, Some(stable_push),
        &bitcoind.client, &electrsd,
    ).await;

    // --- Stabilize entire balance: $1000 all stable, no native ---
    let price = 100_000.0;
    set_mock_price(price);
    let expected_usd = 1000.0;

    let mut user_sc = create_stable_channel(
        &user_node, lsp_pubkey, true, expected_usd, price,
    );

    let (ok, _) = update_balances(&user_node, &mut user_sc);
    assert!(ok);
    print_stable_channel("User (before keysend)", &user_sc);

    let user_sats_before = user_sc.stable_receiver_btc.sats;

    // --- User sends keysend to LSP (single hop, direct channel) ---
    let send_sats: u64 = 75_000;
    let send_msat = send_sats * 1000;

    println!("\n[send] User keysend {} sats to LSP", send_sats);

    user_node
        .spontaneous_payment()
        .send(send_msat, lsp_pubkey, None)
        .expect("User keysend failed");

    expect_payment_successful_event!(user_node);
    println!("[event] User: PaymentSuccessful");
    expect_payment_received_event!(lsp_node);
    println!("[event] LSP: PaymentReceived");

    // Allow channel state to settle
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // --- Simulate PaymentSuccessful reconciliation ---
    let (ok, _) = update_balances(&user_node, &mut user_sc);
    assert!(ok);

    let user_sats_after = user_sc.stable_receiver_btc.sats;
    let sats_spent = user_sats_before.saturating_sub(user_sats_after);

    // Reconcile: backing_sats > actual sats means stable was eaten into
    let old_expected = user_sc.expected_usd.0;
    if let Some(usd_deducted) = reconcile_outgoing(&mut user_sc, price) {
        println!("[reconcile] Deducted ${:.2} from stable", usd_deducted);
        println!("[reconcile] expected_usd: ${:.2} -> ${:.2}", old_expected, user_sc.expected_usd.0);
    }

    print_stable_channel("User (after keysend send)", &user_sc);

    // Verify deduction
    let expected_deduction_usd = send_sats as f64 / 100_000_000.0 * price; // ~$75
    let actual_deduction = expected_usd - user_sc.expected_usd.0;
    println!("\n[verify] Sats spent: {} (expected ~{})", sats_spent, send_sats);
    println!("[verify] Stable deduction: ${:.2} (expected ~${:.2})", actual_deduction, expected_deduction_usd);

    assert!(
        sats_spent >= send_sats,
        "Should spend at least {} sats, spent {}", send_sats, sats_spent
    );
    assert!(
        actual_deduction > expected_deduction_usd * 0.95,
        "Deduction ${:.2} should be close to ${:.2}", actual_deduction, expected_deduction_usd
    );
    assert!(
        actual_deduction < expected_deduction_usd * 1.10,
        "Deduction ${:.2} too large vs ${:.2}", actual_deduction, expected_deduction_usd
    );

    // Stability check after reconciliation
    let result = check_stability(&user_node, &mut user_sc, price);
    println!("[check] User check_stability: {:?}", result.as_ref().map(|i| i.amount_msat));
    assert!(result.is_none(), "Should be stable after keysend reconciliation");

    println!("\n[PASS] test_keysend_send_deducts_from_stable");

    user_node.stop().unwrap();
    lsp_node.stop().unwrap();
}

// ==================================================================
// Test 10: Keysend receive — stable position preserved
//          (single-hop: LSP → User, same reconciliation logic)
// ==================================================================

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires bitcoind + electrs (run with --ignored)"]
async fn test_keysend_receive_preserves_stable() {
    println!("\n=====================================================");
    println!("TEST: Keysend receive preserves stable position");
    println!("=====================================================\n");

    let (bitcoind, electrsd) = setup_bitcoind_and_electrsd();

    let lsp_node = setup_lsp_node(&electrsd);
    let lsp_pubkey = lsp_node.node_id();
    let lsp_addr = lsp_node.listening_addresses().unwrap().first().unwrap().clone();
    let user_node = setup_user_node(&electrsd, lsp_pubkey, lsp_addr);

    // Fund and open channel
    let addr_lsp = lsp_node.onchain_payment().new_address().unwrap();
    let addr_user = user_node.onchain_payment().new_address().unwrap();
    premine_and_distribute_funds(
        &bitcoind.client,
        &electrsd.client,
        vec![addr_lsp, addr_user],
        Amount::from_sat(2_125_000),
    )
    .await;
    lsp_node.sync_wallets().unwrap();
    user_node.sync_wallets().unwrap();

    let stable_funding = 2_000_000;
    let stable_push = (stable_funding / 2) * 1000;
    open_channel_and_confirm(
        &lsp_node, &user_node, stable_funding, Some(stable_push),
        &bitcoind.client, &electrsd,
    ).await;

    // --- Stabilize $500 out of ~$1000 ---
    let price = 100_000.0;
    set_mock_price(price);
    let expected_usd = 500.0;

    let mut user_sc = create_stable_channel(
        &user_node, lsp_pubkey, true, expected_usd, price,
    );

    let (ok, _) = update_balances(&user_node, &mut user_sc);
    assert!(ok);
    print_stable_channel("User (before keysend receive)", &user_sc);

    let user_sats_before = user_sc.stable_receiver_btc.sats;
    let backing_before = user_sc.backing_sats;

    // --- LSP sends keysend to User (single hop, direct channel) ---
    let receive_sats: u64 = 60_000;
    let receive_msat = receive_sats * 1000;

    println!("\n[receive] LSP keysend {} sats to User", receive_sats);

    lsp_node
        .spontaneous_payment()
        .send(receive_msat, user_node.node_id(), None)
        .expect("LSP keysend to User failed");

    expect_payment_successful_event!(lsp_node);
    println!("[event] LSP: PaymentSuccessful");
    expect_payment_received_event!(user_node);
    println!("[event] User: PaymentReceived");

    // Allow channel state to settle
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // --- Simulate PaymentReceived handler: recalculate backing_sats ---
    {
        let (ok, _) = update_balances(&user_node, &mut user_sc);
        assert!(ok);
        reconcile_incoming(&mut user_sc);
    }

    print_stable_channel("User (after keysend receive)", &user_sc);

    let user_sats_after = user_sc.stable_receiver_btc.sats;
    let sats_gained = user_sats_after.saturating_sub(user_sats_before);

    println!("\n[verify] User sats: {} -> {} (gained {})", user_sats_before, user_sats_after, sats_gained);
    println!("[verify] expected_usd: ${:.2} (should be unchanged)", user_sc.expected_usd.0);
    println!("[verify] backing_sats: {} (was {})", user_sc.backing_sats, backing_before);

    // Stable position must be preserved
    assert!(
        (user_sc.expected_usd.0 - expected_usd).abs() < 0.01,
        "expected_usd should stay ${:.2}, got ${:.2}",
        expected_usd, user_sc.expected_usd.0
    );
    assert_eq!(
        user_sc.backing_sats, backing_before,
        "backing_sats should be unchanged after keysend receive"
    );

    // Should have gained sats
    assert!(
        sats_gained >= receive_sats - 100,
        "Should gain ~{} sats, got {}", receive_sats, sats_gained
    );

    // Stability check
    let result = check_stability(&user_node, &mut user_sc, price);
    assert!(result.is_none(), "Should be stable after keysend receive");
    println!("[check] User STABLE after keysend receive (correct)");

    // Verify stability still works with price change after receive
    let drop_price = 95_000.0;
    set_mock_price(drop_price);

    // User-side should be CHECK_ONLY (receiver below par)
    let result = check_stability(&user_node, &mut user_sc, drop_price);
    assert!(result.is_none(), "User should not pay when below par");
    println!("[check] User CHECK_ONLY after price drop (correct)");

    println!("\n[PASS] test_keysend_receive_preserves_stable");

    user_node.stop().unwrap();
    lsp_node.stop().unwrap();
}

// ==================================================================
// Test 11: On-chain send — stable lightning balance unaffected
// ==================================================================

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires bitcoind + electrs (run with --ignored)"]
async fn test_onchain_send_preserves_lightning_stable() {
    println!("\n=====================================================");
    println!("TEST: On-chain send preserves lightning stable");
    println!("=====================================================\n");

    let (bitcoind, electrsd) = setup_bitcoind_and_electrsd();

    let lsp_node = setup_lsp_node(&electrsd);
    let lsp_pubkey = lsp_node.node_id();
    let lsp_addr = lsp_node.listening_addresses().unwrap().first().unwrap().clone();
    let user_node = setup_user_node(&electrsd, lsp_pubkey, lsp_addr);

    // Fund with extra on-chain balance
    let addr_lsp = lsp_node.onchain_payment().new_address().unwrap();
    let addr_user = user_node.onchain_payment().new_address().unwrap();
    // Give user extra on-chain funds beyond what's needed for channel
    let addr_user_extra = user_node.onchain_payment().new_address().unwrap();
    premine_and_distribute_funds(
        &bitcoind.client,
        &electrsd.client,
        vec![addr_lsp, addr_user, addr_user_extra],
        Amount::from_sat(2_125_000),
    )
    .await;
    lsp_node.sync_wallets().unwrap();
    user_node.sync_wallets().unwrap();

    let user_onchain_before_channel = user_node.list_balances().spendable_onchain_balance_sats;
    println!("[state] User onchain before channel: {} sats", user_onchain_before_channel);

    // Open channel
    let funding_sats = 2_000_000;
    let push_msat = (funding_sats / 2) * 1000;
    open_channel_and_confirm(
        &lsp_node, &user_node, funding_sats, Some(push_msat),
        &bitcoind.client, &electrsd,
    ).await;

    // --- Set up stable position ---
    let price = 100_000.0;
    set_mock_price(price);
    let expected_usd = 500.0;

    let mut user_sc = create_stable_channel(
        &user_node, lsp_pubkey, true, expected_usd, price,
    );

    let (ok, _) = update_balances(&user_node, &mut user_sc);
    assert!(ok);
    print_stable_channel("User (before on-chain send)", &user_sc);

    let lightning_sats_before = user_node.list_balances().total_lightning_balance_sats;
    let onchain_before = user_node.list_balances().spendable_onchain_balance_sats;
    let expected_usd_before = user_sc.expected_usd.0;
    let backing_before = user_sc.backing_sats;

    println!("[state] Lightning balance: {} sats", lightning_sats_before);
    println!("[state] On-chain balance: {} sats", onchain_before);

    if onchain_before < 10_000 {
        println!("[skip] Not enough on-chain balance to test send ({}), skipping", onchain_before);
        user_node.stop().unwrap();
        lsp_node.stop().unwrap();
        return;
    }

    // --- Send on-chain to a new address (LSP's address works) ---
    let dest_addr = lsp_node.onchain_payment().new_address().unwrap();
    let send_sats = 50_000u64;

    println!("\n[send] User sending {} sats on-chain", send_sats);

    let txid = user_node
        .onchain_payment()
        .send_to_address(&dest_addr, send_sats, None)
        .expect("On-chain send failed");

    println!("[send] TX: {}", txid);

    // Mine block to confirm
    generate_blocks_and_wait(&bitcoind.client, &electrsd.client, 1).await;
    user_node.sync_wallets().unwrap();
    lsp_node.sync_wallets().unwrap();

    // --- Verify lightning stable balance is UNAFFECTED ---
    let (ok, _) = update_balances(&user_node, &mut user_sc);
    assert!(ok);
    print_stable_channel("User (after on-chain send)", &user_sc);

    let lightning_sats_after = user_node.list_balances().total_lightning_balance_sats;
    let onchain_after = user_node.list_balances().spendable_onchain_balance_sats;

    println!("\n[verify] Lightning: {} -> {} sats", lightning_sats_before, lightning_sats_after);
    println!("[verify] On-chain: {} -> {} sats", onchain_before, onchain_after);
    println!("[verify] expected_usd: ${:.2} (was ${:.2})", user_sc.expected_usd.0, expected_usd_before);
    println!("[verify] backing_sats: {} (was {})", user_sc.backing_sats, backing_before);

    // Lightning balance should be unchanged
    assert_eq!(
        lightning_sats_before, lightning_sats_after,
        "Lightning balance should not change from on-chain send"
    );

    // Stable position should be unchanged
    assert!(
        (user_sc.expected_usd.0 - expected_usd_before).abs() < 0.01,
        "expected_usd should not change from on-chain send"
    );
    assert_eq!(
        user_sc.backing_sats, backing_before,
        "backing_sats should not change from on-chain send"
    );

    // On-chain should have decreased
    assert!(
        onchain_after < onchain_before,
        "On-chain balance should decrease after send"
    );

    // Stability check: still at equilibrium
    let result = check_stability(&user_node, &mut user_sc, price);
    assert!(result.is_none(), "Should be stable after on-chain send");
    println!("[check] User STABLE after on-chain send (correct)");

    println!("\n[PASS] test_onchain_send_preserves_lightning_stable");

    user_node.stop().unwrap();
    lsp_node.stop().unwrap();
}
