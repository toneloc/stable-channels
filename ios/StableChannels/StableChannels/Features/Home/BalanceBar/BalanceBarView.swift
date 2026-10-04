import SwiftUI

struct BalanceBarView: View {
    let allocation: ChannelAllocation
    var maxSellUSD: Double = 0
    var isTrading: Bool = false
    var onDragStarted: (() -> Void)?
    var onTradeRequest: ((TradeRequest) -> Void)?
    var onEmptyInteraction: (() -> Void)?

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    @State private var state = BalanceBarState()
    @State private var animator = BalanceBarAnimationCoordinator()
    @State private var pulseScale: CGFloat = 1.0

    static let defaultThumbDiameter: CGFloat = 22.0
    private let thumbDiameter: CGFloat = Self.defaultThumbDiameter
    private let barHeight: CGFloat = 20
    private let headerHeight: CGFloat = 34
    private let verticalSpacing: CGFloat = 6

    private var isPriceReady: Bool {
        allocation.btcPrice > 0
    }

    private var interactive: Bool {
        if allocation.isEmpty {
            return onEmptyInteraction != nil
        } else {
            return isPriceReady && onTradeRequest != nil
        }
    }

    private var totalHeight: CGFloat {
        interactive ? (headerHeight + verticalSpacing + thumbDiameter) : barHeight
    }

    private var currentBarHeight: CGFloat {
        barHeight
    }

    private var visibleFraction: CGFloat {
        state.effectiveFraction(allocation: allocation, settleFraction: animator.settleFraction)
    }

    var body: some View {
        GeometryReader { geometry in
            let barWidth = geometry.size.width
            let visFrac = visibleFraction
            let thumbX = thumbPosition(barWidth: barWidth, visFrac: visFrac)

            VStack(spacing: verticalSpacing) {
                if interactive {
                    BalanceBarHeaderView(
                        visFrac: visFrac,
                        isPressing: state.isPressing,
                        hasSelectedFraction: state.userSelectedFraction != nil,
                        isAwakening: animator.isAwakening,
                        atSellLimit: state.atSellLimit,
                        maxSellUSD: maxSellUSD,
                        showDepositPrompt: state.showDepositPrompt,
                        onEmptyInteraction: onEmptyInteraction
                    )
                    .frame(height: headerHeight)
                }

                ZStack {
                    BalanceBarTrackView(
                        barWidth: barWidth,
                        height: currentBarHeight,
                        fraction: visFrac,
                        thumbDiameter: thumbDiameter,
                        isEmpty: allocation.isEmpty,
                        isAwakening: animator.isAwakening,
                        floodScale: animator.radialFloodScale,
                        floodOpacity: animator.radialFloodOpacity
                    )

                    if interactive {
                        thumbView(thumbX: thumbX)
                    }
                }
                .frame(width: barWidth, height: interactive ? thumbDiameter : barHeight)
                .contentShape(Rectangle())
                .gesture(
                    DragGesture(minimumDistance: 0)
                        .onChanged { gesture in
                            guard interactive else { return }
                            state.handleDragChange(
                                touchStartX: gesture.startLocation.x,
                                translationX: gesture.translation.width,
                                barWidth: barWidth,
                                currentThumbX: thumbX,
                                thumbDiameter: thumbDiameter,
                                allocation: allocation,
                                maxSellUSD: maxSellUSD,
                                isAwakening: animator.isAwakening,
                                onDragStarted: onDragStarted
                            )
                        }
                        .onEnded { gesture in
                            state.acknowledgeGestureEnd()
                            guard interactive else { return }
                            state.handleDragEnd(
                                translationX: gesture.translation.width,
                                barWidth: barWidth,
                                allocation: allocation,
                                maxSellUSD: maxSellUSD,
                                isAwakening: animator.isAwakening,
                                reduceMotion: reduceMotion,
                                onEmptyInteraction: onEmptyInteraction,
                                onTradeRequest: onTradeRequest
                            )
                        }
                )
            }
            .animation(.spring(response: 0.45, dampingFraction: 0.75), value: allocation.isEmpty)
            .onChange(of: allocation.isEmpty) { wasEmpty, isEmpty in
                if wasEmpty && !isEmpty {
                    state.resetSelection()
                    if !reduceMotion {
                        animator.triggerAwakening(targetFraction: CGFloat(allocation.stableFraction))
                    }
                }
            }
            .onChange(of: isTrading) { wasTrading, isTrading in
                if wasTrading && !isTrading {
                    state.resetSelection()
                }
            }
            .onChange(of: allocation.stableFraction) { _, _ in
                guard !isTrading, !state.isPressing else { return }
                state.resetSelection()
            }
            .onDisappear {
                animator.cancel()
                state.cancelTimers()
            }
        }
        .frame(height: totalHeight)
    }

    // MARK: - Subviews

    private func thumbView(thumbX: CGFloat) -> some View {
        Circle()
            .fill(.white)
            .frame(width: thumbDiameter, height: thumbDiameter)
            .shadow(color: .black.opacity(0.2), radius: 4, y: 2)
            .overlay(
                Circle()
                    .strokeBorder(Color.primary.opacity(0.08), lineWidth: 1)
            )
            .scaleEffect(animator.isAwakening ? animator
                .thumbAwakenScale : (state.isPressing ? 1.15 : (allocation.isEmpty ? 1.0 : pulseScale)))
            .position(x: thumbX, y: thumbDiameter / 2)
            .animation(.easeOut(duration: 0.15), value: state.isPressing)
            .onAppear {
                guard !reduceMotion else { return }
                withAnimation(.easeInOut(duration: 1.5).repeatForever(autoreverses: true)) { pulseScale = 1.08 }
            }
            .onChange(of: reduceMotion) { _, newValue in
                if newValue {
                    pulseScale = 1.0
                } else if !state.isPressing {
                    withAnimation(.easeInOut(duration: 1.5).repeatForever(autoreverses: true)) {
                        pulseScale = 1.08
                    }
                }
            }
    }

    // MARK: - Layout Calculations

    private func thumbPosition(barWidth: CGFloat, visFrac: CGFloat) -> CGFloat {
        BalanceBarTradeCalculator.calculateThumbPosition(
            fraction: visFrac,
            barWidth: barWidth,
            thumbDiameter: thumbDiameter
        )
    }
}
