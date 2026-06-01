import SwiftUI
import LDKNode
import UIKit

struct ReceiveView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.dismiss) private var dismiss
    @State private var amountUSD = ""
    @State private var invoice: String?
    @State private var invoiceAmountSats: UInt64?
    @State private var errorMessage: String?
    @State private var isCopied = false
    @State private var showOnChain = false
    @State private var showFullscreenQR = false

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
            .navigationTitle(String(localized: "title_receive", defaultValue: "Receive"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(String(localized: "button_done", defaultValue: "Done")) { dismiss() }
                }
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        showOnChain = true
                    } label: {
                        Image(systemName: "link")
                    }
                    .accessibilityLabel(String(localized: "toolbar_on_chain", defaultValue: "On-Chain"))
                }
            }
            .navigationDestination(isPresented: $showOnChain) {
                FundWalletView()
            }
            .fullScreenCover(isPresented: $showFullscreenQR) {
                if let invoice, let qrImage = QRCodeUtility.generate(from: invoice) {
                    FullscreenQRZoomView(qrImage: qrImage, isPresented: $showFullscreenQR)
                }
            }
        }
    }

    // MARK: - Amount Input

    private var amountInput: some View {
        VStack(spacing: 16) {
            Text(String(localized: "placeholder_amount", defaultValue: "Amount (USD)"))
                .font(.headline)

            TextField(String(localized: "placeholder_amount_usd", defaultValue: "0.00"), text: $amountUSD)
                .keyboardType(.decimalPad)
                .font(.system(size: 32, weight: .bold, design: .rounded))
                .multilineTextAlignment(.center)
                .onChange(of: amountUSD) { _, new in
                    amountUSD = InputSanitizer.decimal(new)
                }
                .overlay(alignment: .leading) {
                    if !amountUSD.isEmpty {
                        GeometryReader { geo in
                            let textWidth = amountUSD.size(withAttributes: [
                                .font: UIFont.rounded(ofSize: 32, weight: .bold)
                            ]).width
                            Text(String(localized: "label_dollar_sign", defaultValue: "$"))
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
                Text(String(
                    localized: "info_first_payment",
                    defaultValue: "First payment — a channel will be opened automatically via LSP"
                ))
                .font(.caption)
                .foregroundStyle(.orange)
                .multilineTextAlignment(.center)
                .padding(.horizontal)

                Text(String(
                    format: NSLocalizedString("max_channel_limit", comment: ""),
                    Int64(Constants.maxChannelUSD)
                ))
                .font(.caption2)
                .foregroundStyle(.secondary)
            }

            if !hasChannel && enteredUSDValue > Constants.maxChannelUSD {
                let maxAmount = Int(Constants.maxChannelUSD)
                let limitStr = String(
                    localized: "error_exceeds_limit",
                    defaultValue: "Amount exceeds $\(maxAmount) channel limit"
                )
                Text(limitStr)
                    .font(.caption)
                    .foregroundStyle(.red)
            }

            Button(String(localized: "button_generate_invoice", defaultValue: "Generate Invoice")) {
                createInvoice()
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(enteredSats == 0 || (!hasChannel && enteredUSDValue > Constants.maxChannelUSD))

            if hasChannel {
                Button(String(localized: "button_any_amount", defaultValue: "Any Amount")) {
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
                Text(String(localized: "label_any_amount", defaultValue: "Any amount"))
                    .font(.title3)
                    .foregroundStyle(.secondary)
            }

            if let qrImage = QRCodeUtility.generate(from: invoiceStr) {
                VStack(spacing: 4) {
                    Image(uiImage: qrImage)
                        .interpolation(.none)
                        .resizable()
                        .scaledToFit()
                        .frame(width: 220, height: 220)
                        .padding()
                        .onTapGesture {
                            withAnimation(.spring(response: 0.35, dampingFraction: 0.75)) {
                                showFullscreenQR = true
                            }
                        }

                    HStack(spacing: 4) {
                        Image(systemName: "arrow.up.left.and.arrow.down.right")
                            .font(.caption2)
                        Text(String(localized: "Tap to enlarge", defaultValue: "Tap to enlarge"))
                            .font(.caption2)
                    }
                    .foregroundStyle(.secondary)
                }
            }

            VStack(alignment: .leading, spacing: 6) {
                Text(String(localized: "label_your_invoice", defaultValue: "Your Lightning Invoice"))
                    .font(.caption)
                    .foregroundStyle(.secondary)

                Text(invoiceStr)
                    .font(.system(.caption2, design: .monospaced))
                    .lineLimit(3)
                    .truncationMode(.middle)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(12)
                    .background(
                        ZStack {
                            RoundedRectangle(cornerRadius: 10)
                                .fill(.ultraThinMaterial)
                            RoundedRectangle(cornerRadius: 10)
                                .fill(isCopied ? Color.green.opacity(0.25) : Color.clear)
                        }
                    )
                    .overlay(
                        RoundedRectangle(cornerRadius: 10)
                            .stroke(isCopied ? Color.green : Color.primary.opacity(0.1), lineWidth: 1)
                    )
                    .overlay(alignment: .center) {
                        if isCopied {
                            Label(
                                String(localized: "button_copied", defaultValue: "Copied"),
                                systemImage: "checkmark.circle.fill"
                            )
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(.green)
                            .padding(.horizontal, 12)
                            .padding(.vertical, 6)
                            .background(.regularMaterial, in: Capsule())
                            .transition(.scale.combined(with: .opacity))
                        }
                    }
                    .contentShape(Rectangle())
                    .animation(.spring(response: 0.3, dampingFraction: 0.7), value: isCopied)
                    .onTapGesture {
                        UIPasteboard.general.string = invoiceStr
                        isCopied = true
                        Task {
                            try? await Task.sleep(nanoseconds: 2_000_000_000)
                            isCopied = false
                        }
                    }
            }
            .padding(.horizontal)

            HStack(spacing: 12) {
                Button {
                    UIPasteboard.general.string = invoiceStr
                    isCopied = true
                    Task {
                        try? await Task.sleep(nanoseconds: 2_000_000_000)
                        isCopied = false
                    }
                } label: {
                    Image(systemName: isCopied ? "checkmark" : "doc.on.doc")
                        .font(.system(size: 17, weight: .semibold))
                        .frame(width: 44, height: 44)
                        .background(.ultraThinMaterial, in: Circle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(String(localized: "button_copy_invoice", defaultValue: "Copy Invoice"))

                Button {
                    shareQR()
                } label: {
                    Image(systemName: "square.and.arrow.up")
                        .font(.system(size: 17, weight: .semibold))
                        .frame(width: 44, height: 44)
                        .background(.ultraThinMaterial, in: Circle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(String(localized: "button_share_qr", defaultValue: "Share QR"))

                Button {
                    withAnimation(.spring(response: 0.35, dampingFraction: 0.75)) {
                        showFullscreenQR = true
                    }
                } label: {
                    Image(systemName: "arrow.up.left.and.arrow.down.right")
                        .font(.system(size: 17, weight: .semibold))
                        .frame(width: 44, height: 44)
                        .background(.ultraThinMaterial, in: Circle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(String(localized: "button_enlarge_qr", defaultValue: "Enlarge QR"))
            }
        }
    }

    private func shareQR() {
        guard let invoiceStr = invoice,
              let qrImage = QRCodeUtility.generate(from: invoiceStr) else { return }

        let amountText: String?
        if let sats = invoiceAmountSats, sats > 0 {
            amountText = "\(sats.btcSpacedFormatted) BTC"
        } else {
            amountText = nil
        }

        let shareImage = ShareableQRGenerator.generateShareImage(
            qrImage: qrImage,
            invoice: invoiceStr,
            amount: amountText,
            isOnChain: false
        )

        Task { @MainActor in
            showFullscreenQR = false
            try? await Task.sleep(nanoseconds: 100_000_000)

            ShareSheetPresenter.present(items: [shareImage, invoiceStr])
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
}
