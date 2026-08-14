import XCTest
@testable import StableChannels

final class WalletKeychainServiceTests: XCTestCase {
    // Test-specific mnemonic so tests never interfere with real wallet data
    private let testMnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    private let otherMnemonic = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo"

    // MARK: - Helpers

    private var servicesCreated: [WalletKeychainService] = []

    private func makeService() -> WalletKeychainService {
        let svc = WalletKeychainService(
            service: "com.stablechannels.wallet.test",
            account: "seed_phrase_test_\(UUID().uuidString)",
            accessGroup: nil
        )
        servicesCreated.append(svc)
        try? svc.deleteMnemonic()
        return svc
    }

    override func tearDownWithError() throws {
        for svc in servicesCreated {
            try? svc.deleteMnemonic()
        }
        servicesCreated.removeAll()
        try super.tearDownWithError()
    }

    // MARK: - storeMnemonic / loadMnemonic round-trip

    func testStoreAndLoadMnemonicRoundTrip() throws {
        let service = makeService()
        try service.storeMnemonic(testMnemonic)
        let loaded = try service.loadMnemonic()
        XCTAssertEqual(loaded, testMnemonic)
    }

    func testStoreTrimsLeadingAndTrailingWhitespace() throws {
        let service = makeService()
        try service.storeMnemonic(testMnemonic + "\n\n")
        let loaded = try service.loadMnemonic()
        XCTAssertEqual(loaded, testMnemonic)
    }

    func testStoreEmptyStringThrows() {
        let service = makeService()
        XCTAssertThrowsError(try service.storeMnemonic("")) { error in
            XCTAssertTrue(error is WalletKeychainError, "Expected WalletKeychainError, got \(type(of: error))")
        }
    }

    func testStoreWhitespaceOnlyThrows() {
        let service = makeService()
        XCTAssertThrowsError(try service.storeMnemonic(" \n\t ")) { error in
            XCTAssertTrue(error is WalletKeychainError)
        }
    }

    func testStoreMnemonicOverwritesPreviousValue() throws {
        let service = makeService()
        try service.storeMnemonic(testMnemonic)
        try service.storeMnemonic(otherMnemonic)
        let loaded = try service.loadMnemonic()
        XCTAssertEqual(loaded, otherMnemonic)
    }

    func testStoreMnemonicIsIdempotentWhenValueIsUnchanged() throws {
        let service = makeService()
        try service.storeMnemonic(testMnemonic)
        // Store same mnemonic again — should succeed (no-op path)
        XCTAssertNoThrow(try service.storeMnemonic(testMnemonic))
        let loaded = try service.loadMnemonic()
        XCTAssertEqual(loaded, testMnemonic)
    }

    // MARK: - hasMnemonic

    func testHasMnemonicReturnsFalseWhenEmpty() {
        let service = makeService()
        XCTAssertFalse(service.hasMnemonic())
    }

    func testHasMnemonicReturnsTrueAfterStore() throws {
        let service = makeService()
        try service.storeMnemonic(testMnemonic)
        XCTAssertTrue(service.hasMnemonic())
    }

    func testHasMnemonicReturnsFalseAfterDelete() throws {
        let service = makeService()
        try service.storeMnemonic(testMnemonic)
        try? service.deleteMnemonic()
        XCTAssertFalse(service.hasMnemonic())
    }

    // MARK: - loadMnemonic missing key

    func testLoadMnemonicThrowsKeyNotFoundWhenEmpty() {
        let service = makeService()
        XCTAssertThrowsError(try service.loadMnemonic()) { error in
            guard case WalletKeychainError.keyNotFound = error else {
                XCTFail("Expected keyNotFound, got \(error)")
                return
            }
        }
    }

    // MARK: - deleteMnemonic

    func testDeleteMnemonicIsIdempotent() {
        let service = makeService()
        // Must not throw when deleting a key that does not exist
        XCTAssertNoThrow(try service.deleteMnemonic())
    }

    func testDeleteMnemonicRemovesKeyPermanently() throws {
        let service = makeService()
        try service.storeMnemonic(testMnemonic)
        try service.deleteMnemonic()
        XCTAssertFalse(service.hasMnemonic())
        XCTAssertThrowsError(try service.loadMnemonic()) { error in
            guard case WalletKeychainError.keyNotFound = error else {
                XCTFail("Expected keyNotFound after delete, got \(error)")
                return
            }
        }
    }

    // MARK: - Error types are distinguishable

    func testErrorDescriptionsAreNonEmpty() {
        XCTAssertFalse(WalletKeychainError.accessDenied(errSecAuthFailed).localizedDescription.isEmpty)
        XCTAssertFalse(WalletKeychainError.keyNotFound.localizedDescription.isEmpty)
        XCTAssertFalse(WalletKeychainError.dataConversionFailed.localizedDescription.isEmpty)
    }
}
