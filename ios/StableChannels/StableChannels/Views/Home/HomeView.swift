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

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 16) {
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
                    if appState.stableUSD > 0 {
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

                    // Action Buttons
                    actionButtons

                    // Status Message
                    if !appState.statusMessage.isEmpty {
                        statusSection
                    }
                }
                .animation(.easeInOut(duration: 0.3), value: appState.statusMessage)
                .padding(.horizontal)
                .padding(.bottom)
            }
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Text("Stable Channels")
                        .font(.headline)
                }
            }
            .refreshable {
                appState.refreshBalances()
                appState.recordCurrentPrice()
            }
        }
        .onAppear { checkNotifications() }
        .onChange(of: scenePhase) {
            if scenePhase == .active { checkNotifications() }
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

    private var nativeUSD: Double {
        appState.btcPrice > 0
            ? Double(appState.nativeBTC.sats) / Double(Constants.satsInBTC) * appState.btcPrice
            : 0.0
    }

    private var stableSats: UInt64 {
        appState.btcPrice > 0
            ? UInt64(appState.stableUSD / appState.btcPrice * Double(Constants.satsInBTC))
            : 0
    }

    private var balanceBarSection: some View {
        VStack(spacing: 6) {
            BalanceBarView(
                stableUSD: appState.stableUSD,
                nativeSats: appState.nativeBTC.sats,
                totalSats: appState.lightningBalanceSats,
                btcPrice: appState.btcPrice,
                onTradeRequest: { direction, amountUSD in
                    tradeRequest = TradeRequest(direction: direction, amountUSD: amountUSD)
                }
            )

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
                    Text(showBTC ? "\(appState.nativeBTC.sats.btcSpacedFormatted) BTC" : nativeUSD.usdFormatted)
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

            if hasReadyChannel && !appState.isSweeping {
                Button {
                    appState.sweepToChannel()
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "arrow.right.circle.fill")
                            .font(.caption2)
                        Text("Move to Spending & Trading")
                            .font(.caption)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 8)
                    .background(.blue.opacity(0.1))
                    .foregroundStyle(.blue)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
                }
            } else if appState.isSweeping {
                HStack(spacing: 4) {
                    ProgressView()
                        .controlSize(.mini)
                    Text("Moving to channel...")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }

                if let addr = appState.onchainReceiveAddress,
                   let url = URL(string: "https://mempool.space/address/\(addr)") {
                    Link(destination: url) {
                        HStack(spacing: 4) {
                            Text("View on explorer")
                            Image(systemName: "arrow.up.right.square")
                        }
                        .font(.caption2)
                        .foregroundStyle(.blue)
                    }
                }
            }
        }
        .padding(10)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 10))
    }

    // MARK: - Action Buttons

    private var actionButtons: some View {
        VStack(spacing: 8) {
            HStack(spacing: 8) {
                ActionButton(title: "Send", icon: "arrow.up.circle.fill", color: .blue) {
                    showSendSheet = true
                }
                ActionButton(title: "Receive", icon: "arrow.down.circle.fill", color: .green) {
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
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack {
                Image(systemName: icon)
                Text(title)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 14)
            .background(color.opacity(0.1))
            .foregroundStyle(color)
            .clipShape(RoundedRectangle(cornerRadius: 12))
        }
    }
}
