import SwiftUI

// MARK: - Adaptive Premium Button Style

struct TactileKeycapButtonStyle: ButtonStyle {
    @Environment(\.colorScheme) private var colorScheme

    var pulse: Bool = false
    var isBreathing: Bool = false
    var pulseColor: Color = .init(red: 0.25, green: 0.85, blue: 0.55)

    func makeBody(configuration: Configuration) -> some View {
        let isPressed = configuration.isPressed
        let isDark = colorScheme == .dark

        // Surface fill: clean off-white in light mode, deep obsidian in dark mode
        let surfaceFill: Color = isDark
            ? (isPressed ? Color(white: 0.08) : Color(white: 0.12))
            : (isPressed ? Color(white: 0.88) : Color(white: 0.96))

        // Delicate perimeter stroke
        let borderStroke: Color = pulse
            ? pulseColor.opacity(isBreathing ? (isDark ? 0.70 : 0.85) : (isDark ? 0.20 : 0.30))
            : (isDark
                ? Color.white.opacity(isPressed ? 0.04 : 0.08)
                : Color(.separator).opacity(isPressed ? 0.45 : 0.25))

        let shadowColor: Color = pulse && isBreathing
            ? pulseColor.opacity(isDark ? 0.35 : 0.25)
            : (isDark ? Color.clear : Color.black.opacity(isPressed ? 0.01 : 0.04))

        configuration.label
            .background(
                ZStack {
                    RoundedRectangle(cornerRadius: 14, style: .continuous)
                        .fill(surfaceFill)

                    RoundedRectangle(cornerRadius: 14, style: .continuous)
                        .strokeBorder(borderStroke, lineWidth: pulse ? 1.5 : 1)
                }
            )
            .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
            .shadow(
                color: shadowColor,
                radius: pulse && isBreathing ? 6 : 4,
                x: 0,
                y: isPressed ? 0.5 : 1.5
            )
            .scaleEffect(isPressed ? 0.98 : 1.0)
            .animation(.spring(response: 0.22, dampingFraction: 0.8), value: isPressed)
    }
}
