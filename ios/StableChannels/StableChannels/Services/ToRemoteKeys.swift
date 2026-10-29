import CommonCrypto
import CryptoKit
import Foundation

/// The keys LDK pays our side of a channel to when the LSP force-closes it (#364). ldk-node turns
/// on LDK's v2 remote key derivation, which uses one of 1000 keys, m/8'/N' of the LDK seed, so these
/// funds can be found and spent with the wallet seed alone after the channel data is lost. The LDK
/// seed is the private key of the BIP32 master of the wallet's 64-byte seed (ldk-node builder.rs).
/// Same derivation as the force close recovery script. The app only opens anchor channels, so only
/// the anchor output kind is covered.
struct ToRemoteKeys {
    struct Key {
        let index: Int
        let privateKey: Data
        /// `<pubkey> OP_CHECKSIGVERIFY 1 OP_CSV`, paid to as P2WSH.
        let witnessScript: Data
        let scriptPubKey: Data

        init(index: Int, privateKey: Data) throws {
            guard let publicKey = Secp256k1.publicKey(privateKey) else { throw KeyError.invalidKey }
            self.index = index
            self.privateKey = privateKey
            witnessScript = Data([0x21]) + publicKey + Data([0xAD, 0x51, 0xB2])
            scriptPubKey = Data([0x00, 0x20]) + Data(SHA256.hash(data: witnessScript))
        }
    }

    static let count = 1000

    let keys: [Key]
    private let byScript: [Data: Int]

    /// `seed` is the 64-byte wallet seed: the BIP39 seed, or the keys_seed file of old wallets.
    init(seed: Data) throws {
        let ldkSeed = Self.master(seed).key
        let base = try Self.master(ldkSeed).hardenedChild(8)
        keys = try (0 ..< Self.count).map { index in
            try Key(index: index, privateKey: base.hardenedChild(UInt32(index)).key)
        }
        byScript = Dictionary(uniqueKeysWithValues: keys.map { ($0.scriptPubKey, $0.index) })
    }

    init(mnemonic: String) throws {
        try self.init(seed: Self.bip39Seed(mnemonic))
    }

    func match(_ scriptPubKey: Data) -> Key? {
        byScript[scriptPubKey].map { keys[$0] }
    }

    /// BIP39: PBKDF2-HMAC-SHA512 of the words with salt "mnemonic", 2048 rounds, no passphrase.
    static func bip39Seed(_ mnemonic: String) -> Data {
        let words = mnemonic.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        let password = Array(words.decomposedStringWithCompatibilityMapping.utf8)
        let salt = Array("mnemonic".utf8)
        var seed = [UInt8](repeating: 0, count: 64)
        _ = password.withUnsafeBufferPointer { passwordBytes in
            CCKeyDerivationPBKDF(
                CCPBKDFAlgorithm(kCCPBKDF2),
                passwordBytes.baseAddress.map { UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self) },
                password.count,
                salt,
                salt.count,
                CCPseudoRandomAlgorithm(kCCPRFHmacAlgSHA512),
                2048,
                &seed,
                seed.count
            )
        }
        return Data(seed)
    }

    /// A BIP32 private key with its chain code.
    private struct ExtendedKey {
        let key: Data
        let chainCode: Data

        func hardenedChild(_ index: UInt32) throws -> ExtendedKey {
            var data = Data([0x00]) + key
            withUnsafeBytes(of: (index | 0x8000_0000).bigEndian) { data.append(contentsOf: $0) }
            let i = Data(HMAC<SHA512>.authenticationCode(for: data, using: SymmetricKey(data: chainCode)))
            guard let child = Secp256k1.add(key, i.prefix(32)) else { throw KeyError.invalidKey }
            return ExtendedKey(key: child, chainCode: Data(i.suffix(32)))
        }
    }

    private static func master(_ seed: Data) -> ExtendedKey {
        let i = Data(HMAC<SHA512>.authenticationCode(for: seed, using: SymmetricKey(data: Data("Bitcoin seed".utf8))))
        return ExtendedKey(key: Data(i.prefix(32)), chainCode: Data(i.suffix(32)))
    }

    enum KeyError: Error {
        case invalidKey
    }
}
