import CryptoKit
import Foundation

/// In-app BIP-39 validation (wordlist membership + checksum).
///
/// LDKNode's generated Swift binding aborts the process (`try!`) when handed an
/// invalid mnemonic, so every phrase that could be user-supplied or read from
/// mutable storage MUST pass this check before reaching
/// `NodeEntropy.fromBip39Mnemonic`. Returning `false` here is always safe; the
/// wordlist is extracted verbatim from the rust-bip39 crate (see
/// BIP39WordList.swift), so a valid seed can never be falsely rejected.
enum BIP39 {
    /// Full BIP-39 validation: word count, wordlist membership, and checksum.
    static func isValid(_ mnemonic: String) -> Bool {
        let words = mnemonic
            .lowercased()
            .split(whereSeparator: \.isWhitespace)
            .map(String.init)
        guard [12, 15, 18, 21, 24].contains(words.count) else { return false }

        // Concatenate the 11-bit word indices.
        var bits = [Bool]()
        bits.reserveCapacity(words.count * 11)
        for word in words {
            guard let index = wordIndex[word] else { return false }
            for shift in stride(from: 10, through: 0, by: -1) {
                bits.append((index >> shift) & 1 == 1)
            }
        }

        // Split into entropy and checksum: CS = ENT/32, and CS bits = wordCount/3.
        let checksumLength = words.count / 3
        let entropyLength = bits.count - checksumLength
        var entropy = [UInt8](repeating: 0, count: entropyLength / 8)
        for (index, bit) in bits.prefix(entropyLength).enumerated() where bit {
            entropy[index / 8] |= 1 << (7 - (index % 8))
        }

        // The checksum is the first CS bits of SHA-256(entropy).
        let hashBytes = Array(SHA256.hash(data: Data(entropy)))
        for index in 0..<checksumLength {
            let expected = (hashBytes[index / 8] >> (7 - (index % 8))) & 1 == 1
            if bits[entropyLength + index] != expected { return false }
        }
        return true
    }

    private static let wordIndex: [String: Int] = {
        var map = [String: Int](minimumCapacity: 2048)
        for (index, word) in BIP39WordList.english.enumerated() {
            map[word] = index
        }
        return map
    }()
}
