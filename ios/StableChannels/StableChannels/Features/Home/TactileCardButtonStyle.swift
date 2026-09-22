import SwiftUI

// MARK: - Flat Premium Button Style

struct TactileKeycapButtonStyle: ButtonStyle {
    var pulse: Bool = false
    var isBreathing: Bool = false
    var pulseColor: Color = .init(red: 0.25, green: 0.85, blue: 0.55)

    func makeBody(configuration: Configuration) -> some View {
        let isPressed = configuration.isPressed

        configuration.label
            .background(
                ZStack {
                    // Sleek flat surface: rich deep obsidian charcoal
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(isPressed ? Color(white: 0.08) : Color(white: 0.11))

                    // Delicate hairline perimeter border
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .strokeBorder(
                            pulse
                                ? pulseColor.opacity(isBreathing ? 0.65 : 0.20)
                                : Color.white.opacity(isPressed ? 0.04 : 0.08),
                            lineWidth: pulse ? 1.5 : 1
                        )
                }
            )
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .shadow(
                color: pulse && isBreathing ? pulseColor.opacity(0.35) : Color.clear,
                radius: 6,
                x: 0,
                y: 0
            )
            .scaleEffect(isPressed ? 0.98 : 1.0)
            .animation(.spring(response: 0.22, dampingFraction: 0.8), value: isPressed)
    }
}
