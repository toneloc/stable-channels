import SwiftUI

struct ActionButton: View {
    let title: String
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
            HStack(spacing: 8) {
                Image(systemName: icon)
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(color)

                Text(title)
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(Color.white)
                    .lineLimit(1)
                    .minimumScaleFactor(0.85)
            }
            .padding(.horizontal, 16)
            .frame(maxWidth: .infinity)
            .frame(height: 48)
        }
        .buttonStyle(
            AddToCartKeycapButtonStyle(
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

// MARK: - 3D Keycap Button Style (inspired by Opensource UI AddToCartButton)

struct AddToCartKeycapButtonStyle: ButtonStyle {
    var pulse: Bool = false
    var isBreathing: Bool = false
    var pulseColor: Color = .init(red: 0.25, green: 0.85, blue: 0.55)

    func makeBody(configuration: Configuration) -> some View {
        let isPressed = configuration.isPressed

        configuration.label
            .background(
                ZStack {
                    // Base background surface: neutral-800 (#262626) idle -> neutral-900 (#171717) pressed
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(isPressed ? Color(red: 0.09, green: 0.09, blue: 0.09) : Color(
                            red: 0.15,
                            green: 0.15,
                            blue: 0.15
                        ))

                    if !isPressed {
                        // Inset bottom recess: inset 0 -3px 6px rgba(0,0,0,0.55)
                        VStack {
                            Spacer()
                            LinearGradient(
                                colors: [Color.black.opacity(0.0), Color.black.opacity(0.55)],
                                startPoint: .top,
                                endPoint: .bottom
                            )
                            .frame(height: 10)
                        }
                        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

                        // Inset top sheen / rim: inset 0 1px 2px rgba(255,255,255,0.14)
                        VStack {
                            LinearGradient(
                                colors: [Color.white.opacity(0.16), Color.white.opacity(0.0)],
                                startPoint: .top,
                                endPoint: .bottom
                            )
                            .frame(height: 4)
                            Spacer()
                        }
                        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

                        // Subtle outer rim highlight stroke
                        RoundedRectangle(cornerRadius: 12, style: .continuous)
                            .strokeBorder(
                                LinearGradient(
                                    colors: [Color.white.opacity(0.14), Color.white.opacity(0.02)],
                                    startPoint: .top,
                                    endPoint: .bottom
                                ),
                                lineWidth: 1
                            )
                    } else {
                        // Pressed state: sunken key with top interior shadow: inset 0 2px 6px rgba(0,0,0,0.55)
                        VStack {
                            LinearGradient(
                                colors: [Color.black.opacity(0.70), Color.black.opacity(0.0)],
                                startPoint: .top,
                                endPoint: .bottom
                            )
                            .frame(height: 12)
                            Spacer()
                        }
                        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

                        // Inset bottom specular bounce: inset 0 -1px 1px rgba(255,255,255,0.06)
                        VStack {
                            Spacer()
                            LinearGradient(
                                colors: [Color.clear, Color.white.opacity(0.06)],
                                startPoint: .top,
                                endPoint: .bottom
                            )
                            .frame(height: 3)
                        }
                        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

                        // Deep inner border
                        RoundedRectangle(cornerRadius: 12, style: .continuous)
                            .strokeBorder(Color.black.opacity(0.5), lineWidth: 1)
                    }

                    // Pulse accent border if channel requires funding
                    if pulse {
                        RoundedRectangle(cornerRadius: 12, style: .continuous)
                            .strokeBorder(
                                pulseColor.opacity(isBreathing ? 0.75 : 0.15),
                                lineWidth: 1.5
                            )
                    }
                }
            )
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            // Multi-tier 3D drop shadows (idle: 0 1px 1px, 0 3px 6px, 0 8px 16px; pressed: 0 1px 2px)
            .shadow(
                color: isPressed ? Color.black.opacity(0.25) : Color.black.opacity(0.35),
                radius: isPressed ? 1 : 1,
                x: 0,
                y: isPressed ? 1 : 1
            )
            .shadow(
                color: isPressed ? Color.clear : Color.black.opacity(0.28),
                radius: 3,
                x: 0,
                y: 3
            )
            .shadow(
                color: isPressed ? Color.clear : Color.black.opacity(0.22),
                radius: 8,
                x: 0,
                y: 8
            )
            .shadow(
                color: pulse && isBreathing ? pulseColor.opacity(0.40) : Color.clear,
                radius: 8,
                x: 0,
                y: 2
            )
            .offset(y: isPressed ? 1.5 : 0)
            .scaleEffect(isPressed ? 0.985 : 1.0)
            .animation(.spring(response: 0.25, dampingFraction: 0.75), value: isPressed)
    }
}
