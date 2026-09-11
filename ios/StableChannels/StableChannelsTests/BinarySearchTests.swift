import XCTest
@testable import StableChannels

final class BinarySearchTests: XCTestCase {
    func testLowerBoundComparable() {
        let numbers = [10, 20, 30, 40, 50]

        XCTAssertEqual(numbers.lowerBound(for: 5), 0)
        XCTAssertEqual(numbers.lowerBound(for: 10), 0)
        XCTAssertEqual(numbers.lowerBound(for: 25), 2)
        XCTAssertEqual(numbers.lowerBound(for: 30), 2)
        XCTAssertEqual(numbers.lowerBound(for: 50), 4)
        XCTAssertEqual(numbers.lowerBound(for: 55), 5)
    }

    func testUpperBoundComparable() {
        let numbers = [10, 20, 20, 30, 40]

        XCTAssertEqual(numbers.upperBound(for: 5), 0)
        XCTAssertEqual(numbers.upperBound(for: 10), 1)
        XCTAssertEqual(numbers.upperBound(for: 20), 3)
        XCTAssertEqual(numbers.upperBound(for: 30), 4)
        XCTAssertEqual(numbers.upperBound(for: 40), 5)
    }

    func testBinarySearchNearest() {
        struct Point {
            let x: Double
            let y: Double
        }

        let points = [
            Point(x: 100.0, y: 1.0),
            Point(x: 200.0, y: 2.0),
            Point(x: 300.0, y: 3.0),
            Point(x: 400.0, y: 4.0)
        ]

        XCTAssertEqual(points.binarySearchNearest(target: 50.0, keySelector: \.x)?.x, 100.0)
        XCTAssertEqual(points.binarySearchNearest(target: 140.0, keySelector: \.x)?.x, 100.0)
        XCTAssertEqual(points.binarySearchNearest(target: 160.0, keySelector: \.x)?.x, 200.0)
        XCTAssertEqual(points.binarySearchNearest(target: 290.0, keySelector: \.x)?.x, 300.0)
        XCTAssertEqual(points.binarySearchNearest(target: 450.0, keySelector: \.x)?.x, 400.0)
    }

    func testBinarySearchNearestDate() {
        struct Event {
            let date: Date
            let id: Int
        }
        let events = [
            Event(date: Date(timeIntervalSince1970: 100), id: 1),
            Event(date: Date(timeIntervalSince1970: 200), id: 2),
            Event(date: Date(timeIntervalSince1970: 300), id: 3)
        ]
        XCTAssertEqual(events.binarySearchNearest(target: Date(timeIntervalSince1970: 140)) { $0.date }?.id, 1)
        XCTAssertEqual(events.binarySearchNearest(target: Date(timeIntervalSince1970: 160)) { $0.date }?.id, 2)
    }

    func testBinarySearchNearestWithCustomDistance() {
        struct Item {
            let value: String
            let length: Int
        }
        let items = [
            Item(value: "a", length: 1),
            Item(value: "ccc", length: 3),
            Item(value: "ffffff", length: 6)
        ]
        let nearest = items.binarySearchNearest(target: 4, keySelector: \.length) { abs($0 - $1) }
        XCTAssertEqual(nearest?.length, 3)
    }

    func testBinarySearchNearestInteger() {
        // Values above 2^53 (9_007_199_254_740_992) where Double loses unit precision for odd integers
        let values: [Int64] = [
            9_007_199_254_740_993,
            9_007_199_254_740_995,
            9_007_199_254_740_997
        ]
        XCTAssertEqual(
            values.binarySearchNearest(target: 9_007_199_254_740_995, keySelector: { $0 }),
            9_007_199_254_740_995
        )
        XCTAssertEqual(
            values.binarySearchNearest(target: 9_007_199_254_740_997, keySelector: { $0 }),
            9_007_199_254_740_997
        )
        XCTAssertEqual(
            values.binarySearchNearest(target: 9_007_199_254_740_994, keySelector: { $0 }),
            9_007_199_254_740_993
        )
    }

    func testEmptyCollection() {
        let empty = [Int]()
        XCTAssertEqual(empty.lowerBound(for: 10), 0)
        XCTAssertEqual(empty.upperBound(for: 10), 0)
        XCTAssertNil(empty.binarySearchNearest(target: 10, keySelector: { $0 }))
    }
}
