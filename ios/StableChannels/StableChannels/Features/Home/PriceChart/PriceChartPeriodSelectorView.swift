import SwiftUI

struct PriceChartPeriodSelectorView: View {
    @Binding var chartPeriod: ChartPeriod
    @Environment(\.colorScheme) private var colorScheme
    @Namespace private var pillNamespace
    private let selectionFeedback = UISelectionFeedbackGenerator()

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(ChartPeriod.allCases, id: \.self) { period in
                    let isSelected = chartPeriod == period
                    Button {
                        guard chartPeriod != period else { return }
                        selectionFeedback.selectionChanged()
                        withAnimation(.spring(response: 0.28, dampingFraction: 0.78)) {
                            chartPeriod = period
                        }
                    } label: {
                        Text(period.rawValue)
                            .font(.system(size: 11, weight: isSelected ? .bold : .medium))
                            .padding(.horizontal, 10)
                            .padding(.vertical, 5)
                            .foregroundStyle(
                                isSelected
                                    ? Color.white
                                    : (colorScheme == .dark ? Color.primary : Color.secondary)
                            )
                            .background {
                                if isSelected {
                                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                                        .fill(Color.blue)
                                        .matchedGeometryEffect(id: "activePeriodPill", in: pillNamespace)
                                } else {
                                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                                        .fill(colorScheme == .dark ? Color(.systemGray5) : Color(white: 0.88))
                                }
                            }
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 14)
        }
    }
}
