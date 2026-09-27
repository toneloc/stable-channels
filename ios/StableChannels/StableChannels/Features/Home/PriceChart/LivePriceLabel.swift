import SwiftUI

struct LivePriceLabel: View {
    @Environment(AppState.self) private var appState: AppState?
    var priceOverride: Double?

    @State private var tickColor: Color = .primary
    @State private var colorResetTask: Task<Void, Never>?

    private var currentPrice: Double {
        priceOverride ?? appState?.btcPrice ?? 0
    }

    var body: some View {
        Group {
            if currentPrice > 0 {
                Text(currentPrice.usdFormatted)
                    .font(.headline.bold().monospacedDigit())
                    .foregroundStyle(tickColor)
                    .contentTransition(.numericText())
            } else {
                Text("---")
                    .font(.headline.bold().monospacedDigit())
                    .foregroundStyle(.secondary)
            }
        }
        .animation(.snappy(duration: 0.28, extraBounce: 0.05), value: currentPrice)
        .onChange(of: currentPrice) { old, new in
            guard old > 0, new > 0, old != new else { return }
            colorResetTask?.cancel()
            withAnimation(.easeOut(duration: 0.15)) {
                tickColor = new > old ? .green : .red
            }
            colorResetTask = Task { @MainActor in
                try? await Task.sleep(nanoseconds: 450_000_000)
                guard !Task.isCancelled else { return }
                withAnimation(.easeOut(duration: 0.35)) {
                    tickColor = .primary
                }
            }
        }
    }
}

// MARK: - Previews

#Preview {
    LivePriceLabelPreviewContainer()
        .environment(AppState())
}

private struct LivePriceLabelPreviewContainer: View {
    @State private var previewPrice: Double = 95_100.00

    var body: some View {
        VStack(spacing: 24) {
            HStack(spacing: 10) {
                Image(systemName: "bitcoinsign.circle.fill")
                    .font(.system(size: 22))
                    .foregroundStyle(.orange)

                VStack(alignment: .leading, spacing: 2) {
                    Text(String(localized: "label_btc_price", defaultValue: "BTC Price"))
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.secondary)
                }

                Spacer()

                LivePriceLabel(priceOverride: previewPrice)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 16))

            VStack(spacing: 10) {
                HStack(spacing: 10) {
                    Button("+$1") { previewPrice += 1 }
                    Button("-$1") { previewPrice = max(0, previewPrice - 1) }
                }

                HStack(spacing: 10) {
                    Button("+$250") { previewPrice += 250 }
                    Button("-$500") { previewPrice = max(0, previewPrice - 500) }
                }

                HStack(spacing: 10) {
                    Button("$100,000") { previewPrice = 100_000.50 }
                    Button("$0") { previewPrice = 0 }
                }
            }
            .buttonStyle(.bordered)
        }
        .padding()
    }
}
