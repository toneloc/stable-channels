package com.stablechannels.app.util

object InputSanitizer {
    /**
     * Keeps digits + at most one dot, trims excess decimals, strips leading zeros, prepends zero to leading dot.
     * "00012.3a." with maxDecimals: 2 -> "12.3", "." -> "0.", ".5" -> "0.5", "" -> "".
     */
    fun decimal(raw: String, maxDecimals: Int = 2): String {
        var start = 0
        while (start < raw.length && raw[start] == '0' && (raw.length - start) > 1) {
            if (start + 1 < raw.length && raw[start + 1] == '.') {
                break
            }
            start++
        }

        val sb = StringBuilder(raw.length - start + 2)
        var seenDot = false
        var decimals = 0

        for (i in start until raw.length) {
            val ch = raw[i]
            if (ch.isDigit()) {
                if (seenDot) {
                    decimals++
                    if (decimals > maxDecimals) {
                        continue
                    }
                }
                sb.append(ch)
            } else if (ch == '.' && !seenDot) {
                seenDot = true
                if (maxDecimals > 0) {
                    sb.append(ch)
                }
            }
        }

        if (sb.isEmpty()) {
            return ""
        }
        if (sb.startsWith(".")) {
            return "0$sb"
        }
        return sb.toString()
    }
}
