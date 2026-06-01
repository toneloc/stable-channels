import SwiftUI
import UIKit

struct FundWalletView: View {
    @Environment(AppState.self) private var appState
    @State private var address: String?
    @State private var bitcoinURI: String?
    @State private var isCopied = false
    @State private var showFullscreenQR = false
    @State private var loadError: Error?

    var body: some View {
        ScrollView {
            VStack(spacing: 24) {
                if let address {
                    qrCodeSection(address: address)

                    addressDisplay(address: address)

                    copyButton(address: address)
                } else if let error = loadError {
                    errorView(error: error)
                } else {
                    ProgressView()
                        .task {
                            do {
                                let addr = try await appState.nodeService.newOnchainAddress()
                                address = addr
                                bitcoinURI = QRCodeUtility.generateBitcoinURI(from: addr)
                                appState.onchainReceiveAddress = addr
                            } catch {
                                loadError = error
                            }
                        }
                }
            }
            .padding(32)
        }
        .navigationTitle(String(localized: "title_on_chain_receive", defaultValue: "On-chain Receive"))
        .navigationBarTitleDisplayMode(.inline)
        .fullScreenCover(isPresented: $showFullscreenQR) {
            if let uri = bitcoinURI,
               let qrImage = QRCodeUtility.generate(from: uri) {
                FullscreenQRZoomView(qrImage: qrImage, isPresented: $showFullscreenQR)
            }
        }
    }

    // MARK: - Subviews

    private func qrCodeSection(address _: String) -> some View {
        VStack(spacing: 4) {
            if let uri = bitcoinURI,
               let qrImage = QRCodeUtility.generate(from: uri) {
                Image(uiImage: qrImage)
                    .interpolation(.none)
                    .resizable()
                    .scaledToFit()
                    .frame(width: 200, height: 200)
                    .accessibilityLabel(String(
                        localized: "Bitcoin address QR code",
                        defaultValue: "Bitcoin address QR code"
                    ))
                    .accessibilityHint(String(localized: "Tap to enlarge", defaultValue: "Tap to enlarge"))
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
    }

    private func addressDisplay(address: String) -> some View {
        Text(address)
            .font(.system(.caption, design: .monospaced))
            .padding()
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 8))
            .textSelection(.enabled)
    }

    private func copyButton(address: String) -> some View {
        VStack(spacing: 12) {
            Button {
                UIPasteboard.general.string = address
                isCopied = true
                Task {
                    try? await Task.sleep(nanoseconds: 2_000_000_000)
                    isCopied = false
                }
            } label: {
                Label(
                    isCopied
                        ? String(localized: "button_copied", defaultValue: "Copied")
                        : String(localized: "button_copy_address", defaultValue: "Copy Address"),
                    systemImage: isCopied ? "checkmark" : "doc.on.doc"
                )
            }
            .buttonStyle(.borderedProminent)
            .accessibilityLabel(isCopied
                ? String(localized: "button_copied", defaultValue: "Copied")
                : String(localized: "button_copy_address", defaultValue: "Copy Address"))

            Button {
                shareQR()
            } label: {
                Label(
                    String(localized: "button_share_qr", defaultValue: "Share QR"),
                    systemImage: "square.and.arrow.up"
                )
            }
            .buttonStyle(.bordered)
        }
    }

    private func shareQR() {
        guard let uri = bitcoinURI,
              let qrImage = QRCodeUtility.generate(from: uri),
              let addr = address else { return }

        let shareImage = ShareableQRGenerator.generateShareImage(
            qrImage: qrImage,
            invoice: addr,
            amount: nil,
            isOnChain: true
        )

        Task { @MainActor in
            showFullscreenQR = false
            try? await Task.sleep(nanoseconds: 100_000_000)

            let addressToShare = uri.replacingOccurrences(of: "bitcoin:", with: "")
            ShareSheetPresenter.present(items: [shareImage, addressToShare])
        }
    }

    private func errorView(error: Error) -> some View {
        VStack(spacing: 16) {
            Image(systemName: "exclamationmark.triangle")
                .font(.largeTitle)
                .foregroundStyle(.orange)
            Text(error.localizedDescription)
                .font(.body)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Button(String(localized: "button_retry", defaultValue: "Retry")) {
                loadError = nil
                address = nil
            }
            .buttonStyle(.bordered)
        }
        .padding()
    }
}
