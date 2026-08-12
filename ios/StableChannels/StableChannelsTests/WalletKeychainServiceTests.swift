import XCTest
@testable import StableChannels

final class WalletKeychainServiceTests: XCTestCase {
    // Test-specific mnemonic so tests never interfere with real wallet data
    private let testMnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    private let otherMnemonic = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo"

    // MARK: - Helpers

    private func makeService() -> WalletKeychainService {
        let svc = WalletKeychainService.shared
        svc.deleteMnemonic()
        return svc
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
        service.deleteMnemonic()
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
        service.deleteMnemonic()
    }

    func testDeleteMnemonicRemovesKeyPermanently() throws {
        let service = makeService()
        try service.storeMnemonic(testMnemonic)
        service.deleteMnemonic()
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
