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

        // Empty collection
        XCTAssertTrue(PriceChartAlgorithms.lttbDownsample([PriceRecord](), targetCount: 10).isEmpty)

        // Single element collection
        let single = makeRecord(100, 50000)
        let singleResult = PriceChartAlgorithms.lttbDownsample([single], targetCount: 10)
        XCTAssertEqual(singleResult.count, 1)
        XCTAssertEqual(singleResult.first?.timestamp, single.timestamp)

        // targetCount <= 0 returns empty
        XCTAssertTrue(PriceChartAlgorithms.lttbDownsample(records, targetCount: 0).isEmpty)
        XCTAssertTrue(PriceChartAlgorithms.lttbDownsample(records, targetCount: -5).isEmpty)

        // targetCount == 1 returns first element
        let onePoint = PriceChartAlgorithms.lttbDownsample(records, targetCount: 1)
        XCTAssertEqual(onePoint.count, 1)
        XCTAssertEqual(onePoint.first?.timestamp, records.first?.timestamp)

        // targetCount == 2 returns first and last endpoints
        let twoPoints = PriceChartAlgorithms.lttbDownsample(records, targetCount: 2)
        XCTAssertEqual(twoPoints.count, 2)
        XCTAssertEqual(twoPoints.first?.timestamp, records.first?.timestamp)
        XCTAssertEqual(twoPoints.last?.timestamp, records.last?.timestamp)

        // targetCount >= records.count returns original records
        let sameCount = PriceChartAlgorithms.lttbDownsample(records, targetCount: 10)
        XCTAssertEqual(sameCount.count, 10)
        let largerCount = PriceChartAlgorithms.lttbDownsample(records, targetCount: 50)
        XCTAssertEqual(largerCount.count, 10)

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

    func testLttbDownsampleZeroCopySlice() {
        var allRecords: [PriceRecord] = []
        for i in 0..<100 {
            let price: Double = (i == 40) ? 99999.0 : Double(50000 + i * 10)
            allRecords.append(makeRecord(Int64(i * 100), price))
        }

        // Subslice starting at offset 20 (non-zero startIndex)
        let slice = allRecords[20..<80]
        XCTAssertEqual(slice.startIndex, 20)
        XCTAssertEqual(slice.count, 60)

        let targetCount = 15
        let sampled = PriceChartAlgorithms.lttbDownsample(slice, targetCount: targetCount)
        XCTAssertEqual(sampled.count, targetCount)
        XCTAssertEqual(sampled.first?.timestamp, slice.first?.timestamp)
        XCTAssertEqual(sampled.last?.timestamp, slice.last?.timestamp)
        XCTAssertTrue(
            sampled.contains { $0.price == 99999.0 },
            "Must preserve peak even from non-zero startIndex slice"
        )
    }

    func testDailyDateParsing() {
        // Epoch 0: 1970-01-01
        XCTAssertEqual(PriceHistoryService.parseDailyDateToTimestamp("1970-01-01"), 0)

        // Valid leap day: 2024-02-29
        XCTAssertEqual(PriceHistoryService.parseDailyDateToTimestamp("2024-02-29"), 1709164800)

        // Valid century leap day (divisible by 400): 2000-02-29
        XCTAssertEqual(PriceHistoryService.parseDailyDateToTimestamp("2000-02-29"), 951782400)

        // Target reference date: 2026-09-21
        XCTAssertEqual(PriceHistoryService.parseDailyDateToTimestamp("2026-09-21"), 1789948800)

        // Valid 30-day month end: 2026-04-30
        XCTAssertNotNil(PriceHistoryService.parseDailyDateToTimestamp("2026-04-30"))

        // Invalid February dates
        XCTAssertNil(PriceHistoryService.parseDailyDateToTimestamp("2026-02-29"), "2026 is not a leap year")
        XCTAssertNil(PriceHistoryService.parseDailyDateToTimestamp("2026-02-30"))
        XCTAssertNil(PriceHistoryService.parseDailyDateToTimestamp("2026-02-31"))
        XCTAssertNil(
            PriceHistoryService.parseDailyDateToTimestamp("1900-02-29"),
            "1900 is not a leap year (century rule)"
        )

        // Invalid days for 30-day months
        XCTAssertNil(PriceHistoryService.parseDailyDateToTimestamp("2026-04-31"), "April has 30 days")
        XCTAssertNil(PriceHistoryService.parseDailyDateToTimestamp("2026-06-31"), "June has 30 days")
        XCTAssertNil(PriceHistoryService.parseDailyDateToTimestamp("2026-09-31"), "September has 30 days")
        XCTAssertNil(PriceHistoryService.parseDailyDateToTimestamp("2026-11-31"), "November has 30 days")

        // Invalid month/day ranges
        XCTAssertNil(PriceHistoryService.parseDailyDateToTimestamp("2026-00-15"), "Month 0 is invalid")
        XCTAssertNil(PriceHistoryService.parseDailyDateToTimestamp("2026-13-15"), "Month 13 is invalid")
        XCTAssertNil(PriceHistoryService.parseDailyDateToTimestamp("2026-05-00"), "Day 0 is invalid")
        XCTAssertNil(PriceHistoryService.parseDailyDateToTimestamp("2026-05-32"), "Day 32 is invalid")
        XCTAssertNil(PriceHistoryService.parseDailyDateToTimestamp("2026-99-99"))

        // Malformed format (strictly 10 characters yyyy-MM-dd)
        XCTAssertNil(PriceHistoryService.parseDailyDateToTimestamp("invalid-date"))
        XCTAssertNil(PriceHistoryService.parseDailyDateToTimestamp("2026-4-5"))
        XCTAssertNil(PriceHistoryService.parseDailyDateToTimestamp("2026-04-05T00:00:00Z"))
    }
}
