import XCTest
@testable import StableChannels

final class SendAmountCalculatorTests: XCTestCase {
    func testComputeEffectiveSatsAcrossUnits() {
        let btcPrice: Double = 65_000

        // USD mode: $65.00 @ $65,000/BTC = 0.001 BTC = 100,000 sats
        XCTAssertEqual(
            SendAmountCalculator.computeEffectiveSats(text: "65.00", unit: .usd, btcPrice: btcPrice),
            100_000
        )

        // Sats mode
        XCTAssertEqual(
            SendAmountCalculator.computeEffectiveSats(text: "50000", unit: .sats, btcPrice: btcPrice),
            50_000
        )

        // BTC mode: 0.001 BTC = 100,000 sats
        XCTAssertEqual(
            SendAmountCalculator.computeEffectiveSats(text: "0.001", unit: .btc, btcPrice: btcPrice),
            100_000
        )
    }

    func testComputeEffectiveSats_edgeCases() {
        // Zero or negative btcPrice
        XCTAssertEqual(SendAmountCalculator.computeEffectiveSats(text: "100.00", unit: .usd, btcPrice: 0), 0)
        XCTAssertEqual(SendAmountCalculator.computeEffectiveSats(text: "100.00", unit: .usd, btcPrice: -50_000), 0)

        // Empty string
        XCTAssertEqual(SendAmountCalculator.computeEffectiveSats(text: "", unit: .sats, btcPrice: 65_000), 0)
        XCTAssertEqual(SendAmountCalculator.computeEffectiveSats(text: "   ", unit: .usd, btcPrice: 65_000), 0)

        // Invalid non-numeric input
        XCTAssertEqual(SendAmountCalculator.computeEffectiveSats(text: "abc", unit: .btc, btcPrice: 65_000), 0)

        // Negative input string
        XCTAssertEqual(SendAmountCalculator.computeEffectiveSats(text: "-50", unit: .sats, btcPrice: 65_000), 0)
        XCTAssertEqual(SendAmountCalculator.computeEffectiveSats(text: "-10.00", unit: .usd, btcPrice: 65_000), 0)

        // Overflow
        XCTAssertEqual(
            SendAmountCalculator.computeEffectiveSats(
                text: "99999999999999999999999999",
                unit: .sats,
                btcPrice: 65_000
            ),
            0
        )
    }

    func testFlooredDivisionPreventsSubSatoshiRoundingOverdraw() {
        let btcPrice: Double = 60_000
        // $0.01 @ $60,000/BTC = 0.01 / 60,000 = 1.6666...e-7 BTC = 16.666... sats
        // Integer-floored conversion must be exactly 16 sats, not rounded up to 17 sats
        let sats = SendAmountCalculator.computeEffectiveSats(text: "0.01", unit: .usd, btcPrice: btcPrice)
        XCTAssertEqual(sats, 16)
    }

    func testSwitchUnitPreservesValue() {
        let btcPrice: Double = 65_000

        // USD ($65.00) -> Sats (100,000)
        let toSats = SendAmountCalculator.switchUnit(from: .usd, to: .sats, text: "65.00", btcPrice: btcPrice)
        XCTAssertEqual(toSats, "100000")

        // Sats (100,000) -> BTC (0.00100000)
        let toBtc = SendAmountCalculator.switchUnit(from: .sats, to: .btc, text: "100000", btcPrice: btcPrice)
        XCTAssertEqual(toBtc, "0.00100000")

        // BTC (0.00100000) -> USD (65.00)
        let toUsd = SendAmountCalculator.switchUnit(from: .btc, to: .usd, text: "0.00100000", btcPrice: btcPrice)
        XCTAssertEqual(toUsd, "65.00")

        // Empty text preserves empty
        XCTAssertEqual(SendAmountCalculator.switchUnit(from: .usd, to: .sats, text: "", btcPrice: btcPrice), "")

        // Zero price with USD
        XCTAssertEqual(SendAmountCalculator.switchUnit(from: .sats, to: .usd, text: "10000", btcPrice: 0), "")
    }

    func testApplyPercentage() {
        let btcPrice: Double = 65_000
        let totalSats: UInt64 = 200_000

        // 50% of 200,000 = 100,000 sats
        XCTAssertEqual(
            SendAmountCalculator.applyPercentage(50, totalBalanceSats: totalSats, btcPrice: btcPrice, unit: .sats),
            "100000"
        )

        // 50% in USD = $65.00
        XCTAssertEqual(
            SendAmountCalculator.applyPercentage(50, totalBalanceSats: totalSats, btcPrice: btcPrice, unit: .usd),
            "65.00"
        )

        // 50% in BTC = 0.00100000
        XCTAssertEqual(
            SendAmountCalculator.applyPercentage(50, totalBalanceSats: totalSats, btcPrice: btcPrice, unit: .btc),
            "0.00100000"
        )

        // Edge cases: 0 balance or 0 price
        XCTAssertEqual(
            SendAmountCalculator.applyPercentage(50, totalBalanceSats: 0, btcPrice: btcPrice, unit: .usd),
            ""
        )
        XCTAssertEqual(
            SendAmountCalculator.applyPercentage(50, totalBalanceSats: totalSats, btcPrice: 0, unit: .usd),
            ""
        )
    }

    func testNormalizeAmountInput() {
        // USD normalization
        XCTAssertEqual(SendAmountCalculator.normalizeAmountInput(text: "12", unit: .usd), "12.00")
        XCTAssertEqual(SendAmountCalculator.normalizeAmountInput(text: "12.5", unit: .usd), "12.50")
        XCTAssertEqual(SendAmountCalculator.normalizeAmountInput(text: "12.", unit: .usd), "12.00")
        XCTAssertEqual(SendAmountCalculator.normalizeAmountInput(text: ".5", unit: .usd), "0.50")

        // Sats normalization
        XCTAssertEqual(SendAmountCalculator.normalizeAmountInput(text: "0050", unit: .sats), "50")
        XCTAssertEqual(SendAmountCalculator.normalizeAmountInput(text: "100.5", unit: .sats), "100")

        // BTC normalization
        XCTAssertEqual(SendAmountCalculator.normalizeAmountInput(text: ".001", unit: .btc), "0.001")
        XCTAssertEqual(SendAmountCalculator.normalizeAmountInput(text: "0.123456789", unit: .btc), "0.12345679")

        // Empty text
        XCTAssertEqual(SendAmountCalculator.normalizeAmountInput(text: "", unit: .usd), "")
    }

    func testSendAmountUnitProperties() {
        XCTAssertEqual(SendAmountUnit.usd.maxDecimals, 2)
        XCTAssertEqual(SendAmountUnit.sats.maxDecimals, 0)
        XCTAssertEqual(SendAmountUnit.btc.maxDecimals, 8)

        XCTAssertEqual(SendAmountUnit.usd.symbolOrSuffix, "$")
        XCTAssertEqual(SendAmountUnit.sats.symbolOrSuffix, "sats")
        XCTAssertEqual(SendAmountUnit.btc.symbolOrSuffix, "BTC")

        XCTAssertEqual(SendAmountUnit.usd.menuTitle, "US Dollar (USD)")
        XCTAssertEqual(SendAmountUnit.sats.menuTitle, "Satoshis (sats)")
        XCTAssertEqual(SendAmountUnit.btc.menuTitle, "Bitcoin (BTC)")
    }
}
