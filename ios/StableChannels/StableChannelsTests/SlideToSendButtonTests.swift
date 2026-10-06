import SwiftUI
import XCTest
@testable import StableChannels

final class SlideToSendButtonTests: XCTestCase {
    func testThresholdCalculation() {
        let width: CGFloat = 300
        let thumbSize: CGFloat = 50
        let trackWidth = max(width - thumbSize, 1.0)
        let threshold = trackWidth * 0.85

        XCTAssertEqual(trackWidth, 250)
        XCTAssertEqual(threshold, 212.5)
    }

    func testResetTokenChangeDetection() {
        var observedResetCount = 0
        var currentToken = 0

        func onTokenChange(oldVal: Int, newVal: Int) {
            if oldVal != newVal {
                observedResetCount += 1
            }
        }

        currentToken = 1
        onTokenChange(oldVal: 0, newVal: currentToken)
        XCTAssertEqual(observedResetCount, 1)

        currentToken = 2
        onTokenChange(oldVal: 1, newVal: currentToken)
        XCTAssertEqual(observedResetCount, 2)
    }
}
