import XCTest
@testable import StableChannels

final class SliderConversionMetricsTests: XCTestCase {
    func testSliderConversionMetricsSymmetricSideWidth() {
        let metrics5545 = SliderConversionMetrics.calculate(usdPct: 55, btcPct: 45)
        let metrics4555 = SliderConversionMetrics.calculate(usdPct: 45, btcPct: 55)

        XCTAssertEqual(metrics5545.largestPct, 55)
        XCTAssertEqual(metrics4555.largestPct, 55)
        XCTAssertEqual(metrics5545.sideWidth, metrics4555.sideWidth)
        XCTAssertGreaterThan(metrics5545.sideWidth, 0)
    }

    func testSliderConversionMetricsAntiFlickerTwoDigitStability() {
        // Tabular digits guarantee all 2-digit combinations produce identical sideWidth
        let baseMetrics = SliderConversionMetrics.calculate(usdPct: 50, btcPct: 50)
        let testSplits: [(Int, Int)] = [
            (55, 45),
            (60, 40),
            (75, 25),
            (88, 12),
            (99, 1),
            (11, 89)
        ]

        for (usd, btc) in testSplits {
            let m = SliderConversionMetrics.calculate(usdPct: usd, btcPct: btc)
            XCTAssertEqual(
                m.sideWidth,
                baseMetrics.sideWidth,
                "Side width for \(usd)/\(btc) must equal base 50/50 width to prevent jitter"
            )
        }
    }

    func testSliderConversionMetricsBoundaryExpansion() {
        let metrics100 = SliderConversionMetrics.calculate(usdPct: 100, btcPct: 0)
        let metrics99 = SliderConversionMetrics.calculate(usdPct: 99, btcPct: 1)

        XCTAssertEqual(metrics100.largestPct, 100)
        XCTAssertGreaterThanOrEqual(metrics100.sideWidth, metrics99.sideWidth)
    }
}
