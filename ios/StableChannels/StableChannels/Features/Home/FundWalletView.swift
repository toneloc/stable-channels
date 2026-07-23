import SwiftUI
import UIKit

struct FundWalletView: View {
    @Environment(AppState.self) private var appState
    @State private var address: String?
    @State private var bitcoinURI: String?
    @State private var isCopied = false
    @State private var showFullscreenQR = false
    @State private var loadError: Error?
    @State private var copyResetTask: Task<Void, Never>?

    var body: some View {
        ScrollView {
            VStack(spacing: 24) {
                if let address {
                    qrCodeSection(address: address)

                    addressDisplay(address: address)

                    actionButtons(address: address)
                } else if let error = loadError {
                    errorView(error: error)
                } else {
                    ProgressView()
                        .task {
                            do {
                                let addr = try await appState.nodeService.newOnchainAddress()
                                address = addr
                                bitcoinURI = QRCodeUtility.generateBitcoinURI(from: addr)
                                let oldAddr = appState.transactionLinkService.onchainReceiveAddress
                                appState.transactionLinkService.onchainReceiveAddress = addr
                                if let oldAddr, !oldAddr.isEmpty, oldAddr != addr {
                                    appState.mempoolWebSocketService.untrackAddress(oldAddr)
                                }
                                appState.mempoolWebSocketService.trackAddress(addr)
                            } catch {
                                loadError = error
                            }
                        }
                }
            }
            .padding(32)
        }
        .navigationTitle(String(localized: "title_on_chain_receive", defaultValue: "Onchain Receive"))
        .navigationBarTitleDisplayMode(.inline)
        .fullScreenCover(isPresented: $showFullscreenQR) {
            if let uri = bitcoinURI,
               let qrImage = QRCodeUtility.generate(from: uri) {
                FullscreenQRZoomView(qrImage: qrImage, isPresented: $showFullscreenQR)
            }
        }
        .onDisappear { copyResetTask?.cancel() }
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
        VStack(alignment: .leading, spacing: 6) {
            Text(String(localized: "label_your_address", defaultValue: "Your Bitcoin Address"))
                .font(.caption)
                .foregroundStyle(.secondary)

            Text(address)
                .font(.system(.caption, design: .monospaced))
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
                    copyAddress(address)
                }
        }
    }

    private func actionButtons(address: String) -> some View {
        HStack(spacing: 12) {
            Button {
                copyAddress(address)
            } label: {
                Image(systemName: isCopied ? "checkmark" : "doc.on.doc")
                    .font(.system(size: 17, weight: .semibold))
                    .frame(width: 44, height: 44)
                    .background(.ultraThinMaterial, in: Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(String(localized: "button_copy_address", defaultValue: "Copy Address"))

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

    private func copyAddress(_ address: String) {
        UIPasteboard.general.string = address
        isCopied = true
        copyResetTask?.cancel()
        copyResetTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            guard !Task.isCancelled else { return }
            isCopied = false
        }
    }
}
