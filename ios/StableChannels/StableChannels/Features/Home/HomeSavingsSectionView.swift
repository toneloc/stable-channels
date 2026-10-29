import SwiftUI

struct HomeSavingsSectionView: View {
    @Environment(AppState.self) private var appState
    let card: HomeOnchainCard
    let showBTC: Bool

    @State private var isExpanded = false

    private var onchainUSD: Double {
        appState.btcPrice > 0
            ? BitcoinUnits.usd(sats: appState.onchainBalanceSats, price: appState.btcPrice)
            : 0
    }

    var body: some View {
        VStack(spacing: 6) {
            header

            if let summary = card.collapsedSummary, !isExpanded {
                Text(summaryText(summary))
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }

            if !card.isExpandable || isExpanded {
                VStack(spacing: 8) {
                    if let moveButton = card.moveButton {
                        moveRow(moveButton)
                    }
                    ForEach(Array(card.rows.enumerated()), id: \.offset) { _, row in
                        pendingRow(row)
                    }
                }
                .transition(.opacity.combined(with: .move(edge: .top)))
            }

            if card.showsActivationHint {
                Text(String(
                    localized: "hint_create_wallet",
                    defaultValue: "Get your first payment over Lightning to activate your account"
                ))
                .font(.caption2)
                .foregroundStyle(.secondary)
            }

            ForEach(Array(card.sendRows.enumerated()), id: \.offset) { _, row in
                pendingRow(row)
            }
        }
        .padding(10)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 10))
        .contentShape(Rectangle())
        .onTapGesture {
            guard card.isExpandable else { return }
            withAnimation(.easeInOut(duration: 0.2)) { isExpanded.toggle() }
        }
        .accessibilityAddTraits(card.isExpandable ? .isButton : [])
    }

    private var header: some View {
        HStack {
            Text(String(localized: "label_on_chain", defaultValue: "Onchain Account"))
                .font(.caption.bold())
            Spacer()
            Text(showBTC
                ? "\(appState.onchainBalanceSats.btcSpacedFormatted) BTC"
                : onchainUSD.usdFormatted)
                .font(.caption)
                .foregroundStyle(.secondary)
            if card.isExpandable {
                Image(systemName: "chevron.right")
                    .font(.caption2.bold())
                    .foregroundStyle(.secondary)
                    .rotationEffect(.degrees(isExpanded ? 90 : 0))
                    .accessibilityLabel(isExpanded
                        ? String(localized: "a11y_collapse", defaultValue: "Collapse")
                        : String(localized: "a11y_expand", defaultValue: "Expand"))
            }
        }
    }

    private func moveRow(_ moveButton: HomeOnchainCard.MoveButton) -> some View {
        HStack {
            Text(moveLabel(moveButton))
                .font(.caption2)
                .foregroundStyle(.secondary)
            Spacer()
            Button {
                appState.sweepToChannel()
            } label: {
                Text(String(localized: "button_swap", defaultValue: "Move"))
                    .font(.caption.bold())
                    .padding(.horizontal, 16)
                    .padding(.vertical, 6)
                    .background(.blue.opacity(0.1))
                    .foregroundStyle(.blue)
                    .clipShape(Capsule())
            }
        }
    }

    private func pendingRow(_ row: HomeOnchainRow) -> some View {
        HStack(spacing: 6) {
            if let progress = row.progress {
                ConfirmationRing(progress: progress)
            } else {
                Image(systemName: "hourglass")
                    .font(.caption)
                    .foregroundStyle(.orange)
            }
            VStack(alignment: .leading, spacing: 1) {
                Text(label(row.kind))
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                if let caption = caption(row) {
                    Text(caption)
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
            }
            Spacer()
            if let txid = row.txid, let url = Constants.txExplorerLink(for: txid) {
                Link(destination: url) {
                    HStack(spacing: 2) {
                        Text(String(localized: "view_on_explorer", defaultValue: "View on explorer"))
                            .font(.caption2)
                        Image(systemName: "arrow.up.right.square")
                            .font(.caption2)
                    }
                    .foregroundStyle(.blue)
                }
            }
        }
    }

    // MARK: - Strings

    private func label(_ kind: HomeOnchainRow.Kind) -> String {
        switch kind {
        case .receiving, .depositFallback:
            String(localized: "status_onchain_receiving", defaultValue: "Receiving onchain...")
        case .sending, .splice(isOutgoing: true):
            String(localized: "status_onchain_sending", defaultValue: "Sending onchain...")
        case .splice(isOutgoing: false):
            String(localized: "status_sweeping", defaultValue: "Move pending...")
        case .channelClosing:
            String(localized: "status_channel_closing", defaultValue: "Channel closing...")
        case .channelClosingNoLink:
            String(localized: "info_close_pending_confirmation", defaultValue: "Channel closing - pending confirmation")
        case .channelOpening:
            String(localized: "status_channel_opening", defaultValue: "Deposit confirming...")
        }
    }

    private func caption(_ row: HomeOnchainRow) -> String? {
        let amount = row.amountSats.map {
            HomeOnchainRows.amountText(sats: $0, btcPrice: appState.btcPrice, showBTC: showBTC, sign: row.sign)
        }
        let detail: String? = switch row.caption {
        case let .progress(confirmations, required):
            String(
                localized: "caption_confirmations_progress",
                defaultValue: "\(confirmations)/\(required) confirmations"
            )
        case .awaitingTxid:
            String(localized: "caption_pending_confirmation", defaultValue: "pending confirmation")
        case .amountOnly, .none:
            nil
        }
        let parts = [amount, detail].compactMap(\.self)
        return parts.isEmpty ? nil : parts.joined(separator: " \u{00B7} ")
    }

    private func moveLabel(_ moveButton: HomeOnchainCard.MoveButton) -> String {
        switch moveButton {
        case .plain:
            return String(localized: "move_to_trading", defaultValue: "Move to Lightning Account")
        case let .amount(sats):
            let amount = HomeOnchainRows.amountText(sats: sats, btcPrice: appState.btcPrice, showBTC: showBTC)
            return String(localized: "move_amount_to_lightning", defaultValue: "Move \(amount) to Lightning")
        }
    }

    private func summaryText(_ summary: HomeOnchainCard.Summary) -> String {
        let lead = switch summary.lead {
        case .readyToMove: String(localized: "summary_ready_to_move", defaultValue: "Ready to move")
        case .movePending: String(localized: "summary_move_pending", defaultValue: "Move pending")
        case .sendPending: String(localized: "summary_send_pending", defaultValue: "Send pending")
        }
        let receiving = String(localized: "summary_receiving_onchain", defaultValue: "Receiving onchain")
        let suffix = switch summary.receives {
        case .none: ""
        case let .progress(confirmations, required): " (\(confirmations)/\(required))"
        case let .count(count):
            " \u{00B7} " + String(localized: "summary_deposits_count", defaultValue: "\(count) deposits")
        }
        return "\(lead) \u{00B7} \(receiving)\(suffix)"
    }
}

/// Confirmation progress ring sized to sit in place of the hourglass.
private struct ConfirmationRing: View {
    let progress: Double

    var body: some View {
        ZStack {
            Circle().stroke(Color.blue.opacity(0.15), lineWidth: 2)
            Circle()
                .trim(from: 0, to: progress)
                .stroke(Color.blue, style: StrokeStyle(lineWidth: 2, lineCap: .round))
                .rotationEffect(.degrees(-90))
        }
        .frame(width: 12, height: 12)
        .accessibilityHidden(true)
    }
}
