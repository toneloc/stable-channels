import SwiftUI
import CoreImage.CIFilterBuiltins
import LDKNode

struct ReceiveView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.dismiss) private var dismiss
    @State private var amountUSD = ""
    @State private var invoice: String?
    @State private var invoiceAmountSats: UInt64?
    @State private var errorMessage: String?
    @State private var isCopied = false
    @State private var showOnChain = false

    private var hasChannel: Bool {
        appState.nodeService.channels.contains { $0.isChannelReady }
    }

    private var enteredUSDValue: Double {
        Double(amountUSD) ?? 0
    }

    private var enteredSats: UInt64 {
        let price = appState.btcPrice
        guard price > 0, enteredUSDValue > 0 else { return 0 }
        return UInt64(enteredUSDValue / price * Double(Constants.satsInBTC))
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 24) {
                if let invoice {
                    invoiceDisplay(invoice)
                } else {
                    amountInput
                }

                if let error = errorMessage {
                    Text(error)
                        .foregroundStyle(.red)
                        .font(.caption)
                }

                Spacer()
            }
            .navigationTitle("Receive")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                }
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        showOnChain = true
                    } label: {
                        Label("On-Chain", systemImage: "link")
                    }
                }
            }
            .navigationDestination(isPresented: $showOnChain) {
                FundWalletView()
            }
        }
    }

    // MARK: - Amount Input

    private var amountInput: some View {
        VStack(spacing: 16) {
            Text("Amount (USD)")
                .font(.headline)

            TextField("0.00", text: $amountUSD)
                .keyboardType(.decimalPad)
                .font(.system(size: 32, weight: .bold, design: .rounded))
                .multilineTextAlignment(.center)
                .overlay(alignment: .leading) {
                    if !amountUSD.isEmpty {
                        GeometryReader { geo in
                            let textWidth = amountUSD.size(withAttributes: [
                                .font: UIFont.rounded(ofSize: 32, weight: .bold)
                            ]).width
                            Text("$")
                                .font(.system(size: 32, weight: .bold, design: .rounded))
                                .position(x: geo.size.width / 2 - textWidth / 2 - 10,
                                          y: geo.size.height / 2)
                        }
                    }
                }

            if enteredSats > 0 {
                Text("\(enteredSats.btcSpacedFormatted) BTC")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }

            if !hasChannel {
                Text("First payment — a channel will be opened automatically via LSP")
                    .font(.caption)
                    .foregroundStyle(.orange)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal)

                Text("$\(Int(Constants.maxChannelUSD)) Maximum")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }

            if !hasChannel && enteredUSDValue > Constants.maxChannelUSD {
                Text("Amount exceeds $\(Int(Constants.maxChannelUSD)) channel limit")
                    .font(.caption)
                    .foregroundStyle(.red)
            }

            Button("Generate Invoice") {
                createInvoice()
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(enteredSats == 0 || (!hasChannel && enteredUSDValue > Constants.maxChannelUSD))

            if hasChannel {
                Button("Any Amount") {
                    createVariableInvoice()
                }
                .buttonStyle(.bordered)
            }
        }
        .padding()
    }

    // MARK: - Invoice Display

    private func invoiceDisplay(_ invoiceStr: String) -> some View {
        VStack(spacing: 16) {
            // Amount summary
            if let sats = invoiceAmountSats, sats > 0 {
                VStack(spacing: 2) {
                    let price = appState.btcPrice
                    if price > 0 {
                        let usd = Double(sats) / Double(Constants.satsInBTC) * price
                        Text(usd.usdFormatted)
                            .font(.title2.bold())
                    }
                    Text("\(sats.btcSpacedFormatted) BTC")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
            } else {
                Text("Any amount")
                    .font(.title3)
                    .foregroundStyle(.secondary)
            }

            // QR Code
            if let qrImage = generateQRCode(from: invoiceStr) {
                Image(uiImage: qrImage)
                    .interpolation(.none)
                    .resizable()
                    .scaledToFit()
                    .frame(width: 220, height: 220)
                    .padding()
            }

            // Invoice text
            Text(invoiceStr)
                .font(.system(.caption2, design: .monospaced))
                .lineLimit(3)
                .truncationMode(.middle)
                .padding(.horizontal)
                .textSelection(.enabled)

            Button {
                UIPasteboard.general.string = invoiceStr
                isCopied = true
                DispatchQueue.main.asyncAfter(deadline: .now() + 2) { isCopied = false }
            } label: {
                Label(isCopied ? "Copied" : "Copy Invoice", systemImage: isCopied ? "checkmark" : "doc.on.doc")
            }
            .buttonStyle(.borderedProminent)
        }
    }

    // MARK: - Invoice Creation

    private func createInvoice() {
        let sats = enteredSats
        guard sats > 0 else { return }
        errorMessage = nil
        do {
            let inv: Bolt11Invoice
            if hasChannel {
                inv = try appState.nodeService.receivePayment(
                    amountMsat: sats * 1000,
                    description: "Stable Channels payment"
                )
            } else {
                // No channel yet — use JIT channel via LSPS2
                inv = try appState.nodeService.receiveViaJitChannel(
                    amountMsat: sats * 1000,
                    description: "Stable Channels payment"
                )
            }
            invoiceAmountSats = sats
            invoice = inv.description
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    private func createVariableInvoice() {
        errorMessage = nil
        do {
            let inv = try appState.nodeService.receiveVariablePayment(
                description: "Stable Channels payment"
            )
            invoiceAmountSats = nil
            invoice = inv.description
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    private func generateQRCode(from string: String) -> UIImage? {
        let context = CIContext()
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(string.uppercased().utf8)

        guard let outputImage = filter.outputImage else { return nil }
        let scaledImage = outputImage.transformed(by: CGAffineTransform(scaleX: 10, y: 10))
        guard let cgImage = context.createCGImage(scaledImage, from: scaledImage.extent) else { return nil }
        return UIImage(cgImage: cgImage)
    }
}
