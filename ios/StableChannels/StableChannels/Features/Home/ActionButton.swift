import SwiftUI

struct ActionButton: View {
    let title: String
    let icon: String
    var color: Color = .white
    var textColor: Color = .white
    var pulse: Bool = false
    let action: () -> Void

    @State private var isBreathing = false

    var body: some View {
        Button(action: {
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
            action()
        }) {
            HStack(spacing: 10) {
                // Colored circular logo badge
                ZStack {
                    Circle()
                        .fill(color)
                        .frame(width: 26, height: 26)

                    Image(systemName: icon)
                        .font(.system(size: 12, weight: .bold))
                        .foregroundStyle(Color.white)
                }

                Text(title)
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(textColor)
                    .lineLimit(1)
                    .minimumScaleFactor(0.85)
            }
            .padding(.horizontal, 14)
            .frame(maxWidth: .infinity)
            .frame(height: 48)
        }
        .buttonStyle(
            TactileKeycapButtonStyle(
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
