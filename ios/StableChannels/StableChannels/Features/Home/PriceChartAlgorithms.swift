import Foundation

/// Mathematical and sampling helpers for time-series charts.
enum PriceChartAlgorithms {
    /// Binary search for the first index where record date >= targetDate.
    /// Assumes records are sorted chronologically in ascending order.
    static func lowerBound(in records: [PriceRecord], cutoff: Date) -> Int {
        records.lowerBound(target: cutoff) { $0.date }
    }

    /// Binary search to find the record closest in time to targetDate.
    static func nearestRecord(in records: [PriceRecord], targetDate: Date) -> PriceRecord? {
        records.binarySearchNearest(target: targetDate) { $0.date }
    }

    /// Single-pass min and max calculation with 2% margin padding.
    static func chartBounds(in records: [PriceRecord]) -> (min: Double, max: Double) {
        guard !records.isEmpty else { return (0, 100) }
        var lo = Double.infinity
        var hi = -Double.infinity
        for r in records {
            if r.price < lo { lo = r.price }
            if r.price > hi { hi = r.price }
        }
        return (lo * 0.98, hi * 1.02)
    }

    /// Largest Triangle Three Buckets (LTTB) downsampling algorithm.
    /// Preserves critical visual extrema (peaks and troughs).
    static func lttbDownsample(_ records: [PriceRecord], targetCount: Int) -> [PriceRecord] {
        guard records.count > targetCount, targetCount > 2 else {
            return records
        }

        var sampled: [PriceRecord] = []
        sampled.reserveCapacity(targetCount)

        // Always include the first point
        sampled.append(records[0])

        let bucketSize = Double(records.count - 2) / Double(targetCount - 2)
        var a = 0

        for i in 0..<(targetCount - 2) {
            // Calculate point average for next bucket (bucket C)
            var avgX = 0.0
            var avgY = 0.0
            let nextBucketStart = Int(floor(Double(i + 1) * bucketSize)) + 1
            let nextBucketEnd = min(Int(floor(Double(i + 2) * bucketSize)) + 1, records.count)
            let nextBucketCount = Double(nextBucketEnd - nextBucketStart)

            if nextBucketCount > 0 {
                for j in nextBucketStart..<nextBucketEnd {
                    avgX += Double(records[j].timestamp)
                    avgY += records[j].price
                }
                avgX /= nextBucketCount
                avgY /= nextBucketCount
            } else if nextBucketStart < records.count {
                avgX = Double(records[nextBucketStart].timestamp)
                avgY = records[nextBucketStart].price
            }

            // Current bucket range (bucket B)
            let currentBucketStart = Int(floor(Double(i) * bucketSize)) + 1
            let currentBucketEnd = min(Int(floor(Double(i + 1) * bucketSize)) + 1, records.count)

            let pointA = records[a]
            let pointAX = Double(pointA.timestamp)
            let pointAY = pointA.price

            var maxArea = -1.0
            var maxAreaIndex = currentBucketStart

            for j in currentBucketStart..<currentBucketEnd {
                let currentX = Double(records[j].timestamp)
                let currentY = records[j].price

                let area = abs((pointAX - avgX) * (currentY - pointAY) - (pointAX - currentX) * (avgY - pointAY)) * 0.5

                if area > maxArea {
                    maxArea = area
                    maxAreaIndex = j
                }
            }

            sampled.append(records[maxAreaIndex])
            a = maxAreaIndex
        }

        // Always include the last point
        sampled.append(records[records.count - 1])
        return sampled
    }
}
