package com.stablechannels.app.domain.send

/** Validation and normalisation of on-chain Bitcoin addresses. */
object OnchainAddress {
    private val TEST_NETWORKS = setOf(BitcoinNetwork.TESTNET, BitcoinNetwork.SIGNET)
    private val BASE58_TEST_NETWORKS = TEST_NETWORKS + BitcoinNetwork.REGTEST

    /** True when [lower] starts like an address, so a failed check means "invalid address". */
    fun looksLikeAddress(lower: String): Boolean =
        listOf("bc1", "tb1", "bcrt1", "1", "3", "m", "n", "2").any { lower.startsWith(it) }

    /**
     * True when [address] is a valid segwit, taproot or Base58 address. With a [network], the
     * address must also belong to it; signet shares the testnet prefixes.
     */
    fun isValid(address: String, network: BitcoinNetwork? = null): Boolean {
        if (address.length !in 26..90) return false
        val lower = address.lowercase()
        fun allows(vararg networks: BitcoinNetwork) = network == null || network in networks

        return when {
            lower.startsWith("bc1") ->
                allows(BitcoinNetwork.BITCOIN) && Bech32.isValidSegwitAddress(address, "bc")
            lower.startsWith("tb1") ->
                allows(*TEST_NETWORKS.toTypedArray()) && Bech32.isValidSegwitAddress(address, "tb")
            lower.startsWith("bcrt1") ->
                allows(BitcoinNetwork.REGTEST) && Bech32.isValidSegwitAddress(address, "bcrt")
            address.startsWith("1") || address.startsWith("3") ->
                allows(BitcoinNetwork.BITCOIN) && isValidBase58(address)
            address.startsWith("m") || address.startsWith("n") || address.startsWith("2") ->
                allows(*BASE58_TEST_NETWORKS.toTypedArray()) && isValidBase58(address)
            else -> false
        }
    }

    private fun isValidBase58(address: String): Boolean =
        address.length in 26..35 && Base58Check.isValidAddress(address)

    /**
     * Lowercases segwit and taproot addresses (BIP-173/BIP-350 allow either case) and keeps Base58
     * addresses exactly as typed, since their case is significant.
     */
    fun normalize(address: String): String {
        val trimmed = address.trim()
        val lower = trimmed.lowercase()
        val isSegwit = listOf("bc1", "tb1", "bcrt1").any { lower.startsWith(it) }
        return if (isSegwit) lower else trimmed
    }
}
