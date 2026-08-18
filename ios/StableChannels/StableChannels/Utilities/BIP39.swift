import CryptoKit
import Foundation

/// In-app BIP-39 validation (wordlist membership + checksum).
///
/// LDKNode's generated Swift binding aborts the process (`try!`) when handed an
/// invalid mnemonic, so every phrase that could be user-supplied or read from
/// mutable storage MUST be canonicalized and validated before reaching
/// `NodeEntropy.fromBip39Mnemonic`. The wordlist is extracted verbatim from the
/// rust-bip39 crate (see BIP39WordList.swift), so a valid English seed can never
/// be falsely rejected.
enum BIP39 {
    /// Returns the lowercase, single-spaced mnemonic after full BIP-39
    /// validation (word count, wordlist membership, and checksum).
    ///
    /// Callers must pass this returned value—not the original input—to LDKNode.
    /// rust-bip39's parser is case-sensitive, while accepting mixed-case input is
    /// useful at the UI/storage boundary.
    static func validatedCanonicalMnemonic(_ mnemonic: String) -> String? {
        let words = mnemonic
            .lowercased()
            .split(whereSeparator: \.isWhitespace)
            .map(String.init)
        guard [12, 15, 18, 21, 24].contains(words.count) else { return nil }

        // Concatenate the 11-bit word indices.
        var bits = [Bool]()
        bits.reserveCapacity(words.count * 11)
        for word in words {
            guard let index = wordIndex[word] else { return nil }
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
            if bits[entropyLength + index] != expected { return nil }
        }
        return words.joined(separator: " ")
    }

    static func isValid(_ mnemonic: String) -> Bool {
        validatedCanonicalMnemonic(mnemonic) != nil
    }

    private static let wordIndex: [String: Int] = {
        var map = [String: Int](minimumCapacity: 2048)
        for (index, word) in BIP39WordList.english.enumerated() {
            map[word] = index
        }
        return map
    }()
}
