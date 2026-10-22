import SwiftUI
import UIKit

struct ChannelSettingsView: View {
    @Environment(AppState.self) private var appState
    @State private var showCloseChannelAlert = false

    var body: some View {
        // Read observable property on AppState to ensure view invalidation when channel state changes,
        // as NodeService is not directly observable.
        let _ = appState.hasReadyChannel
        let _ = appState.isOnline
        List {
            if let channel = appState.nodeService.channels.first {
                Section {
                    HStack {
                        Text(String(localized: .labelCapacity))
                        Spacer()
                        Text("\(channel.channelValueSats.btcSpacedFormatted) BTC")
                    }
                    HStack {
                        Text(String(localized: .labelStatus))
                        Spacer()
                        channelStatusBadge(isReady: channel.isChannelReady)
                    }
                    HStack {
                        Text(String(localized: .labelOutbound))
                        Spacer()
                        Text("\((channel.outboundCapacityMsat / 1000).btcSpacedFormatted) BTC")
                    }
                    HStack {
                        Text(String(localized: .labelInbound))
                        Spacer()
                        Text("\((channel.inboundCapacityMsat / 1000).btcSpacedFormatted) BTC")
                    }

                    if let closeTxid = appState.transactionLinkService.lastCloseTxid, !closeTxid.isEmpty {
                        HStack {
                            Text(String(localized: .labelCloseTx))
                                .foregroundStyle(.secondary)
                            Spacer()
                            Text(String(closeTxid.prefix(8)) + "..." + String(closeTxid.suffix(8)))
                                .font(.system(.caption, design: .monospaced))
                                .foregroundStyle(.secondary)
                        }
                        if let url = Constants.txExplorerLink(for: closeTxid) {
                            Link(destination: url) {
                                HStack(spacing: 4) {
                                    Text(String(localized: .viewOnExplorer))
                                    Image(systemName: "arrow.up.right.square")
                                }
                                .font(.caption)
                                .foregroundStyle(.blue)
                            }
                        }
                    } else if appState.isChannelClosing {
                        Text(String(localized: .infoClosePendingConfirmation))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    } else if let txid = appState.fundingTxid, !txid.isEmpty {
                        HStack {
                            Text(String(localized: .labelFundingTx))
                                .foregroundStyle(.secondary)
                            Spacer()
                            Text(String(txid.prefix(8)) + "..." + String(txid.suffix(8)))
                                .font(.system(.caption, design: .monospaced))
                                .foregroundStyle(.secondary)
                        }
                        if let url = Constants.txExplorerLink(for: txid) {
                            Link(destination: url) {
                                HStack(spacing: 4) {
                                    Text(String(localized: .viewOnExplorer))
                                    Image(systemName: "arrow.up.right.square")
                                }
                                .font(.caption)
                                .foregroundStyle(.blue)
                            }
                        }
                    }
                }

                if !appState.isChannelClosing {
                    Section {
                        Button(
                            String(localized: .buttonCloseChannel),
                            role: .destructive
                        ) {
                            showCloseChannelAlert = true
                        }
                        .disabled(!appState.isOnline)
                        .foregroundStyle(appState.isOnline ? .red : .secondary)
                    } footer: {
                        if !appState.isOnline {
                            Text(String(localized: .infoCloseChannelOffline))
                        }
                    }
                } else {
                    Section {
                        HStack {
                            ProgressView()
                                .padding(.trailing, 8)
                            Text(String(localized: .statusClosingChannel))
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            } else if appState.isChannelClosing {
                Section {
                    HStack {
                        ProgressView()
                            .padding(.trailing, 8)
                        Text(String(localized: .statusClosingChannel))
                            .foregroundStyle(.secondary)
                    }
                }
            } else if appState.hasActiveChannel {
                Section {
                    HStack {
                        Text(String(localized: .labelCapacity))
                        Spacer()
                        Text(appState.lightningBalanceSats.satsFormatted)
                    }
                    HStack {
                        Text(String(localized: .labelStatus))
                        Spacer()
                        channelStatusBadge(isReady: appState.hasReadyChannel)
                    }
                    if let txid = appState.fundingTxid, !txid.isEmpty {
                        HStack {
                            Text(String(localized: .labelFundingTx))
                                .foregroundStyle(.secondary)
                            Spacer()
                            Text(String(txid.prefix(8)) + "..." + String(txid.suffix(8)))
                                .font(.system(.caption, design: .monospaced))
                                .foregroundStyle(.secondary)
                        }
                    }
                } footer: {
                    Text(String(localized: .infoChannelOfflineNote))
                }

                Section {
                    Button(
                        String(localized: .buttonCloseChannel),
                        role: .destructive
                    ) {
                        showCloseChannelAlert = true
                    }
                    .disabled(true)
                    .foregroundStyle(.secondary)
                } footer: {
                    Text(String(localized: .infoCloseChannelOffline))
                }
            } else {
                Section {
                    Text(String(localized: .infoNoChannel))
                        .foregroundStyle(.secondary)
                }
            }
        }
        .navigationTitle(String(localized: .titleChannel))
        .navigationBarTitleDisplayMode(.inline)
        .alert(
            String(localized: .alertCloseChannelTitle),
            isPresented: $showCloseChannelAlert
        ) {
            Button(String(localized: .alertCloseChannelCancel), role: .cancel) { }
            Button(String(localized: .alertCloseChannelConfirm),
                   role: .destructive) {
                closeChannel()
            }
        } message: {
            Text(String(localized: .alertCloseChannelMessage))
        }
    }

    @MainActor
    private func closeChannel() {
        guard let channel = appState.nodeService.channels.first,
              let outpoint = channel.fundingTxo else {
            // Edge case: channel exists but no funding outpoint. Log and abort.
            // AuditService.log is synchronous — call directly.
            AuditService.log("CHANNEL_CLOSE_NO_OUTPOINT", data: [
                "user_channel_id": appState.stableChannel.userChannelId
            ])
            return
        }
        appState.isChannelClosing = true
        Task { @MainActor in
            do {
                try await appState.requestChannelClose(
                    userChannelId: channel.userChannelId,
                    counterpartyNodeId: channel.counterpartyNodeId,
                    fundingOutpointTxid: "\(outpoint.txid)",
                    fundingOutpointVout: outpoint.vout
                )
            } catch {
                appState.statusMessage = "Close channel failed: \(error.localizedDescription)"
                appState.isChannelClosing = false
            }
        }
    }

    private func channelStatusBadge(isReady: Bool) -> some View {
        HStack(spacing: 4) {
            if !appState.isOnline {
                Circle()
                    .fill(.orange)
                    .frame(width: 8, height: 8)
                Text(String(localized: .channelStatusOffline))
                    .foregroundStyle(.orange)
            } else {
                Circle()
                    .fill(isReady ? .green : .orange)
                    .frame(width: 8, height: 8)
                Text(isReady
                    ? String(localized: .channelStatusReady)
                    : String(localized: .channelStatusPending))
            }
        }
    }
}
