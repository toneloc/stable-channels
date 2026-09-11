package com.stablechannels.app.util

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
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
}
