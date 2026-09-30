import Foundation

/// BIP-173 Bech32 and BIP-350 Bech32m decoder.
enum Bech32 {
    private static let charset = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
    private static let bech32ChecksumConst: UInt32 = 1
    private static let bech32mChecksumConst: UInt32 = 0x2BC8_30A3

    private static let asciiLookupTable: [Int8] = {
        var table = [Int8](repeating: -1, count: 128)
        let charsetBytes = Array(charset.utf8)
        for (index, byte) in charsetBytes.enumerated() {
            table[Int(byte)] = Int8(index)
        }
        return table
    }()

    enum Error: Swift.Error, LocalizedError, Equatable {
        case invalidLength
        case invalidCharacter(Character)
        case mixedCase
        case missingHrp
        case invalidChecksum
        case bitsConversionFailed
        case invalidUtf8String
        case insecureClearnetScheme

        var errorDescription: String? {
            switch self {
            case .invalidLength:
                return "The Bech32 string is too short."
            case let .invalidCharacter(c):
                return "Invalid Bech32 character: '\(c)'."
            case .mixedCase:
                return "The Bech32 string contains mixed uppercase and lowercase characters."
            case .missingHrp:
                return "The Bech32 string is missing a human-readable prefix (HRP)."
            case .invalidChecksum:
                return "Invalid Bech32 checksum."
            case .bitsConversionFailed:
                return "Failed to convert 5-bit Bech32 data to 8-bit bytes."
            case .invalidUtf8String:
                return "The decoded payload is not a valid UTF-8 string."
            case .insecureClearnetScheme:
                return "LNURL endpoint must use HTTPS."
            }
        }
    }

    enum ChecksumType {
        case bech32
        case bech32m
    }

    // MARK: - Polymod

    @inline(__always)
    private static func polymodStep(_ chk: inout UInt32, value: UInt8) {
        let b = chk >> 25
        chk = ((chk & 0x1FFFFFF) << 5) ^ UInt32(value)
        if (b & 0x01) != 0 { chk ^= 0x3B6A57B2 }
        if (b & 0x02) != 0 { chk ^= 0x26508E6D }
        if (b & 0x04) != 0 { chk ^= 0x1EA119FA }
        if (b & 0x08) != 0 { chk ^= 0x3D4233DD }
        if (b & 0x10) != 0 { chk ^= 0x2A1462B3 }
    }

    private static func determineChecksumType(hrp: some Sequence<UInt8>, data: [UInt8]) -> ChecksumType? {
        var chk: UInt32 = 1
        for byte in hrp {
            polymodStep(&chk, value: byte >> 5)
        }
        polymodStep(&chk, value: 0)
        for byte in hrp {
            polymodStep(&chk, value: byte & 31)
        }
        for val in data {
            polymodStep(&chk, value: val)
        }
        if chk == bech32ChecksumConst { return .bech32 }
        if chk == bech32mChecksumConst { return .bech32m }
        return nil
    }

    // MARK: - Unified Parser

    private static func parse(
        _ bechString: String,
        limitLength: Bool
    ) throws -> (hrp: String, payload5Bit: [UInt8], checksumType: ChecksumType) {
        let utf8Count = bechString.utf8.count
        if limitLength && utf8Count > 90 { throw Error.invalidLength }
        guard utf8Count >= 8 else { throw Error.invalidLength }

        var hasLower = false
        var hasUpper = false
        for byte in bechString.utf8 {
            if byte >= 0x61 && byte <= 0x7A { hasLower = true }
            else if byte >= 0x41 && byte <= 0x5A { hasUpper = true }
            if hasLower && hasUpper {
                throw Error.mixedCase
            }
        }

        let lowercased = bechString.lowercased()
        guard let pos = lowercased.lastIndex(of: "1") else { throw Error.missingHrp }
        let hrp = String(lowercased[..<pos])
        guard !hrp.isEmpty else { throw Error.missingHrp }

        for char in hrp {
            guard let scalar = char.unicodeScalars.first, char.unicodeScalars.count == 1,
                  scalar.value >= 33 && scalar.value <= 126 else {
                throw Error.invalidCharacter(char)
            }
        }

        let dataPart = lowercased[lowercased.index(after: pos)...]
        guard dataPart.count >= 6 else { throw Error.invalidLength }

        var values = [UInt8]()
        values.reserveCapacity(dataPart.count)

        for char in dataPart {
            guard let asciiVal = char.asciiValue, asciiVal < 128 else {
                throw Error.invalidCharacter(char)
            }
            let val = asciiLookupTable[Int(asciiVal)]
            guard val >= 0 else {
                throw Error.invalidCharacter(char)
            }
            values.append(UInt8(val))
        }

        guard let checksumType = determineChecksumType(hrp: hrp.utf8, data: values) else {
            throw Error.invalidChecksum
        }

        let payload5Bit = Array(values.dropLast(6))
        return (hrp, payload5Bit, checksumType)
    }

    // MARK: - Public Verification and Decoding

    /// Verifies if a string is a valid Bech32 or Bech32m checksummed string and returns its lowercased HRP.
    static func verifyChecksum(bechString: String, limitLength: Bool = true) -> String? {
        return (try? parse(bechString, limitLength: limitLength))?.hrp
    }

    /// Verifies that a native Segwit / Taproot address adheres to BIP-173 (v0 with Bech32)
    /// or BIP-350 (v1+ with Bech32m) specifications, including length limits and program sizes.
    static func verifySegwitAddress(_ address: String, expectedHrp: String) -> Bool {
        let cleanAddress = address.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let (hrp, payload, checksumType) = try? parse(cleanAddress, limitLength: true) else {
            return false
        }
        guard hrp == expectedHrp.lowercased(), !payload.isEmpty else { return false }

        let witnessVersion = payload[0]
        guard witnessVersion <= 16 else { return false }

        guard let program = convertBits(data: Array(payload.dropFirst()), fromBits: 5, toBits: 8, pad: false) else {
            return false
        }

        if witnessVersion == 0 {
            // BIP-173: Witness version 0 MUST use Bech32 checksum and length 20 (P2WPKH) or 32 (P2WSH)
            guard checksumType == .bech32 else { return false }
            return program.count == 20 || program.count == 32
        } else {
            // BIP-350: Witness version 1+ MUST use Bech32m checksum and length between 2 and 40
            guard checksumType == .bech32m else { return false }
            return program.count >= 2 && program.count <= 40
        }
    }

    /// Converts an array of 5-bit integers to an array of 8-bit integers (or vice-versa).
    static func convertBits(data: [UInt8], fromBits: Int, toBits: Int, pad: Bool) -> [UInt8]? {
        var acc = 0
        var bits = 0
        var ret = [UInt8]()
        ret.reserveCapacity((data.count * fromBits + toBits - 1) / toBits)
        let maxv = (1 << toBits) - 1
        let maxAcc = (1 << (fromBits + toBits - 1)) - 1

        for value in data {
            if (Int(value) >> fromBits) != 0 {
                return nil
            }
            acc = ((acc << fromBits) | Int(value)) & maxAcc
            bits += fromBits
            while bits >= toBits {
                bits -= toBits
                ret.append(UInt8((acc >> bits) & maxv))
            }
        }

        if pad {
            if bits > 0 {
                ret.append(UInt8((acc << (toBits - bits)) & maxv))
            }
        } else if bits >= fromBits || ((acc << (toBits - bits)) & maxv) != 0 {
            return nil
        }

        return ret
    }

    /// Decodes a Bech32 string into its HRP and 8-bit data payload.
    static func decode(_ bechString: String, limitLength: Bool = true) throws -> (hrp: String, data: Data) {
        let (hrp, payload5Bit, _) = try parse(bechString, limitLength: limitLength)
        guard let converted8Bit = convertBits(data: payload5Bit, fromBits: 5, toBits: 8, pad: false) else {
            throw Error.bitsConversionFailed
        }
        return (hrp, Data(converted8Bit))
    }

    /// Decodes an LNURL bech32 string (`lnurl1...`) into an HTTPS `URL`.
    /// LUD-01 requires standard Bech32 checksum encoding and HTTPS scheme.
    static func decodeLNURL(_ lnurlString: String) throws -> URL {
        var clean = lnurlString.trimmingCharacters(in: .whitespacesAndNewlines)
        if let range = clean.range(of: "lightning://", options: [.caseInsensitive, .anchored]) {
            clean.removeSubrange(range)
        } else if let range = clean.range(of: "lightning:", options: [.caseInsensitive, .anchored]) {
            clean.removeSubrange(range)
        }
        clean = clean.trimmingCharacters(in: .whitespacesAndNewlines)

        let (hrp, payload5Bit, checksumType) = try parse(clean, limitLength: false)
        guard hrp.lowercased() == "lnurl" else {
            throw Error.missingHrp
        }

        guard checksumType == .bech32 else {
            throw Error.invalidChecksum
        }

        guard let converted8Bit = convertBits(data: payload5Bit, fromBits: 5, toBits: 8, pad: false) else {
            throw Error.bitsConversionFailed
        }

        guard let urlString = String(data: Data(converted8Bit), encoding: .utf8),
              let url = URL(string: urlString) else {
            throw Error.invalidUtf8String
        }

        guard let scheme = url.scheme?.lowercased(), let host = url.host?.lowercased() else {
            throw Error.invalidUtf8String
        }

        if host.hasSuffix(".onion") {
            guard scheme == "http" || scheme == "https" else {
                throw Error.insecureClearnetScheme
            }
        } else {
            guard scheme == "https" else {
                throw Error.insecureClearnetScheme
            }
        }

        return url
    }
}
