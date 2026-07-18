import SwiftUI
import UserNotifications

struct HomeView: View {
    @Environment(AppState.self) private var appState
    @Environment(PaymentDetailCoordinator.self) private var paymentCoordinator
    @State private var showSendSheet = false
    @State private var showReceiveSheet = false
    @State private var showBuySheet = false
    @State private var showSellSheet = false
    @State private var prefillTradeAmount: Double = 0
    @State private var tradeRequest: TradeRequest?
    @Environment(\.scenePhase) private var scenePhase
    @State private var flashScale: CGFloat = 1.0
    @State private var showBTC = false
    @State private var notificationsEnabled = true
    @State private var receivePulse = false
    @State private var showReceiveHint = true

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 16) {
                    HStack {
                        Text(String(localized: "app_name", defaultValue: "Stable Channels"))
                            .font(.headline)
                        Spacer()
                    }

                    // Notification warning
                    if !notificationsEnabled {
                        notificationWarning
                    }

                    // Total Balance
                    balanceSection

                    // Syncing indicator
                    if appState.isSyncing {
                        HStack(spacing: 6) {
                            ProgressView()
                                .controlSize(.small)
                            Text(String(localized: "home_syncing", defaultValue: "Syncing..."))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }

                    // Stable / Native Split
                    if appState.lightningBalanceSats > 0 {
                        balanceBarSection
                    }

                    // On-chain balance
                    if appState.onchainBalanceSats > 0 {
                        savingsSection
                    }

                    // Price Chart
                    if appState.btcPrice > 0 {
                        PriceChartCard(compact: true)
                            .equatable()
                            .padding(.bottom, 8)
                    }

                    // Hint text when no channel
                    if !hasReadyChannel {
                        Text(String(
                            localized: "home_hint_receive",
                            defaultValue: "Receive bitcoin over Lightning to get started"
                        ))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .padding(.bottom, 4)
                    }

                    // Action Buttons
                    actionButtons

                    // Status
                    if !appState.statusMessage.isEmpty {
                        statusSection
                    }
                }
                .animation(.easeInOut(duration: 0.3), value: appState.statusMessage)
                .padding(.horizontal)
                .padding(.bottom)
            }
            .navigationBarHidden(true)
            .refreshable {
                appState.refreshBalances()
                appState.recordCurrentPrice()
            }
        }
        .onAppear {
            checkNotifications()
            Task.detached { [appState] in appState.ensureLSPConnected() }
        }
        .onChange(of: scenePhase) {
            if scenePhase == .active {
                checkNotifications()
                Task.detached { [appState] in appState.ensureLSPConnected() }
            }
        }
        .sheet(isPresented: $showSendSheet) { SendView() }
        .sheet(isPresented: $showReceiveSheet) { ReceiveView() }
        .sheet(isPresented: $showBuySheet) {
            BuyView(prefillAmountUSD: prefillTradeAmount)
        }
        .sheet(isPresented: $showSellSheet) {
            SellView(prefillAmountUSD: prefillTradeAmount)
        }
        .sheet(item: $tradeRequest) { request in
            if request.direction == .buy {
                BuyView(prefillAmountUSD: request.amountUSD)
            } else {
                SellView(prefillAmountUSD: request.amountUSD)
            }
        }
        .onChange(of: appState.paymentFlash) {
            if appState.paymentFlash {
                withAnimation(.easeOut(duration: 0.3)) {
                    flashScale = 1.08
                }
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
                    withAnimation(.easeInOut(duration: 0.4)) {
                        flashScale = 1.0
                    }
                }
            }
        }
    }

    private func checkNotifications() {
        UNUserNotificationCenter.current().getNotificationSettings { settings in
            DispatchQueue.main.async {
                notificationsEnabled = settings.authorizationStatus == .authorized
            }
        }
    }

    // MARK: - Notification Warning

    private var notificationWarning: some View {
        Button {
            if let url = URL(string: UIApplication.openSettingsURLString) {
                UIApplication.shared.open(url)
            }
        } label: {
            HStack(spacing: 8) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.white)
                VStack(alignment: .leading, spacing: 2) {
                    Text(String(localized: "notifications_disabled", defaultValue: "Notifications Disabled"))
                        .font(.subheadline)
                        .fontWeight(.semibold)
                        .foregroundStyle(.white)
                    Text(String(
                        localized: "notifications_disabled_subtitle",
                        defaultValue: "Enable notifications for stability payments"
                    ))
                    .font(.caption)
                    .foregroundStyle(.white.opacity(0.9))
                }
                Spacer()
                Image(systemName: "chevron.right")
                    .font(.caption)
                    .foregroundStyle(.white.opacity(0.7))
            }
            .padding(12)
            .background(.red, in: RoundedRectangle(cornerRadius: 12))
        }
    }

    // MARK: - Balance Section

    private var displaySats: UInt64 {
        appState.totalBalanceSats > 0
            ? appState.totalBalanceSats
            : appState.stableChannel.stableReceiverBTC.sats
    }

    private var balanceSection: some View {
        let hasBalance = appState.totalBalanceUSD > 0 || displaySats > 0

        return VStack(spacing: 4) {
            Text(String(localized: "label_total_balance", defaultValue: "Total Balance"))
                .font(.caption)
                .foregroundStyle(.secondary)

            if !hasBalance && appState.isSyncing {
                Text(String(localized: "label_dash", defaultValue: "—"))
                    .font(.system(size: 42, weight: .bold, design: .rounded))
                    .foregroundStyle(.secondary)

                Text(String(localized: "loading_balance", defaultValue: "Loading balance..."))
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            } else if showBTC {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text("\(displaySats.btcSpacedFormatted)")
                        .font(.system(size: 32, weight: .bold, design: .monospaced))
                        .foregroundStyle(appState.paymentFlash ? .green : .primary)
                        .contentTransition(.numericText())
                        .animation(.default, value: displaySats)
                    Text(String(localized: "label_btc", defaultValue: "BTC"))
                        .font(.system(size: 18, weight: .semibold, design: .rounded))
                        .foregroundStyle(.secondary)
                }

                Text(appState.totalBalanceUSD.usdFormatted)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(appState.totalBalanceUSD.usdFormatted)
                        .font(.system(size: 42, weight: .bold, design: .rounded))
                        .foregroundStyle(appState.paymentFlash ? .green : .primary)
                        .contentTransition(.numericText())
                        .animation(.default, value: appState.totalBalanceUSD)
                        .animation(.easeInOut(duration: 0.3), value: appState.paymentFlash)
                        .accessibilityIdentifier("home_total_balance_usd")
                    Text(String(localized: "label_usd", defaultValue: "USD"))
                        .font(.system(size: 18, weight: .semibold, design: .rounded))
                        .foregroundStyle(.secondary)
                }

                Text("\(displaySats.btcSpacedFormatted) BTC")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .scaleEffect(flashScale)
        .padding(.top, 8)
        .onTapGesture {
            withAnimation(.easeInOut(duration: 0.2)) {
                showBTC.toggle()
            }
        }
    }

    // MARK: - Balance Bar (Stable / Native)

    private var stableSats: UInt64 {
        appState.btcPrice > 0
            ? UInt64(appState.stableUSD / appState.btcPrice * Double(Constants.satsInBTC))
            : 0
    }

    private var nativeSatsDisplay: UInt64 {
        appState.lightningBalanceSats > stableSats
            ? appState.lightningBalanceSats - stableSats
            : 0
    }

    private var nativeUSD: Double {
        appState.btcPrice > 0
            ? Double(nativeSatsDisplay) / Double(Constants.satsInBTC) * appState.btcPrice
            : 0.0
    }

    private var balanceBarSection: some View {
        VStack(spacing: 6) {
            BalanceBarView(
                stableUSD: appState.stableUSD,
                nativeSats: nativeSatsDisplay,
                totalSats: appState.lightningBalanceSats,
                btcPrice: appState.btcPrice,
                onDragStarted: { appState.ensureLSPConnected() },
                onTradeRequest: { direction, amountUSD in
                    tradeRequest = TradeRequest(direction: direction, amountUSD: amountUSD)
                }
            )
            .padding(.horizontal, 24)

            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 1) {
                    HStack(spacing: 4) {
                        Image(systemName: "shield.fill")
                            .font(.caption2)
                        Text(String(localized: "label_usd", defaultValue: "USD"))
                            .font(.caption.bold())
                    }
                    .foregroundStyle(.green)
                    Text(showBTC ? "\(stableSats.btcSpacedFormatted) BTC" : appState.stableUSD.usdFormatted)
                        .font(.caption)
                        .foregroundStyle(.primary)
                        .contentTransition(.numericText())
                }

                Spacer()

                VStack(alignment: .trailing, spacing: 1) {
                    HStack(spacing: 4) {
                        Text(String(localized: "label_btc", defaultValue: "BTC"))
                            .font(.caption.bold())
                        Image(systemName: "bitcoinsign.circle.fill")
                            .font(.caption2)
                    }
                    .foregroundStyle(.orange)
                    Text(showBTC ? "\(nativeSatsDisplay.btcSpacedFormatted) BTC" : nativeUSD.usdFormatted)
                        .font(.caption)
                        .foregroundStyle(.primary)
                        .contentTransition(.numericText())
                }
            }
            .onTapGesture {
                withAnimation(.easeInOut(duration: 0.2)) {
                    showBTC.toggle()
                }
            }
        }
    }

    // MARK: - Savings (On-Chain)

    private var onchainUSD: Double {
        appState.btcPrice > 0
            ? Double(appState.onchainBalanceSats) / Double(Constants.satsInBTC) * appState.btcPrice
            : 0
    }

    private var hasReadyChannel: Bool {
        appState.hasReadyChannel
    }

    private var savingsSection: some View {
        VStack(spacing: 6) {
            HStack {
                Text(String(localized: "label_on_chain", defaultValue: "Onchain"))
                    .font(.caption.bold())
                Spacer()
                Text(showBTC
                    ? "\(appState.onchainBalanceSats.btcSpacedFormatted) BTC"
                    : onchainUSD.usdFormatted)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            if appState.isSweeping {
                // 1. Splice-in in progress
                pendingRow(kind: .sweep(txid: appState.spliceTxid))
            } else if appState.isChannelClosing {
                if let closeTxid = appState.transactionLinkService.lastCloseTxid, !closeTxid.isEmpty {
                    pendingRow(kind: .close(txid: closeTxid))
                } else {
                    pendingRow(kind: .closeNoLink)
                }
            } else if hasReadyChannel && appState.spendableOnchainSats > 0 {
                // 2. Channel + confirmed funds — offer to sweep
                HStack {
                    Text(String(localized: "move_to_trading", defaultValue: "Move to Lightning Account"))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button {
                        appState.sweepToChannel()
                    } label: {
                        Text(String(localized: "button_swap", defaultValue: "Swap"))
                            .font(.caption.bold())
                            .padding(.horizontal, 16)
                            .padding(.vertical, 6)
                            .background(.blue.opacity(0.1))
                            .foregroundStyle(.blue)
                            .clipShape(Capsule())
                    }
                }
            } else if appState.spendableOnchainSats == 0 {
                if appState.isOpeningChannel, let fundingTx = appState.fundingTxid {
                    pendingRow(kind: .deposit(txid: fundingTx))
                } else {
                    pendingRow(kind: .onchainReceive(txid: appState.transactionLinkService.lastReceiveTxid))
                }
                if !hasReadyChannel {
                    Text(String(
                        localized: "hint_create_wallet",
                        defaultValue: "Receive over Lightning to create your Stable Wallet"
                    ))
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                }
            } else {
                // 4. No channel, confirmed deposit — just needs a Lightning receive
                Text(String(
                    localized: "hint_create_wallet",
                    defaultValue: "Receive over Lightning to create your Stable Wallet"
                ))
                .font(.caption2)
                .foregroundStyle(.secondary)
            }
        }
        .padding(10)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 10))
    }

    private enum PendingRowKind {
        case deposit(txid: String?)
        case sweep(txid: String?)
        case close(txid: String?)
        case closeNoLink
        case onchainReceive(txid: String?)
    }

    @ViewBuilder
    private func pendingRow(kind: PendingRowKind) -> some View {
        switch kind {
        case .deposit(let txid):
            pendingRowImpl(
                text: String(localized: "status_channel_opening", defaultValue: "Deposit confirming..."),
                txid: txid
            )
        case .sweep(let txid):
            pendingRowImpl(
                text: String(localized: "status_sweeping", defaultValue: "Swap pending..."),
                txid: txid
            )
        case .close(let txid):
            pendingRowImpl(
                text: String(localized: "status_channel_closing", defaultValue: "Channel closing…"),
                txid: txid
            )
        case .closeNoLink:
            HStack(spacing: 6) {
                Image(systemName: "hourglass")
                    .font(.caption)
                    .foregroundStyle(.orange)
                Text(String(
                    localized: "info_close_pending_confirmation",
                    defaultValue: "Channel closing - pending confirmation"
                ))
                .font(.caption2)
                .foregroundStyle(.secondary)
                Spacer()
            }
        case .onchainReceive(let txid):
            pendingRowImpl(
                text: String(localized: "status_onchain_receiving", defaultValue: "Receiving onchain..."),
                txid: txid
            )
        }
    }

    private func pendingRowImpl(text: String, txid: String?) -> some View {
        HStack(spacing: 6) {
            Image(systemName: "hourglass")
                .font(.caption)
                .foregroundStyle(.orange)
            Text(text)
                .font(.caption2)
                .foregroundStyle(.secondary)
            Spacer()
            if let txid, !txid.isEmpty {
                if let url = Constants.txExplorerLink(for: txid) {
                    Link(destination: url) {
                        HStack(spacing: 2) {
                            Text(String(localized: "view_on_explorer", defaultValue: "View on explorer"))
                                .font(.caption2)
                            Image(systemName: "arrow.up.right.square")
                                .font(.caption2)
                        }
                        .foregroundStyle(.blue)
                    }
                }
            }
        }
    }

    // MARK: - Action Buttons

    private var actionButtons: some View {
        return VStack(spacing: 8) {
            HStack(spacing: 8) {
                ActionButton(
                    title: String(localized: "button_send", defaultValue: "Send"),
                    icon: "arrow.up.circle.fill",
                    color: .blue
                ) {
                    showSendSheet = true
                }
                ActionButton(
                    title: String(localized: "button_receive", defaultValue: "Receive"),
                    icon: "arrow.down.circle.fill",
                    color: .green,
                    pulse: !hasReadyChannel
                ) {
                    showReceiveSheet = true
                }
            }

            HStack(spacing: 8) {
                ActionButton(
                    title: String(localized: "button_buy_btc", defaultValue: "USD → BTC"),
                    icon: "arrow.up.right.circle.fill",
                    color: .orange
                ) {
                    showBuySheet = true
                }
                ActionButton(
                    title: String(localized: "button_sell_btc", defaultValue: "BTC → USD"),
                    icon: "arrow.down.right.circle.fill",
                    color: .purple
                ) {
                    showSellSheet = true
                }
            }
        }
    }

    // MARK: - Status Section

    private var statusSection: some View {
        Button(action: { openPaymentDetail() }) {
            Text(appState.statusMessage)
                .font(.caption)
                .foregroundStyle(.secondary)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(.ultraThinMaterial, in: Capsule())
                .transition(.move(edge: .bottom).combined(with: .opacity))
        }
        .buttonStyle(.plain)
    }

    private func openPaymentDetail() {
        guard let payment = appState.databaseService?.latestReceivedPayment() else { return }
        paymentCoordinator.open(payment)
    }
}

// MARK: - Action Button

struct ActionButton: View {
    let title: String
    let icon: String
    let color: Color
    var pulse: Bool = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack {
                Image(systemName: icon)
                Text(title)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 14)
            .foregroundStyle(color)
        }
        .background(color.opacity(0.1))
        .clipShape(RoundedRectangle(cornerRadius: 12))
        .overlay {
            if pulse {
                PulseOverlay(color: color)
            }
        }
    }
}

struct PulseOverlay: View {
    let color: Color
    @State private var on = false

    var body: some View {
        RoundedRectangle(cornerRadius: 12)
            .fill(color.opacity(on ? 0.25 : 0.0))
            .animation(.easeInOut(duration: 1.0).repeatForever(autoreverses: true), value: on)
            .onAppear { on = true }
            .allowsHitTesting(false)
    }
}
