import Foundation
import libsecp256k1

/// The few libsecp256k1 calls the force close recovery needs (#364).
enum Secp256k1 {
    private static let context = secp256k1_context_create(UInt32(SECP256K1_CONTEXT_NONE))!

    /// `(key + tweak) mod n`, for BIP32 private derivation.
    static func add(_ key: Data, _ tweak: Data) -> Data? {
        var result = [UInt8](key)
        let tweak = [UInt8](tweak)
        guard result.count == 32, tweak.count == 32,
              secp256k1_ec_seckey_tweak_add(context, &result, tweak) == 1 else { return nil }
        return Data(result)
    }

    /// The 33-byte compressed public key.
    static func publicKey(_ key: Data) -> Data? {
        var pubkey = secp256k1_pubkey()
        var output = [UInt8](repeating: 0, count: 33)
        var length = output.count
        guard secp256k1_ec_pubkey_create(context, &pubkey, [UInt8](key)) == 1,
              secp256k1_ec_pubkey_serialize(
                  context, &output, &length, &pubkey, UInt32(SECP256K1_EC_COMPRESSED)
              ) == 1 else { return nil }
        return Data(output)
    }

    /// DER ECDSA signature of a 32-byte hash, with RFC 6979 nonces and low S.
    static func sign(hash: Data, key: Data) -> Data? {
        var signature = secp256k1_ecdsa_signature()
        var output = [UInt8](repeating: 0, count: 72)
        var length = output.count
        guard hash.count == 32,
              secp256k1_ecdsa_sign(context, &signature, [UInt8](hash), [UInt8](key), nil, nil) == 1,
              secp256k1_ecdsa_signature_serialize_der(context, &output, &length, &signature) == 1
        else { return nil }
        return Data(output.prefix(length))
    }
}
