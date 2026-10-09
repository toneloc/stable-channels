import SwiftUI

/// UI presentation state for the balance bar component.
/// Strictly coordinates UI state transitions, delegates geometry and trade evaluation
/// to BalanceBarTradeCalculator, and delegates haptic execution to an injected BalanceBarHaptics provider.
@Observable
final class BalanceBarState {
    var userSelectedFraction: CGFloat?
    var isPressing = false
    var atSellLimit = false
    var showDepositPrompt = false

    private let haptics: BalanceBarHaptics
    private var hasTriggeredTradeThresholdHaptic = false
    private var hasTriggeredSellLimitHaptic = false
    private var lastTranslationX: CGFloat = 0
    private var lastTranslationY: CGFloat = 0
    private(set) var cumulativeDragDistance: CGFloat = 0
    private var depositPromptTimer: DispatchWorkItem?
    private var dragStartBaseFraction: CGFloat?
    private var startedEmpty = false
    private var isGestureCancelled = false

    init(haptics: BalanceBarHaptics = SystemBalanceBarHaptics()) {
        self.haptics = haptics
    }

    func effectiveFraction(allocation: ChannelAllocation, settleFraction: CGFloat?) -> CGFloat {
        if let userFraction = userSelectedFraction { return userFraction }
        if let settled = settleFraction { return settled }
        return allocation.isEmpty ? 0.5 : CGFloat(allocation.stableFraction)
    }

    func handleDragChange(
        touchStartX: CGFloat,
        translationX: CGFloat,
        translationY: CGFloat = 0,
        barWidth: CGFloat,
        currentThumbX: CGFloat,
        thumbDiameter: CGFloat,
        allocation: ChannelAllocation,
        maxSellUSD: Double,
        isAwakening: Bool,
        onDragStarted: (() -> Void)?
    ) {
        guard barWidth > 0, !isAwakening, !isGestureCancelled else { return }

        if !isPressing {
            let withinThumb = BalanceBarTradeCalculator.isWithinThumb(
                touchX: touchStartX,
                thumbX: currentThumbX,
                thumbDiameter: thumbDiameter
            )
            guard allocation.isEmpty || withinThumb else { return }
            isPressing = true
            hasTriggeredTradeThresholdHaptic = false
            hasTriggeredSellLimitHaptic = false
            atSellLimit = false
            lastTranslationX = 0
            lastTranslationY = 0
            cumulativeDragDistance = 0
            dragStartBaseFraction = allocation.isEmpty ? 0.5 : CGFloat(allocation.stableFraction)
            startedEmpty = allocation.isEmpty
            depositPromptTimer?.cancel()
            showDepositPrompt = false
            onDragStarted?()
            haptics.tick()
        }
        guard isPressing else { return }

        let deltaX = translationX - lastTranslationX
        let deltaY = translationY - lastTranslationY
        cumulativeDragDistance += hypot(deltaX, deltaY)
        lastTranslationX = translationX
        lastTranslationY = translationY

        let baseFraction = dragStartBaseFraction ?? (allocation.isEmpty ? 0.5 : CGFloat(allocation.stableFraction))
        let rawFraction = BalanceBarTradeCalculator.calculateTargetFraction(
            initialFraction: baseFraction,
            translationX: translationX,
            barWidth: barWidth,
            thumbDiameter: thumbDiameter
        )

        if allocation.isEmpty || startedEmpty {
            userSelectedFraction = rawFraction
            return
        }

        let clampedResult = BalanceBarTradeCalculator.clampFraction(
            initialFraction: baseFraction,
            rawFraction: rawFraction,
            totalUSD: allocation.totalUSD,
            stableUSD: allocation.stableUSD,
            maxSellUSD: maxSellUSD
        )

        userSelectedFraction = clampedResult.fraction

        if clampedResult.isAtSellLimit {
            if !atSellLimit {
                atSellLimit = true
                triggerSellLimitHaptic()
            }
        } else {
            atSellLimit = false
        }

        if !hasTriggeredTradeThresholdHaptic {
            let evaluation = BalanceBarTradeCalculator.calculateSelection(
                initialFraction: baseFraction,
                targetFraction: clampedResult.fraction,
                totalUSD: allocation.totalUSD,
                stableUSD: allocation.stableUSD,
                maxSellUSD: maxSellUSD
            )
            if evaluation.isValidTrade {
                hasTriggeredTradeThresholdHaptic = true
                haptics.tick()
            }
        }
    }

    func handleDragEnd(
        translationX: CGFloat = 0,
        translationY: CGFloat = 0,
        barWidth: CGFloat,
        allocation: ChannelAllocation,
        maxSellUSD: Double,
        isAwakening: Bool,
        reduceMotion: Bool,
        onEmptyInteraction: (() -> Void)?,
        onTradeRequest: ((TradeRequest) -> Void)?
    ) {
        let wasCancelled = isGestureCancelled
        isGestureCancelled = false
        guard isPressing, !wasCancelled else { return }
        isPressing = false
        atSellLimit = false

        let wasStartedEmpty = startedEmpty
        let startBaseFraction = dragStartBaseFraction
        dragStartBaseFraction = nil
        startedEmpty = false
        lastTranslationX = 0
        lastTranslationY = 0

        if allocation.isEmpty || wasStartedEmpty {
            let totalDistance = max(cumulativeDragDistance, hypot(translationX, translationY))
            if BalanceBarTradeCalculator.isTap(totalDistance: totalDistance) {
                userSelectedFraction = nil
                haptics.tick()
                onEmptyInteraction?()
            } else {
                haptics.tick()
                withAnimation(reduceMotion ? .easeInOut(duration: 0.25) : .spring(
                    response: 0.38,
                    dampingFraction: 0.68
                )) {
                    userSelectedFraction = nil
                }
                showDepositPrompt = true
                depositPromptTimer?.cancel()
                let timer = DispatchWorkItem { [weak self] in
                    withAnimation(.easeInOut(duration: 0.2)) {
                        self?.showDepositPrompt = false
                    }
                }
                depositPromptTimer = timer
                DispatchQueue.main.asyncAfter(deadline: .now() + 2.5, execute: timer)
            }
            return
        }

        guard barWidth > 0, !isAwakening,
              let selected = userSelectedFraction else { return }

        let baseFraction = startBaseFraction ?? CGFloat(allocation.stableFraction)
        let evaluation = BalanceBarTradeCalculator.calculateSelection(
            initialFraction: baseFraction,
            targetFraction: selected,
            totalUSD: allocation.totalUSD,
            stableUSD: allocation.stableUSD,
            maxSellUSD: maxSellUSD
        )

        if evaluation.isValidTrade, let direction = evaluation.direction, let onTradeRequest {
            haptics.impact()
            let request = TradeRequest(direction: direction, amountUSD: evaluation.clampedUSD)
            onTradeRequest(request)
        } else {
            withAnimation(.spring(response: 0.25, dampingFraction: 0.8)) {
                userSelectedFraction = nil
            }
        }
    }

    func resetSelection() {
        if isPressing {
            isGestureCancelled = true
        }
        isPressing = false
        dragStartBaseFraction = nil
        startedEmpty = false
        showDepositPrompt = false
        depositPromptTimer?.cancel()
        lastTranslationX = 0
        lastTranslationY = 0
        cumulativeDragDistance = 0
        withAnimation(.spring(response: 0.35, dampingFraction: 0.8)) {
            userSelectedFraction = nil
        }
    }

    func acknowledgeGestureEnd() {
        isGestureCancelled = false
    }

    func cancelTimers() {
        depositPromptTimer?.cancel()
    }

    private func triggerSellLimitHaptic() {
        guard !hasTriggeredSellLimitHaptic else { return }
        hasTriggeredSellLimitHaptic = true
        haptics.warning()
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self] in
            self?.hasTriggeredSellLimitHaptic = false
        }
    }
}
