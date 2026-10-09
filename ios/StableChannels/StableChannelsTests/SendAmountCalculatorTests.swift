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
        // Excess precision is truncated, never rounded up: normalization can only lower an amount.
        XCTAssertEqual(SendAmountCalculator.normalizeAmountInput(text: "0.123456789", unit: .btc), "0.12345678")
        XCTAssertEqual(SendAmountCalculator.normalizeAmountInput(text: "12.345", unit: .usd), "12.34")
        XCTAssertEqual(SendAmountCalculator.normalizeAmountInput(text: "12.349", unit: .usd), "12.34")

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

    func testComputeEffectiveSats_readsDestinationOnchainAmountWhenInputEmpty() {
        let dest = SendDestination.onchain(address: "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq", amountSats: 2500)
        XCTAssertEqual(
            SendAmountCalculator.computeEffectiveSats(destination: dest, inputText: "", unit: .sats, btcPrice: 50_000),
            2500
        )
        XCTAssertEqual(
            SendAmountCalculator.computeEffectiveSats(
                destination: dest,
                inputText: "   ",
                unit: .usd,
                btcPrice: 50_000
            ),
            2500
        )

        // When user explicitly enters an amount, the entered amount takes precedence
        XCTAssertEqual(
            SendAmountCalculator.computeEffectiveSats(
                destination: dest,
                inputText: "5000",
                unit: .sats,
                btcPrice: 50_000
            ),
            5000
        )
    }

    func testComputeEffectiveSats_isExactForInputsThatAreInexactAsDoubles() {
        // 0.0003 * 1e8 is 29999.999999999996 as a binary double; flooring that drops a sat.
        // Decimal arithmetic must give the amount the user actually typed.
        XCTAssertEqual(SendAmountCalculator.computeEffectiveSats(text: "0.0003", unit: .btc, btcPrice: 65_000), 30_000)
        XCTAssertEqual(SendAmountCalculator.computeEffectiveSats(text: "0.0006", unit: .btc, btcPrice: 65_000), 60_000)
        XCTAssertEqual(SendAmountCalculator.computeEffectiveSats(text: "0.0012", unit: .btc, btcPrice: 65_000), 120_000)
        XCTAssertEqual(SendAmountCalculator.computeEffectiveSats(text: "0.00000001", unit: .btc, btcPrice: 65_000), 1)
        // 4.55 / 65,000 * 1e8 is 6999.999999999999 as a double.
        XCTAssertEqual(SendAmountCalculator.computeEffectiveSats(text: "4.55", unit: .usd, btcPrice: 65_000), 7_000)
        // Sub-sat precision is still floored, never rounded up.
        XCTAssertEqual(SendAmountCalculator.computeEffectiveSats(text: "0.000000019", unit: .btc, btcPrice: 65_000), 1)
        XCTAssertEqual(SendAmountCalculator.computeEffectiveSats(text: "0.015", unit: .usd, btcPrice: 60_000), 25)
    }

    func testComputeEffectiveSats_rejectsMalformedNumbers() {
        XCTAssertEqual(SendAmountCalculator.computeEffectiveSats(text: "1e5", unit: .sats, btcPrice: 65_000), 0)
        XCTAssertEqual(SendAmountCalculator.computeEffectiveSats(text: "1.2.3", unit: .btc, btcPrice: 65_000), 0)
        XCTAssertEqual(SendAmountCalculator.computeEffectiveSats(text: "+5", unit: .sats, btcPrice: 65_000), 0)
        XCTAssertEqual(SendAmountCalculator.computeEffectiveSats(text: "0x10", unit: .sats, btcPrice: 65_000), 0)
    }

    func testFormatSatsForUnit_btcIsExactAndRoundTripsWithoutDrift() {
        let btcPrice: Double = 65_000
        XCTAssertEqual(SendAmountCalculator.formatSatsForUnit(30_000, unit: .btc, btcPrice: btcPrice), "0.00030000")
        XCTAssertEqual(SendAmountCalculator.formatSatsForUnit(12_345_678, unit: .btc, btcPrice: btcPrice), "0.12345678")
        XCTAssertEqual(SendAmountCalculator.formatSatsForUnit(1, unit: .btc, btcPrice: btcPrice), "0.00000001")

        // Sats -> BTC -> Sats must return the same sats, for values that are inexact as doubles.
        for sats: UInt64 in [1, 30_000, 59_999, 119_999, 12_345_678, 2_100_000_000_000_000] {
            let btcText = SendAmountCalculator.formatSatsForUnit(sats, unit: .btc, btcPrice: btcPrice)
            XCTAssertEqual(
                SendAmountCalculator.switchUnit(from: .btc, to: .sats, text: btcText, btcPrice: btcPrice),
                "\(sats)",
                "round-trip drifted for \(sats) sats via \(btcText)"
            )
        }
    }

    func testFormatSatsForUnit_usdTruncatesToCents() {
        // 10,001 sats @ $65,000 = $6.50065 -> $6.50, never $6.51.
        XCTAssertEqual(SendAmountCalculator.formatSatsForUnit(10_001, unit: .usd, btcPrice: 65_000), "6.50")
        // 1 sat @ $65,000 is below a cent.
        XCTAssertEqual(SendAmountCalculator.formatSatsForUnit(1, unit: .usd, btcPrice: 65_000), "0.00")
    }
}
