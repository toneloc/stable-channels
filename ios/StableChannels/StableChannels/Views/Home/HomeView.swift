import SwiftUI

struct HomeView: View {
    @Environment(AppState.self) private var appState
    @State private var showSendSheet = false
    @State private var showReceiveSheet = false
    @State private var showBuySheet = false
    @State private var showSellSheet = false
    @State private var flashScale: CGFloat = 1.0

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 24) {
                    // Total Balance
                    balanceSection

                    // Stable / Native Split
                    if appState.stableUSD > 0 {
                        balanceBarSection
                    }

                    // Action Buttons
                    actionButtons

                    // BTC Price
                    priceSection

                    // Status Message
                    if !appState.statusMessage.isEmpty {
                        statusSection
                    }
                }
                .animation(.easeInOut(duration: 0.3), value: appState.statusMessage)
                .padding()
            }
            .navigationTitle("Stable Channels")
            .refreshable {
                appState.refreshBalances()
                appState.recordCurrentPrice()
            }
        }
        .sheet(isPresented: $showSendSheet) { SendView() }
        .sheet(isPresented: $showReceiveSheet) { ReceiveView() }
        .sheet(isPresented: $showBuySheet) { BuyView() }
        .sheet(isPresented: $showSellSheet) { SellView() }
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

    // MARK: - Balance Section

    private var balanceSection: some View {
        VStack(spacing: 8) {
            Text(appState.totalBalanceUSD.usdFormatted)
                .font(.system(size: 48, weight: .bold, design: .rounded))
                .foregroundStyle(appState.paymentFlash ? .green : .primary)
                .contentTransition(.numericText())
                .animation(.default, value: appState.totalBalanceUSD)
                .animation(.easeInOut(duration: 0.3), value: appState.paymentFlash)

            Text(appState.totalBalanceSats.satsFormatted)
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
        .scaleEffect(flashScale)
        .padding(.top, 16)
    }

    // MARK: - Balance Bar (Stable / Native)

    private var balanceBarSection: some View {
        VStack(spacing: 8) {
            BalanceBarView(
                stableUSD: appState.stableUSD,
                nativeSats: appState.nativeBTC.sats,
                totalSats: appState.lightningBalanceSats,
                btcPrice: appState.btcPrice
            )

            HStack {
                Label(appState.stableUSD.usdFormatted, systemImage: "shield.fill")
                    .font(.caption)
                    .foregroundStyle(.green)
                Spacer()
                Label(appState.nativeBTC.sats.satsFormatted, systemImage: "bitcoinsign.circle.fill")
                    .font(.caption)
                    .foregroundStyle(.orange)
            }
        }
        .padding(.horizontal)
    }

    // MARK: - Action Buttons

    private var actionButtons: some View {
        VStack(spacing: 12) {
            HStack(spacing: 12) {
                ActionButton(title: "Send", icon: "arrow.up.circle.fill", color: .blue) {
                    showSendSheet = true
                }
                ActionButton(title: "Receive", icon: "arrow.down.circle.fill", color: .green) {
                    showReceiveSheet = true
                }
            }

            if !appState.nodeService.channels.isEmpty {
                HStack(spacing: 12) {
                    ActionButton(title: "Buy BTC", icon: "arrow.up.right.circle.fill", color: .orange) {
                        showBuySheet = true
                    }
                    ActionButton(title: "Sell BTC", icon: "arrow.down.right.circle.fill", color: .purple) {
                        showSellSheet = true
                    }
                }
            }
        }
    }

    // MARK: - Price Section

    private var priceSection: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text("BTC Price")
                    .foregroundStyle(.secondary)
                if appState.priceService.lastUpdate != .distantPast {
                    Text(appState.priceService.lastUpdate, style: .relative)
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
            }
            Spacer()
            if appState.priceService.isUpdating {
                ProgressView()
                    .controlSize(.small)
                    .padding(.trailing, 4)
            }
            Text(appState.btcPrice.usdFormatted)
                .fontWeight(.medium)
        }
        .padding()
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 12))
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
