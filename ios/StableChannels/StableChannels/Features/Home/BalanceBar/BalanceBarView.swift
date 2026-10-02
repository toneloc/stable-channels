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

    private let thumbDiameter: CGFloat = 28
    private let barHeight: CGFloat = 20
    private let baseHeaderHeight: CGFloat = 24
    private let verticalSpacing: CGFloat = 6

    private var interactive: Bool {
        onTradeRequest != nil || onEmptyInteraction != nil
    }

    private var currentHeaderHeight: CGFloat {
        state.atSellLimit ? 34 : baseHeaderHeight
    }

    private var totalHeight: CGFloat {
        interactive ? (currentHeaderHeight + verticalSpacing + thumbDiameter) : 10
    }

    private var currentBarHeight: CGFloat {
        interactive ? barHeight : 10
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
                    .frame(height: currentHeaderHeight)
                    .animation(.easeInOut(duration: 0.15), value: state.atSellLimit)
                }

                ZStack {
                    BalanceBarTrackView(
                        barWidth: barWidth,
                        height: currentBarHeight,
                        fraction: visFrac,
                        isEmpty: allocation.isEmpty,
                        isAwakening: animator.isAwakening,
                        floodScale: animator.radialFloodScale,
                        floodOpacity: animator.radialFloodOpacity
                    )

                    if interactive {
                        thumbView(thumbX: thumbX)
                    }
                }
                .frame(width: barWidth, height: interactive ? thumbDiameter : 10)
                .contentShape(Rectangle())
                .gesture(
                    DragGesture(minimumDistance: 0)
                        .onChanged { gesture in
                            guard interactive else { return }
                            state.handleDragChange(
                                gesture: gesture,
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
                            guard interactive else { return }
                            state.handleDragEnd(
                                gesture: gesture,
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
                if wasEmpty && !isEmpty && !reduceMotion {
                    animator.triggerAwakening(targetFraction: CGFloat(allocation.stableFraction))
                }
            }
            .onChange(of: isTrading) { wasTrading, isTrading in
                if wasTrading && !isTrading {
                    state.resetSelection()
                }
            }
            .onChange(of: allocation.stableFraction) { _, _ in
                guard !isTrading else { return }
                state.resetSelection()
            }
            .onDisappear {
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
                withAnimation(.easeInOut(duration: 1.5).repeatForever(autoreverses: true)) { pulseScale = 1.08 }
            }
    }

    // MARK: - Layout Calculations

    private func thumbPosition(barWidth: CGFloat, visFrac: CGFloat) -> CGFloat {
        thumbDiameter / 2 + (barWidth - thumbDiameter) * visFrac
    }
}
