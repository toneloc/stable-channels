import CryptoKit
import Foundation

/// Zero-dependency Base58Check decoder and checksum verifier.
enum Base58Check {
    private static let pszBase58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    private static let base58Map: [Int8] = {
        var map = [Int8](repeating: -1, count: 128)
        for (i, c) in pszBase58.enumerated() {
            if let ascii = c.asciiValue, ascii < 128 {
                map[Int(ascii)] = Int8(i)
            }
        }
        return map
    }()

    static func decode(_ string: String) -> [UInt8]? {
        let trimmed = string.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        var zeroes = 0
        for char in trimmed {
            if char == "1" { zeroes += 1 } else { break }
        }

        var b256 = [UInt8](repeating: 0, count: trimmed.count * 733 / 1000 + 1)
        for char in trimmed {
            guard let ascii = char.asciiValue, ascii < 128 else { return nil }
            let carry = base58Map[Int(ascii)]
            guard carry >= 0 else { return nil }
            var c = Int(carry)
            for j in (0..<b256.count).reversed() {
                c += 58 * Int(b256[j])
                b256[j] = UInt8(c & 0xFF)
                c >>= 8
            }
            guard c == 0 else { return nil }
        }

        var start = 0
        while start < b256.count && b256[start] == 0 {
            start += 1
        }

        var result = [UInt8](repeating: 0, count: zeroes)
        result.append(contentsOf: b256[start...])
        return result
    }

    static func verify(_ string: String) -> Bool {
        guard let decoded = decode(string), decoded.count == 25 else { return false }
        let payload = decoded.prefix(21)
        let checksum = decoded.suffix(4)
        let hash1 = SHA256.hash(data: Data(payload))
        let hash2 = SHA256.hash(data: Data(hash1))
        let expected = Array(hash2.prefix(4))
        guard Array(checksum) == expected else { return false }

        guard let version = payload.first else { return false }
        switch version {
        case 0x00, 0x05, 0x6F, 0xC4:
            return true
        default:
            return false
        }
    }
}
