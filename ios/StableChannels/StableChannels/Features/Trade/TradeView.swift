import SwiftUI

/// The USD <-> BTC conversion sheet. Buy and sell share every step; only the limit, copy and
/// service call differ by `action`.
struct TradeView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.dismiss) private var dismiss
    @State private var amountStr: String
    @State private var step: Step = .amount
    @State private var errorMessage: String?
    @State private var isExecuting = false
    @State private var pendingPaymentId: String?
    let action: TradeAction
    let prefillAmountUSD: Double

    init(action: TradeAction, prefillAmountUSD: Double = 0) {
        self.action = action
        self.prefillAmountUSD = prefillAmountUSD
        let prefill = action == .buyBTC ? BuyAmountPolicy.maximumUsd(prefillAmountUSD) : prefillAmountUSD
        _amountStr = State(initialValue: prefillAmountUSD > 0 ? String(format: "%.2f", prefill) : "")
    }

    enum Step {
        case amount
        case confirm
        case done
    }

    private var form: TradeFormModel {
        TradeFormModel(
            action: action,
            amountText: amountStr,
            maxTradeUSD: TradeFormModel.maxTradeUSD(
                action: action,
                expectedUSD: appState.stableChannel.expectedUSD.amount,
                maxSellCents: appState.tradeService?.maxSellCents(sc: appState.stableChannel, price: tradePrice) ?? 0
            ),
            price: tradePrice
        )
    }

    private var maxTradeUSD: Double { form.maxTradeUSD }
    private var amountUSD: Double { form.amountUSD }
    private var feeUSD: Double { form.feeUSD }

    /// What the trade credits: BTC for a buy, the post-fee USD for a sell.
    private var receiveText: String {
        action == .buyBTC ? btcAmountFinalText : String(format: "$%.2f USD", form.netAmountUSD)
    }

    private var tradePrice: Double {
        appState.effectiveTradePrice
    }

    private var feeLabel: String {
        String(format: "Fee (%.0f%%)", Constants.stableChannelTradeFeeRate * 100)
    }

    private var btcAmountFinalText: String {
        Bitcoin.fromBTC(form.btcAmountFinal).formatted
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
            .navigationTitle(action == .buyBTC
                ? String(localized: "title_buy_btc", defaultValue: "USD → BTC")
                : String(localized: "title_sell_btc", defaultValue: "BTC → USD"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(
                        step == .done
                            ? String(localized: "button_close", defaultValue: "Close")
                            : String(localized: "button_cancel", defaultValue: "Cancel")
                    ) { dismiss() }
                }
            }
        }
    }

    private var amountScreen: some View {
        VStack(spacing: 20) {
            Text(action == .buyBTC
                ? String(localized: "headline_how_much_usd", defaultValue: "How much USD to convert to BTC?")
                : String(localized: "headline_how_much_btc", defaultValue: "How much BTC to convert to USD?"))
                .font(.headline)

            TextField(String(localized: "placeholder_amount_usd", defaultValue: "0.00"), text: $amountStr)
                .keyboardType(.decimalPad)
                .font(.system(size: 36, weight: .bold, design: .rounded))
                .multilineTextAlignment(.center)
                .overlay(alignment: .leading) {
                    if !amountStr.isEmpty {
                        // Measure the text width to position $ immediately before it
                        GeometryReader { geo in
                            let textWidth = amountStr.size(withAttributes: [
                                .font: UIFont.rounded(ofSize: 36, weight: .bold)
                            ]).width
                            Text(String(localized: "label_dollar_sign", defaultValue: "$"))
                                .font(.system(size: 36, weight: .bold, design: .rounded))
                                .position(x: geo.size.width / 2 - textWidth / 2 - 10,
                                          y: geo.size.height / 2)
                        }
                    }
                }

            if amountUSD > 0 {
                Text("≈ \(Bitcoin.fromBTC(form.btcAmount).formatted)")
                    .foregroundStyle(.secondary)
            }

            if !appState.isOnline {
                OfflineBadgeView()
            } else {
                switch action {
                case .buyBTC:
                    Text(String(localized: "available_balance", defaultValue: "Available: ") + maxTradeUSD.usdFormatted)
                        .foregroundStyle(.secondary)
                case .sellBTC:
                    Text(StabilizationPolicy.maximumMessage(form.maxTradeCents))
                        .foregroundStyle(.secondary)
                    Button(String(localized: "button_max", defaultValue: "Max")) {
                        amountStr = String(format: "%.2f", maxTradeUSD)
                    }
                }
            }

            if form.exceedsLimit {
                Text(action == .buyBTC
                    ? String(localized: "error_exceeds_balance", defaultValue: "Exceeds available stable balance")
                    : StabilizationPolicy.limitExceededMessage(form.maxTradeCents))
                    .font(.caption)
                    .multilineTextAlignment(.center)
                    .foregroundStyle(.red)
            }

            if tradePrice <= 0 {
                Text("A fresh BTC/USD consensus is required before trading")
                    .font(.caption)
                    .foregroundStyle(.orange)
            }

            Spacer()

            Button(String(localized: "button_continue", defaultValue: "Continue")) { step = .confirm }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .disabled(!form.acceptsAmount || tradePrice <= 0 || !appState.isOnline)
        }
    }

    private var confirmScreen: some View {
        VStack(spacing: 20) {
            Text(action == .buyBTC
                ? String(localized: "title_confirm_buy", defaultValue: "Review USD -> BTC")
                : String(localized: "title_confirm_sell", defaultValue: "Review BTC -> USD"))
                .font(.title2.bold())
            Text(String(localized: "subtitle_manage_exposure", defaultValue: "Manage your BTC exposure"))
                .font(.subheadline)
                .foregroundStyle(.secondary)

            VStack(spacing: 12) {
                confirmRow(
                    String(localized: "label_amount", defaultValue: "Amount"),
                    String(format: "$%.2f", amountUSD)
                )
                confirmRow(
                    feeLabel,
                    String(format: "$%.2f", feeUSD)
                )
                confirmRow(
                    String(localized: "label_btc_price", defaultValue: "BTC Price"),
                    tradePrice.usdFormatted
                )
                Divider()
                confirmRow(
                    String(localized: "label_you_receive", defaultValue: "You receive"),
                    receiveText,
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
                    Text(String(localized: "button_confirm_order", defaultValue: "Confirm Order"))
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
    private var tradeOutcome: TradeOutcome? {
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

                Text(String(localized: "status_trade_rejected", defaultValue: "Order Rejected"))
                    .font(.title2.bold())

                Text(tradeOutcome?.message ?? "The provider could not process the trade.")
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            } else if tradeConfirmed {
                Image(systemName: "checkmark.circle.fill")
                    .font(.system(size: 64))
                    .foregroundStyle(.green)

                Text(String(localized: "status_trade_confirmed", defaultValue: "Order Confirmed"))
                    .font(.title2.bold())

                Text((action == .buyBTC
                        ? String(localized: "trade_bought_btc_for", defaultValue: "Converted ")
                        : String(localized: "trade_sold_btc_for", defaultValue: "Converted "))
                    + btcAmountFinalText + " for " + form.tradedUSD.usdFormatted)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            } else {
                CurveProgressIndicator(curve: .sixPetalSpiral, size: 72, tint: .orange)
                    .padding(.bottom, 4)

                Text(String(localized: "status_order_pending", defaultValue: "Order Pending"))
                    .font(.title2.bold())

                Text((action == .buyBTC
                        ? String(localized: "trade_buying_btc_for", defaultValue: "Converting ")
                        : String(localized: "trade_selling_btc_for", defaultValue: "Converting "))
                    + btcAmountFinalText + " for " + form.tradedUSD.usdFormatted)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)

                Text(String(localized: "status_waiting_lsp", defaultValue: "Waiting for LSP confirmation..."))
                    .font(.caption)
                    .foregroundStyle(.tertiary)

                Text(String(
                    localized: "note_trade_result_unknown",
                    defaultValue: "Do not place this order again while its result is unknown."
                ))
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            }

            if PaymentResultControls.showsTradeDone(outcome: tradeOutcome) {
                Button(String(localized: "button_done", defaultValue: "Done")) { dismiss() }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
            }
        }
        .task(id: pendingPaymentId) {
            guard let pid = pendingPaymentId else { return }
            await TradeOutcomePolling.awaitOutcome(
                isKnown: { appState.tradeOutcomes[pid] != nil },
                refresh: { appState.refreshTradeOutcome(paymentId: pid) }
            )
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
            errorMessage = String(
                localized: "error_offline_trade",
                defaultValue: "You’re offline. Trades cannot be executed until network connectivity is restored."
            )
            return
        }
        isExecuting = true
        errorMessage = nil
        appState.ensureLSPConnected()
        let sc = appState.stableChannel
        let price = tradePrice
        guard price > 0 else {
            errorMessage = "A fresh BTC/USD consensus is required before trading"
            isExecuting = false
            return
        }
        // A generic "order failed" belongs only to an actually absent service. A refusal from
        // the service throws its own TradeValidationError reason and is rendered as-is below,
        // never re-labeled (issue #272).
        guard let service = appState.tradeService else {
            errorMessage = String(
                localized: "error_trade_service_unavailable",
                defaultValue: "Trade service unavailable"
            )
            isExecuting = false
            return
        }
        do {
            let execute = action == .buyBTC ? service.executeBuy : service.executeSell
            let result = try execute(sc, amountUSD, feeUSD, price)

            // View cache only; the prepared correlation was persisted before the fee send.
            appState.pendingTradePayments[result.paymentId] = PendingTradePayment(
                newExpectedUSD: result.newExpectedUSD,
                price: price,
                tradeDbId: result.tradeDbId,
                action: action.rawValue,
                status: "sent"
            )

            pendingPaymentId = result.paymentId
            appState.statusMessage = action == .buyBTC
                ? String(format: "Buy pending (fee: $%.2f)", feeUSD)
                : String(format: "Sell pending (fee: $%.2f)", feeUSD)
            step = .done
        } catch {
            errorMessage = error.localizedDescription
        }
        isExecuting = false
    }
}

/// Polls until the trade's outcome is known. The notification extension commits results
/// straight to the database without touching the in-memory outcome map, so a trade resolved
/// while backgrounded would otherwise stay pending.
enum TradeOutcomePolling {
    @MainActor
    static func awaitOutcome(
        isKnown: () -> Bool,
        refresh: () -> Void,
        interval: Duration = .seconds(2)
    ) async {
        while !Task.isCancelled, !isKnown() {
            refresh()
            try? await Task.sleep(for: interval)
        }
    }
}

#Preview("Trade - Order Pending") {
    VStack(spacing: 20) {
        CurveProgressIndicator(curve: .sixPetalSpiral, size: 72, tint: .orange)
            .padding(.bottom, 4)

        Text("Order Pending")
            .font(.title2.bold())

        Text("Converting 0.00500000 BTC for $485.20")
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
