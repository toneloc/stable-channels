import SwiftUI

struct SellView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.dismiss) private var dismiss
    @State private var amountStr = ""
    @State private var step: Step = .amount
    @State private var errorMessage: String?
    @State private var isExecuting = false
    @State private var pendingPaymentId: String?

    enum Step {
        case amount
        case confirm
        case done
    }

    private var maxSellUSD: Double {
        guard appState.btcPrice > 0 else { return 0 }
        let nativeSats = appState.stableChannel.nativeChannelBTC.sats
        return Double(nativeSats) / Double(Constants.satsInBTC) * appState.btcPrice
    }

    private var amountUSD: Double {
        Double(amountStr) ?? 0
    }

    private var btcAmount: Double {
        guard appState.btcPrice > 0 else { return 0 }
        return amountUSD / appState.btcPrice
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
            .navigationTitle("Sell BTC")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
        }
    }

    private var amountScreen: some View {
        VStack(spacing: 20) {
            Text("How much BTC to convert to USD?")
                .font(.headline)

            TextField("$0.00", text: $amountStr)
                .keyboardType(.decimalPad)
                .font(.system(size: 36, weight: .bold, design: .rounded))
                .multilineTextAlignment(.center)

            if amountUSD > 0 {
                Text(String(format: "≈ %.8f BTC", btcAmount))
                    .foregroundStyle(.secondary)
            }

            Text("Available: \(maxSellUSD.usdFormatted) in native BTC")
                .foregroundStyle(.secondary)

            if amountUSD > maxSellUSD && amountUSD > 0 {
                Text("Exceeds available native BTC")
                    .font(.caption)
                    .foregroundStyle(.red)
            }

            Spacer()

            Button("Continue") { step = .confirm }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .disabled(amountUSD <= 0 || amountUSD > maxSellUSD)
        }
    }

    private var confirmScreen: some View {
        VStack(spacing: 20) {
            Text("Confirm Sell")
                .font(.title2.bold())

            VStack(spacing: 12) {
                confirmRow("Amount", String(format: "$%.2f", amountUSD))
                confirmRow("Fee (1%)", String(format: "$%.2f", amountUSD * 0.01))
                confirmRow("BTC Price", appState.btcPrice.usdFormatted)
                Divider()
                confirmRow("You receive", String(format: "$%.2f USD", amountUSD * 0.99), bold: true)
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
                    Text("Sell BTC")
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

                Text("Trade Confirmed")
                    .font(.title2.bold())

                Text(String(format: "Sold %.8f BTC for %@", btcAmount, (amountUSD * 0.99).usdFormatted))
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            } else {
                Image(systemName: "clock.circle.fill")
                    .font(.system(size: 64))
                    .foregroundStyle(.orange)

                Text("Trade Pending")
                    .font(.title2.bold())

                Text(String(format: "Selling %.8f BTC for %@", btcAmount, (amountUSD * 0.99).usdFormatted))
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)

                ProgressView()
                    .padding(.top, 4)

                Text("Waiting for LSP confirmation...")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }

            Button("Done") { dismiss() }
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
        let sc = appState.stableChannel
        let totalUSD = USD.fromBitcoin(sc.stableReceiverBTC, price: appState.btcPrice).amount
        let feeUSD = amountUSD * 0.01  // 1% fee
        let price = appState.btcPrice
        do {
            guard let result = try appState.tradeService?.executeSell(
                sc: sc,
                amountUSD: amountUSD,
                feeUSD: feeUSD,
                price: price,
                maxUSD: totalUSD
            ) else {
                errorMessage = "Trade failed — check amount and try again"
                isExecuting = false
                return
            }

            // Do NOT apply trade yet — wait for PaymentSuccessful event (matches desktop)

            // Record trade in DB as pending
            let tradeDbId = try appState.databaseService?.recordTrade(
                channelId: sc.channelId,
                action: "sell",
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
                action: "sell"
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
