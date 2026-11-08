package com.stablechannels.app.services

import fr.acinq.bitcoin.ByteVector
import fr.acinq.bitcoin.OutPoint
import fr.acinq.bitcoin.Satoshi
import fr.acinq.bitcoin.ScriptWitness
import fr.acinq.bitcoin.SigHash
import fr.acinq.bitcoin.SigVersion
import fr.acinq.bitcoin.Transaction
import fr.acinq.bitcoin.TxId
import fr.acinq.bitcoin.TxIn
import fr.acinq.bitcoin.TxOut

/**
 * A 1-in 1-out transaction that moves a found force close output to the wallet (#364), built like
 * the force close recovery script: version 2 and sequence 1 for the output's 1 block lock (1
 * OP_CSV), a BIP143 SIGHASH_ALL signature, and the witness `[sig, witnessScript]`.
 */
object ToRemoteSweep {
    /** Below this the output would be dust for any wallet address type. */
    const val DUST_SATS = 330L

    fun withFee(
        key: ToRemoteKeys.Key,
        txid: String,
        vout: Int,
        valueSats: Long,
        destinationScript: ByteArray,
        feeSats: Long,
    ): Transaction {
        val unsigned =
            Transaction(
                2,
                listOf(
                    TxIn(
                        OutPoint(TxId(txid), vout.toLong()),
                        ByteVector.empty,
                        1,
                        ScriptWitness.empty,
                    )
                ),
                listOf(TxOut(Satoshi(valueSats - feeSats), destinationScript)),
                0,
            )
        val sig =
            unsigned.signInput(
                0,
                key.witnessScript,
                SigHash.SIGHASH_ALL,
                Satoshi(valueSats),
                SigVersion.SIGVERSION_WITNESS_V0,
                key.privateKey,
            )
        return unsigned.updateWitness(
            0,
            ScriptWitness(listOf(ByteVector(sig), ByteVector(key.witnessScript))),
        )
    }

    /** Signs with the fee for [feeRateSatVb], or returns null when what's left would be dust. */
    fun build(
        key: ToRemoteKeys.Key,
        txid: String,
        vout: Int,
        valueSats: Long,
        destinationScript: ByteArray,
        feeRateSatVb: Long,
    ): Transaction? {
        val sized = withFee(key, txid, vout, valueSats, destinationScript, feeSats = 0)
        // One extra vbyte: a signature can come out a byte longer than the one used for sizing.
        val feeSats = ((sized.weight() + 3) / 4 + 1) * maxOf(feeRateSatVb, 1)
        if (valueSats - feeSats < DUST_SATS) return null
        return withFee(key, txid, vout, valueSats, destinationScript, feeSats)
    }
}
