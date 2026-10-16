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

    func testHasMnemonicReturnsFalseWhenEmpty() throws {
        let service = makeService()
        XCTAssertFalse(try service.hasMnemonic())
    }

    func testHasMnemonicReturnsTrueAfterStore() throws {
        let service = makeService()
        try service.storeMnemonic(testMnemonic)
        XCTAssertTrue(try service.hasMnemonic())
    }

    func testHasMnemonicReturnsFalseAfterDelete() throws {
        let service = makeService()
        try service.storeMnemonic(testMnemonic)
        try? service.deleteMnemonic()
        XCTAssertFalse(try service.hasMnemonic())
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
        XCTAssertFalse(try service.hasMnemonic())
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

    // MARK: - Pending Mnemonic Tests & Slot Isolation

    func testStoreAndLoadPendingMnemonicRoundTrip() throws {
        let service = makeService()
        try service.storePendingMnemonic(testMnemonic)
        let loaded = try service.loadPendingMnemonic()
        XCTAssertEqual(loaded, testMnemonic)
        XCTAssertTrue(try service.hasPendingMnemonic())

        try service.deletePendingMnemonic()
        XCTAssertFalse(try service.hasPendingMnemonic())
        XCTAssertThrowsError(try service.loadPendingMnemonic())
    }

    func testActiveAndPendingSlotIsolation() throws {
        let service = makeService()
        try service.storeMnemonic(testMnemonic)
        try service.storePendingMnemonic(otherMnemonic)

        XCTAssertEqual(try service.loadMnemonic(), testMnemonic)
        XCTAssertEqual(try service.loadPendingMnemonic(), otherMnemonic)

        // Deleting pending slot must not affect active slot
        try service.deletePendingMnemonic()
        XCTAssertFalse(try service.hasPendingMnemonic())
        XCTAssertTrue(try service.hasMnemonic())
        XCTAssertEqual(try service.loadMnemonic(), testMnemonic)

        // Storing pending slot again and deleting active must not affect pending
        try service.storePendingMnemonic(otherMnemonic)
        try service.deleteMnemonic()
        XCTAssertFalse(try service.hasMnemonic())
        XCTAssertTrue(try service.hasPendingMnemonic())
        XCTAssertEqual(try service.loadPendingMnemonic(), otherMnemonic)

        try service.deletePendingMnemonic()
    }

    func testStorePendingMnemonicTrimsWhitespace() throws {
        let service = makeService()
        try service.storePendingMnemonic(testMnemonic + "\n  \t")
        let loaded = try service.loadPendingMnemonic()
        XCTAssertEqual(loaded, testMnemonic)
        try service.deletePendingMnemonic()
    }

    func testStorePendingEmptyStringThrows() {
        let service = makeService()
        XCTAssertThrowsError(try service.storePendingMnemonic("")) { error in
            XCTAssertTrue(error is WalletKeychainError)
        }
    }

    func testDeletePendingMnemonicIsIdempotent() {
        let service = makeService()
        XCTAssertNoThrow(try service.deletePendingMnemonic())
    }
}
