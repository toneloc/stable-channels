import Foundation

/// Pure interaction model for the balance bar slider.
/// Defines mathematical coordinate translation and touch hit-testing without UI side effects.
enum BalanceBarInteraction {
    static let defaultTapThreshold: CGFloat = 5.0
    static let defaultThumbHitMultiplier: CGFloat = 1.5

    /// Mathematical formula unifying coordinate translation across platforms:
    /// fraction = clamp(initialFraction + translation / barWidth, 0.0, 1.0)
    static func calculateTargetFraction(
        initialFraction: CGFloat,
        translationX: CGFloat,
        barWidth: CGFloat
    ) -> CGFloat {
        guard barWidth > 0 else { return initialFraction }
        let proposed = initialFraction + (translationX / barWidth)
        return min(max(proposed, 0.0), 1.0)
    }

    /// Computes horizontal thumb center position along the track within bounds.
    static func calculateThumbPosition(
        fraction: CGFloat,
        barWidth: CGFloat,
        thumbDiameter: CGFloat
    ) -> CGFloat {
        guard barWidth > 0 else { return thumbDiameter / 2 }
        let clamped = min(max(fraction, 0.0), 1.0)
        return thumbDiameter / 2 + (barWidth - thumbDiameter) * clamped
    }

    /// Evaluates if gesture displacement qualifies as a tap rather than a drag.
    static func isTap(
        translationX: CGFloat,
        translationY: CGFloat = 0.0,
        threshold: CGFloat = defaultTapThreshold
    ) -> Bool {
        hypot(translationX, translationY) < threshold
    }

    /// Determines if an initial touch falls within the interactive hit area of the thumb.
    static func isWithinThumb(
        touchX: CGFloat,
        thumbX: CGFloat,
        thumbDiameter: CGFloat,
        multiplier: CGFloat = defaultThumbHitMultiplier
    ) -> Bool {
        abs(touchX - thumbX) < thumbDiameter * multiplier
    }
}
