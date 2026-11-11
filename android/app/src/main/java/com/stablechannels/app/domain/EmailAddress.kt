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
         * Characters the local part may use besides letters and digits; the server accepts the same
         * set.
         */
        private const val LOCAL_SYMBOLS = "!#$%&'*+/=?^_`{|}~-"

        /**
         * Returns null unless [raw] is a dot-separated local part of plain address characters, one
         * `@`, and a dotted ASCII domain whose last label has two or more characters, the same rule
         * the server applies.
         */
        fun parse(raw: String): EmailAddress? {
            val trimmed = raw.trim().lowercase()
            if (trimmed.length > MAX_LENGTH) return null

            val parts = trimmed.split("@")
            if (parts.size != 2) return null
            val (local, domain) = parts
            if (local.length > MAX_LOCAL_PART_LENGTH) return null
            val localOk =
                local.split(".").all { part ->
                    part.isNotEmpty() && part.all { isAlphanumeric(it) || it in LOCAL_SYMBOLS }
                }
            if (!localOk) return null

            val labels = domain.split(".")
            if (labels.size < 2 || !labels.all(::isDomainLabel) || labels.last().length < 2) {
                return null
            }
            return EmailAddress(trimmed)
        }

        private fun isAlphanumeric(c: Char) = c in 'a'..'z' || c in '0'..'9'

        private fun isDomainLabel(label: String) =
            label.isNotEmpty() &&
                !label.startsWith('-') &&
                !label.endsWith('-') &&
                label.all { isAlphanumeric(it) || it == '-' }
    }
}
