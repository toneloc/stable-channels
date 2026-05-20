import SwiftUI

struct BuyView: View {
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

    private var maxBuyUSD: Double {
        appState.stableChannel.expectedUSD.amount
    }

    private var amountUSD: Double {
        Double(amountStr) ?? 0
    }

    private var btcAmount: Double {
        guard appState.btcPrice > 0 else { return 0 }
        return amountUSD / appState.btcPrice
    }

    private var btcAmountFinal: Double {
        btcAmount * 0.99
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
            .navigationTitle(String(localized: "title_buy_btc", defaultValue: "Buy BTC"))
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
            Text(String(localized: "headline_how_much_usd", defaultValue: "How much USD to convert to BTC?"))
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
                Text(String(format: "≈ %.8f BTC", btcAmount))
                    .foregroundStyle(.secondary)
            }

            let availableStr = String(localized: "available_balance", defaultValue: "Available: ") + maxBuyUSD
                .usdFormatted
            Text(availableStr)
                .foregroundStyle(.secondary)

            if amountUSD > maxBuyUSD && amountUSD > 0 {
                Text(String(localized: "error_exceeds_balance", defaultValue: "Exceeds available stable balance"))
                    .font(.caption)
                    .foregroundStyle(.red)
            }

            Spacer()

            Button(String(localized: "button_continue", defaultValue: "Continue")) { step = .confirm }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .disabled(amountUSD <= 0 || amountUSD > maxBuyUSD)
        }
    }

    private var confirmScreen: some View {
        VStack(spacing: 20) {
            Text(String(localized: "title_confirm_buy", defaultValue: "Confirm Buy"))
                .font(.title2.bold())

            VStack(spacing: 12) {
                confirmRow(
                    String(localized: "label_amount", defaultValue: "Amount"),
                    String(format: "$%.2f", amountUSD)
                )
                confirmRow(
                    String(localized: "label_fee_percent", defaultValue: "Fee (1%)"),
                    String(format: "$%.2f", amountUSD * 0.01)
                )
                confirmRow(
                    String(localized: "label_btc_price", defaultValue: "BTC Price"),
                    appState.btcPrice.usdFormatted
                )
                Divider()
                confirmRow(
                    String(localized: "label_you_receive", defaultValue: "You receive"),
                    String(format: "%.8f BTC", btcAmount * 0.99),
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
                    Text(String(localized: "title_buy_btc", defaultValue: "Buy BTC"))
                        .frame(maxWidth: .infinity)
                }
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(isExecuting)
        }
    }

    private var tradeConfirmed: Bool {
        guard let pid = pendingPaymentId else { return false }
        return appState.pendingTradePayments[pid] == nil
    }

    private var doneScreen: some View {
        VStack(spacing: 20) {
            if tradeConfirmed {
                Image(systemName: "checkmark.circle.fill")
                    .font(.system(size: 64))
                    .foregroundStyle(.green)

                Text(String(localized: "status_trade_confirmed", defaultValue: "Trade Confirmed"))
                    .font(.title2.bold())

                Text(String(localized: "trade_bought_btc_for", defaultValue: "Bought ") + String(
                    format: "%.8f",
                    btcAmountFinal
                ) + " BTC for " + amountUSD.usdFormatted)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            } else {
                Image(systemName: "clock.circle.fill")
                    .font(.system(size: 64))
                    .foregroundStyle(.orange)

                Text(String(localized: "status_waiting_lsp", defaultValue: "Trade Pending"))
                    .font(.title2.bold())

                Text(String(localized: "trade_buying_btc_for", defaultValue: "Buying ") + String(
                    format: "%.8f",
                    btcAmountFinal
                ) + " BTC for " + amountUSD.usdFormatted)
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
        let feeUSD = amountUSD * 0.01 // 1% fee
        let price = appState.btcPrice
        do {
            guard let result = try appState.tradeService?.executeBuy(
                sc: sc,
                amountUSD: amountUSD,
                feeUSD: feeUSD,
                price: price
            ) else {
                errorMessage = String(
                    localized: "error_trade_failed",
                    defaultValue: "Trade failed — check amount and try again"
                )
                isExecuting = false
                return
            }

            // Do NOT apply trade yet — wait for PaymentSuccessful event (matches desktop)

            // Record trade in DB as pending
            let tradeDbId = try appState.databaseService?.recordTrade(
                channelId: sc.channelId,
                action: "buy",
                amountUSD: amountUSD,
                amountBTC: result.btcAmount,
                btcPrice: price,
                feeUSD: feeUSD,
                paymentId: result.paymentId,
                status: "pending"
            ) ?? 0

            // Track so PaymentSuccessful/PaymentFailed can apply or revert
            appState.pendingTradePayments[result.paymentId] = PendingTradePayment(
                newExpectedUSD: result.newExpectedUSD,
                price: price,
                tradeDbId: tradeDbId,
                action: "buy"
            )

            pendingPaymentId = result.paymentId
            appState.statusMessage = String(format: "Buy pending (fee: $%.2f)", feeUSD)
            step = .done
        } catch {
            errorMessage = error.localizedDescription
        }
        isExecuting = false
    }
}
