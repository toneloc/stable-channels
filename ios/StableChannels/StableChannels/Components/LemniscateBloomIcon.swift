import SwiftUI
import UIKit

/// Bernoulli Lemniscate vector path (infinity shape).
/// Parametric equations:
/// x(t) = a * cos(t) / (1 + sin²(t))
/// y(t) = a * sin(t) * cos(t) / (1 + sin²(t))
struct LemniscateShape: Shape {
    /// Precomputed normalized unit coordinates of the Bernoulli Lemniscate.
    /// Precalculating static vertices amortizes transcendental math to O(1) runtime per render.
    private static let unitPoints: [CGPoint] = {
        let steps = 120
        var points: [CGPoint] = []
        points.reserveCapacity(steps + 1)
        for step in 0...steps {
            let u = Double(step) / Double(steps)
            let t = u * 2.0 * .pi
            let sinT = sin(t)
            let cosT = cos(t)
            let denom = 1.0 + sinT * sinT
            let x0 = (1.41421356 * cosT) / denom
            let y0 = (1.41421356 * sinT * cosT) / denom
            points.append(CGPoint(x: x0, y: y0))
        }
        return points
    }()

    func path(in rect: CGRect) -> Path {
        var path = Path()
        guard let first = Self.unitPoints.first else { return path }
        let midX = rect.midX
        let midY = rect.midY
        let scaleX = (rect.width * 0.46) / 1.41421356
        let scaleY = (rect.height * 0.46) / 0.5

        path.move(to: CGPoint(x: midX + first.x * scaleX, y: midY + first.y * scaleY))
        for pt in Self.unitPoints.dropFirst() {
            path.addLine(to: CGPoint(x: midX + pt.x * scaleX, y: midY + pt.y * scaleY))
        }
        path.closeSubpath()
        return path
    }
}

/// Dynamic Lemniscate Bloom icon that replaces the static infinity symbol.
/// When activated ("Send All" or "Max Funds" selected), it executes a single, rapid
/// bloom animation tracing the Bernoulli Lemniscate curve, provides haptic feedback,
/// and smoothly settles into a solid, vibrant glowing infinity emblem without continuous GPU noise.
struct LemniscateBloomIcon: View {
    let isActive: Bool
    var size: CGFloat = 24
    var tint: Color = .green

    @State private var trimEnd: CGFloat = 0.0
    @State private var isBloomed: Bool = false
    @State private var scaleBounce: CGFloat = 1.0
    @State private var bloomTask: Task<Void, Never>?

    var body: some View {
        let width = size * 1.5
        let height = size

        ZStack {
            // Background subtle track
            LemniscateShape()
                .stroke(
                    isActive ? tint.opacity(0.25) : Color.secondary.opacity(0.35),
                    style: StrokeStyle(lineWidth: max(1.5, size * 0.10), lineCap: .round, lineJoin: .round)
                )

            // Animated Bloom Sweep on Activation
            if isActive {
                // Soft background glow
                LemniscateShape()
                    .stroke(
                        tint.opacity(isBloomed ? 0.35 : 0.15),
                        style: StrokeStyle(lineWidth: max(3.0, size * 0.22), lineCap: .round, lineJoin: .round)
                    )
                    .blur(radius: isBloomed ? 3 : 1)

                // Active Traced / Filled Stroke
                LemniscateShape()
                    .trim(from: 0, to: isBloomed ? 1.0 : trimEnd)
                    .stroke(
                        tint,
                        style: StrokeStyle(lineWidth: max(2.0, size * 0.12), lineCap: .round, lineJoin: .round)
                    )

                // Optional subtle fill when settled
                if isBloomed {
                    LemniscateShape()
                        .fill(tint.opacity(0.12))
                        .transition(.opacity)
                }
            }
        }
        .frame(width: width, height: height)
        .scaleEffect(scaleBounce)
        .onAppear {
            if isActive {
                trimEnd = 1.0
                isBloomed = true
                scaleBounce = 1.0
            }
        }
        .onChange(of: isActive, initial: false) { _, active in
            if active {
                triggerBloom()
            } else {
                resetBloom()
            }
        }
    }

    private func triggerBloom() {
        bloomTask?.cancel()
        bloomTask = Task { @MainActor in
            trimEnd = 0.0
            isBloomed = false

            UIImpactFeedbackGenerator(style: .medium).impactOccurred()

            withAnimation(.spring(response: 0.65, dampingFraction: 0.75)) {
                trimEnd = 1.0
                scaleBounce = 1.14
            }

            try? await Task.sleep(nanoseconds: 650_000_000)
            guard !Task.isCancelled else { return }

            withAnimation(.spring(response: 0.35, dampingFraction: 0.70)) {
                scaleBounce = 1.0
                isBloomed = true
            }
            UINotificationFeedbackGenerator().notificationOccurred(.success)
        }
    }

    private func resetBloom() {
        bloomTask?.cancel()
        bloomTask = nil
        withAnimation(.easeOut(duration: 0.2)) {
            trimEnd = 0.0
            isBloomed = false
            scaleBounce = 1.0
        }
    }
}
