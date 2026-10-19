import CryptoKit
import Foundation

/// A 1-in 1-out transaction that moves a found force close output to the wallet (#364), built like
/// the force close recovery script: version 2 and sequence 1 for the output's 1 block lock
/// (1 OP_CSV), a BIP143 SIGHASH_ALL signature, and the witness `[sig, witnessScript]`.
enum ToRemoteSweep {
    /// Below this the output would be dust for any wallet address type.
    static let dustSats: UInt64 = 330

    struct Signed: Equatable {
        let hex: String
        let vsize: Int
    }

    static func withFee(
        key: ToRemoteKeys.Key,
        txid: String,
        vout: UInt32,
        valueSats: UInt64,
        destinationScript: Data,
        feeSats: UInt64
    ) throws -> Signed {
        guard let txidBytes = Data(hex: txid), txidBytes.count == 32, feeSats < valueSats else {
            throw SweepError.invalidInput
        }
        let outpoint = join([Data(txidBytes.reversed()), le(vout)])
        let sequence = le(UInt32(1))
        let output = join([le(valueSats - feeSats), varInt(destinationScript.count), destinationScript])
        let scriptCode = join([varInt(key.witnessScript.count), key.witnessScript])

        // BIP143 signature hash, SIGHASH_ALL
        let sighash = hash256(join([
            le(UInt32(2)), hash256(outpoint), hash256(sequence), outpoint, scriptCode,
            le(valueSats), sequence, hash256(output), le(UInt32(0)), le(UInt32(1))
        ]))
        guard let der = Secp256k1.sign(hash: sighash, key: key.privateKey) else { throw SweepError.signingFailed }
        let sig = join([der, Data([0x01])])

        let inputsAndOutputs = join([varInt(1), outpoint, varInt(0), sequence, varInt(1), output])
        let witness = join([varInt(2), varInt(sig.count), sig, scriptCode])
        let base = join([le(UInt32(2)), inputsAndOutputs, le(UInt32(0))])
        let tx = join([le(UInt32(2)), Data([0x00, 0x01]), inputsAndOutputs, witness, le(UInt32(0))])
        let weight = base.count * 3 + tx.count
        return Signed(hex: tx.map { String(format: "%02x", $0) }.joined(), vsize: (weight + 3) / 4)
    }

    /// Signs with the fee for `feeRateSatVb`, or returns nil when what's left would be dust.
    static func build(
        key: ToRemoteKeys.Key,
        txid: String,
        vout: UInt32,
        valueSats: UInt64,
        destinationScript: Data,
        feeRateSatVb: UInt64
    ) throws -> Signed? {
        let sized = try withFee(key: key, txid: txid, vout: vout, valueSats: valueSats,
                                destinationScript: destinationScript, feeSats: 0)
        // One extra vbyte: a signature can come out a byte longer than the one used for sizing.
        let feeSats = UInt64(sized.vsize + 1) * max(feeRateSatVb, 1)
        guard valueSats >= feeSats + dustSats else { return nil }
        return try withFee(key: key, txid: txid, vout: vout, valueSats: valueSats,
                           destinationScript: destinationScript, feeSats: feeSats)
    }

    enum SweepError: Error {
        case invalidInput
        case signingFailed
    }

    private static func le(_ value: some FixedWidthInteger) -> Data {
        withUnsafeBytes(of: value.littleEndian) { Data($0) }
    }

    private static func varInt(_ n: Int) -> Data {
        switch n {
        case ..<0xFD: Data([UInt8(n)])
        case ...0xFFFF: Data([0xFD]) + le(UInt16(n))
        default: Data([0xFE]) + le(UInt32(n))
        }
    }

    private static func join(_ parts: [Data]) -> Data {
        parts.reduce(into: Data()) { $0.append($1) }
    }

    private static func hash256(_ data: Data) -> Data {
        Data(SHA256.hash(data: Data(SHA256.hash(data: data))))
    }
}

extension Data {
    init?(hex: String) {
        guard hex.count.isMultiple(of: 2) else { return nil }
        var bytes = [UInt8]()
        bytes.reserveCapacity(hex.count / 2)
        var index = hex.startIndex
        while index < hex.endIndex {
            let next = hex.index(index, offsetBy: 2)
            guard let byte = UInt8(hex[index ..< next], radix: 16) else { return nil }
            bytes.append(byte)
            index = next
        }
        self.init(bytes)
    }

    var sha256Hex: String {
        SHA256.hash(data: self).map { String(format: "%02x", $0) }.joined()
    }
}
