package com.stablechannels.app.util

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.abs

class BinarySearchTest {

    data class SamplePoint(val x: Double, val y: Double)

    @Test
    fun `lowerBound on comparable list`() {
        val numbers = listOf(10, 20, 30, 40, 50)
        assertEquals(0, numbers.lowerBound(5))
        assertEquals(0, numbers.lowerBound(10))
        assertEquals(2, numbers.lowerBound(25))
        assertEquals(2, numbers.lowerBound(30))
        assertEquals(4, numbers.lowerBound(50))
        assertEquals(5, numbers.lowerBound(55))
    }

    @Test
    fun `upperBound on comparable list`() {
        val numbers = listOf(10, 20, 20, 30, 40)
        assertEquals(0, numbers.upperBound(5))
        assertEquals(1, numbers.upperBound(10))
        assertEquals(3, numbers.upperBound(20))
        assertEquals(4, numbers.upperBound(30))
        assertEquals(5, numbers.upperBound(40))
    }

    @Test
    fun `binarySearchNearest on points list`() {
        val points = listOf(
            SamplePoint(100.0, 1.0),
            SamplePoint(200.0, 2.0),
            SamplePoint(300.0, 3.0),
            SamplePoint(400.0, 4.0)
        )

        assertEquals(100.0, points.binarySearchNearest(50.0) { it.x }?.x ?: 0.0, 0.001)
        assertEquals(100.0, points.binarySearchNearest(140.0) { it.x }?.x ?: 0.0, 0.001)
        assertEquals(200.0, points.binarySearchNearest(160.0) { it.x }?.x ?: 0.0, 0.001)
        assertEquals(300.0, points.binarySearchNearest(290.0) { it.x }?.x ?: 0.0, 0.001)
        assertEquals(400.0, points.binarySearchNearest(450.0) { it.x }?.x ?: 0.0, 0.001)
    }

    @Test
    fun `binarySearchNearest on dates list`() {
        data class TimedEvent(val date: java.util.Date, val id: Int)
        val events = listOf(
            TimedEvent(java.util.Date(100), 1),
            TimedEvent(java.util.Date(200), 2),
            TimedEvent(java.util.Date(300), 3)
        )
        assertEquals(1, events.binarySearchNearest(java.util.Date(140)) { it.date }?.id)
        assertEquals(2, events.binarySearchNearest(java.util.Date(160)) { it.date }?.id)
    }

    @Test
    fun `binarySearchNearest with custom distance function`() {
        data class Item(val name: String, val length: Int)
        val items = listOf(
            Item("a", 1),
            Item("ccc", 3),
            Item("ffffff", 6)
        )
        val nearest = items.binarySearchNearest(4, { it.length }) { a: Int, b: Int -> abs(a - b) }
        assertEquals(3, nearest?.length)
    }

    @Test
    fun `binarySearchNearest on Long integers beyond Double precision`() {
        // Values above 2^53 (9_007_199_254_740_992L) where Double loses unit precision for odd integers
        val values = listOf(
            9_007_199_254_740_993L,
            9_007_199_254_740_995L,
            9_007_199_254_740_997L
        )
        assertEquals(9_007_199_254_740_995L, values.binarySearchNearest(9_007_199_254_740_995L) { it })
        assertEquals(9_007_199_254_740_997L, values.binarySearchNearest(9_007_199_254_740_997L) { it })
        assertEquals(9_007_199_254_740_993L, values.binarySearchNearest(9_007_199_254_740_994L) { it })
    }

    @Test
    fun `empty list returns boundary defaults`() {
        val empty = emptyList<Int>()
        assertEquals(0, empty.lowerBound(10))
        assertEquals(0, empty.upperBound(10))
        val target: Int = 10
        assertNull(empty.binarySearchNearest(target) { it })
    }

    @Test
    fun `single-element collection edge cases`() {
        val single = listOf(42L)
        assertEquals(0, single.lowerBound(10L))
        assertEquals(0, single.lowerBound(42L))
        assertEquals(1, single.lowerBound(50L))

        assertEquals(0, single.upperBound(10L))
        assertEquals(1, single.upperBound(42L))
        assertEquals(1, single.upperBound(50L))

        assertEquals(42L, single.binarySearchNearest(10L) { it })
        assertEquals(42L, single.binarySearchNearest(42L) { it })
        assertEquals(42L, single.binarySearchNearest(50L) { it })

        // Also test Int overload with explicit Int variable
        val singleInt = listOf(42)
        val targetInt: Int = 10
        assertEquals(42, singleInt.binarySearchNearest(targetInt) { x: Int -> x })
    }

    @Test
    fun `duplicate and repeated elements lowerBound and upperBound`() {
        val list = listOf(10L, 20L, 20L, 20L, 30L)
        assertEquals(1, list.lowerBound(20L))
        assertEquals(4, list.upperBound(20L))
        assertEquals(1, list.lowerBound(15L))
        assertEquals(1, list.upperBound(15L))
        assertEquals(4, list.lowerBound(25L))
        assertEquals(4, list.upperBound(25L))
        assertEquals(20L, list.binarySearchNearest(20L) { it })
    }

    @Test
    fun `all identical elements`() {
        val list = listOf(5L, 5L, 5L, 5L, 5L)
        assertEquals(0, list.lowerBound(5L))
        assertEquals(5, list.upperBound(5L))
        assertEquals(0, list.lowerBound(1L))
        assertEquals(0, list.upperBound(1L))
        assertEquals(5, list.lowerBound(10L))
        assertEquals(5, list.upperBound(10L))
        assertEquals(5L, list.binarySearchNearest(5L) { it })
    }

    @Test
    fun `nearest boundary targets and tie breaking`() {
        val list = listOf(10L, 20L, 30L)
        assertEquals(10L, list.binarySearchNearest(-100L) { it })
        assertEquals(30L, list.binarySearchNearest(1000L) { it })

        // Equidistant tie-breaking should return one of the adjacent values deterministically without crashing
        val pair = listOf(10L, 20L)
        val nearest = pair.binarySearchNearest(15L) { it }
        assertTrue(nearest == 10L || nearest == 20L)
    }

    @Test
    fun `large monotonic collection binary search accuracy`() {
        val large = (0 until 1000).map { it * 2L } // 0L, 2L, 4L, ..., 1998L
        assertEquals(200, large.lowerBound(400L))
        assertEquals(201, large.lowerBound(401L))
        assertEquals(201, large.upperBound(400L))
        assertEquals(201, large.upperBound(401L))
        assertEquals(400L, large.binarySearchNearest(400L) { it })
        assertEquals(400L, large.binarySearchNearest(400.4) { it.toDouble() })
        assertEquals(402L, large.binarySearchNearest(401.6) { it.toDouble() })
    }
}
