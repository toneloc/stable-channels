import SwiftUI
import CoreImage.CIFilterBuiltins

struct FundWalletView: View {
    @Environment(AppState.self) private var appState
    @State private var address: String?
    @State private var isCopied = false

    var body: some View {
        ScrollView {
            VStack(spacing: 24) {
                Image(systemName: "arrow.down.circle")
                    .font(.system(size: 48))
                    .foregroundStyle(.blue)

                Text("Fund Your Wallet")
                    .font(.title2.bold())

                Text("Send Bitcoin to the address below to get started.")
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)

                if let address {
                    VStack(spacing: 16) {
                        // QR Code
                        if let qrImage = generateQRCode(from: "bitcoin:\(address)") {
                            Image(uiImage: qrImage)
                                .interpolation(.none)
                                .resizable()
                                .scaledToFit()
                                .frame(width: 200, height: 200)
                        }

                        Text(address)
                            .font(.system(.caption, design: .monospaced))
                            .padding()
                            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 8))
                            .textSelection(.enabled)

                        Button {
                            UIPasteboard.general.string = address
                            isCopied = true
                            DispatchQueue.main.asyncAfter(deadline: .now() + 2) { isCopied = false }
                        } label: {
                            Label(isCopied ? "Copied" : "Copy Address", systemImage: isCopied ? "checkmark" : "doc.on.doc")
                        }
                        .buttonStyle(.borderedProminent)
                    }
                } else {
                    ProgressView()
                        .task {
                            address = try? appState.nodeService.newOnchainAddress()
                        }
                }
            }
            .padding(32)
        }
        .navigationTitle("Receive On-Chain")
        .navigationBarTitleDisplayMode(.inline)
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
