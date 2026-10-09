import SwiftUI

struct BalanceBarTrackView: View {
    let barWidth: CGFloat
    let height: CGFloat
    let fraction: CGFloat
    var thumbDiameter: CGFloat = BalanceBarTradeCalculator.defaultThumbDiameter
    var isEmpty: Bool = false
    let isAwakening: Bool
    let floodScale: CGFloat
    let floodOpacity: Double

    var body: some View {
        ZStack {
            if isEmpty && !isAwakening {
                emptyTrack(fraction: fraction)
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

    private func emptyTrack(fraction: CGFloat) -> some View {
        let thumbX = BalanceBarTradeCalculator.calculateThumbPosition(
            fraction: fraction,
            barWidth: barWidth,
            thumbDiameter: thumbDiameter
        )
        let greenWidth = thumbX
        let orangeWidth = max(barWidth - greenWidth, 0)
        let cornerRadius: CGFloat = 6

        return HStack(spacing: 0) {
            Rectangle()
                .fill(
                    LinearGradient(
                        colors: [Color.green.opacity(0.16), Color.green.opacity(0.24)],
                        startPoint: .leading,
                        endPoint: .trailing
                    )
                )
                .frame(width: max(greenWidth, 0), height: height)

            Rectangle()
                .fill(
                    LinearGradient(
                        colors: [Color.orange.opacity(0.24), Color.orange.opacity(0.16)],
                        startPoint: .leading,
                        endPoint: .trailing
                    )
                )
                .frame(width: max(orangeWidth, 0), height: height)
        }
        .frame(width: barWidth, height: height)
        .clipShape(RoundedRectangle(cornerRadius: cornerRadius))
        .overlay(
            RoundedRectangle(cornerRadius: cornerRadius)
                .strokeBorder(Color.primary.opacity(0.08), lineWidth: 1)
        )
    }

    @ViewBuilder
    private func trackBar(fraction: CGFloat) -> some View {
        let alpha = 0.85
        let thumbX = BalanceBarTradeCalculator.calculateThumbPosition(
            fraction: fraction,
            barWidth: barWidth,
            thumbDiameter: thumbDiameter
        )
        let hasGreen = fraction > 0.001
        let hasOrange = (1.0 - fraction) > 0.001
        let greenWidth = hasOrange ? thumbX : barWidth
        let orangeWidth = hasGreen ? max(barWidth - greenWidth, 0) : barWidth
        let cornerRadius: CGFloat = 6

        HStack(spacing: 0) {
            if hasGreen {
                Rectangle()
                    .fill(LinearGradient(
                        colors: [Color.green.opacity(alpha * 0.8), Color.green.opacity(alpha)],
                        startPoint: .leading,
                        endPoint: .trailing
                    ))
                    .frame(width: max(greenWidth, 0), height: height)
            }
            if hasOrange {
                Rectangle()
                    .fill(LinearGradient(
                        colors: [Color.orange.opacity(alpha), Color.orange.opacity(alpha * 0.8)],
                        startPoint: .leading,
                        endPoint: .trailing
                    ))
                    .frame(width: max(orangeWidth, 0), height: height)
            }
        }
        .frame(width: barWidth, height: height)
        .clipShape(RoundedRectangle(cornerRadius: cornerRadius))
    }
}
