import SwiftUI

struct HomeActionButtonsView: View {
    let hasReadyChannel: Bool
    let onSend: () -> Void
    let onReceive: () -> Void
    let onBuy: () -> Void
    let onSell: () -> Void

    // Muted, sophisticated badge tints matching the Cash App reference
    private let sendBlue = Color(red: 0.32, green: 0.58, blue: 0.85)
    private let receiveGreen = Color(red: 0.35, green: 0.75, blue: 0.50)
    private let buyAmber = Color(red: 0.88, green: 0.58, blue: 0.28)
    private let sellPlum = Color(red: 0.70, green: 0.50, blue: 0.82)

    var body: some View {
        VStack(spacing: 12) {
            HStack(spacing: 12) {
                ActionButton(
                    title: String(localized: "button_send", defaultValue: "Send"),
                    subtitle: String(localized: "action_send_subtitle", defaultValue: "Pay invoice or address"),
                    icon: "arrow.up",
                    badgeColor: sendBlue,
                    action: onSend
                )
                ActionButton(
                    title: String(localized: "button_receive", defaultValue: "Receive"),
                    subtitle: String(localized: "action_receive_subtitle", defaultValue: "Get paid instantly"),
                    icon: "arrow.down",
                    badgeColor: receiveGreen,
                    pulse: !hasReadyChannel,
                    action: onReceive
                )
            }

            HStack(spacing: 12) {
                ActionButton(
                    title: String(localized: "button_buy_btc", defaultValue: "Buy BTC"),
                    subtitle: String(localized: "action_buy_subtitle", defaultValue: "Convert USD to Bitcoin"),
                    icon: "bitcoinsign",
                    badgeColor: buyAmber,
                    action: onBuy
                )
                ActionButton(
                    title: String(localized: "button_sell_btc", defaultValue: "Sell BTC"),
                    subtitle: String(localized: "action_sell_subtitle", defaultValue: "Lock in USD balance"),
                    icon: "dollarsign",
                    badgeColor: sellPlum,
                    action: onSell
                )
            }
        }
    }
}
