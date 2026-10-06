import SwiftUI

/// Dedicated presentation view displaying network unavailability in a clean monochrome style.
/// Shown during app startup or pull-to-refresh when the network cannot be reached.
/// Informs the user of the offline state, reassures fund safety, allows retrying,
/// and provides an option to navigate to the cached home view.
struct OfflinePageView: View {
    var isRetrying: Bool = false
    var onRetry: () -> Void
    var onGoToHome: (() -> Void)?

    @State private var spinAngle: Double = 0.0

    var body: some View {
        VStack(spacing: 20) {
            Spacer()

            Image(systemName: "wifi.exclamationmark")
                .font(.system(size: 46, weight: .light))
                .foregroundStyle(.secondary)

            VStack(spacing: 8) {
                Text(String(localized: "offline_title", defaultValue: "No Internet Connection"))
                    .font(.title3.weight(.bold))
                    .foregroundStyle(.primary)

                Text(String(
                    localized: "offline_body",
                    defaultValue: "Cannot connect to the server without an internet connection. Your wallet and keys remain safe on this device."
                ))
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .lineSpacing(3)
                .padding(.horizontal, 32)
            }

            VStack(spacing: 12) {
                Button {
                    triggerRetry()
                } label: {
                    HStack(spacing: 6) {
                        Image(systemName: "arrow.clockwise")
                            .font(.caption.weight(.medium))
                            .rotationEffect(.degrees(spinAngle))

                        Text(String(localized: "try_again", defaultValue: "Try again"))
                            .font(.subheadline.weight(.medium))
                    }
                    .foregroundStyle(.primary)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 2)
                }
                .buttonStyle(.bordered)
                .tint(.primary)
                .clipShape(Capsule())
                .disabled(isRetrying)

                if let onGoToHome {
                    Button {
                        UIImpactFeedbackGenerator(style: .light).impactOccurred()
                        onGoToHome()
                    } label: {
                        Text(String(localized: "go_to_home", defaultValue: "Go to Home"))
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 2)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.top, 4)

            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(uiColor: .systemBackground).ignoresSafeArea())
    }

    private func triggerRetry() {
        guard !isRetrying else { return }
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        withAnimation(.easeInOut(duration: 0.75)) {
            spinAngle += 360
        }
        onRetry()
    }
}

/// Backwards compatibility aliases.
typealias OfflineNoticeCard = OfflinePageView
typealias OfflineHomeView = OfflinePageView
typealias OfflineNoticeSheet = OfflinePageView

#Preview {
    OfflinePageView(
        isRetrying: false,
        onRetry: {},
        onGoToHome: {}
    )
}
