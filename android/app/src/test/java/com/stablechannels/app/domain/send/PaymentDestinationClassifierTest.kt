package com.stablechannels.app.domain.send

import com.stablechannels.app.domain.send.DestinationClassification.Empty
import com.stablechannels.app.domain.send.DestinationClassification.Invalid
import com.stablechannels.app.domain.send.DestinationClassification.Valid
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/** Decodes only the strings registered here; JVM tests cannot load the node library. */
private class FakeLightningInvoiceDecoder(
    private val invoices: Map<String, DecodedBolt11>,
    private val offers: Map<String, DecodedBolt12>,
) : LightningInvoiceDecoder {
    override fun decodeBolt11(raw: String): DecodedBolt11? = invoices[raw]

    override fun decodeBolt12(raw: String): DecodedBolt12? = offers[raw]
}

class PaymentDestinationClassifierTest {
    private companion object {
        const val MAINNET_INVOICE =
            "lnbc1pn8g249pp5f6ytj32ty90jhvw69enf30hwfgdhyymjewywcmfjevflg6s4z86qdqqcqzzgxqyz5vqrzjqwnvuc0u4txn35cafc7w94gxvq5p3cu9dd95f7hlrh0fvs46wpvhdfjjzh2j9f7ye5qqqqryqqqqthqqpysp5mm832athgcal3m7h35sc29j63lmgzvwc5smfjh2es65elc2ns7dq9qrsgqu2xcje2gsnjp0wn97aknyd3h58an7sjj6nhcrm40846jxphv47958c6th76whmec8ttr2wmg6sxwchvxmsc00kqrzqcga6lvsf9jtqgqy5yexa"
        const val MAINNET_INVOICE_WITH_AMOUNT = "lnbc2500u1fakeinvoicewithamount"
        const val TESTNET_INVOICE = "lntb1faketestnetinvoice"
        const val MAINNET_OFFER =
            "lno1pgx9getnwss8vetrw3hhyuckyypwa3eyt44h6txtxquqh7lz5djge4afgfjn7k4rgrkuag0jsd5xvxg"
        const val TESTNET_OFFER = "lno1faketestnetoffer"
        const val SEGWIT = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        const val TAPROOT = "bc1plvty62zgr8ae8vat0xyqrae0qgnlmcra5nnhpamhn46kdjuwh43q0wes3n"
        const val TESTNET_SEGWIT = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"
        const val P2PKH = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"
    }

    private val classifier =
        PaymentDestinationClassifier(
            FakeLightningInvoiceDecoder(
                invoices =
                    mapOf(
                        MAINNET_INVOICE to DecodedBolt11(BitcoinNetwork.BITCOIN, null),
                        MAINNET_INVOICE_WITH_AMOUNT to
                            DecodedBolt11(BitcoinNetwork.BITCOIN, 250_000_000L),
                        TESTNET_INVOICE to DecodedBolt11(BitcoinNetwork.TESTNET, null),
                    ),
                offers =
                    mapOf(
                        MAINNET_OFFER to DecodedBolt12(emptySet()),
                        TESTNET_OFFER to DecodedBolt12(setOf(BitcoinNetwork.TESTNET)),
                    ),
            )
        )

    private fun valid(input: String, network: BitcoinNetwork? = null): SendDestination {
        val result = classifier.classify(input, network)
        if (result !is Valid) fail("Expected Valid for '$input', got $result")
        return (result as Valid).destination
    }

    private inline fun <reified T : SendDestination> validAs(
        input: String,
        network: BitcoinNetwork? = null,
    ): T {
        val destination = valid(input, network)
        if (destination !is T) fail("Expected ${T::class.simpleName}, got $destination")
        return destination as T
    }

    private fun assertInvalid(input: String, network: BitcoinNetwork? = null) {
        val result = classifier.classify(input, network)
        assertTrue("Expected Invalid for '$input', got $result", result is Invalid)
    }

    private fun assertRejected(
        expected: DestinationRejection,
        input: String,
        network: BitcoinNetwork? = null,
    ) {
        assertEquals(Invalid(expected), classifier.classify(input, network))
    }

    @Test
    fun `classifies lightning address`() {
        val dest = validAs<SendDestination.LightningAddress>("alice@lightning.example.com")

        assertEquals("alice", dest.handle)
        assertEquals("lightning.example.com", dest.domain)
        assertEquals(
            "https://lightning.example.com/.well-known/lnurlp/alice",
            dest.url.toString(),
        )
        assertTrue(dest.requiresManualAmount)
    }

    @Test
    fun `classifies onchain taproot address`() {
        val dest = validAs<SendDestination.Onchain>(TAPROOT)

        assertEquals(TAPROOT, dest.address)
        assertNull(dest.amountSats)
        assertTrue(dest.requiresManualAmount)
    }

    @Test
    fun `classifies bip21 onchain address without amount`() {
        val dest = validAs<SendDestination.Onchain>("bitcoin:$TAPROOT")

        assertEquals(TAPROOT, dest.address)
        assertNull(dest.amountSats)
        assertTrue(dest.requiresManualAmount)
    }

    @Test
    fun `bip21 preserves amount in sats`() {
        val dest = validAs<SendDestination.Onchain>("bitcoin:$SEGWIT?amount=0.001")

        assertEquals(SEGWIT, dest.address)
        assertEquals(100_000L, dest.amountSats)
        assertFalse(dest.requiresManualAmount)
    }

    @Test
    fun `bip21 decimal precision and extra params`() {
        assertEquals(
            1L,
            validAs<SendDestination.Onchain>("bitcoin:$SEGWIT?amount=0.00000001&label=test")
                .amountSats,
        )
        assertEquals(
            150_000_000L,
            validAs<SendDestination.Onchain>("bitcoin:$SEGWIT?amount=1.5").amountSats,
        )
        assertNull(validAs<SendDestination.Onchain>("bitcoin:$SEGWIT?amount=-0.5").amountSats)
    }

    @Test
    fun `empty and whitespace input is empty`() {
        assertEquals(Empty, classifier.classify(""))
        assertEquals(Empty, classifier.classify("   \n "))
    }

    @Test
    fun `garbage input is unrecognized`() {
        assertRejected(DestinationRejection.UNRECOGNIZED, "random_garbage_string")
    }

    @Test
    fun `unsupported lightning prefix is rejected`() {
        assertRejected(DestinationRejection.UNSUPPORTED_LIGHTNING_PREFIX, "lnxyz1234567890")
    }

    @Test
    fun `bip21 with lightning param prefers lightning and takes uri amount`() {
        val dest =
            validAs<SendDestination.Bolt11>(
                "bitcoin:$SEGWIT?amount=0.001&lightning=$MAINNET_INVOICE"
            )

        assertEquals(MAINNET_INVOICE, dest.raw)
        assertEquals(100_000_000L, dest.amountMsat)
    }

    @Test
    fun `bip21 lightning invoice with its own amount keeps invoice amount`() {
        val dest =
            validAs<SendDestination.Bolt11>(
                "bitcoin:$SEGWIT?amount=0.001&lightning=$MAINNET_INVOICE_WITH_AMOUNT"
            )

        assertEquals(250_000_000L, dest.amountMsat)
    }

    @Test
    fun `bip21 with invalid lightning param falls back to onchain`() {
        val dest =
            validAs<SendDestination.Onchain>(
                "bitcoin:$SEGWIT?amount=0.001&lightning=invalid_invoice"
            )

        assertEquals(SEGWIT, dest.address)
        assertEquals(100_000L, dest.amountSats)
    }

    @Test
    fun `bip21 uppercase scheme`() {
        assertEquals(SEGWIT, validAs<SendDestination.Onchain>("BITCOIN:$SEGWIT").address)
    }

    @Test
    fun `lightning colon and double slash prefixes`() {
        assertEquals(
            MAINNET_INVOICE,
            validAs<SendDestination.Bolt11>("lightning:$MAINNET_INVOICE").raw,
        )
        assertEquals(
            MAINNET_INVOICE,
            validAs<SendDestination.Bolt11>("lightning://$MAINNET_INVOICE").raw,
        )
    }

    @Test
    fun `bare bolt11 without amount requires manual amount`() {
        val dest = validAs<SendDestination.Bolt11>(MAINNET_INVOICE)

        assertNull(dest.amountMsat)
        assertTrue(dest.requiresManualAmount)
    }

    @Test
    fun `unknown bolt11 is invalid`() {
        assertRejected(DestinationRejection.INVALID_BOLT11, "lnbc1notarealinvoice")
    }

    @Test
    fun `classifies lnurl pay bech32`() {
        val dest =
            validAs<SendDestination.LnurlPay>(
                "lnurl1dp68gurn8ghj7um9wfmxjcm99e3k7mf0v9cxjtmkxyhkcmn4wfkz7urp0yvwqajv"
            )

        assertEquals("service.com", dest.url.host)
    }

    @Test
    fun `legacy and testnet onchain addresses`() {
        val p2sh = "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy"
        val regtest = "bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080"

        for (address in listOf(P2PKH, p2sh, TESTNET_SEGWIT, regtest)) {
            assertEquals(
                Valid(SendDestination.Onchain(address, null)),
                classifier.classify(address),
            )
        }
    }

    @Test
    fun `invalid address lengths and characters`() {
        assertInvalid("1A1zP1eP5QGefi2DMPTfTL5SL")
        assertInvalid("bc1" + "q".repeat(88))
        assertInvalid("bc1qar0srrr7xfkvy5l643!@#\$dnw9re59gtzzwf5mdq")
    }

    @Test
    fun `lightning address edge cases`() {
        val dest = validAs<SendDestination.LightningAddress>("alice+tips@lightning.example.com")
        assertEquals("alice+tips", dest.handle)
        assertEquals("lightning.example.com", dest.domain)
        assertEquals(
            "https://lightning.example.com/.well-known/lnurlp/alice+tips",
            dest.url.toString(),
        )

        assertInvalid("alice @lightning.example.com")
        assertInvalid("@lightning.example.com")
        assertInvalid("alice@")
    }

    @Test
    fun `bip21 double slash with base58 address preserves case`() {
        val dest = validAs<SendDestination.Onchain>("bitcoin://$P2PKH?amount=0.5")

        assertEquals(P2PKH, dest.address)
        assertEquals(50_000_000L, dest.amountSats)
    }

    @Test
    fun `lightning address handle starting with ln`() {
        val dest = validAs<SendDestination.LightningAddress>("lnbits@example.com")
        assertEquals("lnbits", dest.handle)
        assertEquals("example.com", dest.domain)
        assertEquals("https://example.com/.well-known/lnurlp/lnbits", dest.url.toString())

        assertEquals(
            "lnurl",
            validAs<SendDestination.LightningAddress>("lnurl@service.com").handle,
        )
    }

    @Test
    fun `uppercase segwit address is lowercased`() {
        val upper = SEGWIT.uppercase()

        assertEquals(SEGWIT, validAs<SendDestination.Onchain>(upper).address)
    }

    @Test
    fun `bip21 without onchain address uses lightning param`() {
        assertEquals(
            MAINNET_INVOICE,
            validAs<SendDestination.Bolt11>("bitcoin:?lightning=$MAINNET_INVOICE").raw,
        )
    }

    @Test
    fun `bip21 drops sub satoshi and over cap amounts`() {
        assertNull(
            validAs<SendDestination.Onchain>("bitcoin:$SEGWIT?amount=0.000000015").amountSats
        )
        assertNull(validAs<SendDestination.Onchain>("bitcoin:$SEGWIT?amount=21000001.0").amountSats)
    }

    @Test
    fun `bip21 drops exponent amount`() {
        assertNull(validAs<SendDestination.Onchain>("bitcoin:$SEGWIT?amount=1e3").amountSats)
    }

    @Test
    fun `bip21 with invalid onchain address is invalid onchain address`() {
        assertRejected(DestinationRejection.INVALID_ONCHAIN_ADDRESS, "bitcoin:bc1qnotanaddress")
    }

    @Test
    fun `onchain network mismatch is rejected`() {
        assertEquals(
            Valid(SendDestination.Onchain(SEGWIT, null)),
            classifier.classify(SEGWIT, BitcoinNetwork.BITCOIN),
        )
        assertRejected(DestinationRejection.WRONG_NETWORK, TESTNET_SEGWIT, BitcoinNetwork.BITCOIN)
        assertRejected(DestinationRejection.WRONG_NETWORK, SEGWIT, BitcoinNetwork.TESTNET)
    }

    @Test
    fun `signet accepts testnet segwit address`() {
        assertEquals(
            TESTNET_SEGWIT,
            validAs<SendDestination.Onchain>(TESTNET_SEGWIT, BitcoinNetwork.SIGNET).address,
        )
    }

    @Test
    fun `bip21 onchain network mismatch is rejected`() {
        assertRejected(
            DestinationRejection.WRONG_NETWORK,
            "bitcoin:$TESTNET_SEGWIT",
            BitcoinNetwork.BITCOIN,
        )
    }

    @Test
    fun `bolt11 network mismatch is rejected`() {
        assertRejected(DestinationRejection.WRONG_NETWORK, MAINNET_INVOICE, BitcoinNetwork.TESTNET)
        assertRejected(DestinationRejection.WRONG_NETWORK, TESTNET_INVOICE, BitcoinNetwork.BITCOIN)
        assertEquals(
            TESTNET_INVOICE,
            validAs<SendDestination.Bolt11>(TESTNET_INVOICE, BitcoinNetwork.TESTNET).raw,
        )
    }

    @Test
    fun `classifies valid bolt12 offer`() {
        val dest = validAs<SendDestination.Bolt12>(MAINNET_OFFER)

        assertEquals(MAINNET_OFFER, dest.raw)
        assertTrue(dest.requiresManualAmount)
    }

    @Test
    fun `offer with empty chains counts as mainnet`() {
        assertEquals(
            MAINNET_OFFER,
            validAs<SendDestination.Bolt12>(MAINNET_OFFER, BitcoinNetwork.BITCOIN).raw,
        )
    }

    @Test
    fun `bolt12 network mismatch is rejected`() {
        assertRejected(DestinationRejection.WRONG_NETWORK, MAINNET_OFFER, BitcoinNetwork.TESTNET)
        assertRejected(DestinationRejection.WRONG_NETWORK, TESTNET_OFFER, BitcoinNetwork.BITCOIN)
    }

    @Test
    fun `unknown bolt12 is invalid`() {
        assertRejected(DestinationRejection.INVALID_BOLT12, "lno1notarealoffer")
    }

    @Test
    fun `bip21 unhandled req parameter is rejected`() {
        assertRejected(
            DestinationRejection.UNHANDLED_REQUIRED_BIP21_PARAMETER,
            "bitcoin:$SEGWIT?amount=0.01&req-custom-feature=true",
        )
    }

    @Test
    fun `bip21 duplicate parameter is rejected`() {
        assertRejected(
            DestinationRejection.DUPLICATE_BIP21_PARAMETER,
            "bitcoin:$SEGWIT?amount=0.01&amount=0.02",
        )
    }

    @Test
    fun `bip21 pop parameter is rejected`() {
        assertRejected(
            DestinationRejection.UNHANDLED_REQUIRED_BIP21_PARAMETER,
            "bitcoin:$SEGWIT?pop=https://example.com/pop",
        )
    }

    @Test
    fun `bip21 with lno param uses bolt12 offer`() {
        assertEquals(
            MAINNET_OFFER,
            validAs<SendDestination.Bolt12>(
                    "bitcoin:$SEGWIT?lno=$MAINNET_OFFER",
                    BitcoinNetwork.BITCOIN,
                )
                .raw,
        )
    }

    @Test
    fun `lightning address with numeric tld keeps its domain`() {
        val result =
            PaymentDestinationClassifier(FakeLightningInvoiceDecoder(emptyMap(), emptyMap()))
                .classify("a@example.123")
        val destination = (result as Valid).destination as SendDestination.LightningAddress
        assertEquals("example.123", destination.domain)
    }
}
