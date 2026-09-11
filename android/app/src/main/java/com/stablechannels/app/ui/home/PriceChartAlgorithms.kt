package com.stablechannels.app.ui.home

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Path
import com.stablechannels.app.models.PriceRecord
import kotlin.math.abs
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min

/**
 * Mathematical and sampling helpers for price chart rendering.
 */
object PriceChartAlgorithms {

    /**
     * Binary search for the first index where record.timestamp >= cutoffSec.
     * Assumes records are chronologically sorted.
     */
    fun lowerBound(records: List<PriceRecord>, cutoffSec: Long): Int {
        var low = 0
        var high = records.size
        while (low < high) {
            val mid = low + (high - low) / 2
            if (records[mid].timestamp < cutoffSec) {
                low = mid + 1
            } else {
                high = mid
            }
        }
        return low
    }

    /**
     * Single-pass calculation of minimum and maximum prices with 2% margin padding.
     */
    fun minMaxPrices(records: List<PriceRecord>): Pair<Double, Double> {
        if (records.isEmpty()) return 0.0 to 100.0
        var lo = Double.POSITIVE_INFINITY
        var hi = Double.NEGATIVE_INFINITY
        for (r in records) {
            if (r.price < lo) lo = r.price
            if (r.price > hi) hi = r.price
        }
        val minP = lo * 0.98
        val maxP = hi * 1.02
        return minP to maxP
    }

    /**
     * Largest Triangle Three Buckets (LTTB) downsampling.
     * Preserves local visual extrema (peaks and valleys).
     */
    fun lttbDownsample(records: List<PriceRecord>, targetCount: Int): List<PriceRecord> {
        if (records.size <= targetCount || targetCount <= 2) {
            return records
        }

        val sampled = ArrayList<PriceRecord>(targetCount)
        sampled.add(records[0])

        val bucketSize = (records.size - 2).toDouble() / (targetCount - 2).toDouble()
        var a = 0

        for (i in 0 until (targetCount - 2)) {
            // Bucket C average
            var avgX = 0.0
            var avgY = 0.0
            val nextBucketStart = (floor((i + 1) * bucketSize) + 1).toInt()
            val nextBucketEnd = min((floor((i + 2) * bucketSize) + 1).toInt(), records.size)
            val nextBucketCount = (nextBucketEnd - nextBucketStart).toDouble()

            if (nextBucketCount > 0) {
                for (j in nextBucketStart until nextBucketEnd) {
                    avgX += records[j].timestamp.toDouble()
                    avgY += records[j].price
                }
                avgX /= nextBucketCount
                avgY /= nextBucketCount
            } else if (nextBucketStart < records.size) {
                avgX = records[nextBucketStart].timestamp.toDouble()
                avgY = records[nextBucketStart].price
            }

            // Bucket B
            val currentBucketStart = (floor(i * bucketSize) + 1).toInt()
            val currentBucketEnd = min((floor((i + 1) * bucketSize) + 1).toInt(), records.size)

            val pointA = records[a]
            val pointAX = pointA.timestamp.toDouble()
            val pointAY = pointA.price

            var maxArea = -1.0
            var maxAreaIndex = currentBucketStart

            for (j in currentBucketStart until currentBucketEnd) {
                val currentX = records[j].timestamp.toDouble()
                val currentY = records[j].price

                val area = abs((pointAX - avgX) * (currentY - pointAY) - (pointAX - currentX) * (avgY - pointAY)) * 0.5

                if (area > maxArea) {
                    maxArea = area
                    maxAreaIndex = j
                }
            }

            sampled.add(records[maxAreaIndex])
            a = maxAreaIndex
        }

        sampled.add(records[records.size - 1])
        return sampled
    }

    /**
     * Build cubic Bezier line path through projected coordinates.
     */
    fun buildSplinePath(points: List<Offset>): Path {
        val path = Path()
        if (points.isEmpty()) return path
        path.moveTo(points[0].x, points[0].y)

        if (points.size >= 4) {
            for (i in 0 until points.size - 1) {
                val p0 = points[max(i - 1, 0)]
                val p1 = points[i]
                val p2 = points[min(i + 1, points.size - 1)]
                val p3 = points[min(i + 2, points.size - 1)]

                val cp1x = p1.x + (p2.x - p0.x) / 4f
                val cp1y = p1.y + (p2.y - p0.y) / 4f
                val cp2x = p2.x - (p3.x - p1.x) / 4f
                val cp2y = p2.y - (p3.y - p1.y) / 4f

                path.cubicTo(cp1x, cp1y, cp2x, cp2y, p2.x, p2.y)
            }
        } else {
            for (i in 1 until points.size) {
                path.lineTo(points[i].x, points[i].y)
            }
        }
        return path
    }

    /**
     * Build closed area path below the spline line for gradient fill.
     */
    fun buildAreaPath(linePath: Path, width: Float, height: Float): Path {
        return Path().apply {
            addPath(linePath)
            lineTo(width, height)
            lineTo(0f, height)
            close()
        }
    }

    fun formatYAxis(price: Double): String {
        return if (price >= 1000) "$${(price / 1000).toInt()}K" else "$${price.toInt()}"
    }
}
