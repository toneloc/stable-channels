import SwiftUI

/// Screen-level progress indicator displaying mathematical curve loader during network & blockchain verification.
struct SendLoadingView: View {
    let title: String
    let subtitle: String
    var curve: CurveProgressIndicator.CurveType = .roseCurve
    var tint: Color = .orange

    var body: some View {
        VStack(spacing: 12) {
            Spacer()

            CurveProgressIndicator(
                curve: curve,
                size: 110,
                tint: tint,
                particleCount: 140,
                trailSpan: 0.12,
                enablesPulse: true,
                enablesRotation: true
            )

            VStack(spacing: 8) {
                Text(title)
                    .font(.title3.weight(.bold))
                    .foregroundStyle(.primary)

                Text(subtitle)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            .padding(.horizontal, 32)

            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
