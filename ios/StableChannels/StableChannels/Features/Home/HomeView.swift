import SwiftUI
import UserNotifications

struct HomeView: View {
    @Environment(AppState.self) private var appState
    @Environment(PaymentDetailCoordinator.self) private var paymentCoordinator
    @Environment(\.scenePhase) private var scenePhase

    @State private var showSendSheet = false
    @State private var showReceiveSheet = false
    @State private var showBuySheet = false
    @State private var showSellSheet = false
    @State private var prefillTradeAmount: Double = 0
    @State private var tradeRequest: TradeRequest?

    @State private var flashScale: CGFloat = 1.0
    @State private var showBTC = false
    @State private var notificationsEnabled = true

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 16) {
                    if !notificationsEnabled {
                        HomeNotificationBannerView()
                    }

                    balanceSection

                    if appState.isSyncing {
                        syncingIndicator
                    }

                    if appState.lightningBalanceSats > 0 {
                        balanceBarSection
                    }

                    if appState.onchainBalanceSats > 0 {
                        HomeSavingsSectionView(showBTC: showBTC)
                    }

                    PriceChartCard(compact: true)
                        .equatable()
                        .padding(.bottom, 8)

                    if !appState.hasReadyChannel {
                        receiveHintText
                    }

                    HomeActionButtonsView(
                        hasReadyChannel: appState.hasReadyChannel,
                        onSend: { showSendSheet = true },
                        onReceive: { showReceiveSheet = true },
                        onBuy: { showBuySheet = true },
                        onSell: { showSellSheet = true }
                    )

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
        .sheet(isPresented: $showBuySheet) { BuyView(prefillAmountUSD: prefillTradeAmount) }
        .sheet(isPresented: $showSellSheet) { SellView(prefillAmountUSD: prefillTradeAmount) }
        .sheet(item: $tradeRequest) { request in
            if request.direction == .buy {
                BuyView(prefillAmountUSD: request.amountUSD)
            } else {
                SellView(prefillAmountUSD: request.amountUSD)
            }
        }
        .onChange(of: appState.paymentFlash) {
            if appState.paymentFlash {
                withAnimation(.easeOut(duration: 0.3)) { flashScale = 1.08 }
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
                    withAnimation(.easeInOut(duration: 0.4)) { flashScale = 1.0 }
                }
            }
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
                maxSellUSD: Double(appState.tradeService?.maxSellCents(
                    sc: appState.stableChannel,
                    price: appState.accountingBTCPrice
                ) ?? 0) / 100,
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

    // MARK: - Indicators & Status

    private var syncingIndicator: some View {
        HStack(spacing: 6) {
            ProgressView()
                .controlSize(.small)
            Text(String(localized: "home_syncing", defaultValue: "Syncing..."))
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var receiveHintText: some View {
        Text(String(
            localized: "home_hint_receive",
            defaultValue: "Receive bitcoin over Lightning to get started"
        ))
        .font(.caption)
        .foregroundStyle(.secondary)
        .padding(.bottom, 4)
    }

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

    private func checkNotifications() {
        UNUserNotificationCenter.current().getNotificationSettings { settings in
            DispatchQueue.main.async {
                notificationsEnabled = settings.authorizationStatus == .authorized
            }
        }
    }

    private func openPaymentDetail() {
        guard let payment = appState.databaseService?.paymentRepo.latestReceivedPayment() else { return }
        paymentCoordinator.open(payment)
    }
}
