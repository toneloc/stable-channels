import SwiftUI

struct OfflinePageView: View {
    var isRetrying: Bool = false
    var onRetry: () async -> Void
    var onGoToHome: (() -> Void)?

    @State private var isSpinning: Bool = false
    @State private var iconScale: CGFloat = 0.75
    @State private var iconOpacity: Double = 0.0
    @State private var iconBounceTrigger: Int = 0

    var body: some View {
        VStack(spacing: 20) {
            Spacer()

            Image(systemName: "wifi.exclamationmark")
                .font(.system(size: 46, weight: .light))
                .foregroundStyle(.secondary)
                .symbolEffect(.bounce.up.byLayer, value: iconBounceTrigger)
                .scaleEffect(iconScale)
                .opacity(iconOpacity)
                .onAppear {
                    iconScale = 0.75
                    iconOpacity = 0.0
                    withAnimation(.spring(response: 0.5, dampingFraction: 0.62)) {
                        iconScale = 1.0
                        iconOpacity = 1.0
                    }
                    iconBounceTrigger += 1
                }

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
                            .rotationEffect(.degrees(isSpinning ? 360 : 0))
                            .animation(
                                isSpinning
                                    ? .linear(duration: 0.85).repeatForever(autoreverses: false)
                                    : .easeInOut(duration: 0.25),
                                value: isSpinning
                            )

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
                .blur(radius: (isRetrying || isSpinning) ? 1.2 : 0)
                .opacity((isRetrying || isSpinning) ? 0.65 : 1.0)
                .disabled(isRetrying || isSpinning)
                .animation(.easeInOut(duration: 0.25), value: isRetrying || isSpinning)
                .onAppear {
                    if isRetrying {
                        isSpinning = true
                    }
                }
                .onChange(of: isRetrying) { _, retrying in
                    if retrying {
                        isSpinning = true
                        iconBounceTrigger += 1
                    } else {
                        isSpinning = false
                    }
                }

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
        guard !isRetrying && !isSpinning else { return }
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        iconBounceTrigger += 1
        isSpinning = true

        Task { @MainActor in
            async let minSpinDelay: Void = Task.sleep(nanoseconds: 850_000_000)
            async let retryAction: Void = onRetry()
            _ = try? await (minSpinDelay, retryAction)

            isSpinning = false
        }
    }
}

typealias OfflineNoticeCard = OfflinePageView
typealias OfflineHomeView = OfflinePageView
typealias OfflineNoticeSheet = OfflinePageView

/// Reusable compact curved badge indicating offline state.
struct OfflineBadgeView: View {
    var subtitle: String?

    init(subtitle: String? = nil) {
        self.subtitle = subtitle
    }

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "wifi.slash")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.red)
                .symbolEffect(.pulse.byLayer, options: .repeating)

            if let subtitle {
                VStack(alignment: .leading, spacing: 1) {
                    Text(String(localized: "offline_title", defaultValue: "No Internet Connection"))
                        .font(.caption.bold())
                        .foregroundStyle(.red)
                    Text(subtitle)
                        .font(.system(size: 9, weight: .medium))
                        .foregroundStyle(.secondary)
                }
            } else {
                Text(String(localized: "offline_title", defaultValue: "No Internet Connection"))
                    .font(.caption.bold())
                    .foregroundStyle(.red)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 4)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 6))
        .overlay(
            RoundedRectangle(cornerRadius: 6)
                .strokeBorder(Color.red.opacity(0.35), lineWidth: 1)
        )
        .shadow(color: Color.red.opacity(0.18), radius: 4, x: 0, y: 1)
    }
}

#Preview {
    VStack(spacing: 20) {
        OfflineBadgeView()
        OfflineBadgeView(subtitle: "Please check your network connection")
        OfflinePageView(
            isRetrying: false,
            onRetry: {},
            onGoToHome: {}
        )
    }
}
