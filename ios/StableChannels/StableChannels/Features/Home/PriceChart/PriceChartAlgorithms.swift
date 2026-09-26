import Foundation

/// Mathematical and sampling helpers for time-series charts.
enum PriceChartAlgorithms {
    /// Binary search for the first index where record date >= targetDate.
    /// Assumes records are sorted chronologically in ascending order.
    static func lowerBound<C: RandomAccessCollection>(in records: C, cutoff: Date) -> C.Index
        where C.Element == PriceRecord {
        records.lowerBound(target: cutoff) { $0.date }
    }

    /// Binary search to find the record closest in time to targetDate.
    static func nearestRecord<C: RandomAccessCollection>(in records: C, targetDate: Date) -> PriceRecord?
        where C.Element == PriceRecord {
        records.binarySearchNearest(target: targetDate) { $0.date }
    }

    /// Single-pass min and max calculation with 2% margin padding.
    static func chartBounds(in records: some Sequence<PriceRecord>) -> (min: Double, max: Double) {
        var lo = Double.infinity
        var hi = -Double.infinity
        var hasElements = false
        for r in records {
            hasElements = true
            if r.price < lo { lo = r.price }
            if r.price > hi { hi = r.price }
        }
        guard hasElements else { return (0, 100) }
        return (lo * 0.98, hi * 1.02)
    }

    /// Largest Triangle Three Buckets (LTTB) downsampling algorithm.
    /// Preserves critical visual extrema (peaks and troughs).
    /// Accepts any RandomAccessCollection with Int indexing for zero-copy slicing.
    static func lttbDownsample<C: RandomAccessCollection>(
        _ records: C,
        targetCount: Int
    ) -> [PriceRecord] where C.Element == PriceRecord, C.Index == Int {
        guard targetCount > 0, !records.isEmpty else {
            return []
        }
        guard records.count > targetCount else {
            return Array(records)
        }
        if targetCount == 1 {
            return [records[records.startIndex]]
        }
        if targetCount == 2 {
            return [records[records.startIndex], records[records.endIndex - 1]]
        }

        var sampled: [PriceRecord] = []
        sampled.reserveCapacity(targetCount)

        let start = records.startIndex
        // Always include the first point
        sampled.append(records[start])

        let count = records.count
        let bucketSize = Double(count - 2) / Double(targetCount - 2)
        var aIndex = start

        for i in 0..<(targetCount - 2) {
            let (avgX, avgY) = nextBucketAverage(
                records: records,
                start: start,
                bucketIndex: i,
                bucketSize: bucketSize
            )

            // Current bucket range (bucket B)
            let currentBucketStart = start + Int(floor(Double(i) * bucketSize)) + 1
            let currentBucketEnd = min(start + Int(floor(Double(i + 1) * bucketSize)) + 1, records.endIndex)

            let pointA = records[aIndex]
            let pointAX = Double(pointA.timestamp)
            let pointAY = pointA.price

            // Hoist loop invariants outside the bucket search loop
            let dx = pointAX - avgX
            let dy = avgY - pointAY
            let c = dx * pointAY + pointAX * dy

            var maxArea = -1.0
            var maxAreaIndex = currentBucketStart

            for j in currentBucketStart..<currentBucketEnd {
                let r = records[j]
                let currentX = Double(r.timestamp)
                let currentY = r.price

                // Invariant area calculation: omits constant 0.5 multiplication and precomputes baseline
                let area = abs(currentX * dy + currentY * dx - c)

                if area > maxArea {
                    maxArea = area
                    maxAreaIndex = j
                }
            }

            sampled.append(records[maxAreaIndex])
            aIndex = maxAreaIndex
        }

        // Always include the last point
        sampled.append(records[records.endIndex - 1])
        return sampled
    }

    private static func nextBucketAverage<C: RandomAccessCollection>(
        records: C,
        start: Int,
        bucketIndex: Int,
        bucketSize: Double
    ) -> (x: Double, y: Double) where C.Element == PriceRecord, C.Index == Int {
        let nextBucketStart = start + Int(floor(Double(bucketIndex + 1) * bucketSize)) + 1
        let nextBucketEnd = min(start + Int(floor(Double(bucketIndex + 2) * bucketSize)) + 1, records.endIndex)
        let nextBucketCount = Double(nextBucketEnd - nextBucketStart)

        if nextBucketCount > 0 {
            var sumX = 0.0
            var sumY = 0.0
            for j in nextBucketStart..<nextBucketEnd {
                let r = records[j]
                sumX += Double(r.timestamp)
                sumY += r.price
            }
            return (sumX / nextBucketCount, sumY / nextBucketCount)
        } else if nextBucketStart < records.endIndex {
            let r = records[nextBucketStart]
            return (Double(r.timestamp), r.price)
        }
        return (0.0, 0.0)
    }
}
