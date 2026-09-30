import SwiftUI

struct HomeSavingsSectionView: View {
    @Environment(AppState.self) private var appState
    let showBTC: Bool

    private var onchainUSD: Double {
        appState.btcPrice > 0
            ? Double(appState.onchainBalanceSats) / Double(Constants.satsInBTC) * appState.btcPrice
            : 0
    }

    private var hasReadyChannel: Bool {
        appState.hasReadyChannel
    }

    var body: some View {
        VStack(spacing: 6) {
            HStack {
                Text(String(localized: "label_on_chain", defaultValue: "Onchain Account"))
                    .font(.caption.bold())
                Spacer()
                Text(showBTC
                    ? "\(appState.onchainBalanceSats.btcSpacedFormatted) BTC"
                    : onchainUSD.usdFormatted)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            if appState.isSweeping {
                pendingRow(kind: .sweep(txid: appState.spliceTxid))
            } else if appState.isChannelClosing {
                if let closeTxid = appState.transactionLinkService.lastCloseTxid, !closeTxid.isEmpty {
                    pendingRow(kind: .close(txid: closeTxid))
                } else {
                    pendingRow(kind: .closeNoLink)
                }
            } else if hasReadyChannel && appState.spendableOnchainSats > 0 {
                HStack {
                    Text(String(localized: "move_to_trading", defaultValue: "Move to Lightning Account"))
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
            } else if appState.spendableOnchainSats == 0 {
                if appState.isOpeningChannel, let fundingTx = appState.fundingTxid {
                    pendingRow(kind: .deposit(txid: fundingTx))
                } else {
                    pendingRow(kind: .onchainReceive(txid: appState.transactionLinkService.lastReceiveTxid))
                }
                if !hasReadyChannel {
                    Text(String(
                        localized: "hint_create_wallet",
                        defaultValue: "Get your first payment over Lightning to activate your account"
                    ))
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                }
            } else {
                Text(String(
                    localized: "hint_create_wallet",
                    defaultValue: "Get your first payment over Lightning to activate your account"
                ))
                .font(.caption2)
                .foregroundStyle(.secondary)
            }
        }
        .padding(10)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 10))
    }

    private enum PendingRowKind {
        case deposit(txid: String?)
        case sweep(txid: String?)
        case close(txid: String?)
        case closeNoLink
        case onchainReceive(txid: String?)
    }

    @ViewBuilder
    private func pendingRow(kind: PendingRowKind) -> some View {
        switch kind {
        case .deposit(let txid):
            pendingRowImpl(
                text: String(localized: "status_channel_opening", defaultValue: "Deposit confirming..."),
                txid: txid
            )
        case .sweep(let txid):
            pendingRowImpl(
                text: String(localized: "status_sweeping", defaultValue: "Move pending..."),
                txid: txid
            )
        case .close(let txid):
            pendingRowImpl(
                text: String(localized: "status_channel_closing", defaultValue: "Channel closing..."),
                txid: txid
            )
        case .closeNoLink:
            HStack(spacing: 6) {
                Image(systemName: "hourglass")
                    .font(.caption)
                    .foregroundStyle(.orange)
                Text(String(
                    localized: "info_close_pending_confirmation",
                    defaultValue: "Channel closing - pending confirmation"
                ))
                .font(.caption2)
                .foregroundStyle(.secondary)
                Spacer()
            }
        case .onchainReceive(let txid):
            pendingRowImpl(
                text: String(localized: "status_onchain_receiving", defaultValue: "Receiving onchain..."),
                txid: txid
            )
        }
    }

    private func pendingRowImpl(text: String, txid: String?) -> some View {
        HStack(spacing: 6) {
            Image(systemName: "hourglass")
                .font(.caption)
                .foregroundStyle(.orange)
            Text(text)
                .font(.caption2)
                .foregroundStyle(.secondary)
            Spacer()
            if let txid, !txid.isEmpty {
                if let url = Constants.txExplorerLink(for: txid) {
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
    }
}
