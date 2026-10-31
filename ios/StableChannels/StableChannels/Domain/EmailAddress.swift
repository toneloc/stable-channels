import Foundation

/// An email address the user chose to share, trimmed and lowercased so the same address is stored once.
struct EmailAddress: Equatable, Sendable {
    static let maxLength = 254
    static let maxLocalPartLength = 64

    let value: String

    /// Returns nil unless `raw` has one `@`, a non-empty local part and a dotted domain, with no whitespace.
    init?(_ raw: String) {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard trimmed.count <= Self.maxLength,
              !trimmed.contains(where: { $0.isWhitespace || $0.isNewline }) else { return nil }

        let parts = trimmed.split(separator: "@", omittingEmptySubsequences: false)
        guard parts.count == 2 else { return nil }
        let local = parts[0]
        let domain = parts[1]
        guard !local.isEmpty, local.count <= Self.maxLocalPartLength else { return nil }

        let labels = domain.split(separator: ".", omittingEmptySubsequences: false)
        guard labels.count >= 2,
              labels.allSatisfy({ !$0.isEmpty }),
              let tld = labels.last, tld.count >= 2 else { return nil }

        value = trimmed
    }
}
