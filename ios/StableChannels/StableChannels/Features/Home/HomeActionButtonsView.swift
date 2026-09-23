import SwiftUI

struct HomeActionButtonsView: View {
    let hasReadyChannel: Bool
    let onSend: () -> Void
    let onReceive: () -> Void
    let onBuy: () -> Void
    let onSell: () -> Void

    // Sophisticated accent tints for the colored circular badges
    private let sendBlue = Color(red: 0.35, green: 0.65, blue: 1.0)
    private let receiveGreen = Color(red: 0.25, green: 0.85, blue: 0.55)
    private let buyAmber = Color(red: 1.0, green: 0.62, blue: 0.25)
    private let sellPlum = Color(red: 0.78, green: 0.55, blue: 1.0)

    var body: some View {
        VStack(spacing: 10) {
            HStack(spacing: 10) {
                ActionButton(
                    title: String(localized: "button_send", defaultValue: "Send"),
                    icon: "arrow.up",
                    color: sendBlue,
                    textColor: .primary,
                    action: onSend
                )
                ActionButton(
                    title: String(localized: "button_receive", defaultValue: "Receive"),
                    icon: "arrow.down",
                    color: receiveGreen,
                    textColor: !hasReadyChannel ? receiveGreen : .primary,
                    pulse: !hasReadyChannel,
                    action: onReceive
                )
            }

            HStack(spacing: 10) {
                ActionButton(
                    title: String(localized: "button_buy_btc", defaultValue: "USD → BTC"),
                    icon: "arrow.up.right",
                    color: buyAmber,
                    textColor: .primary,
                    action: onBuy
                )
                ActionButton(
                    title: String(localized: "button_sell_btc", defaultValue: "BTC → USD"),
                    icon: "arrow.down.right",
                    color: sellPlum,
                    textColor: .primary,
                    action: onSell
                )
            }
        }
    }
}
