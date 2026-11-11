package com.stablechannels.app.services

import fr.acinq.bitcoin.ByteVector
import fr.acinq.bitcoin.DeterministicWallet
import fr.acinq.bitcoin.MnemonicCode
import fr.acinq.bitcoin.OP_1
import fr.acinq.bitcoin.OP_CHECKSEQUENCEVERIFY
import fr.acinq.bitcoin.OP_CHECKSIGVERIFY
import fr.acinq.bitcoin.OP_PUSHDATA
import fr.acinq.bitcoin.PrivateKey
import fr.acinq.bitcoin.Script

/**
 * The keys LDK pays our side of a channel to when the LSP force-closes it (#364). ldk-node turns on
 * LDK's v2 remote key derivation, which uses one of 1000 keys, m/8'/N' of the LDK seed, so these
 * funds can be found and spent with the wallet seed alone after the channel data is lost. The LDK
 * seed is the private key of the BIP32 master of the wallet's 64-byte seed (ldk-node builder.rs).
 * Same derivation as the force close recovery script. The app only opens anchor channels, so only
 * the anchor output kind is covered.
 */
class ToRemoteKeys private constructor(ldkSeed: ByteArray) {
    class Key(val index: Int, val privateKey: PrivateKey) {
        /** `<pubkey> OP_CHECKSIGVERIFY 1 OP_CSV`, paid to as P2WSH. */
        val witnessScript: ByteArray =
            Script.write(
                listOf(
                    OP_PUSHDATA(privateKey.publicKey()),
                    OP_CHECKSIGVERIFY,
                    OP_1,
                    OP_CHECKSEQUENCEVERIFY,
                )
            )
        val scriptPubKey: ByteArray = Script.write(Script.pay2wsh(witnessScript))
    }

    val keys: List<Key>
    private val byScript: Map<ByteVector, Key>

    init {
        val base =
            DeterministicWallet.generate(ldkSeed).derivePrivateKey(DeterministicWallet.hardened(8))
        keys =
            (0 until COUNT).map {
                Key(it, base.derivePrivateKey(DeterministicWallet.hardened(it.toLong())).privateKey)
            }
        byScript = keys.associateBy { ByteVector(it.scriptPubKey) }
    }

    fun match(scriptPubKey: ByteArray): Key? = byScript[ByteVector(scriptPubKey)]

    companion object {
        const val COUNT = 1000

        /**
         * [seed] is the 64-byte wallet seed: the BIP39 seed, or the keys_seed file of old wallets.
         */
        fun fromSeed(seed: ByteArray) =
            ToRemoteKeys(DeterministicWallet.generate(seed).privateKey.value.toByteArray())

        fun fromMnemonic(words: String) =
            fromSeed(MnemonicCode.toSeed(words.trim().split(Regex("\\s+")), ""))
    }
}
