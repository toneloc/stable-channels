import XCTest
@testable import StableChannels

final class PriceChartAlgorithmsTests: XCTestCase {
    private func makeRecord(_ ts: Int64, _ price: Double) -> PriceRecord {
        PriceRecord(id: ts, price: price, source: "test", timestamp: ts)
    }

    func testLowerBound() {
        let records = [
            makeRecord(100, 50000),
            makeRecord(200, 51000),
            makeRecord(300, 52000),
            makeRecord(400, 53000),
            makeRecord(500, 54000)
        ]

        let d50 = Date(timeIntervalSince1970: 50)
        let d100 = Date(timeIntervalSince1970: 100)
        let d150 = Date(timeIntervalSince1970: 150)
        let d300 = Date(timeIntervalSince1970: 300)
        let d450 = Date(timeIntervalSince1970: 450)
        let d500 = Date(timeIntervalSince1970: 500)
        let d550 = Date(timeIntervalSince1970: 550)

        XCTAssertEqual(PriceChartAlgorithms.lowerBound(in: records, cutoff: d50), 0)
        XCTAssertEqual(PriceChartAlgorithms.lowerBound(in: records, cutoff: d100), 0)
        XCTAssertEqual(PriceChartAlgorithms.lowerBound(in: records, cutoff: d150), 1)
        XCTAssertEqual(PriceChartAlgorithms.lowerBound(in: records, cutoff: d300), 2)
        XCTAssertEqual(PriceChartAlgorithms.lowerBound(in: records, cutoff: d450), 4)
        XCTAssertEqual(PriceChartAlgorithms.lowerBound(in: records, cutoff: d500), 4)
        XCTAssertEqual(PriceChartAlgorithms.lowerBound(in: records, cutoff: d550), 5)
        XCTAssertEqual(PriceChartAlgorithms.lowerBound(in: [], cutoff: d100), 0)
    }

    func testNearestRecord() {
        let records = [
            makeRecord(100, 50000),
            makeRecord(200, 51000),
            makeRecord(300, 52000)
        ]

        XCTAssertEqual(
            PriceChartAlgorithms.nearestRecord(in: records, targetDate: Date(timeIntervalSince1970: 50))?.timestamp,
            100
        )
        XCTAssertEqual(
            PriceChartAlgorithms.nearestRecord(in: records, targetDate: Date(timeIntervalSince1970: 140))?.timestamp,
            100
        )
        XCTAssertEqual(
            PriceChartAlgorithms.nearestRecord(in: records, targetDate: Date(timeIntervalSince1970: 160))?.timestamp,
            200
        )
        XCTAssertEqual(
            PriceChartAlgorithms.nearestRecord(in: records, targetDate: Date(timeIntervalSince1970: 250))?.timestamp,
            200
        )
        XCTAssertEqual(
            PriceChartAlgorithms.nearestRecord(in: records, targetDate: Date(timeIntervalSince1970: 350))?.timestamp,
            300
        )
        XCTAssertNil(PriceChartAlgorithms.nearestRecord(in: [], targetDate: Date(timeIntervalSince1970: 100)))
    }

    func testChartBounds() {
        let records = [
            makeRecord(100, 10000),
            makeRecord(200, 20000),
            makeRecord(300, 15000)
        ]

        let (minP, maxP) = PriceChartAlgorithms.chartBounds(in: records)
        XCTAssertEqual(minP, 9800, accuracy: 0.001)
        XCTAssertEqual(maxP, 20400, accuracy: 0.001)
    }

    func testLttbDownsample() {
        var records: [PriceRecord] = []
        for i in 0..<100 {
            let price = (i == 50) ? 99999.0 : (50000.0 + Double(i) * 10)
            records.append(makeRecord(Int64(i * 100), price))
        }

        let targetCount = 20
        let sampled = PriceChartAlgorithms.lttbDownsample(records, targetCount: targetCount)
        XCTAssertEqual(sampled.count, targetCount)
        XCTAssertEqual(sampled.first?.timestamp, records.first?.timestamp)
        XCTAssertEqual(sampled.last?.timestamp, records.last?.timestamp)
        XCTAssertTrue(sampled.contains { $0.price == 99999 }, "LTTB should preserve local extrema peak")
    }

    func testLttbEdgeCases() {
        let records = (0..<10).map { i in
            makeRecord(Int64(i * 100), 50000.0 + Double(i * 100))
        }

        // targetCount >= records.count returns original
        let sameCount = PriceChartAlgorithms.lttbDownsample(records, targetCount: 10)
        XCTAssertEqual(sameCount.count, 10)
        let largerCount = PriceChartAlgorithms.lttbDownsample(records, targetCount: 20)
        XCTAssertEqual(largerCount.count, 10)

        // targetCount <= 2 returns original records per LTTB boundary guard
        let twoPoints = PriceChartAlgorithms.lttbDownsample(records, targetCount: 2)
        XCTAssertEqual(twoPoints.count, records.count)

        // Preserves local valley (drop)
        var valleyRecords: [PriceRecord] = []
        for i in 0..<100 {
            let price = (i == 50) ? 1000.0 : (50000.0 + Double(i) * 10)
            valleyRecords.append(makeRecord(Int64(i * 100), price))
        }
        let valleySampled = PriceChartAlgorithms.lttbDownsample(valleyRecords, targetCount: 20)
        XCTAssertEqual(valleySampled.count, 20)
        XCTAssertTrue(valleySampled.contains { $0.price == 1000.0 }, "LTTB should preserve local extrema valley")

        // Monotonically increasing data preserves chronological order
        let monotonicRecords = (0..<50).map { i in makeRecord(Int64(i * 10), Double(i)) }
        let monotonicSampled = PriceChartAlgorithms.lttbDownsample(monotonicRecords, targetCount: 10)
        XCTAssertEqual(monotonicSampled.count, 10)
        for i in 0..<(monotonicSampled.count - 1) {
            XCTAssertLessThan(monotonicSampled[i].timestamp, monotonicSampled[i + 1].timestamp)
        }
    }
}
