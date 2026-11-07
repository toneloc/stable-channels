package com.stablechannels.app.domain

/**
 * An email address the user chose to share, trimmed and lowercased so the same address is stored
 * once.
 */
class EmailAddress private constructor(val value: String) {
    override fun equals(other: Any?) = other is EmailAddress && other.value == value

    override fun hashCode() = value.hashCode()

    override fun toString() = value

    companion object {
        const val MAX_LENGTH = 254
        const val MAX_LOCAL_PART_LENGTH = 64

        /**
         * Returns null unless [raw] has one `@`, a non-empty local part and a dotted domain, with
         * no whitespace.
         */
        fun parse(raw: String): EmailAddress? {
            val trimmed = raw.trim().lowercase()
            if (trimmed.length > MAX_LENGTH || trimmed.any { it.isWhitespace() }) return null

            val parts = trimmed.split("@")
            if (parts.size != 2) return null
            val (local, domain) = parts
            if (local.isEmpty() || local.length > MAX_LOCAL_PART_LENGTH) return null

            val labels = domain.split(".")
            if (labels.size < 2 || labels.any { it.isEmpty() } || labels.last().length < 2) {
                return null
            }
            return EmailAddress(trimmed)
        }
    }
}
