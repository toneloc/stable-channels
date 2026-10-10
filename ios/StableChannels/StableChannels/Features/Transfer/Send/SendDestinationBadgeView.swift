import SwiftUI

/// Visual badge showing real-time destination recognition and protocol type.
struct SendDestinationBadgeView: View {
    let classification: PaymentDestinationClassification

    var body: some View {
        switch classification {
        case .valid(let target):
            HStack(spacing: 6) {
                Image(systemName: badgeIcon(for: target))
                    .foregroundStyle(.secondary)
                Text(target.displayTitle)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                Spacer()
            }
            .padding(.horizontal, 4)
        case .invalid(let reason):
            HStack(spacing: 6) {
                Image(systemName: "exclamationmark.circle.fill")
                    .foregroundStyle(.red)
                Text(reason)
                    .font(.footnote)
                    .foregroundStyle(.red)
                Spacer()
            }
            .padding(.horizontal, 4)
        case .empty:
            EmptyView()
        }
    }

    private func badgeIcon(for target: SendDestination) -> String {
        switch target {
        case .bolt11: return "bolt.fill"
        case .bolt12: return "sparkles"
        case .lightningAddress: return "at"
        case .lnurlPay: return "link"
        case .onchain: return "bitcoinsign"
        }
    }
}
