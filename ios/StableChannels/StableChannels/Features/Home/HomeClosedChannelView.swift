import SwiftUI

/// Shown on Home while a closed channel's funds are not in the onchain wallet yet (#358).
struct HomeClosedChannelView: View {
    let funds: ClosedChannelFunds
    let btcPrice: Double

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(String(localized: "label_channel_closed", defaultValue: "Channel closed"))
                .font(.caption.bold())
            Text(funds.message(btcPrice: btcPrice))
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(10)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 10))
    }
}
