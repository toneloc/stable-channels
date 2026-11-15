import SwiftUI

/// Shown on Home while the app's own close of a channel is slow to confirm (#456). The app keeps
/// sending the close itself, so the one thing left for the user is paying a higher fee.
struct HomeOwnForceCloseView: View {
    let closeTxid: String
    @Environment(\.openURL) private var openURL
    @State private var showSpeedUp = false

    var body: some View {
        Button {
            showSpeedUp = true
        } label: {
            HStack(spacing: 8) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(String(localized: "label_close_slow", defaultValue: "Close is slow to confirm"))
                        .font(.caption.bold())
                    Text(String(
                        localized: "info_close_slow",
                        defaultValue: "The app keeps sending it. Tap to pay a higher fee and confirm it sooner."
                    ))
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                }
                Spacer()
                Image(systemName: "chevron.right")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(10)
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 10))
        }
        .buttonStyle(.plain)
        .alert(
            String(localized: "alert_speed_up_title", defaultValue: "Speed up the close"),
            isPresented: $showSpeedUp
        ) {
            Button(String(localized: "alert_speed_up_cancel", defaultValue: "Not now"), role: .cancel) {}
            Button(String(localized: "alert_speed_up_open", defaultValue: "Open mempool.space")) {
                if let url = URL(string: "https://mempool.space/tx/\(closeTxid)") { openURL(url) }
            }
        } message: {
            Text(String(
                localized: "alert_speed_up_message",
                defaultValue: "mempool.space can get your close confirmed sooner for an extra fee. On the page that opens, tap Accelerate and follow the steps."
            ))
        }
    }
}
