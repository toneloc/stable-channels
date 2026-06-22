import Foundation

/// Mnemonic word parsing utilities - full BIP39 validation handled by LDKNode
enum MnemonicUtils {
    /// Regex pattern for validating mnemonic word format (alphabetic only)
    private static let wordPattern: NSRegularExpression? = try? NSRegularExpression(
        pattern: "^[a-z]+$",
        options: .caseInsensitive
    )

    // MARK: - Word Parsing

    /// Parse mnemonic string into array of words, trimmed, lowercased and filtered
    static func parseMnemonic(_ input: String) -> [String] {
        input
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
            .split(separator: " ")
            .map(String.init)
            .filter { !$0.isEmpty }
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

    /// Detect word count from mnemonic string (12 or 24, defaults based on input)
    static func detectWordCount(_ mnemonic: String) -> Int {
        let count = parseMnemonic(mnemonic).count
        if count <= SeedConstants.wordCount12 {
            return SeedConstants.wordCount12
        }
        return SeedConstants.wordCount24
    }

    /// Check if mnemonic has valid word count (12 or 24)
    static func isValidWordCount(_ mnemonic: String) -> Bool {
        let count = parseMnemonic(mnemonic).count
        return count == SeedConstants.wordCount12 || count == SeedConstants.wordCount24
    }

    /// Check if all words contain only alphabetic characters (basic format check)
    /// Note: Full BIP39 validation (wordlist + checksum) is handled by LDKNode
    static func hasValidCharacterFormat(_ mnemonic: String) -> Bool {
        let words = parseMnemonic(mnemonic)
        guard !words.isEmpty else { return false }
        return words.allSatisfy { word in
            wordPattern?.firstMatch(
                in: word,
                options: [],
                range: NSRange(word.startIndex..., in: word)
            ) != nil
        }
    }

    // MARK: - Display

    /// Convert mnemonic to display format (space-separated, trimmed)
    static func formatForDisplay(_ mnemonic: String) -> String {
        parseMnemonic(mnemonic).joined(separator: " ")
    }
}
