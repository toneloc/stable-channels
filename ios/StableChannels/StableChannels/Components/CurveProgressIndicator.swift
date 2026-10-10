import SwiftUI

struct CurveProgressIndicator: View {
    enum CurveType: CaseIterable {
        case sixPetalSpiral
        case spiralSearch
        case roseCurve
        case lemniscateBloom
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

        let activePulseDuration = (curve == .roseCurve) ? 4.6 : (curve == .lemniscateBloom ? 5.0 : pulseDuration)
        let pulseAngle = (time.truncatingRemainder(dividingBy: activePulseDuration) / activePulseDuration) * (2.0 * .pi)
        let detailScale = 0.52 + ((sin(pulseAngle + 0.55) + 1.0) / 2.0) * 0.48

        let activeDuration = curve == .spiralSearch ? 7.8 :
            (curve == .roseCurve ? 5.4 : (curve == .lemniscateBloom ? 5.6 : duration))
        let activeSpan = curve == .spiralSearch ? 0.28 :
            (curve == .roseCurve ? 0.32 : (curve == .lemniscateBloom ? 0.40 : trailSpan))
        let activeParticles = curve == .roseCurve ? 64 : (curve == .lemniscateBloom ? 70 : particleCount)
        let progress = time.truncatingRemainder(dividingBy: activeDuration) / activeDuration

        if curve == .roseCurve {
            let rotationDuration = 28.0
            let rotationAngle = -(time.truncatingRemainder(dividingBy: rotationDuration) / rotationDuration) *
                (2.0 * .pi)
            context.translateBy(x: center.x, y: center.y)
            context.rotate(by: Angle(radians: rotationAngle))
            context.translateBy(x: -center.x, y: -center.y)
        }

        if showTrack {
            var trackPath = Path()
            let steps = (curve == .roseCurve || curve == .lemniscateBloom) ? 240 : 120
            for step in 0...steps {
                let u = Double(step) / Double(steps)
                let pt = Self.pointOnCurve(curve: curve, progress: u, detailScale: detailScale)
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

        for index in 0..<activeParticles {
            let tailOffset = Double(index) / Double(max(1, activeParticles - 1))
            var u = (progress - tailOffset * activeSpan).truncatingRemainder(dividingBy: 1.0)
            if u < 0 { u += 1.0 }

            let pt = Self.pointOnCurve(curve: curve, progress: u, detailScale: detailScale)
            let mapped = CGPoint(
                x: center.x + (pt.x - 50.0) * scale,
                y: center.y + (pt.y - 50.0) * scale
            )

            let fade = pow(1.0 - tailOffset, 0.56)
            let radius = max(1.2, (0.9 + fade * 2.7) * scale)
            let opacity = 0.04 + fade * 0.96

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

    static func pointOnCurve(curve: CurveType, progress: Double, detailScale: Double) -> (x: Double, y: Double) {
        let t = progress * 2.0 * .pi
        switch curve {
        case .sixPetalSpiral:
            let d = 3.0 + detailScale * 0.25
            let baseX = 5.0 * cos(t) + d * cos(5.0 * t)
            let baseY = 5.0 * sin(t) - d * sin(5.0 * t)
            let s = (2.2 + detailScale * 0.45) * 1.85
            return (50.0 + baseX * s, 50.0 + baseY * s)

        case .spiralSearch:
            let angle = t * 4.0
            let radius = (8.0 + (1.0 - cos(t)) * (8.5 + detailScale * 2.4)) * 1.4
            return (50.0 + cos(angle) * radius, 50.0 + sin(angle) * radius)

        case .roseCurve:
            let a = 9.2 + detailScale * 0.6
            let r = a * (0.72 + detailScale * 0.28) * cos(5.0 * t)
            let scaleFactor = 3.25
            return (50.0 + cos(t) * r * scaleFactor, 50.0 + sin(t) * r * scaleFactor)

        case .lemniscateBloom:
            let a = 20.0 + detailScale * 7.0
            let sinT = sin(t)
            let denom = 1.0 + sinT * sinT
            return (
                50.0 + (a * cos(t)) / denom,
                50.0 + (a * sinT * cos(t)) / denom
            )
        }
    }
}

typealias MathCurveLoader = CurveProgressIndicator

#Preview("All Planted Loaders Gallery") {
    ScrollView {
        VStack(spacing: 28) {
            // Standalone Curves
            HStack(spacing: 40) {
                VStack(spacing: 8) {
                    CurveProgressIndicator(curve: .sixPetalSpiral, size: 76, tint: .orange)
                    Text("Six-Petal Spiral")
                        .font(.caption2.bold())
                        .foregroundStyle(.secondary)
                }

                VStack(spacing: 8) {
                    CurveProgressIndicator(curve: .spiralSearch, size: 76, tint: .blue)
                    Text("Spiral Search")
                        .font(.caption2.bold())
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.top, 8)

            Divider()

            // 1. Price Chart Card - Collecting Price Data
            VStack(alignment: .leading, spacing: 8) {
                Text("1. Price Chart (PriceChartView)")
                    .font(.caption.bold())
                    .foregroundStyle(.secondary)

                RoundedRectangle(cornerRadius: 12)
                    .fill(Color(.secondarySystemBackground))
                    .frame(height: 150)
                    .overlay {
                        VStack(spacing: 12) {
                            CurveProgressIndicator(curve: .spiralSearch, size: 68, tint: .blue)
                            Text("Collecting price data...")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
            }

            Divider()

            // 2. Buy / Sell - Order Pending Dialog
            VStack(alignment: .leading, spacing: 8) {
                Text("2. Trade Execution (BuyView / SellView)")
                    .font(.caption.bold())
                    .foregroundStyle(.secondary)

                VStack(spacing: 14) {
                    CurveProgressIndicator(curve: .sixPetalSpiral, size: 72, tint: .orange)
                        .padding(.bottom, 2)

                    Text("Order Pending")
                        .font(.headline.bold())

                    Text("Converting 0.00500000 BTC for $485.20")
                        .font(.caption)
                        .foregroundStyle(.secondary)

                    Text("Waiting for LSP confirmation...")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
                .padding()
                .frame(maxWidth: .infinity)
                .background(Color(.secondarySystemBackground))
                .clipShape(RoundedRectangle(cornerRadius: 16))
            }
        }
        .padding()
    }
}
