import SwiftUI
import XCTest
@testable import StableChannels

@MainActor
final class SendMultiStepViewTests: XCTestCase {
    func testSendFlowStepNavigationTransitions() {
        let model = SendFlowModel()
        XCTAssertEqual(model.step, .recipient)

        model.step = .amount
        XCTAssertEqual(model.step, .amount)

        model.step = .confirm
        XCTAssertEqual(model.step, .confirm)

        model.step = .success
        XCTAssertEqual(model.step, .success)

        model.resetFlow()
        XCTAssertEqual(model.step, .recipient)
    }

    func testSendViewInitializerWithPrefilledInput() {
        let customInput = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        let model = SendFlowModel()
        model.inputText = customInput

        XCTAssertEqual(model.inputText, customInput)
        guard case .valid(let dest) = model.classification else {
            XCTFail("Expected valid onchain destination")
            return
        }
        XCTAssertEqual(dest.rawDestination, customInput)
    }

    func testNetworkFeeSelectorRateFormatting() {
        let baseRate = 12.5
        let priorityRate = NetworkFeeSpeedTier.priority.effectiveRate(baseRate: baseRate)
        let standardRate = NetworkFeeSpeedTier.standard.effectiveRate(baseRate: baseRate)
        let economyRate = NetworkFeeSpeedTier.economy.effectiveRate(baseRate: baseRate)

        XCTAssertGreaterThan(priorityRate, standardRate)
        XCTAssertEqual(standardRate, baseRate)
        XCTAssertLessThan(economyRate, standardRate)
        XCTAssertGreaterThanOrEqual(economyRate, Constants.minAllowedFeeRateSatVb)
    }

    func testDestinationVisualRepresentationExtraction() {
        let p2pkhAddr = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"
        let chunked = AddressVisualChunker.chunkAddress(p2pkhAddr)
        let rep = DestinationVisualRepresentation.onchain(chunked)

        XCTAssertEqual(rep.rawDestination, p2pkhAddr)
    }
}
