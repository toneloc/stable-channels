import SwiftUI

/// Screen-level progress indicator displaying mathematical curve loader during network & blockchain verification.
struct SendLoadingView: View {
    let title: String
    let subtitle: String
    var curve: CurveProgressIndicator.CurveType = .roseCurve
    var tint: Color = .orange

    var body: some View {
        VStack(spacing: 24) {
            Spacer()

            CurveProgressIndicator(
                curve: curve,
                size: 76,
                tint: tint,
                enablesPulse: false,
                enablesRotation: false
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
