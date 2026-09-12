//  Pure functional kinematics engine for the unified balance scale launch animation.
//  Encapsulates time-to-angle mapping, shimmer sweep calculation, and harmonic damping.
//

import Foundation

public struct BalanceScaleKinematics: Sendable {
    public enum Stage: Equatable, Sendable {
        case resting
        case shimmer(progress: Double)
        case oscillating(angle: Double)
        case settling(angle: Double)
        case balanced
    }

    public let shimmerDelay: Double
    public let shimmerDuration: Double
    public let oscillationPeriod: Double
    public let maxAngleDegrees: Double
    public let settleDuration: Double

    public init(
        shimmerDelay: Double = 0.25,
        shimmerDuration: Double = 0.90,
        oscillationPeriod: Double = 2.0,
        maxAngleDegrees: Double = 4.8,
        settleDuration: Double = 1.2
    ) {
        self.shimmerDelay = shimmerDelay
        self.shimmerDuration = shimmerDuration
        self.oscillationPeriod = oscillationPeriod
        self.maxAngleDegrees = maxAngleDegrees
        self.settleDuration = settleDuration
    }

    public func evaluate(
        elapsedSinceStart: Double,
        isSyncComplete: Bool,
        settleElapsed: Double?
    ) -> Stage {
        // Stage 4: Settling requested upon sync completion
        if isSyncComplete, let settleElapsed {
            if settleElapsed >= settleDuration {
                return .balanced
            }
            let progress = max(0.0, min(1.0, settleElapsed / settleDuration))
            let envelope = exp(-3.2 * progress)
            let oscillation = cos(3.5 * 2.0 * .pi * progress)
            let linearFade = 1.0 - progress
            let angle = maxAngleDegrees * envelope * oscillation * linearFade
            return .settling(angle: angle)
        }

        // Stage 1: Initial resting display
        if elapsedSinceStart < shimmerDelay {
            return .resting
        }

        // Stage 2: Single luminous wake-up shimmer sweep
        let shimmerElapsed = elapsedSinceStart - shimmerDelay
        if shimmerElapsed < shimmerDuration {
            let progress = shimmerElapsed / shimmerDuration
            return .shimmer(progress: progress)
        }

        // Stage 3: Continuous harmonic balance oscillation
        let oscillationElapsed = shimmerElapsed - shimmerDuration
        let cycle = oscillationPeriod > 0
            ? (oscillationElapsed / oscillationPeriod).truncatingRemainder(dividingBy: 1.0)
            : 0.0
        let angle = maxAngleDegrees * sin(cycle * 2.0 * .pi)
        return .oscillating(angle: angle)
    }

    public static func shimmerSweepRange(progress: Double) -> (startNorm: Double, endNorm: Double) {
        // Normalizes sweep coordinate from top-left (-0.4) to bottom-right (1.4)
        let sweep = progress * 1.8 - 0.4
        let bandWidth = 0.28
        return (sweep - bandWidth, sweep + bandWidth)
    }
}
