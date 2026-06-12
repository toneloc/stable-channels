import SwiftUI
import PhotosUI

struct QRInputToolbar: ViewModifier {
    @Binding var text: String
    var sanitize: (String) -> String

    @State private var showScanner = false
    @State private var showPhotoPicker = false
    @State private var selectedPhotoItem: PhotosPickerItem?
    @State private var showQRAlert = false
    @State private var qrAlertMessage = ""

    private let qrErrorMessage = String(
        localized: "alert_qr_error",
        defaultValue: "Selected image doesn't contain a QR code."
    )
    private let qrAlertTitle = String(
        localized: "alert_no_qr",
        defaultValue: "No QR Code Found"
    )

    func body(content: Content) -> some View {
        content
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    HStack(spacing: 16) {
                        Button { showPhotoPicker = true } label: {
                            Image(systemName: "photo.on.rectangle")
                        }
                        .accessibilityLabel(String(localized: "button_pick_image", defaultValue: "Pick Image"))

                        Button { showScanner = true } label: {
                            Image(systemName: "qrcode.viewfinder")
                        }
                        .accessibilityLabel(String(localized: "button_scan_qr", defaultValue: "Scan QR"))
                    }
                }
            }
            .photosPicker(isPresented: $showPhotoPicker, selection: $selectedPhotoItem, matching: .images)
            .onChange(of: selectedPhotoItem) { _, newItem in
                guard let item = newItem else { return }
                Task.detached(priority: .userInitiated) {
                    let data = try? await item.loadTransferable(type: Data.self)
                    let code: String? = await {
                        guard let data, let image = UIImage(data: data) else { return nil }
                        return QRCodeExtractor.extract(from: image)
                    }()
                    await MainActor.run {
                        if let code {
                            text = sanitize(code)
                        } else {
                            qrAlertMessage = qrErrorMessage
                            showQRAlert = true
                        }
                        selectedPhotoItem = nil
                    }
                }
            }
            .sheet(isPresented: $showScanner) {
                InvoiceScanView(
                    onScan: { scanned in
                        text = sanitize(scanned)
                        showScanner = false
                    },
                    onCancel: { showScanner = false }
                )
            }
            .alert(qrAlertTitle, isPresented: $showQRAlert) {
                Button(String(localized: "button_ok", defaultValue: "OK"), role: .cancel) {}
            } message: {
                Text(qrAlertMessage)
            }
    }
}

extension View {
    func qrInputToolbar(
        text: Binding<String>,
        sanitize: @escaping (String) -> String = { $0 }
    ) -> some View {
        modifier(QRInputToolbar(text: text, sanitize: sanitize))
    }
}
