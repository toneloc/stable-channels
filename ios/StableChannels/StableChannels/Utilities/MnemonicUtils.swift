import Foundation

/// Mnemonic word parsing utilities - full BIP39 validation handled by LDKNode
enum MnemonicUtils {
    struct ValidationResult {
        let words: [String]
        let isValidWordCount: Bool
        let hasValidCharacterFormat: Bool

        var isValid: Bool {
            isValidWordCount && hasValidCharacterFormat
        }

        var displayString: String {
            words.joined(separator: " ")
        }
    }

    // MARK: - Word Parsing

    /// Parse mnemonic string into array of words, trimmed and lowercased.
    /// Splits across any Unicode/ASCII whitespace (spaces, tabs, newlines).
    static func parseMnemonic(_ input: String) -> [String] {
        input
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
            .split(whereSeparator: { $0.isWhitespace })
            .map(String.init)
    }

    /// Convert word array to filled array of maxWordCount (empty strings for unfilled)
    static func wordsToFields(_ words: [String]) -> [String] {
        var fields = Array(repeating: "", count: SeedConstants.maxWordCount)
        for (index, word) in words.enumerated() where index < SeedConstants.maxWordCount {
            fields[index] = word
        }
        return fields
    }

    // MARK: - Validation

    /// Checks if a single word contains only ASCII alphabetic characters.
    static func isAlphabeticWord(_ word: String) -> Bool {
        guard !word.isEmpty else { return false }
        return word.allSatisfy { ($0 >= "a" && $0 <= "z") || ($0 >= "A" && $0 <= "Z") }
    }

    /// Detect word count from mnemonic string (12 or 24, defaults based on input)
    static func detectWordCount(_ mnemonic: String) -> Int {
        let count = parseMnemonic(mnemonic).count
        if count <= SeedConstants.wordCount12 {
            return SeedConstants.wordCount12
        }
        return SeedConstants.wordCount24
    }

    /// Check if word list has valid word count (12 or 24)
    static func isValidWordCount(words: [String]) -> Bool {
        let count = words.count
        return count == SeedConstants.wordCount12 || count == SeedConstants.wordCount24
    }

    /// Check if mnemonic has valid word count (12 or 24)
    static func isValidWordCount(_ mnemonic: String) -> Bool {
        isValidWordCount(words: parseMnemonic(mnemonic))
    }

    /// Check if all words contain only alphabetic characters
    static func hasValidCharacterFormat(words: [String]) -> Bool {
        guard !words.isEmpty else { return false }
        return words.allSatisfy(isAlphabeticWord)
    }

    /// Check if all words contain only alphabetic characters (basic format check)
    /// Note: Full BIP39 validation (wordlist + checksum) is handled by LDKNode
    static func hasValidCharacterFormat(_ mnemonic: String) -> Bool {
        hasValidCharacterFormat(words: parseMnemonic(mnemonic))
    }

    /// Consolidated validation returning words and validity checks.
    static func validate(_ mnemonic: String) -> ValidationResult {
        let words = parseMnemonic(mnemonic)
        return ValidationResult(
            words: words,
            isValidWordCount: isValidWordCount(words: words),
            hasValidCharacterFormat: hasValidCharacterFormat(words: words)
        )
    }

    // MARK: - Display

    /// Convert mnemonic to display format (space-separated, trimmed)
    static func formatForDisplay(_ mnemonic: String) -> String {
        parseMnemonic(mnemonic).joined(separator: " ")
    }
}
