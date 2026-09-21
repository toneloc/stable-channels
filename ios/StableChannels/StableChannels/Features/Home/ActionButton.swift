import SwiftUI

struct ActionButton: View {
    let title: String
    var subtitle: String?
    let icon: String
    var color: Color = .white
    var pulse: Bool = false
    let action: () -> Void

    @State private var isBreathing = false

    var body: some View {
        Button(action: {
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
            action()
        }) {
            VStack(alignment: .leading, spacing: 0) {
                // Top row: Icon Badge
                HStack {
                    ZStack {
                        RoundedRectangle(cornerRadius: 10, style: .continuous)
                            .fill(color.opacity(0.15))
                            .overlay(
                                RoundedRectangle(cornerRadius: 10, style: .continuous)
                                    .strokeBorder(
                                        color.opacity(0.22),
                                        lineWidth: 1
                                    )
                            )

                        Image(systemName: icon)
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundStyle(color)
                    }
                    .frame(width: 36, height: 36)
                    .shadow(
                        color: pulse && isBreathing ? color.opacity(0.40) : Color.clear,
                        radius: 6,
                        x: 0,
                        y: 0
                    )

                    Spacer()
                }

                Spacer(minLength: 16)

                // Bottom: Title and Subtitle
                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .font(.system(size: 15, weight: .semibold))
                        .foregroundStyle(Color.white)
                        .lineLimit(1)
                        .minimumScaleFactor(0.85)

                    if let subtitle, !subtitle.isEmpty {
                        Text(subtitle)
                            .font(.system(size: 12, weight: .regular))
                            .foregroundStyle(Color(white: 0.60))
                            .lineLimit(1)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .frame(height: 108)
            .padding(16)
        }
        .buttonStyle(
            AddToCartKeycapCardButtonStyle(
                pulse: pulse,
                isBreathing: isBreathing,
                pulseColor: color
            )
        )
        .onAppear {
            if pulse {
                withAnimation(.easeInOut(duration: 1.6).repeatForever(autoreverses: true)) {
                    isBreathing = true
                }
            }
        }
        .onChange(of: pulse) { _, newValue in
            if newValue {
                withAnimation(.easeInOut(duration: 1.6).repeatForever(autoreverses: true)) {
                    isBreathing = true
                }
            } else {
                withAnimation(.easeInOut(duration: 0.2)) {
                    isBreathing = false
                }
            }
        }
    }
}
