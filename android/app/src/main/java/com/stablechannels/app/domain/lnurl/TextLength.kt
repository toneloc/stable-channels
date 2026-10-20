package com.stablechannels.app.domain.lnurl

import java.text.BreakIterator

/**
 * Number of user-perceived characters (grapheme clusters), so an emoji sequence or a letter with
 * combining marks counts once, as LNURL length limits are meant and as iOS counts them.
 */
internal fun String.graphemeCount(): Int {
    val iterator = BreakIterator.getCharacterInstance().also { it.setText(this) }
    var count = 0
    while (iterator.next() != BreakIterator.DONE) count++
    return count
}
