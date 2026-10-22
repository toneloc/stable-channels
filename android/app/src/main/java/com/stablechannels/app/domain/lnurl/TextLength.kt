package com.stablechannels.app.domain.lnurl

/**
 * Number of user-perceived characters (grapheme clusters), so an emoji sequence or a letter with
 * combining marks counts once, as LNURL length limits are meant and as iOS counts them.
 */
internal fun String.graphemeCount(): Int = GRAPHEME.findAll(this).count()

/**
 * `\X` matches one extended grapheme cluster on every supported JDK and on Android.
 * `java.text.BreakIterator` does not: before JDK 20 it splits emoji joined by zero-width joiners.
 */
private val GRAPHEME = Regex("""\X""")
