import SwiftUI

struct ActionButton: View {
    let title: String
    var subtitle: String?
    let icon: String
    var badgeColor: Color = .blue
    var pulse: Bool = false
    let action: () -> Void

    @Environment(\.colorScheme) private var colorScheme
    @State private var isBreathing = false

    private var isDark: Bool { colorScheme == .dark }

    var body: some View {
        Button(action: {
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
            action()
        }) {
            VStack(alignment: .leading, spacing: 0) {
                // Top row: Icon Badge
                HStack {
                    ZStack {
                        RoundedRectangle(cornerRadius: 10, style: .continuous)
                            .fill(badgeBackgroundColor)
                            .overlay(
                                RoundedRectangle(cornerRadius: 10, style: .continuous)
                                    .strokeBorder(
                                        badgeColor.opacity(isDark ? 0.18 : 0.10),
                                        lineWidth: 1
                                    )
                            )

                        Image(systemName: icon)
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundStyle(badgeColor)
                    }
                    .frame(width: 36, height: 36)
                    .shadow(
                        color: pulse && isBreathing ? badgeColor.opacity(0.35) : Color.clear,
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
                        .foregroundStyle(isDark ? Color.white : Color.primary)

                    if let subtitle, !subtitle.isEmpty {
                        Text(subtitle)
                            .font(.system(size: 12, weight: .regular))
                            .foregroundStyle(isDark ? Color(white: 0.55) : Color.secondary)
                            .lineLimit(1)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .frame(height: 110)
            .padding(16)
        }
        .buttonStyle(
            FintechCardButtonStyle(
                isDark: isDark,
                pulse: pulse,
                isBreathing: isBreathing,
                pulseColor: badgeColor
            )
        )
        .onAppear {
            if pulse {
                withAnimation(.easeInOut(duration: 1.8).repeatForever(autoreverses: true)) {
                    isBreathing = true
                }
            }
        }
        .onChange(of: pulse) { _, newValue in
            if newValue {
                withAnimation(.easeInOut(duration: 1.8).repeatForever(autoreverses: true)) {
                    isBreathing = true
                }
            } else {
                withAnimation(.easeInOut(duration: 0.2)) {
                    isBreathing = false
                }
            }
        }
    }

    private var badgeBackgroundColor: Color {
        if isDark {
            return badgeColor.opacity(0.15)
        } else {
            return badgeColor.opacity(0.12)
        }
    }
}

// MARK: - Fintech Card Button Style

struct FintechCardButtonStyle: ButtonStyle {
    let isDark: Bool
    var pulse: Bool = false
    var isBreathing: Bool = false
    var pulseColor: Color = .green

    func makeBody(configuration: Configuration) -> some View {
        let isPressed = configuration.isPressed

        configuration.label
            .background(
                ZStack {
                    RoundedRectangle(cornerRadius: 20, style: .continuous)
                        .fill(cardBackgroundColor(isPressed: isPressed))

                    RoundedRectangle(cornerRadius: 20, style: .continuous)
                        .strokeBorder(
                            pulse
                                ? pulseColor.opacity(isBreathing ? 0.45 : 0.12)
                                : perimeterBorderColor,
                            lineWidth: pulse ? 1.5 : 1
                        )
                }
            )
            .clipShape(RoundedRectangle(cornerRadius: 20, style: .continuous))
            .shadow(
                color: isDark
                    ? Color.black.opacity(isPressed ? 0.20 : 0.45)
                    : Color.black.opacity(isPressed ? 0.02 : 0.05),
                radius: isPressed ? 2 : 6,
                x: 0,
                y: isPressed ? 1 : 3
            )
            .scaleEffect(isPressed ? 0.965 : 1.0)
            .animation(.spring(response: 0.24, dampingFraction: 0.74), value: isPressed)
    }

    // Deep, dark charcoal card surface matching the reference dark mode
    private func cardBackgroundColor(isPressed: Bool) -> Color {
        if isDark {
            return isPressed
                ? Color(red: 0.065, green: 0.068, blue: 0.075) // deep obsidian pressed
                : Color(red: 0.095, green: 0.098, blue: 0.105) // sleek dark charcoal
        } else {
            return isPressed
                ? Color(red: 0.91, green: 0.915, blue: 0.925)
                : Color(red: 0.96, green: 0.965, blue: 0.975)
        }
    }

    private var perimeterBorderColor: Color {
        isDark ? Color.white.opacity(0.04) : Color.black.opacity(0.04)
    }
}
