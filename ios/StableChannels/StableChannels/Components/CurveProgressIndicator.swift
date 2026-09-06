import SwiftUI

struct CurveProgressIndicator: View {
    enum CurveType {
        case sixPetalSpiral
        case spiralSearch
    }

    var curve: CurveType = .sixPetalSpiral
    var size: CGFloat = 64
    var tint: Color = .orange
    var strokeColor: Color?
    var particleCount: Int = 48
    var trailSpan: Double = 0.34
    var duration: Double = 4.6
    var pulseDuration: Double = 4.2
    var showTrack: Bool = true

    var body: some View {
        TimelineView(.animation) { timeline in
            Canvas { context, canvasSize in
                let time = timeline.date.timeIntervalSinceReferenceDate
                render(context: &context, size: canvasSize, time: time)
            }
        }
        .frame(width: size, height: size)
    }

    private func render(context: inout GraphicsContext, size: CGSize, time: TimeInterval) {
        let center = CGPoint(x: size.width / 2.0, y: size.height / 2.0)
        let scale = min(size.width, size.height) / 100.0

        let pulseAngle = (time.truncatingRemainder(dividingBy: pulseDuration) / pulseDuration) * (2.0 * .pi)
        let detailScale = 0.52 + ((sin(pulseAngle + 0.55) + 1.0) / 2.0) * 0.48

        let activeDuration = curve == .spiralSearch ? 7.8 : duration
        let activeSpan = curve == .spiralSearch ? 0.28 : trailSpan
        let progress = time.truncatingRemainder(dividingBy: activeDuration) / activeDuration

        if showTrack {
            var trackPath = Path()
            let steps = 120
            for step in 0...steps {
                let u = Double(step) / Double(steps)
                let pt = pointOnCurve(curve: curve, progress: u, detailScale: detailScale)
                let mapped = CGPoint(
                    x: center.x + (pt.x - 50.0) * scale,
                    y: center.y + (pt.y - 50.0) * scale
                )
                if step == 0 {
                    trackPath.move(to: mapped)
                } else {
                    trackPath.addLine(to: mapped)
                }
            }
            trackPath.closeSubpath()
            let trackColor = (strokeColor ?? tint).opacity(0.12)
            context.stroke(trackPath, with: .color(trackColor), lineWidth: max(1.0, 1.8 * scale))
        }

        for index in 0..<particleCount {
            let tailOffset = Double(index) / Double(max(1, particleCount - 1))
            var u = (progress - tailOffset * activeSpan).truncatingRemainder(dividingBy: 1.0)
            if u < 0 { u += 1.0 }

            let pt = pointOnCurve(curve: curve, progress: u, detailScale: detailScale)
            let mapped = CGPoint(
                x: center.x + (pt.x - 50.0) * scale,
                y: center.y + (pt.y - 50.0) * scale
            )

            let fade = pow(1.0 - tailOffset, 0.56)
            let radius = (0.7 + fade * 2.3) * scale
            let opacity = 0.04 + fade * 0.94

            let particleRect = CGRect(
                x: mapped.x - radius,
                y: mapped.y - radius,
                width: radius * 2.0,
                height: radius * 2.0
            )

            context.fill(
                Path(ellipseIn: particleRect),
                with: .color(tint.opacity(opacity))
            )
        }
    }

    private func pointOnCurve(curve: CurveType, progress: Double, detailScale: Double) -> (x: Double, y: Double) {
        let t = progress * 2.0 * .pi
        switch curve {
        case .sixPetalSpiral:
            let d = 3.0 + detailScale * 0.25
            let baseX = 5.0 * cos(t) + d * cos(5.0 * t)
            let baseY = 5.0 * sin(t) - d * sin(5.0 * t)
            let s = 2.2 + detailScale * 0.45
            return (50.0 + baseX * s, 50.0 + baseY * s)

        case .spiralSearch:
            let angle = t * 4.0
            let radius = 8.0 + (1.0 - cos(t)) * (8.5 + detailScale * 2.4)
            return (50.0 + cos(angle) * radius, 50.0 + sin(angle) * radius)
        }
    }
}

typealias MathCurveLoader = CurveProgressIndicator

#Preview {
    VStack(spacing: 32) {
        CurveProgressIndicator(curve: .sixPetalSpiral, size: 72, tint: .orange)
        CurveProgressIndicator(curve: .spiralSearch, size: 72, tint: .blue)
    }
    .padding()
}
