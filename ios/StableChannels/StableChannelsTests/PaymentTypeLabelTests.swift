@testable import StableChannels
import XCTest

final class PaymentTypeLabelTests: XCTestCase {
    func testEveryStoredTypeHasItsLabel() {
        let expected = [
            "stability": "Stability", "lightning": "Lightning", "splice_in": "Splice In", "splice_out": "Splice Out",
            "onchain": "Onchain", "channel_close": "Channel Close", "bolt12": "Bolt 12"
        ]
        for (type, label) in expected {
            XCTAssertEqual(PaymentTypeLabel.text(for: type, stabilityEnabled: true), label, type)
        }
    }

    func testWithoutStabilityAStabilityPaymentShowsAsLightning() {
        XCTAssertEqual(PaymentTypeLabel.text(for: "stability", stabilityEnabled: false), "Lightning")
        XCTAssertEqual(PaymentTypeLabel.text(for: "onchain", stabilityEnabled: false), "Onchain")
    }

    func testAnUnknownTypeShowsAsStored() {
        XCTAssertEqual(PaymentTypeLabel.text(for: "something_new", stabilityEnabled: true), "something_new")
    }
}
