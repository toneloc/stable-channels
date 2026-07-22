import SwiftUI
import UIKit

struct ChannelSettingsView: View {
    @Environment(AppState.self) private var appState
    @State private var showCloseChannelAlert = false

    var body: some View {
        List {
            if let channel = appState.nodeService.channels.first {
                Section {
                    HStack {
                        Text(String(localized: "label_capacity", defaultValue: "Capacity"))
                        Spacer()
                        Text(channel.channelValueSats.satsFormatted)
                    }
                    HStack {
                        Text(String(localized: "label_status", defaultValue: "Status"))
                        Spacer()
                        HStack(spacing: 4) {
                            Circle()
                                .fill(channel.isChannelReady ? .green : .orange)
                                .frame(width: 8, height: 8)
                            Text(channel.isChannelReady
                                ? String(localized: "channel_status_ready", defaultValue: "Ready")
                                : String(localized: "channel_status_pending", defaultValue: "Pending"))
                        }
                    }
                    HStack {
                        Text(String(localized: "label_outbound", defaultValue: "Outbound"))
                        Spacer()
                        Text((channel.outboundCapacityMsat / 1000).satsFormatted)
                    }
                    HStack {
                        Text(String(localized: "label_inbound", defaultValue: "Inbound"))
                        Spacer()
                        Text(((channel.inboundCapacityMsat) / 1000).satsFormatted)
                    }

                    if let closeTxid = appState.transactionLinkService.lastCloseTxid, !closeTxid.isEmpty {
                        HStack {
                            Text(String(localized: "label_close_tx", defaultValue: "Close Tx"))
                                .foregroundStyle(.secondary)
                            Spacer()
                            Text(String(closeTxid.prefix(8)) + "..." + String(closeTxid.suffix(8)))
                                .font(.system(.caption, design: .monospaced))
                                .foregroundStyle(.secondary)
                        }
                        if let url = Constants.txExplorerLink(for: closeTxid) {
                            Link(destination: url) {
                                HStack(spacing: 4) {
                                    Text(String(localized: "view_on_explorer", defaultValue: "View on explorer"))
                                    Image(systemName: "arrow.up.right.square")
                                }
                                .font(.caption)
                                .foregroundStyle(.blue)
                            }
                        }
                    } else if appState.isChannelClosing {
                        Text(String(
                            localized: "info_close_pending_confirmation",
                            defaultValue: "Channel closing — will appear on explorer after confirmation"
                        ))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    } else if let txid = appState.fundingTxid, !txid.isEmpty {
                        HStack {
                            Text(String(localized: "label_funding_tx", defaultValue: "Funding Tx"))
                                .foregroundStyle(.secondary)
                            Spacer()
                            Text(String(txid.prefix(8)) + "..." + String(txid.suffix(8)))
                                .font(.system(.caption, design: .monospaced))
                                .foregroundStyle(.secondary)
                        }
                        if let url = Constants.txExplorerLink(for: txid) {
                            Link(destination: url) {
                                HStack(spacing: 4) {
                                    Text(String(localized: "view_on_explorer", defaultValue: "View on explorer"))
                                    Image(systemName: "arrow.up.right.square")
                                }
                                .font(.caption)
                                .foregroundStyle(.blue)
                            }
                        }
                    }
                }

                Section {
                    Button(
                        String(localized: "button_close_channel", defaultValue: "Close channel"),
                        role: .destructive
                    ) {
                        showCloseChannelAlert = true
                    }
                }
            } else {
                Section {
                    Text(String(
                        localized: "info_no_channel",
                        defaultValue: "No channel open yet."
                    ))
                    .foregroundStyle(.secondary)
                }
            }
        }
        .navigationTitle(String(localized: "title_channel", defaultValue: "Channel"))
        .navigationBarTitleDisplayMode(.inline)
        .alert(
            String(localized: "alert_close_channel_title", defaultValue: "Close channel"),
            isPresented: $showCloseChannelAlert
        ) {
            Button(String(localized: "alert_close_channel_cancel", defaultValue: "Cancel"), role: .cancel) { }
            Button(String(localized: "alert_close_channel_confirm", defaultValue: "Close channel"), role: .destructive) {
                closeChannel()
            }
        } message: {
            Text(String(
                localized: "alert_close_channel_message",
                defaultValue: "This will cooperatively close the channel and return your funds to your onchain wallet after confirmation."
            ))
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
}
