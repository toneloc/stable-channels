package com.stablechannels.app.util

import kotlin.math.abs

/**
 * Finds the index of the first element that satisfies the given predicate,
 * assuming elements where predicate returns false precede elements where predicate returns true.
 */
fun <T> List<T>.binarySearchPartitionPoint(predicate: (T) -> Boolean): Int {
    var low = 0
    var high = size

    while (low < high) {
        val mid = low + (high - low) / 2
        if (predicate(this[mid])) {
            high = mid
        } else {
            low = mid + 1
        }
    }

    return low
}

/**
 * Finds the first index where element >= target in a list sorted by selector.
 */
fun <T, K : Comparable<K>> List<T>.lowerBound(target: K, selector: (T) -> K): Int {
    return binarySearchPartitionPoint { selector(it) >= target }
}

/**
 * Finds the first index where element >= target in a naturally sorted list.
 */
fun <T : Comparable<T>> List<T>.lowerBound(target: T): Int {
    return lowerBound(target) { it }
}

/**
 * Finds the first index where element > target in a list sorted by selector.
 */
fun <T, K : Comparable<K>> List<T>.upperBound(target: K, selector: (T) -> K): Int {
    return binarySearchPartitionPoint { selector(it) > target }
}

/**
 * Finds the first index where element > target in a naturally sorted list.
 */
fun <T : Comparable<T>> List<T>.upperBound(target: T): Int {
    return upperBound(target) { it }
}

/**
 * Finds the nearest element to the given target key in a list sorted by the key,
 * using a caller-provided distance metric.
 */
fun <T, K : Comparable<K>, D : Comparable<D>> List<T>.binarySearchNearest(
    target: K,
    selector: (T) -> K,
    distance: (K, K) -> D
): T? {
    if (isEmpty()) return null

    val idx = lowerBound(target, selector)
    if (idx == 0) return first()
    if (idx >= size) return last()

    val prev = this[idx - 1]
    val curr = this[idx]

    return if (distance(selector(prev), target) <= distance(selector(curr), target)) prev else curr
}

/**
 * Convenience overload for Long keys (e.g. timestamps, sats).
 * Distance is computed directly in 64-bit integer arithmetic without floating-point conversions.
 * Note: Assumes difference fits within Long without overflow (e.g. timestamps or satoshi amounts).
 * For arbitrary domains with extreme opposing bounds (e.g. Long.MIN_VALUE and Long.MAX_VALUE),
 * callers should supply a custom distance closure.
 */
@JvmName("binarySearchNearestLong")
fun <T> List<T>.binarySearchNearest(
    target: Long,
    selector: (T) -> Long
): T? {
    return binarySearchNearest(target, selector) { a, b -> if (a >= b) a - b else b - a }
}

/**
 * Convenience overload for Int keys.
 * Distance is computed directly in integer arithmetic without floating-point conversions.
 * Note: Assumes difference fits within Int without overflow.
 */
@JvmName("binarySearchNearestInt")
fun <T> List<T>.binarySearchNearest(
    target: Int,
    selector: (T) -> Int
): T? {
    return binarySearchNearest(target, selector) { a, b -> if (a >= b) a - b else b - a }
}

/**
 * Convenience overload for Double keys.
 */
@JvmName("binarySearchNearestDouble")
fun <T> List<T>.binarySearchNearest(
    target: Double,
    selector: (T) -> Double
): T? {
    return binarySearchNearest(target, selector) { a, b -> abs(a - b) }
}

/**
 * Convenience overload for Float keys.
 */
@JvmName("binarySearchNearestFloat")
fun <T> List<T>.binarySearchNearest(
    target: Float,
    selector: (T) -> Float
): T? {
    return binarySearchNearest(target, selector) { a, b -> abs(a - b) }
}

/**
 * Convenience overload for Date keys.
 */
@JvmName("binarySearchNearestDate")
fun <T> List<T>.binarySearchNearest(
    target: java.util.Date,
    selector: (T) -> java.util.Date
): T? {
    return binarySearchNearest(target, selector) { a, b -> abs(a.time - b.time) }
}
