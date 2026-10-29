package com.stablechannels.app.services

import fr.acinq.bitcoin.ByteVector
import fr.acinq.bitcoin.Transaction
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Vectors come from LDK 0.2.7 `KeysManager::possible_v2_counterparty_closed_balance_spks` for this
 * mnemonic, with the seed path of ldk-node's builder, and from rust-bitcoin signing the same sweep
 * the way the force close recovery script does.
 */
class ToRemoteKeysTest {
    private val keys =
        ToRemoteKeys.fromMnemonic(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
        )
    private val destination = hex("0014da7489f432b25150a35ea33c457cb562d9e8572c")
    private val closeTxid = "7b0abcb56f5196eaa8ec0db3ddad1e1a7980ca8c428530b95c39eb9380c89243"
    private val key45Script = "0020976a6b80c817104437bd4024cfd4b44fe495a0b8061bb61998e690b695546811"

    private fun hex(s: String) = ByteVector(s).toByteArray()

    @Test
    fun anchorScriptsMatchLdk() {
        val vectors =
            mapOf(
                0 to "002089af21e96a1c0e6d5fe67b50345a9a2eabf53ae11ca44892be0cbf3f9035761c",
                1 to "0020ebc7c7f178b97ab61b89d7e483287ea3393bea45b8770067dfb9c26f33631634",
                45 to key45Script,
                999 to "0020da1b41af0cf998322b743b0a1cf29e9a2326bef3cf32b8bb34cf5fc9e989f9a2",
            )
        for ((index, script) in vectors) assertSame(keys.keys[index], keys.match(hex(script)))
        assertEquals(ToRemoteKeys.COUNT, keys.keys.size)
    }

    @Test
    fun otherScriptsDoNotMatch() {
        assertNull(keys.match(destination))
        // LDK's P2WPKH kind for key 45, used only by channels without anchors.
        assertNull(keys.match(hex("00144b332e0a32443b65d64b77ccd431acd354591e36")))
    }

    @Test
    fun sweepMatchesReference() {
        val tx =
            ToRemoteSweep.withFee(
                keys.match(hex(key45Script))!!,
                closeTxid,
                vout = 3,
                valueSats = 94_467,
                destinationScript = destination,
                feeSats = 400,
            )
        assertEquals(
            "020000000001014392c88093eb395cb93085428cca80791a1eadddb30deca8ea96516fb5bc0a7b030000" +
                "00000100000001736f010000000000160014da7489f432b25150a35ea33c457cb562d9e8572c024730" +
                "4402203351178dc8fcb9b020100272739b6d587272c21440e2e8108c272e4945f2348e022019af35ed" +
                "69c1e357be72c7f623de90996f270c1067dc8f82e5c4a20097837e3701252103aa09948dbf86e2915c" +
                "3fa9265709d8ce05cdcc967b82b24179af2981998657cead51b200000000",
            ByteVector(Transaction.write(tx)).toHex(),
        )
    }

    @Test
    fun feeRateSweepPaysAtLeastTheRate() {
        val key = keys.match(hex(key45Script))!!
        val tx = ToRemoteSweep.build(key, closeTxid, 3, 94_467, destination, feeRateSatVb = 5L)!!
        val fee = 94_467 - tx.txOut.single().amount.sat
        assertTrue(fee >= (tx.weight() + 3) / 4 * 5L)
        assertNull(ToRemoteSweep.build(key, closeTxid, 3, 800, destination, feeRateSatVb = 5L))
    }
}
