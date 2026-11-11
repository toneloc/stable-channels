import Foundation

/// An email address the user chose to share, trimmed and lowercased so the same address is stored once.
struct EmailAddress: Equatable, Sendable {
    static let maxLength = 254
    static let maxLocalPartLength = 64
    /// Characters the local part may use besides letters and digits; the server accepts the same set.
    private static let localSymbols = Set("!#$%&'*+/=?^_`{|}~-")

    let value: String

    /// Returns nil unless `raw` is a dot-separated local part of plain address characters, one `@`, and a dotted
    /// ASCII domain whose last label has two or more characters, the same rule the server applies.
    init?(_ raw: String) {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard trimmed.count <= Self.maxLength else { return nil }

        let parts = trimmed.split(separator: "@", omittingEmptySubsequences: false)
        guard parts.count == 2, parts[0].count <= Self.maxLocalPartLength else { return nil }

        let localParts = parts[0].split(separator: ".", omittingEmptySubsequences: false)
        guard localParts.allSatisfy({ part in
            !part.isEmpty && part.allSatisfy { Self.isAlphanumeric($0) || Self.localSymbols.contains($0) }
        }) else { return nil }

        let labels = parts[1].split(separator: ".", omittingEmptySubsequences: false)
        guard labels.count >= 2,
              labels.allSatisfy(Self.isDomainLabel),
              let tld = labels.last, tld.count >= 2 else { return nil }

        value = trimmed
    }

    private static func isAlphanumeric(_ c: Character) -> Bool {
        c.isASCII && (c.isLowercase || c.isNumber)
    }

    private static func isDomainLabel(_ label: Substring) -> Bool {
        !label.isEmpty && label.first != "-" && label.last != "-"
            && label.allSatisfy { isAlphanumeric($0) || $0 == "-" }
    }
}
