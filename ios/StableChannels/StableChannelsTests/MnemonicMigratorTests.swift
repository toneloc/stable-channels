import XCTest
@testable import StableChannels

final class MnemonicMigratorTests: XCTestCase {
    private let testMnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    private var tempDirURL: URL!

    override func setUpWithError() throws {
        try super.setUpWithError()
        tempDirURL = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDirURL, withIntermediateDirectories: true)
        WalletKeychainService.shared.deleteMnemonic()
    }

    override func tearDownWithError() throws {
        WalletKeychainService.shared.deleteMnemonic()
        try? FileManager.default.removeItem(at: tempDirURL)
        try super.tearDownWithError()
    }

    func testMigrationMovesPlaintextToKeychain() throws {
        let path = tempDirURL.appendingPathComponent("seed_phrase")
        try testMnemonic.write(to: path, atomically: true, encoding: .utf8)

        let loaded = MnemonicMigrator.loadOrMigrateMnemonic(
            keychain: WalletKeychainService.shared,
            legacyPath: path,
            logError: nil
        )

        XCTAssertEqual(loaded, testMnemonic)
        XCTAssertEqual(try WalletKeychainService.shared.loadMnemonic(), testMnemonic)
        XCTAssertFalse(FileManager.default.fileExists(atPath: path.path))
    }

    func testEncryptedFirstPrefersKeychainOverPlaintext() throws {
        let keychainSeed = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo"
        try WalletKeychainService.shared.storeMnemonic(keychainSeed)

        let path = tempDirURL.appendingPathComponent("seed_phrase")
        try testMnemonic.write(to: path, atomically: true, encoding: .utf8)

        var loggedMismatch = false
        let loaded = MnemonicMigrator.loadOrMigrateMnemonic(
            keychain: WalletKeychainService.shared,
            legacyPath: path,
            logError: { event, _ in
                if event == "KEYCHAIN_PLAINTEXT_MISMATCH" { loggedMismatch = true }
            }
        )

        XCTAssertEqual(loaded, keychainSeed)
        XCTAssertTrue(loggedMismatch)
        XCTAssertTrue(FileManager.default.fileExists(atPath: path.path))
    }
}
