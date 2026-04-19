import SwiftUI
import UserNotifications

struct HomeView: View {
    @Environment(AppState.self) private var appState
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
                        Text("Stable Channels")
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
                            Text("Syncing...")
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
                        PriceChartView(compact: true)
                            .padding(.bottom, 8)
                    }

                    // Hint text when no channel
                    if !hasReadyChannel {
                        Text("Receive bitcoin over Lightning to get started")
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
            appState.ensureLSPConnected()
        }
        .onChange(of: scenePhase) {
            if scenePhase == .active {
                checkNotifications()
                appState.ensureLSPConnected()
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
                    Text("Notifications Disabled")
                        .font(.subheadline)
                        .fontWeight(.semibold)
                        .foregroundStyle(.white)
                    Text("Enable notifications for stability payments")
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
            Text("Total Balance")
                .font(.caption)
                .foregroundStyle(.secondary)

            if !hasBalance && appState.isSyncing {
                Text("—")
                    .font(.system(size: 42, weight: .bold, design: .rounded))
                    .foregroundStyle(.secondary)

                Text("Loading balance...")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            } else if showBTC {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text("\(displaySats.btcSpacedFormatted)")
                        .font(.system(size: 32, weight: .bold, design: .monospaced))
                        .foregroundStyle(appState.paymentFlash ? .green : .primary)
                        .contentTransition(.numericText())
                        .animation(.default, value: displaySats)
                    Text("BTC")
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
                    Text("USD")
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
                        Text("USD")
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
                        Text("BTC")
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
        appState.nodeService.channels.contains { $0.isChannelReady }
    }

    private var savingsSection: some View {
        VStack(spacing: 6) {
            HStack {
                Text("On-chain")
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
                pendingRow(text: "Swap pending...", txid: appState.spliceTxid)
            } else if hasReadyChannel && appState.nodeService.spendableOnchainSats() > 0 {
                // 2. Channel + confirmed funds — offer to sweep
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Move to Trading")
                            .font(.caption2)
                        Text("and Spending Account")
                            .font(.caption2)
                    }
                    .foregroundStyle(.secondary)
                    Spacer()
                    Button {
                        appState.sweepToChannel()
                    } label: {
                        Text("Swap")
                            .font(.caption.bold())
                            .padding(.horizontal, 16)
                            .padding(.vertical, 6)
                            .background(.blue.opacity(0.1))
                            .foregroundStyle(.blue)
                            .clipShape(Capsule())
                    }
                }
            } else if appState.nodeService.spendableOnchainSats() == 0 {
                // 3. Unconfirmed deposit (with or without channel)
                pendingRow(text: "Deposit confirming...", txid: appState.fundingTxid)
                if !hasReadyChannel {
                    Text("Receive over Lightning to create your Trading and Spending Account")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            } else {
                // 4. No channel, confirmed deposit — just needs a Lightning receive
                Text("Receive over Lightning to create your Trading and Spending Account")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(10)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 10))
    }

    private func pendingRow(text: String, txid: String?) -> some View {
        HStack(spacing: 6) {
            Image(systemName: "hourglass")
                .font(.caption)
                .foregroundStyle(.orange)
            Text(text)
                .font(.caption2)
                .foregroundStyle(.secondary)
            Spacer()
            if let txid, !txid.isEmpty, let txURL = Constants.explorerTxURL(txid: txid) {
                Link(destination: txURL) {
                    HStack(spacing: 2) {
                        Text("View on explorer")
                            .font(.caption2)
                        Image(systemName: "arrow.up.right.square")
                            .font(.caption2)
                    }
                    .foregroundStyle(.blue)
                }
            }
        }
    }

    // MARK: - Action Buttons

    private var actionButtons: some View {
        let noChannel = !hasReadyChannel && appState.totalBalanceSats == 0

        return VStack(spacing: 8) {
            if noChannel {
                Text("Receive BTC to get started")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }

            HStack(spacing: 8) {
                ActionButton(title: "Send", icon: "arrow.up.circle.fill", color: .blue) {
                    showSendSheet = true
                }
                ActionButton(title: "Receive", icon: "arrow.down.circle.fill", color: .green, pulse: !hasReadyChannel) {
                    showReceiveSheet = true
                }
            }

            HStack(spacing: 8) {
                ActionButton(title: "Buy BTC", icon: "arrow.up.right.circle.fill", color: .orange) {
                    showBuySheet = true
                }
                ActionButton(title: "Sell BTC", icon: "arrow.down.right.circle.fill", color: .purple) {
                    showSellSheet = true
                }
            }
        }
    }

    // MARK: - Status Section

    private var statusSection: some View {
        Text(appState.statusMessage)
            .font(.caption)
            .foregroundStyle(.secondary)
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(.ultraThinMaterial, in: Capsule())
            .transition(.move(edge: .bottom).combined(with: .opacity))
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
            .fill(color.opacity(on ? 0.15 : 0.0))
            .scaleEffect(on ? 1.04 : 1.0)
            .animation(.easeInOut(duration: 0.8).repeatForever(autoreverses: true), value: on)
            .onAppear { on = true }
            .allowsHitTesting(false)
    }
}
