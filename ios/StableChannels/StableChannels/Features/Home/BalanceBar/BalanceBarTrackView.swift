import SwiftUI

struct BalanceBarTrackView: View {
    let barWidth: CGFloat
    let height: CGFloat
    let fraction: CGFloat
    var isEmpty: Bool = false
    let isAwakening: Bool
    let floodScale: CGFloat
    let floodOpacity: Double

    var body: some View {
        ZStack {
            if isEmpty && !isAwakening {
                emptyTrack
            } else {
                trackBar(fraction: fraction)
            }

            if isAwakening {
                Circle()
                    .fill(
                        RadialGradient(
                            colors: [
                                Color.orange.opacity(floodOpacity * 0.95),
                                Color.yellow.opacity(floodOpacity * 0.5),
                                .clear
                            ],
                            center: .center,
                            startRadius: 0,
                            endRadius: barWidth * 0.65 * floodScale
                        )
                    )
                    .frame(width: barWidth * 1.3 * floodScale, height: barWidth * 1.3 * floodScale)
                    .position(x: barWidth * 0.5, y: height / 2)
                    .blendMode(.plusLighter)
                    .allowsHitTesting(false)
            }
        }
    }

    private var emptyTrack: some View {
        let spacing: CGFloat = 2
        let halfWidth = max((barWidth - spacing) / 2, 0)

        return HStack(spacing: spacing) {
            RoundedRectangle(cornerRadius: 5)
                .fill(
                    LinearGradient(
                        colors: [Color.green.opacity(0.16), Color.green.opacity(0.24)],
                        startPoint: .leading,
                        endPoint: .trailing
                    )
                )
                .frame(width: halfWidth, height: height)

            RoundedRectangle(cornerRadius: 5)
                .fill(
                    LinearGradient(
                        colors: [Color.orange.opacity(0.24), Color.orange.opacity(0.16)],
                        startPoint: .leading,
                        endPoint: .trailing
                    )
                )
                .frame(width: halfWidth, height: height)
        }
        .frame(width: barWidth, height: height)
        .clipShape(RoundedRectangle(cornerRadius: 5))
        .overlay(
            RoundedRectangle(cornerRadius: 5)
                .strokeBorder(Color.primary.opacity(0.08), lineWidth: 1)
        )
    }

    @ViewBuilder
    private func trackBar(fraction: CGFloat) -> some View {
        let alpha = 0.85
        let hasGreen = fraction > 0.005
        let hasOrange = (1.0 - fraction) > 0.005
        let spacing: CGFloat = (hasGreen && hasOrange) ? 2 : 0
        let availableWidth = max(barWidth - spacing, 0)
        let greenWidth = hasOrange ? availableWidth * fraction : barWidth
        let orangeWidth = hasGreen ? (availableWidth - greenWidth) : barWidth

        HStack(spacing: spacing) {
            if hasGreen {
                RoundedRectangle(cornerRadius: 5)
                    .fill(LinearGradient(
                        colors: [Color.green.opacity(alpha * 0.8), Color.green.opacity(alpha)],
                        startPoint: .leading,
                        endPoint: .trailing
                    ))
                    .frame(width: max(greenWidth, 0), height: height)
            }
            if hasOrange {
                RoundedRectangle(cornerRadius: 5)
                    .fill(LinearGradient(
                        colors: [Color.orange.opacity(alpha), Color.orange.opacity(alpha * 0.8)],
                        startPoint: .leading,
                        endPoint: .trailing
                    ))
                    .frame(width: max(orangeWidth, 0), height: height)
            }
        }
        .frame(width: barWidth, height: height)
        .clipShape(RoundedRectangle(cornerRadius: 5))
    }
}
