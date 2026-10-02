import Observation
import SwiftUI

@Observable
final class BalanceBarAnimationCoordinator {
    var isAwakening: Bool = false
    var thumbAwakenScale: CGFloat = 1.0
    var radialFloodScale: CGFloat = 0.01
    var radialFloodOpacity: Double = 0.0
    var settleFraction: CGFloat?

    /// Generation token to prevent racing callbacks if awakening is triggered rapidly.
    private var awakeningGeneration: Int = 0

    /// Initial balanced position (50% USD / 50% BTC) where the Awakening surge starts before settling.
    private let awakeningInitialFraction: CGFloat = 0.5

    func triggerAwakening(targetFraction: CGFloat) {
        awakeningGeneration += 1
        let currentGen = awakeningGeneration

        isAwakening = true
        thumbAwakenScale = 1.0
        radialFloodScale = 0.01
        radialFloodOpacity = 0.0
        settleFraction = awakeningInitialFraction

        withAnimation(.spring(response: 0.22, dampingFraction: 0.45)) {
            self.thumbAwakenScale = 1.35
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.18) { [weak self] in
            guard let self, self.awakeningGeneration == currentGen else { return }
            withAnimation(.spring(response: 0.45, dampingFraction: 0.65)) {
                self.radialFloodScale = 1.0
                self.radialFloodOpacity = 0.55
            }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.42) { [weak self] in
            guard let self, self.awakeningGeneration == currentGen else { return }
            withAnimation(.easeInOut(duration: 0.4)) {
                self.radialFloodOpacity = 0.0
                self.thumbAwakenScale = 1.0
            }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.72) { [weak self] in
            guard let self, self.awakeningGeneration == currentGen else { return }
            withAnimation(.spring(response: 0.65, dampingFraction: 0.78)) {
                self.settleFraction = targetFraction
            }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.4) { [weak self] in
            guard let self, self.awakeningGeneration == currentGen else { return }
            self.isAwakening = false
            self.settleFraction = nil
        }
    }
}
