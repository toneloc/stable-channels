import SwiftUI
import XCTest
@testable import StableChannels

final class CurveGeometryTests: XCTestCase {
    func testCurveProgressIndicator_parametricCurvesAreFiniteAndBounded() {
        for curve in CurveProgressIndicator.CurveType.allCases {
            for step in 0...20 {
                let progress = Double(step) / 20.0
                let pt = CurveProgressIndicator.pointOnCurve(curve: curve, progress: progress, detailScale: 1.0)
                XCTAssertTrue(pt.x.isFinite, "Curve \(curve) point x must be finite at progress \(progress)")
                XCTAssertTrue(pt.y.isFinite, "Curve \(curve) point y must be finite at progress \(progress)")
                XCTAssertGreaterThanOrEqual(pt.x, -50.0)
                XCTAssertLessThanOrEqual(pt.x, 150.0)
                XCTAssertGreaterThanOrEqual(pt.y, -50.0)
                XCTAssertLessThanOrEqual(pt.y, 150.0)
            }
        }
    }

    func testLemniscateShape_createsValidBoundedPath() {
        let shape = LemniscateShape()
        let rect = CGRect(x: 0, y: 0, width: 100, height: 60)
        let path = shape.path(in: rect)
        let bounds = path.boundingRect

        XCTAssertFalse(bounds.isNull)
        XCTAssertFalse(bounds.isEmpty)
        XCTAssertGreaterThan(bounds.width, 0)
        XCTAssertGreaterThan(bounds.height, 0)
        XCTAssertLessThanOrEqual(bounds.maxX, rect.maxX + 1.0)
        XCTAssertGreaterThanOrEqual(bounds.minX, rect.minX - 1.0)
    }
}
