import SwiftUI

struct SellView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.dismiss) private var dismiss
    @State private var amountStr = ""
    @State private var step: Step = .amount
    @State private var errorMessage: String?
    @State private var isExecuting = false

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

    private var doneScreen: some View {
        VStack(spacing: 20) {
            Image(systemName: "clock.arrow.circlepath")
                .font(.system(size: 64))
                .foregroundStyle(.orange)

            Text("Trade Pending")
                .font(.title2.bold())

            Text(String(format: "Selling %.8f BTC for $%.2f\nWaiting for payment confirmation...", btcAmount, amountUSD))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)

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
        do {
            guard let result = try appState.tradeService?.executeSell(
                sc: sc,
                amountUSD: amountUSD,
                feeUSD: feeUSD,
                price: appState.btcPrice,
                maxUSD: totalUSD
            ) else {
                errorMessage = "Trade failed — check amount and try again"
                isExecuting = false
                return
            }

            // Record trade as pending in DB
            let tradeDbId = try appState.databaseService?.recordTrade(
                channelId: sc.channelId,
                action: "sell",
                amountUSD: amountUSD,
                amountBTC: result.btcAmount,
                btcPrice: appState.btcPrice,
                feeUSD: feeUSD,
                paymentId: result.paymentId,
                status: "pending"
            ) ?? 0

            // Store pending trade — will be applied on PaymentSuccessful
            appState.pendingTradePayments[result.paymentId] = PendingTradePayment(
                newExpectedUSD: result.newExpectedUSD,
                price: appState.btcPrice,
                tradeDbId: tradeDbId,
                action: "sell"
            )

            step = .done
        } catch {
            errorMessage = error.localizedDescription
        }
        isExecuting = false
    }
}
