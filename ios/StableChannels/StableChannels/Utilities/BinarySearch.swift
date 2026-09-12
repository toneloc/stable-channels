import Foundation

extension RandomAccessCollection {
    /// Returns the index of the first element for which the predicate returns false,
    /// assuming the collection is partitioned such that elements where predicate returns
    /// true precede elements where predicate returns false.
    func binarySearchPartitionPoint(where predicate: (Element) -> Bool) -> Index {
        var low = startIndex
        var count = self.count

        while count > 0 {
            let step = count / 2
            let mid = index(low, offsetBy: step)

            if predicate(self[mid]) {
                low = index(after: mid)
                count -= step + 1
            } else {
                count = step
            }
        }

        return low
    }

    /// Finds the first index where element is not ordered before target.
    func lowerBound(
        by areInIncreasingOrder: (Element, Element) -> Bool,
        for target: Element
    ) -> Index {
        binarySearchPartitionPoint { areInIncreasingOrder($0, target) }
    }

    /// Finds the first index where target is ordered before element.
    func upperBound(
        by areInIncreasingOrder: (Element, Element) -> Bool,
        for target: Element
    ) -> Index {
        binarySearchPartitionPoint { !areInIncreasingOrder(target, $0) }
    }

    /// Finds the first index where element key >= target in a collection sorted by key.
    func lowerBound<K: Comparable>(
        target: K,
        keySelector: (Element) -> K
    ) -> Index {
        binarySearchPartitionPoint { keySelector($0) < target }
    }

    /// Finds the first index where element key > target in a collection sorted by key.
    func upperBound<K: Comparable>(
        target: K,
        keySelector: (Element) -> K
    ) -> Index {
        binarySearchPartitionPoint { keySelector($0) <= target }
    }
}

extension RandomAccessCollection where Element: Comparable {
    /// Finds the first index where element >= target.
    func lowerBound(for target: Element) -> Index {
        lowerBound(by: <, for: target)
    }

    /// Finds the first index where element > target.
    func upperBound(for target: Element) -> Index {
        upperBound(by: <, for: target)
    }
}

extension RandomAccessCollection {
    /// Finds the nearest element to the given target key in a collection sorted by the key,
    /// using a caller-provided distance metric.
    func binarySearchNearest<K: Comparable>(
        target: K,
        keySelector: (Element) -> K,
        distance: (K, K) -> some Comparable
    ) -> Element? {
        guard !isEmpty else { return nil }

        let idx = binarySearchPartitionPoint { keySelector($0) < target }

        if idx == startIndex {
            return self[startIndex]
        }
        if idx == endIndex {
            return self[index(before: endIndex)]
        }

        let prevIdx = index(before: idx)
        let prev = self[prevIdx]
        let curr = self[idx]

        return distance(keySelector(prev), target) <= distance(keySelector(curr), target) ? prev : curr
    }

    /// Convenience overload for Date keys.
    func binarySearchNearest(
        target: Date,
        keySelector: (Element) -> Date
    ) -> Element? {
        binarySearchNearest(target: target, keySelector: keySelector) { abs($0.timeIntervalSince($1)) }
    }

    /// Convenience overload for BinaryFloatingPoint keys.
    func binarySearchNearest<K: BinaryFloatingPoint>(
        target: K,
        keySelector: (Element) -> K
    ) -> Element? {
        binarySearchNearest(target: target, keySelector: keySelector) { abs($0 - $1) }
    }

    /// Convenience overload for BinaryInteger keys (e.g. Int, Int64, UInt64 timestamps, block heights, or counts).
    /// Distance is calculated directly in integer space (a >= b ? a - b : b - a) without floating-point conversions.
    /// Note: Assumes values reside within a non-overflowing difference range. For arbitrary wide opposite-signed
    /// domains,
    /// callers should provide a custom distance or widening closure.
    func binarySearchNearest<K: BinaryInteger>(
        target: K,
        keySelector: (Element) -> K
    ) -> Element? {
        binarySearchNearest(target: target, keySelector: keySelector) { $0 >= $1 ? $0 - $1 : $1 - $0 }
    }
}
