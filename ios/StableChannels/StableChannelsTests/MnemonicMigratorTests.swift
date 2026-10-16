import XCTest
@testable import StableChannels

final class MnemonicMigratorTests: XCTestCase {
    private let testMnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    private var tempDirURL: URL!
    private var testKeychain: MockMnemonicStorage!

    override func setUpWithError() throws {
        try super.setUpWithError()
        tempDirURL = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDirURL, withIntermediateDirectories: true)
        testKeychain = MockMnemonicStorage()
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: tempDirURL)
        try super.tearDownWithError()
    }

    func testMigrationCopiesPlaintextToKeychainAndDeletesFile() throws {
        let path = tempDirURL.appendingPathComponent("seed_phrase")
        try testMnemonic.write(to: path, atomically: true, encoding: .utf8)

        let loaded = try MnemonicMigrator.loadOrMigrateMnemonic(
            keychain: testKeychain,
            legacyPath: path,
            logError: nil
        )

        XCTAssertEqual(loaded, testMnemonic)
        XCTAssertEqual(try testKeychain.loadMnemonic(), testMnemonic)
        // Plaintext file is permanently deleted after verified Keychain migration
        XCTAssertFalse(FileManager.default.fileExists(atPath: path.path))
    }

    func testEncryptedFirstThrowsMismatch() throws {
        let keychainSeed = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo"
        try testKeychain.storeMnemonic(keychainSeed)

        let path = tempDirURL.appendingPathComponent("seed_phrase")
        try testMnemonic.write(to: path, atomically: true, encoding: .utf8)

        var loggedMismatch = false
        XCTAssertThrowsError(try MnemonicMigrator.loadOrMigrateMnemonic(
            keychain: testKeychain,
            legacyPath: path,
            logError: { event, _ in
                if event == "KEYCHAIN_PLAINTEXT_MISMATCH" { loggedMismatch = true }
            }
        )) { error in
            guard case MnemonicMigrationError.seedMismatch = error else {
                XCTFail("Expected seedMismatch, got \(error)")
                return
            }
        }

        XCTAssertTrue(loggedMismatch)
        XCTAssertTrue(FileManager.default.fileExists(atPath: path.path))
    }

    func testLingeringMatchingPlaintextIsDeleted() throws {
        try testKeychain.storeMnemonic(testMnemonic)

        let path = tempDirURL.appendingPathComponent("seed_phrase")
        try testMnemonic.write(to: path, atomically: true, encoding: .utf8)

        let loaded = try MnemonicMigrator.loadOrMigrateMnemonic(
            keychain: testKeychain,
            legacyPath: path,
            logError: nil
        )

        XCTAssertEqual(loaded, testMnemonic)
        // Matching plaintext is permanently deleted upon verification
        XCTAssertFalse(FileManager.default.fileExists(atPath: path.path))
    }

    // MARK: - Error Handling Security Tests

    func testLoadErrorAccessDeniedThrowsFailClosed() throws {
        let mockSvc = MockMnemonicStorage()
        mockSvc.mockLoadError = WalletKeychainError.accessDenied(errSecAuthFailed)

        let path = tempDirURL.appendingPathComponent("seed_phrase")
        try testMnemonic.write(to: path, atomically: true, encoding: .utf8)

        var loggedLoadFailed = false
        XCTAssertThrowsError(try MnemonicMigrator.loadOrMigrateMnemonic(
            keychain: mockSvc,
            legacyPath: path,
            logError: { event, _ in
                if event == "KEYCHAIN_LOAD_FAILED" { loggedLoadFailed = true }
            }
        )) { error in
            guard case WalletKeychainError.accessDenied = error else {
                XCTFail("Expected accessDenied, got \(error)")
                return
            }
        }

        XCTAssertTrue(loggedLoadFailed)
        XCTAssertTrue(FileManager.default.fileExists(atPath: path.path))
    }

    func testLoadErrorKeyNotFoundAllowsMigration() throws {
        let mockSvc = MockMnemonicStorage()
        mockSvc.mockLoadError = WalletKeychainError.keyNotFound

        let path = tempDirURL.appendingPathComponent("seed_phrase")
        try testMnemonic.write(to: path, atomically: true, encoding: .utf8)

        let loaded = try MnemonicMigrator.loadOrMigrateMnemonic(
            keychain: mockSvc,
            legacyPath: path,
            logError: nil
        )

        XCTAssertEqual(loaded, testMnemonic)
        // The plaintext file is permanently deleted after verified migration
        XCTAssertFalse(FileManager.default.fileExists(atPath: path.path))
    }

    func testUnreadablePlaintextThrowsInsteadOfReadingAsAbsent() throws {
        let mockSvc = MockMnemonicStorage()
        mockSvc.mockLoadError = WalletKeychainError.keyNotFound

        // A seed_phrase that EXISTS but cannot be read must fail closed: returning nil
        // here is what routes NodeService.start() toward wipe-and-generate.
        let path = tempDirURL.appendingPathComponent("seed_phrase")
        try testMnemonic.write(to: path, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o000], ofItemAtPath: path.path)
        defer {
            try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: path.path)
        }

        var loggedUnreadable = false
        XCTAssertThrowsError(try MnemonicMigrator.loadOrMigrateMnemonic(
            keychain: mockSvc,
            legacyPath: path,
            logError: { event, _ in
                if event == "PLAINTEXT_SEED_UNREADABLE" { loggedUnreadable = true }
            }
        )) { error in
            XCTAssertEqual(error as? MnemonicMigrationError, .plaintextUnreadable)
        }
        XCTAssertTrue(loggedUnreadable)
        XCTAssertNil(mockSvc.mockMnemonic, "no migration may occur from an unreadable file")
    }

    func testLoadErrorDataConversionFailedThrowsFailClosed() throws {
        let mockSvc = MockMnemonicStorage()
        mockSvc.mockLoadError = WalletKeychainError.dataConversionFailed

        let path = tempDirURL.appendingPathComponent("seed_phrase")
        try testMnemonic.write(to: path, atomically: true, encoding: .utf8)

        var loggedLoadFailed = false
        XCTAssertThrowsError(try MnemonicMigrator.loadOrMigrateMnemonic(
            keychain: mockSvc,
            legacyPath: path,
            logError: { event, _ in
                if event == "KEYCHAIN_LOAD_FAILED" { loggedLoadFailed = true }
            }
        )) { error in
            guard case WalletKeychainError.dataConversionFailed = error else {
                XCTFail("Expected dataConversionFailed, got \(error)")
                return
            }
        }

        XCTAssertTrue(loggedLoadFailed)
        XCTAssertTrue(FileManager.default.fileExists(atPath: path.path))
    }

    func testMigrationFailureThrowsFailClosed() throws {
        let mockSvc = MockMnemonicStorage()
        mockSvc.mockStoreError = WalletKeychainError.accessDenied(errSecAuthFailed)

        let path = tempDirURL.appendingPathComponent("seed_phrase")
        try testMnemonic.write(to: path, atomically: true, encoding: .utf8)

        var loggedMigrationFailed = false
        XCTAssertThrowsError(try MnemonicMigrator.loadOrMigrateMnemonic(
            keychain: mockSvc,
            legacyPath: path,
            logError: { event, _ in
                if event == "KEYCHAIN_MIGRATION_FAILED" { loggedMigrationFailed = true }
            }
        )) { error in
            guard case WalletKeychainError.accessDenied = error else {
                XCTFail("Expected accessDenied, got \(error)")
                return
            }
        }

        XCTAssertTrue(loggedMigrationFailed)
        XCTAssertTrue(FileManager.default.fileExists(atPath: path.path))
    }

    func testInternalWhitespaceCanonicalization() throws {
        try testKeychain.storeMnemonic(testMnemonic)

        let path = tempDirURL.appendingPathComponent("seed_phrase")
        let nonCanonical = "abandon   abandon \n abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
        try nonCanonical.write(to: path, atomically: true, encoding: .utf8)

        let loaded = try MnemonicMigrator.loadOrMigrateMnemonic(
            keychain: testKeychain,
            legacyPath: path,
            logError: nil
        )

        XCTAssertEqual(loaded, testMnemonic)
        // Whitespace-equivalent plaintext is treated as matching and deleted
        XCTAssertFalse(FileManager.default.fileExists(atPath: path.path))
    }

    func testCanonicalizeMnemonicLowercasesAndCollapsesWhitespace() {
        let input = "  ABANDON   abandon \n ABANDON   About  "
        let expected = "abandon abandon abandon about"
        XCTAssertEqual(BIP39.canonicalize(input), expected)
    }
}

// MARK: - Mocks

private final class MockMnemonicStorage: MnemonicStorageProtocol {
    var mockLoadError: Error?
    var mockStoreError: Error?
    var mockMnemonic: String?
    var mockPendingMnemonic: String?

    func storeMnemonic(_ mnemonic: String) throws {
        if let error = mockStoreError {
            throw error
        }
        mockMnemonic = mnemonic
    }

    func loadMnemonic() throws -> String {
        if let error = mockLoadError {
            throw error
        }
        if let mnemonic = mockMnemonic {
            return mnemonic
        }
        throw WalletKeychainError.keyNotFound
    }

    func deleteMnemonic() throws {
        mockMnemonic = nil
    }

    func hasMnemonic() throws -> Bool {
        if let error = mockLoadError {
            throw error
        }
        return mockMnemonic != nil
    }

    func storePendingMnemonic(_ mnemonic: String) throws {
        mockPendingMnemonic = mnemonic
    }

    func loadPendingMnemonic() throws -> String {
        if let pending = mockPendingMnemonic {
            return pending
        }
        throw WalletKeychainError.keyNotFound
    }

    func deletePendingMnemonic() throws {
        mockPendingMnemonic = nil
    }

    func hasPendingMnemonic() throws -> Bool {
        return mockPendingMnemonic != nil
    }
}
