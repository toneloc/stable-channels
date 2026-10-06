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

                    HomeBalanceSectionView(showBTC: $showBTC, flashScale: flashScale)

                    HomeSyncSpinnerView()

                    balanceBarSection
                        .transition(.opacity.combined(with: .scale(scale: 0.98)))

                    if appState.onchainBalanceSats > 0 {
                        HomeSavingsSectionView(showBTC: showBTC)
                    }

                    PriceChartCard(compact: true)
                        .equatable()
                        .padding(.bottom, 8)

                    if !appState.hasReadyChannel && appState.totalBalanceSats == 0 && !appState.isOpeningChannel {
                        receiveHintText
                    }

                    HomeActionButtonsView(
                        hasReadyChannel: appState.hasReadyChannel,
                        pulseReceive: !appState.hasReadyChannel && appState.totalBalanceSats == 0 && !appState
                            .isOpeningChannel,
                        isOffline: !appState.networkMonitor.isOnline,
                        onSend: { showSendSheet = true },
                        onReceive: { showReceiveSheet = true },
                        onBuy: { showBuySheet = true },
                        onSell: { showSellSheet = true }
                    )

                    HomeSyncStatusSectionView(onOpenPaymentDetail: { openPaymentDetail() })
                }
                .animation(.easeInOut(duration: 0.3), value: appState.statusMessage)
                .padding(.horizontal)
                .padding(.bottom)
            }
            .scrollBounceBehavior(.basedOnSize)
            .navigationBarHidden(true)
            .refreshable {
                if !appState.networkMonitor.isOnline {
                    withAnimation(.easeInOut(duration: 0.25)) {
                        appState.phase = .offline
                    }
                    return
                }
                appState.refreshBalances()
                await appState.priceService.fetchPrice()
                appState.recordCurrentPrice()
                appState.ensureLSPConnected()
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

    // MARK: - Balance Bar (Stable / Native)

    private var allocation: ChannelAllocation {
        ChannelAllocation(
            stableUSD: appState.stableUSD,
            lightningBalanceSats: appState.lightningBalanceSats,
            btcPrice: appState.btcPrice,
            backingSatsOverride: appState.stableChannel.backingSats
        )
    }

    private var balanceBarSection: some View {
        VStack(spacing: 6) {
            BalanceBarView(
                allocation: allocation,
                maxSellUSD: Double(appState.tradeService?.maxSellCents(
                    sc: appState.stableChannel,
                    price: appState.accountingBTCPrice > 0 ? appState.accountingBTCPrice : appState.btcPrice
                ) ?? 0) / 100,
                isTrading: tradeRequest != nil,
                onDragStarted: { appState.ensureLSPConnected() },
                onTradeRequest: appState.hasReadyChannel ? { request in
                    guard appState.networkMonitor.isOnline else { return }
                    tradeRequest = request
                } : nil,
                onEmptyInteraction: {
                    showReceiveSheet = true
                }
            )
            .disabled(!appState.networkMonitor.isOnline)
            .opacity(appState.networkMonitor.isOnline ? 1.0 : 0.85)
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
                    RollingDigitLabel(
                        text: showBTC ? "\(allocation.stableSats.btcSpacedFormatted) BTC" : appState.stableUSD
                            .usdFormatted,
                        value: showBTC ? Double(allocation.stableSats) : appState.stableUSD,
                        font: .caption,
                        baseColor: .primary
                    )
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
                    RollingDigitLabel(
                        text: showBTC ? "\(allocation.nativeSats.btcSpacedFormatted) BTC" : allocation.nativeUSD
                            .usdFormatted,
                        value: showBTC ? Double(allocation.nativeSats) : allocation.nativeUSD,
                        font: .caption,
                        baseColor: .primary
                    )
                }
            }
            .onTapGesture {
                withAnimation(.easeInOut(duration: 0.2)) {
                    showBTC.toggle()
                }
            }
        }
    }

    // MARK: - Helpers

    private var receiveHintText: some View {
        Text(String(
            localized: "home_hint_receive",
            defaultValue: "Receive bitcoin over Lightning to get started"
        ))
        .font(.caption)
        .foregroundStyle(.secondary)
        .padding(.bottom, 4)
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
