import SwiftUI

// MARK: - 3D Keycap Card Button Style

struct AddToCartKeycapCardButtonStyle: ButtonStyle {
    var pulse: Bool = false
    var isBreathing: Bool = false
    var pulseColor: Color = .init(red: 0.25, green: 0.85, blue: 0.55)

    func makeBody(configuration: Configuration) -> some View {
        let isPressed = configuration.isPressed

        configuration.label
            .background(
                ZStack {
                    // Base background surface: neutral-800 (#262626) idle -> neutral-900 (#171717) pressed
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
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
                            .frame(height: 14)
                        }
                        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))

                        // Inset top sheen / rim: inset 0 1px 2px rgba(255,255,255,0.14)
                        VStack {
                            LinearGradient(
                                colors: [Color.white.opacity(0.16), Color.white.opacity(0.0)],
                                startPoint: .top,
                                endPoint: .bottom
                            )
                            .frame(height: 6)
                            Spacer()
                        }
                        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))

                        // Subtle outer rim highlight stroke
                        RoundedRectangle(cornerRadius: 18, style: .continuous)
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
                            .frame(height: 16)
                            Spacer()
                        }
                        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))

                        // Inset bottom specular bounce: inset 0 -1px 1px rgba(255,255,255,0.06)
                        VStack {
                            Spacer()
                            LinearGradient(
                                colors: [Color.clear, Color.white.opacity(0.06)],
                                startPoint: .top,
                                endPoint: .bottom
                            )
                            .frame(height: 4)
                        }
                        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))

                        // Deep inner border
                        RoundedRectangle(cornerRadius: 18, style: .continuous)
                            .strokeBorder(Color.black.opacity(0.5), lineWidth: 1)
                    }

                    // Pulse accent border if channel requires funding
                    if pulse {
                        RoundedRectangle(cornerRadius: 18, style: .continuous)
                            .strokeBorder(
                                pulseColor.opacity(isBreathing ? 0.75 : 0.15),
                                lineWidth: 1.5
                            )
                    }
                }
            )
            .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
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
            .offset(y: isPressed ? 2.0 : 0)
            .scaleEffect(isPressed ? 0.975 : 1.0)
            .animation(.spring(response: 0.24, dampingFraction: 0.74), value: isPressed)
    }
}
