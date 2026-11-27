@testable import StableChannels
import XCTest

final class StabilityNotificationCopyTests: XCTestCase {
    func testTheNormalBuildKeepsTheStabilityWording() {
        let copy = StabilityNotificationCopy(stabilityEnabled: true)
        XCTAssertEqual(copy.receivedTitle, "Stability Payment Received")
        XCTAssertEqual(copy.sentTitle, "Stability Payment Sent")
        XCTAssertEqual(copy.sentBody(sats: 1_500, dollars: 1.25), "Sent 1500 sats ($1.25) to maintain stable position")
        XCTAssertEqual(copy.checkTitle, "Stability Check")
        XCTAssertEqual(copy.checkBody, "Position is stable")
        XCTAssertEqual(copy.processInApp, "Open app to process stability payment")
        XCTAssertEqual(copy.finishSyncInApp, "Open app to finish syncing the stability payment")
        XCTAssertEqual(copy.syncPositionInApp, "Open app to sync stable position")
    }

    func testTheAppStoreBuildNeverMentionsStability() {
        let copy = StabilityNotificationCopy(stabilityEnabled: false)
        let texts = [
            copy.receivedTitle, copy.sentTitle, copy.sentBody(sats: 1_500, dollars: 1.25), copy.checkTitle,
            copy.checkBody, copy.processInApp, copy.finishSyncInApp, copy.syncPositionInApp
        ]
        XCTAssertEqual(copy.receivedTitle, "Payment Received")
        XCTAssertEqual(copy.sentBody(sats: 1_500, dollars: 1.25), "Sent 1500 sats ($1.25)")
        for text in texts {
            XCTAssertFalse(text.localizedCaseInsensitiveContains("stabil"), "Unexpected stability wording: \(text)")
            XCTAssertFalse(text.localizedCaseInsensitiveContains("stable"), "Unexpected stability wording: \(text)")
        }
    }

    func testTheNormalBuildUsesTheStabilityWording() {
        XCTAssertEqual(StabilityNotificationCopy.current, StabilityNotificationCopy(stabilityEnabled: true))
    }
}
