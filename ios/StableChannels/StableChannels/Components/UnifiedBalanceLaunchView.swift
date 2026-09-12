//  Unified cinematic app launch hero component.
//  Executes a seamless staged choreography:
//  1. Appearance in balance
//  2. Single radiant wake-up shimmer sweep
//  3. Active harmonic balance oscillation while syncing
//  4. Exponentially damped settling into perfect horizontal equilibrium

import SwiftUI

public struct UnifiedBalanceLaunchView: View {
    public var isSyncComplete: Bool
    public var size: CGFloat
    public var baseColor: Color
    public var onBalanced: (() -> Void)?

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    @State private var startTime: Double = 0.0
    @State private var settleStartTime: Double?
    @State private var hasNotifiedBalanced = false

    private let kinematics: BalanceScaleKinematics

    private static let fulcrumX: CGFloat = 511.5
    private static let fulcrumY: CGFloat = 361.0
    private static let leftPivotX: CGFloat = 273.0
    private static let leftPivotY: CGFloat = 361.0

    // Bounding box dimensions with safe vertical swing headroom:
    // Y: 210.0 ... 778.0 (height = 568.0)
    // X: 171.9 ... 868.3 (width = 696.4)
    private static let markWidth: CGFloat = 696.4
    private static let markHeight: CGFloat = 568.0
    private static let markMinX: CGFloat = 171.9
    private static let markMinY: CGFloat = 210.0

    public init(
        isSyncComplete: Bool = false,
        size: CGFloat = 120,
        baseColor: Color = Color(red: 0.969, green: 0.576, blue: 0.102),
        kinematics: BalanceScaleKinematics = BalanceScaleKinematics(),
        onBalanced: (() -> Void)? = nil
    ) {
        self.isSyncComplete = isSyncComplete
        self.size = size
        self.baseColor = baseColor
        self.kinematics = kinematics
        self.onBalanced = onBalanced
    }

    private var contentHeight: CGFloat {
        size * (Self.markHeight / Self.markWidth)
    }

    public var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 60.0)) { timeline in
            let now = timeline.date.timeIntervalSinceReferenceDate
            let effectiveStart = startTime == 0.0 ? now : startTime
            let elapsed = max(0.0, now - effectiveStart)

            let settleElapsed = settleStartTime.map { max(0.0, now - $0) }
            let stage = reduceMotion
                ? .balanced
                : kinematics.evaluate(
                    elapsedSinceStart: elapsed,
                    isSyncComplete: isSyncComplete,
                    settleElapsed: settleElapsed
                )

            Canvas { context, canvasSize in
                drawStage(
                    stage: stage,
                    context: context,
                    canvasSize: canvasSize
                )
            }
            .onChange(of: stage) { _, newStage in
                if newStage == .balanced && !hasNotifiedBalanced {
                    hasNotifiedBalanced = true
                    onBalanced?()
                }
            }
        }
        .frame(width: size, height: contentHeight)
        .onAppear {
            startTime = Date().timeIntervalSinceReferenceDate
            if isSyncComplete {
                settleStartTime = startTime
            }
        }
        .onChange(of: isSyncComplete) { _, complete in
            if complete && settleStartTime == nil {
                settleStartTime = Date().timeIntervalSinceReferenceDate
            }
        }
    }

    private func drawStage(
        stage: BalanceScaleKinematics.Stage,
        context: GraphicsContext,
        canvasSize: CGSize
    ) {
        let sX = canvasSize.width / Self.markWidth
        let sY = canvasSize.height / Self.markHeight
        let fitTransform = CGAffineTransform(
            translationX: -Self.markMinX * sX,
            y: -Self.markMinY * sY
        ).scaledBy(x: sX, y: sY)

        let unitRect = CGRect(x: 0, y: 0, width: 1024, height: 1024)

        // Determine current tilt angle
        let angleDegrees: Double
        switch stage {
        case .resting, .balanced:
            angleDegrees = 0.0
        case .shimmer:
            angleDegrees = 0.0
        case .oscillating(let angle), .settling(let angle):
            angleDegrees = angle
        }

        // 1. Draw Grounded Stand
        let standPath = BalanceScaleStandShape().path(in: unitRect).applying(fitTransform)
        context.fill(standPath, with: .color(baseColor), style: FillStyle(eoFill: true))

        let rad = CGFloat(angleDegrees * .pi / 180.0)

        // 2. Crossbeam and Right Coin Assembly
        let armTransform = CGAffineTransform(translationX: Self.fulcrumX, y: Self.fulcrumY)
            .rotated(by: rad)
            .translatedBy(x: -Self.fulcrumX, y: -Self.fulcrumY)
            .concatenating(fitTransform)

        let beamPath = BalanceScaleBeamShape().path(in: unitRect).applying(armTransform)
        let coinPath = BalanceScaleCoinShape().path(in: unitRect).applying(armTransform)
        let btcPath = BalanceScaleBtcShape().path(in: unitRect).applying(armTransform)

        context.fill(beamPath, with: .color(baseColor), style: FillStyle(eoFill: true))
        context.fill(coinPath, with: .color(baseColor), style: FillStyle(eoFill: true))
        context.fill(btcPath, with: .color(baseColor), style: FillStyle(eoFill: true))

        // 3. Hanging Left Pan
        let leftArmDX = Self.leftPivotX - Self.fulcrumX
        let leftPivotNewX = Self.fulcrumX + leftArmDX * cos(rad)
        let leftPivotNewY = Self.fulcrumY + leftArmDX * sin(rad)
        let leftPanShiftX = leftPivotNewX - Self.leftPivotX
        let leftPanShiftY = leftPivotNewY - Self.leftPivotY

        let panTransform = CGAffineTransform(translationX: leftPanShiftX, y: leftPanShiftY)
            .concatenating(fitTransform)

        let panPath = BalanceScalePanShape().path(in: unitRect).applying(panTransform)
        context.fill(panPath, with: .color(baseColor), style: FillStyle(eoFill: true))

        // 4. Single Wake-Up Shimmer Wave
        if case .shimmer(let progress) = stage {
            let (startNorm, endNorm) = BalanceScaleKinematics.shimmerSweepRange(progress: progress)

            let pStart = CGPoint(
                x: CGFloat(startNorm) * canvasSize.width,
                y: CGFloat(startNorm) * canvasSize.height
            )
            let pEnd = CGPoint(
                x: CGFloat(endNorm) * canvasSize.width,
                y: CGFloat(endNorm) * canvasSize.height
            )

            let shimmerGradient = Gradient(stops: [
                .init(color: .clear, location: 0.0),
                .init(color: Color.white.opacity(0.15), location: 0.32),
                .init(color: Color.white.opacity(0.85), location: 0.50),
                .init(color: Color.white.opacity(0.15), location: 0.68),
                .init(color: .clear, location: 1.0)
            ])

            // Clip and illuminate each shape individually to prevent even-odd overlap cancellation
            let paths = [standPath, beamPath, coinPath, btcPath, panPath]
            for shapePath in paths {
                var itemContext = context
                itemContext.clip(to: shapePath, style: FillStyle(eoFill: true))
                itemContext.fill(
                    Path(CGRect(origin: .zero, size: canvasSize)),
                    with: .linearGradient(shimmerGradient, startPoint: pStart, endPoint: pEnd)
                )
            }
        }
    }
}

// MARK: - Previews

#Preview("Unified Launch - Interactive") {
    UnifiedLaunchPreviewContainer()
}

private struct UnifiedLaunchPreviewContainer: View {
    @State private var isSyncComplete = false
    @State private var status = "Syncing..."

    var body: some View {
        VStack(spacing: 24) {
            Spacer()

            UnifiedBalanceLaunchView(
                isSyncComplete: isSyncComplete,
                size: 130,
                onBalanced: {
                    status = "Equilibrium Achieved"
                }
            )

            VStack(spacing: 6) {
                Text(String(localized: "app_name", defaultValue: "Stable Channels"))
                    .font(.title3.weight(.semibold))
                    .foregroundStyle(.primary)

                Text(status)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }

            Spacer()

            Button(isSyncComplete ? "Restart Flow" : "Complete Sync") {
                if isSyncComplete {
                    isSyncComplete = false
                    status = "Syncing..."
                } else {
                    isSyncComplete = true
                }
            }
            .buttonStyle(.borderedProminent)
            .padding(.bottom, 24)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .preferredColorScheme(.dark)
    }
}

// MARK: - Shapes

import SwiftUI

public struct BalanceScaleStandShape: Shape {
    public func path(in _: CGRect) -> Path {
        let path = CGMutablePath()
        let transform = CGAffineTransform.identity

        path.move(to: CGPoint(x: 519.50, y: 352.50))
        path.addLine(to: CGPoint(x: 519.20, y: 334.20))
        path.addCurve(
            to: CGPoint(x: 520.50, y: 316.00),
            control1: CGPoint(x: 519.00, y: 318.20),
            control2: CGPoint(x: 519.10, y: 316.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 525.30, y: 314.40),
            control1: CGPoint(x: 521.40, y: 316.00),
            control2: CGPoint(x: 523.50, y: 315.30),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 539.20, y: 300.30),
            control1: CGPoint(x: 531.80, y: 311.10),
            control2: CGPoint(x: 535.80, y: 306.90),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 542.50, y: 283.60),
            control1: CGPoint(x: 542.30, y: 294.10),
            control2: CGPoint(x: 542.50, y: 293.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 535.00, y: 260.70),
            control1: CGPoint(x: 542.50, y: 272.30),
            control2: CGPoint(x: 540.90, y: 267.40),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 514.50, y: 251.20),
            control1: CGPoint(x: 529.30, y: 254.20),
            control2: CGPoint(x: 523.90, y: 251.70),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 484.30, y: 267.80),
            control1: CGPoint(x: 500.10, y: 250.40),
            control2: CGPoint(x: 490.10, y: 255.90),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 481.50, y: 284.00),
            control1: CGPoint(x: 481.80, y: 272.90),
            control2: CGPoint(x: 481.50, y: 274.50),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 483.90, y: 299.50),
            control1: CGPoint(x: 481.50, y: 293.10),
            control2: CGPoint(x: 481.80, y: 295.20),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 499.30, y: 314.50),
            control1: CGPoint(x: 487.20, y: 306.10),
            control2: CGPoint(x: 493.60, y: 312.30),
            transform: transform
        )
        path.addLine(to: CGPoint(x: 504.00, y: 316.30))
        path.addLine(to: CGPoint(x: 504.00, y: 334.60))
        path.addLine(to: CGPoint(x: 504.00, y: 353.00))
        path.addLine(to: CGPoint(x: 504.00, y: 369.00))
        path.addLine(to: CGPoint(x: 504.00, y: 565.50))
        path.addLine(to: CGPoint(x: 504.00, y: 762.00))
        path.addLine(to: CGPoint(x: 421.60, y: 762.00))
        path.addCurve(
            to: CGPoint(x: 337.60, y: 763.60),
            control1: CGPoint(x: 348.00, y: 762.00),
            control2: CGPoint(x: 339.00, y: 762.20),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 337.20, y: 775.80),
            control1: CGPoint(x: 335.70, y: 765.50),
            control2: CGPoint(x: 335.40, y: 774.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 511.60, y: 777.00),
            control1: CGPoint(x: 338.10, y: 776.70),
            control2: CGPoint(x: 378.20, y: 777.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 688.00, y: 769.10),
            control1: CGPoint(x: 706.30, y: 777.00),
            control2: CGPoint(x: 688.00, y: 777.80),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 686.80, y: 763.20),
            control1: CGPoint(x: 688.00, y: 766.50),
            control2: CGPoint(x: 687.50, y: 763.90),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 602.30, y: 762.00),
            control1: CGPoint(x: 685.90, y: 762.30),
            control2: CGPoint(x: 665.80, y: 762.00),
            transform: transform
        )
        path.addLine(to: CGPoint(x: 519.00, y: 762.00))
        path.addLine(to: CGPoint(x: 519.00, y: 565.50))
        path.addLine(to: CGPoint(x: 519.00, y: 369.00))
        path.addLine(to: CGPoint(x: 519.50, y: 352.50))
        path.closeSubpath()

        return Path(path)
    }
}

public struct BalanceScaleBeamShape: Shape {
    public func path(in _: CGRect) -> Path {
        let path = CGMutablePath()
        let transform = CGAffineTransform.identity

        path.move(to: CGPoint(x: 270.00, y: 353.00))
        path.addLine(to: CGPoint(x: 630.00, y: 353.00))
        path.addLine(to: CGPoint(x: 630.00, y: 369.00))
        path.addLine(to: CGPoint(x: 270.00, y: 369.00))
        path.closeSubpath()

        return Path(path)
    }
}

public struct BalanceScalePanShape: Shape {
    public func path(in _: CGRect) -> Path {
        let path = CGMutablePath()
        let transform = CGAffineTransform.identity

        path.move(to: CGPoint(x: 273.00, y: 361.00))
        path.addLine(to: CGPoint(x: 265.10, y: 353.00))
        path.addLine(to: CGPoint(x: 262.60, y: 355.20))
        path.addCurve(
            to: CGPoint(x: 224.40, y: 425.00),
            control1: CGPoint(x: 261.30, y: 356.50),
            control2: CGPoint(x: 244.10, y: 387.90),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 176.00, y: 521.40),
            control1: CGPoint(x: 171.90, y: 524.20),
            control2: CGPoint(x: 176.00, y: 516.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 201.00, y: 565.90),
            control1: CGPoint(x: 176.00, y: 533.20),
            control2: CGPoint(x: 186.30, y: 551.50),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 230.70, y: 585.10),
            control1: CGPoint(x: 207.10, y: 571.80),
            control2: CGPoint(x: 223.40, y: 582.40),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 267.00, y: 591.10),
            control1: CGPoint(x: 247.70, y: 591.30),
            control2: CGPoint(x: 247.30, y: 591.20),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 290.50, y: 590.10),
            control1: CGPoint(x: 277.20, y: 591.10),
            control2: CGPoint(x: 287.80, y: 590.60),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 323.00, y: 576.00),
            control1: CGPoint(x: 299.70, y: 588.40),
            control2: CGPoint(x: 313.30, y: 582.50),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 349.00, y: 550.20),
            control1: CGPoint(x: 330.90, y: 570.70),
            control2: CGPoint(x: 345.80, y: 556.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 352.10, y: 546.00),
            control1: CGPoint(x: 350.30, y: 547.90),
            control2: CGPoint(x: 351.70, y: 546.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 353.60, y: 543.70),
            control1: CGPoint(x: 352.50, y: 546.00),
            control2: CGPoint(x: 353.20, y: 545.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 357.10, y: 535.60),
            control1: CGPoint(x: 353.90, y: 542.50),
            control2: CGPoint(x: 355.50, y: 538.90),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 360.00, y: 521.80),
            control1: CGPoint(x: 359.60, y: 530.70),
            control2: CGPoint(x: 360.00, y: 528.60),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 356.10, y: 508.00),
            control1: CGPoint(x: 360.00, y: 514.20),
            control2: CGPoint(x: 358.20, y: 508.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 354.50, y: 505.00),
            control1: CGPoint(x: 355.60, y: 508.00),
            control2: CGPoint(x: 354.90, y: 506.70),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 351.50, y: 498.60),
            control1: CGPoint(x: 354.10, y: 503.40),
            control2: CGPoint(x: 352.80, y: 500.50),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 347.00, y: 490.80),
            control1: CGPoint(x: 350.20, y: 496.70),
            control2: CGPoint(x: 348.20, y: 493.20),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 341.10, y: 479.50),
            control1: CGPoint(x: 345.80, y: 488.40),
            control2: CGPoint(x: 343.20, y: 483.30),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 334.10, y: 466.50),
            control1: CGPoint(x: 339.00, y: 475.60),
            control2: CGPoint(x: 335.90, y: 469.80),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 328.10, y: 455.50),
            control1: CGPoint(x: 332.40, y: 463.20),
            control2: CGPoint(x: 329.70, y: 458.20),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 323.10, y: 446.50),
            control1: CGPoint(x: 326.50, y: 452.70),
            control2: CGPoint(x: 324.30, y: 448.70),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 317.60, y: 436.50),
            control1: CGPoint(x: 321.90, y: 444.30),
            control2: CGPoint(x: 319.50, y: 439.80),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 311.60, y: 425.50),
            control1: CGPoint(x: 315.80, y: 433.20),
            control2: CGPoint(x: 313.10, y: 428.20),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 305.60, y: 414.50),
            control1: CGPoint(x: 310.20, y: 422.70),
            control2: CGPoint(x: 307.50, y: 417.80),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 299.10, y: 402.50),
            control1: CGPoint(x: 303.80, y: 411.20),
            control2: CGPoint(x: 300.80, y: 405.80),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 293.00, y: 391.50),
            control1: CGPoint(x: 297.30, y: 399.20),
            control2: CGPoint(x: 294.60, y: 394.20),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 288.00, y: 382.00),
            control1: CGPoint(x: 291.40, y: 388.70),
            control2: CGPoint(x: 289.10, y: 384.50),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 284.60, y: 376.00),
            control1: CGPoint(x: 286.80, y: 379.50),
            control2: CGPoint(x: 285.30, y: 376.80),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 282.20, y: 371.70),
            control1: CGPoint(x: 283.90, y: 375.20),
            control2: CGPoint(x: 282.90, y: 373.30),
            transform: transform
        )
        path.addLine(to: CGPoint(x: 281.00, y: 369.00))
        path.addLine(to: CGPoint(x: 273.00, y: 361.00))
        path.closeSubpath()
        path.move(to: CGPoint(x: 269.80, y: 386.30))
        path.addCurve(
            to: CGPoint(x: 284.70, y: 413.50),
            control1: CGPoint(x: 271.10, y: 388.60),
            control2: CGPoint(x: 277.80, y: 400.80),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 306.00, y: 452.50),
            control1: CGPoint(x: 291.60, y: 426.10),
            control2: CGPoint(x: 301.20, y: 443.70),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 339.00, y: 513.40),
            control1: CGPoint(x: 331.60, y: 499.20),
            control2: CGPoint(x: 339.00, y: 512.90),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 268.00, y: 514.00),
            control1: CGPoint(x: 339.00, y: 513.70),
            control2: CGPoint(x: 307.00, y: 514.00),
            transform: transform
        )
        path.addLine(to: CGPoint(x: 197.00, y: 514.00))
        path.addLine(to: CGPoint(x: 198.20, y: 511.80))
        path.addCurve(
            to: CGPoint(x: 200.20, y: 508.60),
            control1: CGPoint(x: 198.90, y: 510.60),
            control2: CGPoint(x: 199.80, y: 509.10),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 214.00, y: 481.50),
            control1: CGPoint(x: 200.60, y: 508.00),
            control2: CGPoint(x: 206.80, y: 495.80),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 227.80, y: 455.00),
            control1: CGPoint(x: 221.20, y: 467.30),
            control2: CGPoint(x: 227.40, y: 455.30),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 232.70, y: 445.80),
            control1: CGPoint(x: 228.20, y: 454.70),
            control2: CGPoint(x: 230.50, y: 450.60),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 238.00, y: 435.80),
            control1: CGPoint(x: 235.00, y: 441.00),
            control2: CGPoint(x: 237.40, y: 436.50),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 246.10, y: 420.50),
            control1: CGPoint(x: 238.50, y: 435.10),
            control2: CGPoint(x: 242.20, y: 428.20),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 254.00, y: 406.00),
            control1: CGPoint(x: 250.00, y: 412.80),
            control2: CGPoint(x: 253.60, y: 406.30),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 259.00, y: 396.60),
            control1: CGPoint(x: 254.40, y: 405.70),
            control2: CGPoint(x: 256.60, y: 401.50),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 264.60, y: 386.20),
            control1: CGPoint(x: 261.30, y: 391.60),
            control2: CGPoint(x: 263.90, y: 387.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 266.00, y: 383.40),
            control1: CGPoint(x: 265.40, y: 385.50),
            control2: CGPoint(x: 266.00, y: 384.20),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 269.80, y: 386.30),
            control1: CGPoint(x: 266.00, y: 380.90),
            control2: CGPoint(x: 267.50, y: 381.90),
            transform: transform
        )

        path.move(to: CGPoint(x: 341.00, y: 529.90))
        path.addCurve(
            to: CGPoint(x: 325.10, y: 552.10),
            control1: CGPoint(x: 341.00, y: 531.90),
            control2: CGPoint(x: 330.30, y: 546.80),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 298.20, y: 570.00),
            control1: CGPoint(x: 318.20, y: 559.00),
            control2: CGPoint(x: 306.00, y: 567.10),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 268.50, y: 575.40),
            control1: CGPoint(x: 286.30, y: 574.50),
            control2: CGPoint(x: 280.90, y: 575.50),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 248.00, y: 572.50),
            control1: CGPoint(x: 258.30, y: 575.30),
            control2: CGPoint(x: 255.20, y: 574.90),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 220.50, y: 558.50),
            control1: CGPoint(x: 237.80, y: 569.30),
            control2: CGPoint(x: 228.60, y: 564.60),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 206.80, y: 545.40),
            control1: CGPoint(x: 211.70, y: 552.00),
            control2: CGPoint(x: 210.00, y: 550.40),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 202.90, y: 541.00),
            control1: CGPoint(x: 205.10, y: 543.00),
            control2: CGPoint(x: 203.40, y: 541.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 202.00, y: 540.00),
            control1: CGPoint(x: 202.40, y: 541.00),
            control2: CGPoint(x: 202.00, y: 540.60),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 199.50, y: 535.40),
            control1: CGPoint(x: 202.00, y: 539.50),
            control2: CGPoint(x: 200.90, y: 537.40),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 197.00, y: 530.40),
            control1: CGPoint(x: 198.10, y: 533.40),
            control2: CGPoint(x: 197.00, y: 531.10),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 269.00, y: 529.00),
            control1: CGPoint(x: 197.00, y: 529.20),
            control2: CGPoint(x: 208.90, y: 529.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 341.00, y: 529.90),
            control1: CGPoint(x: 309.30, y: 529.00),
            control2: CGPoint(x: 341.00, y: 529.40),
            transform: transform
        )

        return Path(path)
    }
}

public struct BalanceScaleCoinShape: Shape {
    public func path(in _: CGRect) -> Path {
        let path = CGMutablePath()
        let transform = CGAffineTransform.identity

        path.move(to: CGPoint(x: 630.00, y: 361.00))
        path.addLine(to: CGPoint(x: 630.40, y: 377.20))
        path.addCurve(
            to: CGPoint(x: 678.00, y: 459.70),
            control1: CGPoint(x: 633.00, y: 408.40),
            control2: CGPoint(x: 651.00, y: 439.60),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 705.00, y: 473.90),
            control1: CGPoint(x: 684.70, y: 464.70),
            control2: CGPoint(x: 698.90, y: 472.20),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 711.50, y: 476.30),
            control1: CGPoint(x: 707.50, y: 474.60),
            control2: CGPoint(x: 710.40, y: 475.70),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 717.50, y: 478.10),
            control1: CGPoint(x: 712.60, y: 476.90),
            control2: CGPoint(x: 715.30, y: 477.70),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 726.00, y: 480.00),
            control1: CGPoint(x: 719.70, y: 478.50),
            control2: CGPoint(x: 723.50, y: 479.40),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 767.50, y: 480.30),
            control1: CGPoint(x: 731.80, y: 481.50),
            control2: CGPoint(x: 758.40, y: 481.70),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 807.40, y: 466.20),
            control1: CGPoint(x: 779.80, y: 478.30),
            control2: CGPoint(x: 792.50, y: 473.80),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 847.20, y: 428.50),
            control1: CGPoint(x: 820.70, y: 459.30),
            control2: CGPoint(x: 837.40, y: 443.50),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 854.50, y: 417.50),
            control1: CGPoint(x: 850.40, y: 423.50),
            control2: CGPoint(x: 853.70, y: 418.60),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 856.00, y: 414.50),
            control1: CGPoint(x: 855.30, y: 416.50),
            control2: CGPoint(x: 856.00, y: 415.10),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 858.00, y: 409.70),
            control1: CGPoint(x: 856.00, y: 413.90),
            control2: CGPoint(x: 856.90, y: 411.80),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 860.00, y: 404.30),
            control1: CGPoint(x: 859.10, y: 407.70),
            control2: CGPoint(x: 860.00, y: 405.20),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 861.90, y: 398.60),
            control1: CGPoint(x: 860.00, y: 403.40),
            control2: CGPoint(x: 860.80, y: 400.80),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 864.90, y: 388.00),
            control1: CGPoint(x: 862.90, y: 396.30),
            control2: CGPoint(x: 864.20, y: 391.60),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 867.00, y: 376.00),
            control1: CGPoint(x: 865.50, y: 384.40),
            control2: CGPoint(x: 866.50, y: 379.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 865.50, y: 338.50),
            control1: CGPoint(x: 868.30, y: 368.60),
            control2: CGPoint(x: 867.40, y: 345.90),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 863.00, y: 328.50),
            control1: CGPoint(x: 864.60, y: 335.20),
            control2: CGPoint(x: 863.50, y: 330.70),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 853.30, y: 305.60),
            control1: CGPoint(x: 861.80, y: 323.10),
            control2: CGPoint(x: 855.50, y: 308.20),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 849.60, y: 299.80),
            control1: CGPoint(x: 852.30, y: 304.40),
            control2: CGPoint(x: 850.60, y: 301.80),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 846.10, y: 294.30),
            control1: CGPoint(x: 848.60, y: 297.80),
            control2: CGPoint(x: 847.00, y: 295.30),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 841.70, y: 288.40),
            control1: CGPoint(x: 845.20, y: 293.30),
            control2: CGPoint(x: 843.20, y: 290.60),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 822.20, y: 269.00),
            control1: CGPoint(x: 838.80, y: 284.10),
            control2: CGPoint(x: 823.60, y: 269.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 818.00, y: 266.00),
            control1: CGPoint(x: 821.70, y: 269.00),
            control2: CGPoint(x: 819.90, y: 267.70),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 792.00, y: 252.10),
            control1: CGPoint(x: 814.30, y: 262.70),
            control2: CGPoint(x: 800.60, y: 255.40),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 761.10, y: 245.10),
            control1: CGPoint(x: 783.10, y: 248.70),
            control2: CGPoint(x: 772.20, y: 246.20),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 730.50, y: 245.50),
            control1: CGPoint(x: 748.40, y: 243.80),
            control2: CGPoint(x: 742.10, y: 243.80),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 686.70, y: 260.70),
            control1: CGPoint(x: 713.60, y: 247.80),
            control2: CGPoint(x: 700.60, y: 252.40),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 630.80, y: 345.30),
            control1: CGPoint(x: 656.60, y: 278.80),
            control2: CGPoint(x: 635.80, y: 310.30),
            transform: transform
        )
        path.addLine(to: CGPoint(x: 629.70, y: 353.00))
        path.addLine(to: CGPoint(x: 630.00, y: 361.00))
        path.closeSubpath()
        path.move(to: CGPoint(x: 760.00, y: 261.90))
        path.addCurve(
            to: CGPoint(x: 792.50, y: 272.30),
            control1: CGPoint(x: 774.60, y: 264.00),
            control2: CGPoint(x: 777.10, y: 264.90),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 819.00, y: 291.00),
            control1: CGPoint(x: 804.60, y: 278.20),
            control2: CGPoint(x: 809.00, y: 281.20),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 847.70, y: 339.90),
            control1: CGPoint(x: 833.70, y: 305.30),
            control2: CGPoint(x: 842.60, y: 320.50),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 848.60, y: 381.80),
            control1: CGPoint(x: 850.80, y: 351.80),
            control2: CGPoint(x: 851.20, y: 370.20),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 835.40, y: 415.20),
            control1: CGPoint(x: 846.20, y: 392.50),
            control2: CGPoint(x: 840.90, y: 405.90),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 804.50, y: 447.50),
            control1: CGPoint(x: 829.60, y: 424.80),
            control2: CGPoint(x: 813.60, y: 441.60),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 703.90, y: 454.00),
            control1: CGPoint(x: 772.50, y: 468.30),
            control2: CGPoint(x: 736.90, y: 470.60),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 648.60, y: 380.00),
            control1: CGPoint(x: 674.70, y: 439.20),
            control2: CGPoint(x: 654.40, y: 412.20),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 649.10, y: 342.50),
            control1: CGPoint(x: 646.30, y: 367.40),
            control2: CGPoint(x: 646.60, y: 345.20),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 650.00, y: 338.80),
            control1: CGPoint(x: 649.60, y: 342.00),
            control2: CGPoint(x: 650.00, y: 340.30),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 659.10, y: 314.10),
            control1: CGPoint(x: 650.00, y: 334.70),
            control2: CGPoint(x: 654.10, y: 323.60),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 681.80, y: 285.60),
            control1: CGPoint(x: 664.20, y: 304.60),
            control2: CGPoint(x: 673.70, y: 292.60),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 702.40, y: 272.00),
            control1: CGPoint(x: 688.80, y: 279.70),
            control2: CGPoint(x: 700.30, y: 272.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 705.20, y: 270.70),
            control1: CGPoint(x: 703.20, y: 272.00),
            control2: CGPoint(x: 704.50, y: 271.40),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 730.00, y: 262.90),
            control1: CGPoint(x: 707.50, y: 268.40),
            control2: CGPoint(x: 719.20, y: 264.70),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 739.00, y: 260.80),
            control1: CGPoint(x: 734.10, y: 262.20),
            control2: CGPoint(x: 738.20, y: 261.30),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 760.00, y: 261.90),
            control1: CGPoint(x: 740.60, y: 259.90),
            control2: CGPoint(x: 749.20, y: 260.40),
            transform: transform
        )

        return Path(path)
    }
}

public struct BalanceScaleBtcShape: Shape {
    public func path(in _: CGRect) -> Path {
        let path = CGMutablePath()
        let transform = CGAffineTransform.identity
        path.move(to: CGPoint(x: 750.00, y: 288.00))
        path.addCurve(
            to: CGPoint(x: 748.00, y: 298.50),
            control1: CGPoint(x: 748.30, y: 289.70),
            control2: CGPoint(x: 748.00, y: 291.30),
            transform: transform
        )
        path.addLine(to: CGPoint(x: 748.00, y: 307.00))
        path.addLine(to: CGPoint(x: 745.00, y: 307.00))
        path.addLine(to: CGPoint(x: 742.00, y: 307.00))
        path.addLine(to: CGPoint(x: 742.00, y: 298.60))
        path.addCurve(
            to: CGPoint(x: 733.00, y: 287.00),
            control1: CGPoint(x: 742.00, y: 288.10),
            control2: CGPoint(x: 741.20, y: 287.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 724.00, y: 298.60),
            control1: CGPoint(x: 724.80, y: 287.00),
            control2: CGPoint(x: 724.00, y: 288.10),
            transform: transform
        )
        path.addLine(to: CGPoint(x: 724.00, y: 307.00))
        path.addLine(to: CGPoint(x: 713.90, y: 307.00))
        path.addCurve(
            to: CGPoint(x: 702.00, y: 308.00),
            control1: CGPoint(x: 708.40, y: 307.00),
            control2: CGPoint(x: 703.00, y: 307.40),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 699.60, y: 322.50),
            control1: CGPoint(x: 698.40, y: 310.00),
            control2: CGPoint(x: 696.80, y: 318.90),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 709.30, y: 325.00),
            control1: CGPoint(x: 700.80, y: 324.10),
            control2: CGPoint(x: 702.60, y: 324.60),
            transform: transform
        )
        path.addLine(to: CGPoint(x: 717.50, y: 325.50))
        path.addLine(to: CGPoint(x: 717.80, y: 364.20))
        path.addLine(to: CGPoint(x: 718.00, y: 403.00))
        path.addLine(to: CGPoint(x: 710.60, y: 403.00))
        path.addCurve(
            to: CGPoint(x: 700.00, y: 412.00),
            control1: CGPoint(x: 701.20, y: 403.00),
            control2: CGPoint(x: 700.00, y: 404.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 713.60, y: 421.00),
            control1: CGPoint(x: 700.00, y: 420.40),
            control2: CGPoint(x: 700.80, y: 421.00),
            transform: transform
        )
        path.addLine(to: CGPoint(x: 724.00, y: 421.00))
        path.addLine(to: CGPoint(x: 724.00, y: 429.90))
        path.addCurve(
            to: CGPoint(x: 732.30, y: 442.00),
            control1: CGPoint(x: 724.00, y: 440.80),
            control2: CGPoint(x: 724.80, y: 442.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 738.60, y: 441.20),
            control1: CGPoint(x: 735.20, y: 442.00),
            control2: CGPoint(x: 738.00, y: 441.60),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 742.00, y: 428.60),
            control1: CGPoint(x: 741.00, y: 439.70),
            control2: CGPoint(x: 742.00, y: 435.80),
            transform: transform
        )
        path.addLine(to: CGPoint(x: 742.00, y: 421.00))
        path.addLine(to: CGPoint(x: 745.00, y: 421.00))
        path.addLine(to: CGPoint(x: 748.00, y: 421.00))
        path.addLine(to: CGPoint(x: 748.00, y: 429.40))
        path.addCurve(
            to: CGPoint(x: 750.20, y: 439.40),
            control1: CGPoint(x: 748.00, y: 437.00),
            control2: CGPoint(x: 748.20, y: 438.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 757.70, y: 441.00),
            control1: CGPoint(x: 751.60, y: 440.40),
            control2: CGPoint(x: 754.60, y: 441.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 767.00, y: 429.10),
            control1: CGPoint(x: 765.60, y: 441.00),
            control2: CGPoint(x: 767.00, y: 439.10),
            transform: transform
        )
        path.addLine(to: CGPoint(x: 767.00, y: 421.10))
        path.addLine(to: CGPoint(x: 772.80, y: 420.00))
        path.addCurve(
            to: CGPoint(x: 789.50, y: 413.90),
            control1: CGPoint(x: 780.90, y: 418.40),
            control2: CGPoint(x: 783.80, y: 417.30),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 799.50, y: 398.70),
            control1: CGPoint(x: 794.60, y: 410.80),
            control2: CGPoint(x: 797.70, y: 406.10),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 801.10, y: 395.40),
            control1: CGPoint(x: 799.90, y: 397.20),
            control2: CGPoint(x: 800.60, y: 395.70),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 801.00, y: 379.50),
            control1: CGPoint(x: 802.30, y: 394.70),
            control2: CGPoint(x: 802.20, y: 380.30),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 800.00, y: 376.40),
            control1: CGPoint(x: 800.50, y: 379.20),
            control2: CGPoint(x: 800.00, y: 377.80),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 792.40, y: 363.00),
            control1: CGPoint(x: 800.00, y: 372.90),
            control2: CGPoint(x: 797.50, y: 368.40),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 788.60, y: 356.60),
            control1: CGPoint(x: 789.40, y: 359.80),
            control2: CGPoint(x: 788.20, y: 357.70),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 790.10, y: 355.00),
            control1: CGPoint(x: 788.90, y: 355.70),
            control2: CGPoint(x: 789.60, y: 355.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 792.40, y: 325.40),
            control1: CGPoint(x: 793.10, y: 355.00),
            control2: CGPoint(x: 794.90, y: 331.50),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 784.00, y: 314.30),
            control1: CGPoint(x: 790.90, y: 321.90),
            control2: CGPoint(x: 788.30, y: 318.50),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 769.00, y: 308.00),
            control1: CGPoint(x: 781.40, y: 311.80),
            control2: CGPoint(x: 772.30, y: 308.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 767.00, y: 299.20),
            control1: CGPoint(x: 767.20, y: 308.00),
            control2: CGPoint(x: 767.00, y: 307.30),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 766.20, y: 289.40),
            control1: CGPoint(x: 767.00, y: 294.40),
            control2: CGPoint(x: 766.60, y: 290.00),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 750.00, y: 288.00),
            control1: CGPoint(x: 763.70, y: 285.60),
            control2: CGPoint(x: 753.30, y: 284.70),
            transform: transform
        )
        path.move(to: CGPoint(x: 766.60, y: 328.00))
        path.addCurve(
            to: CGPoint(x: 773.00, y: 334.40),
            control1: CGPoint(x: 769.50, y: 329.40),
            control2: CGPoint(x: 771.40, y: 331.40),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 771.90, y: 346.40),
            control1: CGPoint(x: 775.60, y: 339.20),
            control2: CGPoint(x: 775.30, y: 342.10),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 749.50, y: 352.40),
            control1: CGPoint(x: 768.00, y: 351.40),
            control2: CGPoint(x: 764.80, y: 352.30),
            transform: transform
        )
        path.addLine(to: CGPoint(x: 735.50, y: 352.50))
        path.addLine(to: CGPoint(x: 735.20, y: 339.20))
        path.addLine(to: CGPoint(x: 734.90, y: 326.00))
        path.addLine(to: CGPoint(x: 748.70, y: 326.00))
        path.addCurve(
            to: CGPoint(x: 766.60, y: 328.00),
            control1: CGPoint(x: 760.40, y: 326.00),
            control2: CGPoint(x: 763.00, y: 326.30),
            transform: transform
        )
        path.move(to: CGPoint(x: 776.50, y: 376.00))
        path.addCurve(
            to: CGPoint(x: 780.00, y: 395.00),
            control1: CGPoint(x: 783.60, y: 382.80),
            control2: CGPoint(x: 784.60, y: 388.50),
            transform: transform
        )
        path.addCurve(
            to: CGPoint(x: 752.30, y: 402.80),
            control1: CGPoint(x: 775.60, y: 401.20),
            control2: CGPoint(x: 771.40, y: 402.30),
            transform: transform
        )
        path.addLine(to: CGPoint(x: 735.00, y: 403.20))
        path.addLine(to: CGPoint(x: 735.00, y: 387.50))
        path.addLine(to: CGPoint(x: 735.00, y: 371.80))
        path.addLine(to: CGPoint(x: 753.90, y: 372.20))
        path.addLine(to: CGPoint(x: 772.90, y: 372.50))
        path.addLine(to: CGPoint(x: 776.50, y: 376.00))

        return Path(path)
    }
}
