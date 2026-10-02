import Foundation
import Observation
import SwiftUI

/// Pure mathematical transforms deriving visual presentation properties from normalized animation progress (0.0 to
/// 1.0).
enum BalanceBarAnimationMath {
    /// Thumb scales up during surge, then returns to resting scale.
    static func thumbScale(progress: Double) -> CGFloat {
        if progress <= 0.0 || progress >= 0.6 { return 1.0 }
        if progress < 0.22 {
            let phase = progress / 0.22
            return 1.0 + CGFloat(phase * 0.35)
        } else {
            let phase = (progress - 0.22) / (0.6 - 0.22)
            return 1.35 - CGFloat(phase * 0.35)
        }
    }

    /// Radial flood wave expands smoothly outward.
    static func floodScale(progress: Double) -> CGFloat {
        if progress <= 0.05 { return 0.01 }
        if progress >= 0.55 { return 1.0 }
        let phase = (progress - 0.05) / 0.50
        return 0.01 + CGFloat(phase * 0.99)
    }

    /// Radial flood alpha flares up and smoothly fades away.
    static func floodOpacity(progress: Double) -> Double {
        if progress <= 0.05 || progress >= 0.65 { return 0.0 }
        if progress < 0.28 {
            let phase = (progress - 0.05) / (0.28 - 0.05)
            return phase * 0.55
        } else {
            let phase = (progress - 0.28) / (0.65 - 0.28)
            return 0.55 * (1.0 - phase)
        }
    }

    /// Settle fraction remains at center during the surge, then smoothly interpolates to target fraction.
    static func settleFraction(
        initialFraction: CGFloat = 0.5,
        targetFraction: CGFloat,
        progress: Double
    ) -> CGFloat? {
        if progress <= 0.0 { return nil }
        if progress < 0.45 { return initialFraction }
        let phase = CGFloat(min((progress - 0.45) / 0.55, 1.0))
        return initialFraction + (targetFraction - initialFraction) * phase
    }
}

/// Declarative coordinator for the Awakening animation sequence.
/// Derives all visual properties from a single normalized progress value.
@Observable
final class BalanceBarAnimationCoordinator {
    var isAwakening: Bool = false
    var progress: Double = 0.0
    var targetFraction: CGFloat = 0.5

    var thumbAwakenScale: CGFloat {
        BalanceBarAnimationMath.thumbScale(progress: progress)
    }

    var radialFloodScale: CGFloat {
        BalanceBarAnimationMath.floodScale(progress: progress)
    }

    var radialFloodOpacity: Double {
        BalanceBarAnimationMath.floodOpacity(progress: progress)
    }

    var settleFraction: CGFloat? {
        guard isAwakening else { return nil }
        return BalanceBarAnimationMath.settleFraction(
            initialFraction: 0.5,
            targetFraction: targetFraction,
            progress: progress
        )
    }

    private var animationTask: Task<Void, Never>?

    func triggerAwakening(targetFraction: CGFloat) {
        animationTask?.cancel()
        self.targetFraction = targetFraction
        self.isAwakening = true
        self.progress = 0.0

        let startTime = Date()
        let duration: TimeInterval = 1.2

        animationTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                let elapsed = Date().timeIntervalSince(startTime)
                let currentProgress = min(elapsed / duration, 1.0)
                self?.progress = currentProgress

                if currentProgress >= 1.0 {
                    self?.isAwakening = false
                    self?.progress = 0.0
                    break
                }
                try? await Task.sleep(nanoseconds: 16_666_667)
            }
        }
    }

    func cancel() {
        animationTask?.cancel()
        isAwakening = false
        progress = 0.0
    }
}
