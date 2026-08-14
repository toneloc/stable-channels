import XCTest
@testable import StableChannels

final class MnemonicMigratorTests: XCTestCase {
    private let testMnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    private var tempDirURL: URL!
    private var testKeychain: WalletKeychainService!

    override func setUpWithError() throws {
        try super.setUpWithError()
        tempDirURL = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDirURL, withIntermediateDirectories: true)
        testKeychain = WalletKeychainService(
            service: "com.stablechannels.wallet.test",
            account: "seed_phrase_test_\(UUID().uuidString)",
            accessGroup: nil
        )
    }

    override func tearDownWithError() throws {
        try? testKeychain.deleteMnemonic()
        try? FileManager.default.removeItem(at: tempDirURL)
        try super.tearDownWithError()
    }

    func testMigrationMovesPlaintextToKeychain() throws {
        let path = tempDirURL.appendingPathComponent("seed_phrase")
        try testMnemonic.write(to: path, atomically: true, encoding: .utf8)

        let loaded = MnemonicMigrator.loadOrMigrateMnemonic(
            keychain: testKeychain,
            legacyPath: path,
            logError: nil
        )

        XCTAssertEqual(loaded, testMnemonic)
        XCTAssertEqual(try testKeychain.loadMnemonic(), testMnemonic)
        XCTAssertFalse(FileManager.default.fileExists(atPath: path.path))
    }

    func testEncryptedFirstPrefersKeychainOverPlaintext() throws {
        let keychainSeed = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo"
        try testKeychain.storeMnemonic(keychainSeed)

        let path = tempDirURL.appendingPathComponent("seed_phrase")
        try testMnemonic.write(to: path, atomically: true, encoding: .utf8)

        var loggedMismatch = false
        let loaded = MnemonicMigrator.loadOrMigrateMnemonic(
            keychain: testKeychain,
            legacyPath: path,
            logError: { event, _ in
                if event == "KEYCHAIN_PLAINTEXT_MISMATCH" { loggedMismatch = true }
            }
        )

        XCTAssertEqual(loaded, keychainSeed)
        XCTAssertTrue(loggedMismatch)
        XCTAssertTrue(FileManager.default.fileExists(atPath: path.path))
    }

    // MARK: - Error Handling Security Tests (Comment 3)

    func testLoadErrorAccessDeniedFailsClosed() throws {
        let mockSvc = MockWalletKeychainService(
            service: "com.stablechannels.wallet.test",
            account: "seed_phrase_test_\(UUID().uuidString)",
            accessGroup: nil
        )
        mockSvc.mockLoadError = WalletKeychainError.accessDenied(errSecAuthFailed)

        let path = tempDirURL.appendingPathComponent("seed_phrase")
        try testMnemonic.write(to: path, atomically: true, encoding: .utf8)

        var loggedLoadFailed = false
        let loaded = MnemonicMigrator.loadOrMigrateMnemonic(
            keychain: mockSvc,
            legacyPath: path,
            logError: { event, _ in
                if event == "KEYCHAIN_LOAD_FAILED" { loggedLoadFailed = true }
            }
        )

        XCTAssertNil(loaded)
        XCTAssertTrue(loggedLoadFailed)
        XCTAssertTrue(FileManager.default.fileExists(atPath: path.path))
    }

    func testLoadErrorKeyNotFoundAllowsMigration() throws {
        let mockSvc = MockWalletKeychainService(
            service: "com.stablechannels.wallet.test",
            account: "seed_phrase_test_\(UUID().uuidString)",
            accessGroup: nil
        )
        mockSvc.mockLoadError = WalletKeychainError.keyNotFound

        let path = tempDirURL.appendingPathComponent("seed_phrase")
        try testMnemonic.write(to: path, atomically: true, encoding: .utf8)

        let loaded = MnemonicMigrator.loadOrMigrateMnemonic(
            keychain: mockSvc,
            legacyPath: path,
            logError: nil
        )

        XCTAssertEqual(loaded, testMnemonic)
        XCTAssertFalse(FileManager.default.fileExists(atPath: path.path))
    }
}

// MARK: - Mocks

private final class MockWalletKeychainService: WalletKeychainService {
    var mockLoadError: Error?
    var mockMnemonic: String?

    override func loadMnemonic() throws -> String {
        if let error = mockLoadError {
            throw error
        }
        if let mnemonic = mockMnemonic {
            return mnemonic
        }
        return try super.loadMnemonic()
    }
}
