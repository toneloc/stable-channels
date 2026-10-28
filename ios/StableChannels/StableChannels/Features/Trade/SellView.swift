import SwiftUI

struct SellView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.dismiss) private var dismiss
    @State private var amountStr: String
    @State private var step: Step = .amount
    @State private var errorMessage: String?
    @State private var isExecuting = false
    @State private var pendingPaymentId: String?
    let prefillAmountUSD: Double

    init(prefillAmountUSD: Double = 0) {
        self.prefillAmountUSD = prefillAmountUSD
        _amountStr = State(initialValue: prefillAmountUSD > 0
            ? String(format: "%.2f", prefillAmountUSD)
            : "")
    }

    enum Step {
        case amount
        case confirm
        case done
    }

    private var tradePrice: Double {
        appState.effectiveTradePrice
    }

    private var maxSellUSD: Double {
        Double(appState.tradeService?.maxSellCents(sc: appState.stableChannel, price: tradePrice) ?? 0) / 100
    }

    private var amountUSD: Double {
        Double(amountStr) ?? 0
    }

    private var feeUSD: Double {
        amountUSD * Constants.stableChannelTradeFeeRate
    }

    private var netAmountUSD: Double {
        amountUSD - feeUSD
    }

    private var feeLabel: String {
        String(format: "Fee (%.0f%%)", Constants.stableChannelTradeFeeRate * 100)
    }

    private var btcAmount: Double {
        guard tradePrice > 0 else { return 0 }
        return amountUSD / tradePrice
    }

    private var btcAmountFinalText: String {
        Bitcoin.fromBTC(btcAmountFinal).formatted
    }

    private var btcAmountFinal: Double {
        guard tradePrice > 0 else { return 0 }
        return netAmountUSD / tradePrice
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 24) {
                switch step {
                case .amount:
                    amountScreen
                case .confirm:
                    confirmScreen
                case .done:
                    doneScreen
                }
            }
            .padding()
            .navigationTitle(String(localized: .titleSellBtc))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(String(localized: .buttonCancel)) { dismiss() }
                }
            }
        }
    }

    private var amountScreen: some View {
        VStack(spacing: 20) {
            Text(String(localized: .headlineHowMuchBtc))
                .font(.headline)

            TextField(String(localized: .placeholderAmountUsd), text: $amountStr)
                .keyboardType(.decimalPad)
                .font(.system(size: 36, weight: .bold, design: .rounded))
                .multilineTextAlignment(.center)
                .overlay(alignment: .leading) {
                    if !amountStr.isEmpty {
                        GeometryReader { geo in
                            let textWidth = amountStr.size(withAttributes: [
                                .font: UIFont.rounded(ofSize: 36, weight: .bold)
                            ]).width
                            Text(String(localized: .labelDollarSign))
                                .font(.system(size: 36, weight: .bold, design: .rounded))
                                .position(x: geo.size.width / 2 - textWidth / 2 - 10,
                                          y: geo.size.height / 2)
                        }
                    }
                }

            if amountUSD > 0 {
                Text("≈ \(Bitcoin.fromBTC(btcAmount).formatted)")
                    .foregroundStyle(.secondary)
            }

            if !appState.isOnline {
                OfflineBadgeView()
            } else {
                Text(StabilizationPolicy.maximumMessage(UInt64(maxSellUSD * 100 + 1e-7)))
                    .foregroundStyle(.secondary)
                Button(String(localized: .buttonMax)) {
                    amountStr = String(format: "%.2f", maxSellUSD)
                }
            }
            if amountUSD > maxSellUSD && amountUSD > 0 {
                Text(StabilizationPolicy.limitExceededMessage(UInt64(maxSellUSD * 100 + 1e-7)))
                    .font(.caption)
                    .multilineTextAlignment(.center)
                    .foregroundStyle(.red)
            }

            if tradePrice <= 0 {
                Text(String(localized: .errorFreshPriceRequired))
                    .font(.caption)
                    .foregroundStyle(.orange)
            }

            Spacer()

            Button(String(localized: .buttonContinue)) { step = .confirm }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .disabled(amountUSD <= 0 || amountUSD > maxSellUSD || tradePrice <= 0 || !appState
                    .isOnline)
        }
    }

    private var confirmScreen: some View {
        VStack(spacing: 20) {
            Text(String(localized: .titleConfirmSell))
                .font(.title2.bold())
            Text(String(localized: .subtitleManageExposure))
                .font(.subheadline)
                .foregroundStyle(.secondary)

            VStack(spacing: 12) {
                confirmRow(
                    String(localized: .labelAmount),
                    String(format: "$%.2f", amountUSD)
                )
                confirmRow(
                    feeLabel,
                    String(format: "$%.2f", feeUSD)
                )
                confirmRow(
                    String(localized: .labelBtcPrice),
                    tradePrice.usdFormatted
                )
                Divider()
                confirmRow(
                    String(localized: .labelYouReceive),
                    String(format: "$%.2f USD", netAmountUSD),
                    bold: true
                )
            }
            .padding()
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 12))

            if let error = errorMessage {
                Text(error)
                    .font(.caption)
                    .foregroundStyle(.red)
                    .padding(.horizontal)
            }

            Spacer()

            Button {
                executeTrade()
            } label: {
                if isExecuting {
                    ProgressView()
                        .frame(maxWidth: .infinity)
                } else {
                    Text(String(localized: .buttonConfirmOrder))
                        .frame(maxWidth: .infinity)
                }
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(isExecuting || tradePrice <= 0)
        }
    }

    // Only a signed, correlated acceptance confirms the order and only a signed
    // rejection fails it. Absence from the pending map proves nothing — a rejection
    // also clears it, and the old absence heuristic showed "Order Confirmed" for
    // rejected trades (caught by e2e flow 13).
    private var tradeOutcome: AppState.TradeOutcome? {
        guard let pid = pendingPaymentId else { return nil }
        return appState.tradeOutcomes[pid]
    }

    private var tradeConfirmed: Bool { tradeOutcome?.accepted == true }
    private var tradeRejected: Bool { tradeOutcome?.accepted == false }

    private var doneScreen: some View {
        VStack(spacing: 20) {
            if tradeRejected {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 64))
                    .foregroundStyle(.red)

                Text(String(localized: .statusTradeRejected))
                    .font(.title2.bold())

                Text(tradeOutcome?.message ?? String(localized: .rejectionDefault))
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            } else if tradeConfirmed {
                Image(systemName: "checkmark.circle.fill")
                    .font(.system(size: 64))
                    .foregroundStyle(.green)

                Text(String(localized: .statusTradeConfirmed))
                    .font(.title2.bold())

                Text(.tradeSoldBtcFor(btcAmountFinalText, netAmountUSD.usdFormatted))
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            } else {
                CurveProgressIndicator(curve: .sixPetalSpiral, size: 72, tint: .orange)
                    .padding(.bottom, 4)

                Text(String(localized: .statusStabilityPending))
                    .font(.title2.bold())

                Text(.tradeSellingBtcFor(btcAmountFinalText, netAmountUSD.usdFormatted))
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)

                Text(String(localized: .statusWaitingLsp))
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }

            Button(String(localized: .buttonDone)) { dismiss() }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
        }
        // The NSE commits results straight to SQLite without touching the in-memory
        // outcome map, so poll the database while the result is unknown — otherwise a
        // trade resolved while backgrounded stays "Order Pending".
        .task(id: pendingPaymentId) {
            guard let pid = pendingPaymentId else { return }
            while !Task.isCancelled, appState.tradeOutcomes[pid] == nil {
                appState.refreshTradeOutcome(paymentId: pid)
                try? await Task.sleep(nanoseconds: 2_000_000_000)
            }
        }
    }

    private func confirmRow(_ label: String, _ value: String, bold: Bool = false) -> some View {
        HStack {
            Text(label)
            Spacer()
            Text(value)
                .fontWeight(bold ? .semibold : .regular)
        }
    }

    private func executeTrade() {
        guard appState.isOnline else {
            errorMessage = String(localized: .errorOfflineTrade)
            return
        }
        isExecuting = true
        errorMessage = nil
        appState.ensureLSPConnected()
        let sc = appState.stableChannel
        let price = tradePrice
        guard price > 0 else {
            errorMessage = String(localized: .errorFreshPriceRequired)
            isExecuting = false
            return
        }
        // A generic "order failed" belongs only to an actually absent service. A refusal from
        // the service throws its own TradeValidationError reason and is rendered as-is below,
        // never re-labeled (issue #272).
        guard let service = appState.tradeService else {
            errorMessage = String(localized: .errorTradeServiceUnavailable)
            isExecuting = false
            return
        }
        do {
            let result = try service.executeSell(
                sc: sc,
                amountUSD: amountUSD,
                feeUSD: feeUSD,
                price: price
            )

            // View cache only; the prepared correlation was persisted before the fee send.
            appState.pendingTradePayments[result.paymentId] = PendingTradePayment(
                newExpectedUSD: result.newExpectedUSD,
                price: price,
                tradeDbId: result.tradeDbId,
                action: "sell",
                status: "sent"
            )

            pendingPaymentId = result.paymentId
            appState.statusMessage = String(localized: .statusStabilizePendingFee(feeUSD.usdFormatted))
            step = .done
        } catch {
            errorMessage = error.localizedDescription
        }
        isExecuting = false
    }
}

#Preview("Stabilize - Pending") {
    VStack(spacing: 20) {
        CurveProgressIndicator(curve: .sixPetalSpiral, size: 72, tint: .orange)
            .padding(.bottom, 4)

        Text("Stability Change Pending")
            .font(.title2.bold())

        Text("Stabilizing 0.00500000 BTC as $485.20")
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)

        Text("Waiting for LSP confirmation...")
            .font(.caption)
            .foregroundStyle(.tertiary)

        Button("Done") { }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
    }
    .padding()
}
