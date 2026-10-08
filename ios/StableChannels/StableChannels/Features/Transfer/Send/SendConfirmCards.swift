import SwiftUI

/// Shared UI cards used across Send Confirm Step and Onchain Review Sheet.

/// Displays the cryptocurrency asset and route category.
struct SendConfirmAssetCard: View {
    let routeDescription: String

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(String(localized: "header_account_asset", defaultValue: "Asset & Network"))
                .font(.caption.weight(.medium))
                .foregroundStyle(.secondary)

            HStack(spacing: 12) {
                ZStack {
                    Circle().fill(Color.orange).frame(width: 32, height: 32)
                    Image(systemName: "bitcoinsign").font(.system(size: 16, weight: .bold)).foregroundStyle(.white)
                }

                VStack(alignment: .leading, spacing: 2) {
                    Text(String(localized: "label_bitcoin", defaultValue: "Bitcoin"))
                        .font(.headline)
                    Text(verbatim: routeDescription)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
            }
            .padding(14)
            .background(Color(uiColor: .secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 14))
        }
    }
}

/// Displays the destination address or invoice with visual chunking, tap-to-copy, and verification warning.
struct SendConfirmAddressCard: View {
    let headerTitle: String
    let representation: DestinationVisualRepresentation
    let rawAddress: String
    var avatarData: Data?

    @State private var hasCopiedAddress = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(headerTitle)
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
                Spacer()
                if hasCopiedAddress {
                    Text(String(localized: "label_copied", defaultValue: "Copied"))
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.orange)
                        .transition(.opacity)
                }
            }

            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 10) {
                    if let avatarData, let uiImage = UIImage(data: avatarData) {
                        Image(uiImage: uiImage)
                            .resizable()
                            .scaledToFill()
                            .frame(width: 32, height: 32)
                            .clipShape(Circle())
                    }
                    AddressVisualChunkView(representation: representation)
                }

                if case .onchain = representation {
                    Text(String(
                        localized: "hint_verify_full_address",
                        defaultValue: "Tip: Always verify the middle and full address to prevent prefix-spoofing."
                    ))
                    .font(.caption2)
                    .foregroundStyle(Color(uiColor: .tertiaryLabel))
                    .padding(.top, 2)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(12)
            .background(Color(uiColor: .secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 14))
            .contentShape(Rectangle())
            .onTapGesture {
                UIPasteboard.general.string = rawAddress
                UINotificationFeedbackGenerator().notificationOccurred(.success)
                withAnimation(.easeInOut(duration: 0.2)) { hasCopiedAddress = true }
                Task {
                    try? await Task.sleep(nanoseconds: 1_800_000_000)
                    withAnimation(.easeInOut(duration: 0.2)) { hasCopiedAddress = false }
                }
            }
        }
    }
}

/// Displays the net amount the recipient will receive in USD and BTC.
struct SendConfirmReceivesCard: View {
    let amountSats: UInt64
    let btcPrice: Double

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(String(localized: "header_recipient_receives", defaultValue: "Recipient Receives"))
                .font(.caption.weight(.medium))
                .foregroundStyle(.secondary)

            VStack(alignment: .leading, spacing: 4) {
                let usd = (Double(amountSats) / Double(Constants.satsInBTC)) * btcPrice
                if btcPrice > 0 {
                    Text(usd.usdFormatted)
                        .font(.system(size: 26, weight: .bold, design: .rounded))
                        .foregroundStyle(.primary)
                }
                Text("\(amountSats.btcSpacedFormatted) BTC (\(amountSats) sats)")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(14)
            .background(Color(uiColor: .secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 14))
        }
    }
}

/// Displays the fee breakdown and total debit from user's balance.
struct SendConfirmFeeTotalCard: View {
    var feeLabel: String = .init(localized: "label_total_fees", defaultValue: "Network Fee")
    let estimatedFeeSats: UInt64
    var rateSatVb: Double?
    let totalDebitSats: UInt64
    let btcPrice: Double

    var body: some View {
        VStack(spacing: 8) {
            HStack {
                HStack(spacing: 4) {
                    Text(feeLabel)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                    if let rate = rateSatVb, rate > 0 {
                        let formattedRate = rate
                            .truncatingRemainder(dividingBy: 1) == 0 ? String(format: "%.0f", rate) : String(
                                format: "%.1f",
                                rate
                            )
                        Text("(\(formattedRate) sat/vB)")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                Spacer()
                let feeUSD = (Double(estimatedFeeSats) / Double(Constants.satsInBTC)) * btcPrice
                VStack(alignment: .trailing, spacing: 1) {
                    Text(feeUSD.usdFormatted)
                        .font(.subheadline.weight(.medium))
                    Text("\(estimatedFeeSats) sats")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 4)

            Divider()

            HStack {
                Text(String(localized: "label_total_spent", defaultValue: "Total Debit"))
                    .font(.subheadline.weight(.semibold))
                Spacer()
                let totalUSD = (Double(totalDebitSats) / Double(Constants.satsInBTC)) * btcPrice
                VStack(alignment: .trailing, spacing: 1) {
                    Text(totalUSD.usdFormatted)
                        .font(.headline.weight(.bold))
                    Text("\(totalDebitSats.btcSpacedFormatted) BTC")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 4)
        }
        .padding(14)
        .background(Color(uiColor: .secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 14))
    }
}
