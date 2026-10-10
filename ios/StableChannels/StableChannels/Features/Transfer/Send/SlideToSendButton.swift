import SwiftUI
import UIKit

/// Interactive swipe-to-confirm button inspired by high-security wallet patterns.
/// Prevents accidental payment broadcasts while providing tactile feedback.
struct SlideToSendButton: View {
    let title: String
    let isSending: Bool
    var resetToken: Int = 0
    let onConfirmed: () -> Void

    @State private var dragOffset: CGFloat = 0
    @State private var hasTriggered = false

    private let handleSize: CGFloat = 50
    private let trackHeight: CGFloat = 58
    private let cornerRadius: CGFloat = 16

    var body: some View {
        GeometryReader { geometry in
            let totalWidth = geometry.size.width
            let maxDrag = max(0, totalWidth - handleSize - 8)

            ZStack(alignment: .leading) {
                // Background Track: Deep solid gradient with crisp border
                RoundedRectangle(cornerRadius: cornerRadius)
                    .fill(
                        LinearGradient(
                            colors: [Color.deepSendNavy, Color.deepSendBlue],
                            startPoint: .leading,
                            endPoint: .trailing
                        )
                    )
                    .frame(height: trackHeight)
                    .overlay(
                        RoundedRectangle(cornerRadius: cornerRadius)
                            .stroke(Color.white.opacity(0.18), lineWidth: 1)
                    )

                // Track Progress Fill
                RoundedRectangle(cornerRadius: cornerRadius)
                    .fill(Color.white.opacity(0.15))
                    .frame(width: max(0, dragOffset + handleSize + 4), height: trackHeight)

                // Center Label or Broadcasting Indicator
                if isSending {
                    HStack(spacing: 10) {
                        Spacer()
                        ProgressView()
                            .progressViewStyle(CircularProgressViewStyle(tint: .white))
                        Text(String(localized: "label_broadcasting", defaultValue: "Broadcasting..."))
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(.white)
                        Spacer()
                    }
                } else {
                    HStack(spacing: 6) {
                        Spacer()
                        Text(title)
                            .font(.subheadline.weight(.bold))
                            .foregroundStyle(.white)
                        Image(systemName: "chevron.right.2")
                            .font(.caption.weight(.bold))
                            .foregroundStyle(.white.opacity(0.55))
                        Spacer()
                    }
                    .opacity(max(0, 1.0 - Double(dragOffset / max(1, maxDrag * 0.75))))
                }

                // Draggable Handle
                if !isSending {
                    ZStack {
                        RoundedRectangle(cornerRadius: 13)
                            .fill(Color.white)
                            .shadow(color: Color.black.opacity(0.24), radius: 6, x: 1, y: 2)
                            .frame(width: handleSize, height: handleSize)

                        Image(systemName: "arrow.right")
                            .font(.system(size: 18, weight: .bold))
                            .foregroundStyle(Color.deepSendBlue)
                    }
                    .padding(.leading, 4)
                    .offset(x: dragOffset)
                    .gesture(
                        DragGesture()
                            .onChanged { value in
                                guard !hasTriggered, !isSending else { return }
                                let newOffset = min(max(0, value.translation.width), maxDrag)
                                let threshold = maxDrag * 0.90

                                if (newOffset >= threshold && dragOffset < threshold) ||
                                    (newOffset < threshold && dragOffset >= threshold) {
                                    UIImpactFeedbackGenerator(style: .medium).impactOccurred()
                                } else if abs(newOffset - dragOffset) > 20 {
                                    UIImpactFeedbackGenerator(style: .light).impactOccurred()
                                }
                                dragOffset = newOffset
                            }
                            .onEnded { _ in
                                guard !hasTriggered, !isSending else { return }
                                let completionThreshold = maxDrag * 0.90
                                if dragOffset >= completionThreshold {
                                    hasTriggered = true
                                    withAnimation(.easeOut(duration: 0.12)) {
                                        dragOffset = maxDrag
                                    }
                                    UIImpactFeedbackGenerator(style: .heavy).impactOccurred()
                                    UINotificationFeedbackGenerator().notificationOccurred(.success)
                                    onConfirmed()
                                } else {
                                    withAnimation(.spring(response: 0.35, dampingFraction: 0.72)) {
                                        dragOffset = 0
                                    }
                                }
                            }
                    )
                }
            }
            .frame(height: trackHeight)
        }
        .frame(height: trackHeight)
        .disabled(isSending)
        .onChange(of: isSending) { _, sending in
            if !sending {
                withAnimation(.spring(response: 0.3, dampingFraction: 0.7)) {
                    dragOffset = 0
                    hasTriggered = false
                }
            }
        }
        .onChange(of: resetToken) { _, _ in
            withAnimation(.spring(response: 0.3, dampingFraction: 0.7)) {
                dragOffset = 0
                hasTriggered = false
            }
        }
    }
}
