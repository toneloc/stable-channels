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

    private var tradePrice: Double { appState.accountingBTCPrice }

    private var maxSellUSD: Double {
        guard tradePrice > 0 else { return 0 }
        let stableSats = UInt64(appState.stableUSD / tradePrice * Double(Constants.satsInBTC))
        let nativeSats = appState.lightningBalanceSats > stableSats
            ? appState.lightningBalanceSats - stableSats : 0
        return Double(nativeSats) / Double(Constants.satsInBTC) * tradePrice
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
            .navigationTitle(String(localized: "title_sell_btc", defaultValue: "BTC → USD"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(String(localized: "button_cancel", defaultValue: "Cancel")) { dismiss() }
                }
            }
        }
    }

    private var amountScreen: some View {
        VStack(spacing: 20) {
            Text(String(localized: "headline_how_much_btc", defaultValue: "How much BTC to convert to USD?"))
                .font(.headline)

            TextField(String(localized: "placeholder_amount_usd", defaultValue: "0.00"), text: $amountStr)
                .keyboardType(.decimalPad)
                .font(.system(size: 36, weight: .bold, design: .rounded))
                .multilineTextAlignment(.center)
                .overlay(alignment: .leading) {
                    if !amountStr.isEmpty {
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
                Text(String(format: "≈ %.8f BTC", btcAmount))
                    .foregroundStyle(.secondary)
            }

            let availableStr = String(localized: "available_native_btc", defaultValue: "Available: ") + maxSellUSD
                .usdFormatted + " in native BTC"
            Text(availableStr)
                .foregroundStyle(.secondary)

            if amountUSD > maxSellUSD && amountUSD > 0 {
                Text(String(localized: "error_exceeds_native", defaultValue: "Exceeds available native BTC"))
                    .font(.caption)
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
                .disabled(amountUSD <= 0 || amountUSD > maxSellUSD || tradePrice <= 0)
        }
    }

    private var confirmScreen: some View {
        VStack(spacing: 20) {
            Text(String(localized: "title_confirm_sell", defaultValue: "Review BTC -> USD"))
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

                Text(String(localized: "trade_sold_btc_for", defaultValue: "Converted ") + String(
                    format: "%.8f",
                    btcAmountFinal
                ) + " BTC for " + netAmountUSD.usdFormatted)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            } else {
                Image(systemName: "clock.circle.fill")
                    .font(.system(size: 64))
                    .foregroundStyle(.orange)

                Text(String(localized: "status_waiting_lsp", defaultValue: "Order Pending"))
                    .font(.title2.bold())

                Text(String(localized: "trade_selling_btc_for", defaultValue: "Converting ") + String(
                    format: "%.8f",
                    btcAmountFinal
                ) + " BTC for " + netAmountUSD.usdFormatted)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)

                ProgressView()
                    .padding(.top, 4)

                Text(String(localized: "status_waiting_lsp", defaultValue: "Waiting for LSP confirmation..."))
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }

            Button(String(localized: "button_done", defaultValue: "Done")) { dismiss() }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
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
        let totalUSD = USD.fromBitcoin(sc.stableReceiverBTC, price: price).amount
        do {
            guard let result = try appState.tradeService?.executeSell(
                sc: sc,
                amountUSD: amountUSD,
                feeUSD: feeUSD,
                price: price,
                maxUSD: totalUSD
            ) else {
                errorMessage = String(
                    localized: "error_trade_failed",
                    defaultValue: "Order failed — check amount and try again"
                )
                isExecuting = false
                return
            }

            // View cache only; the prepared correlation was persisted before the fee send.
            appState.pendingTradePayments[result.paymentId] = PendingTradePayment(
                newExpectedUSD: result.newExpectedUSD,
                price: price,
                tradeDbId: result.tradeDbId,
                action: "sell",
                status: "sent"
            )

            pendingPaymentId = result.paymentId
            appState.statusMessage = String(format: "Sell pending (fee: $%.2f)", feeUSD)
            step = .done
        } catch {
            errorMessage = error.localizedDescription
        }
        isExecuting = false
    }
}
