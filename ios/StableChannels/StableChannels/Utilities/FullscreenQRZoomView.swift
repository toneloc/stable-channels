import SwiftUI
import UIKit

struct FullscreenQRZoomView: View {
    private let horizontalPadding: CGFloat = 48
    let qrImage: UIImage
    @Binding var isPresented: Bool

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()

            GeometryReader { geo in
                let side = min(geo.size.width, geo.size.height) - horizontalPadding
                Image(uiImage: qrImage)
                    .interpolation(.none)
                    .resizable()
                    .scaledToFit()
                    .frame(width: side, height: side)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .accessibilityLabel(String(localized: "QR Code", defaultValue: "QR Code"))
            .accessibilityHint(String(localized: "Tap to close", defaultValue: "Tap to close"))

            VStack {
                Spacer()
                HStack(spacing: 4) {
                    Image(systemName: "xmark.circle")
                        .font(.caption)
                    Text(String(localized: "Tap to close", defaultValue: "Tap to close"))
                        .font(.caption)
                }
                .foregroundStyle(.white.opacity(0.7))
                .padding(.bottom, 48)
            }
        }
        .onTapGesture {
            withAnimation(.spring(response: 0.3, dampingFraction: 0.8)) {
                isPresented = false
            }
        }
    }
}
