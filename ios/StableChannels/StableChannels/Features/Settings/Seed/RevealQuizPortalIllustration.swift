import SwiftUI

// MARK: - Editorial Vault Monolith Illustration

struct RevealQuizPortalIllustration: View {
    @State private var isPulsing = false

    var body: some View {
        ZStack {
            // Radial spotlight / stipple halo
            Circle()
                .fill(
                    RadialGradient(
                        colors: [
                            Color.orange.opacity(0.32),
                            Color.stablePrimary.opacity(0.18),
                            Color.clear
                        ],
                        center: .center,
                        startRadius: 25,
                        endRadius: 120
                    )
                )
                .frame(width: 240, height: 240)
                .scaleEffect(isPulsing ? 1.05 : 0.95)
                .animation(
                    .easeInOut(duration: 3.2).repeatForever(autoreverses: true),
                    value: isPulsing
                )

            // Radial Stippled Line Beams (Screen-print vector lines)
            ForEach(0..<12) { i in
                Rectangle()
                    .fill(
                        LinearGradient(
                            colors: [
                                Color.orange.opacity(0.35),
                                Color.stablePrimary.opacity(0.15),
                                Color.clear
                            ],
                            startPoint: .center,
                            endPoint: .trailing
                        )
                    )
                    .frame(width: 160, height: 1.5)
                    .rotationEffect(.degrees(Double(i) * 30))
            }

            // Outer Arch Frame (Cryptographic Vault Monolith)
            ZStack {
                // Vault background fill
                UnevenRoundedRectangle(
                    topLeadingRadius: 48,
                    bottomLeadingRadius: 6,
                    bottomTrailingRadius: 6,
                    topTrailingRadius: 48,
                    style: .continuous
                )
                .fill(
                    LinearGradient(
                        colors: [
                            Color(red: 0.12, green: 0.14, blue: 0.2),
                            Color(red: 0.06, green: 0.08, blue: 0.12)
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                )
                .frame(width: 104, height: 140)
                .overlay(
                    // Sharp dual-tone border (Bitcoin Orange to Stable Cyan)
                    UnevenRoundedRectangle(
                        topLeadingRadius: 48,
                        bottomLeadingRadius: 6,
                        bottomTrailingRadius: 6,
                        topTrailingRadius: 48,
                        style: .continuous
                    )
                    .stroke(
                        LinearGradient(
                            colors: [
                                Color.orange,
                                Color.orange.opacity(0.7),
                                Color.stablePrimary
                            ],
                            startPoint: .topLeading,
                            endPoint: .bottomTrailing
                        ),
                        lineWidth: 2.5
                    )
                )
                .shadow(color: Color.orange.opacity(0.25), radius: 14, x: 0, y: 6)

                // Inner Keyhole & Bitcoin ₿ Seal
                VStack(spacing: 8) {
                    ZStack {
                        Circle()
                            .fill(Color.orange.opacity(0.18))
                            .frame(width: 42, height: 42)

                        Text("₿")
                            .font(.system(size: 26, weight: .bold, design: .rounded))
                            .foregroundStyle(Color.orange)
                    }

                    // Keyhole silhouette
                    VStack(spacing: 0) {
                        Circle()
                            .fill(Color.white.opacity(0.85))
                            .frame(width: 10, height: 10)
                        UnevenRoundedRectangle(
                            topLeadingRadius: 0,
                            bottomLeadingRadius: 2,
                            bottomTrailingRadius: 2,
                            topTrailingRadius: 0
                        )
                        .fill(Color.white.opacity(0.85))
                        .frame(width: 6, height: 12)
                    }
                }
                .offset(y: -4)
            }
            .offset(y: -10)

            // Floating Cryptographic Badges (Minimalist Geometric Accents)
            Group {
                Image(systemName: "lock.shield.fill")
                    .font(.system(size: 15))
                    .foregroundStyle(Color.stablePrimary.opacity(0.85))
                    .offset(x: -64, y: -44)

                Image(systemName: "key.fill")
                    .font(.system(size: 14))
                    .foregroundStyle(Color.orange.opacity(0.85))
                    .offset(x: 64, y: -36)

                Image(systemName: "sparkle")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Color.white.opacity(0.75))
                    .offset(x: -54, y: 32)

                Image(systemName: "circle.grid.2x2.fill")
                    .font(.system(size: 12))
                    .foregroundStyle(Color.stablePrimary.opacity(0.75))
                    .offset(x: 58, y: 28)
            }

            // Slate Floor Base
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(
                    LinearGradient(
                        colors: [
                            Color(white: 0.18),
                            Color(white: 0.08)
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                )
                .frame(width: 160, height: 14)
                .overlay(
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .stroke(Color.white.opacity(0.12), lineWidth: 1)
                )
                .offset(y: 64)
                .shadow(color: .black.opacity(0.4), radius: 8, x: 0, y: 4)
        }
        .frame(height: 200)
        .onAppear {
            isPulsing = true
        }
    }
}
