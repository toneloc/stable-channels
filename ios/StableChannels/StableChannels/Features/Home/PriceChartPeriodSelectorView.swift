import SwiftUI

struct PriceChartPeriodSelectorView: View {
    @Binding var chartPeriod: ChartPeriod
    @Environment(\.colorScheme) private var colorScheme
    private let selectionFeedback = UISelectionFeedbackGenerator()

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(ChartPeriod.allCases, id: \.self) { period in
                    let isSelected = chartPeriod == period
                    Button {
                        selectionFeedback.selectionChanged()
                        withAnimation(.spring(response: 0.28, dampingFraction: 0.8)) {
                            chartPeriod = period
                        }
                    } label: {
                        Text(period.rawValue)
                            .font(.system(size: 11, weight: isSelected ? .bold : .medium))
                            .padding(.horizontal, 10)
                            .padding(.vertical, 5)
                            .background(
                                RoundedRectangle(cornerRadius: 8, style: .continuous)
                                    .fill(
                                        isSelected
                                            ? Color.blue
                                            : (colorScheme == .dark ? Color(.systemGray5) : Color(.systemGray6))
                                    )
                            )
                            .foregroundStyle(
                                isSelected
                                    ? Color.white
                                    : (colorScheme == .dark ? Color.primary : Color.secondary)
                            )
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 14)
        }
    }
}
