import XCTest
@testable import StableChannels

final class WalletLifecycleManagerTests: XCTestCase {
    private let testMnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    private let otherMnemonic = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo"
    private var tempDirURL: URL!
    private var testAppGroup: String!
    private var mockStorage: MockLifecycleMnemonicStorage!
    private var manager: WalletLifecycleManager!

    override func setUpWithError() throws {
        try super.setUpWithError()
        tempDirURL = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDirURL, withIntermediateDirectories: true)
        testAppGroup = "group.test.\(UUID().uuidString)"
        mockStorage = MockLifecycleMnemonicStorage()
        manager = WalletLifecycleManager(
            keychain: mockStorage,
            userDataDir: tempDirURL,
            appGroupIdentifier: testAppGroup
        )
    }

    override func tearDownWithError() throws {
        UserDefaults(suiteName: testAppGroup)?.removePersistentDomain(forName: testAppGroup)
        try? FileManager.default.removeItem(at: tempDirURL)
        try super.tearDownWithError()
    }

    // MARK: - StartupState Detection Matrix

    func testDetectStartupStateNewWallet() {
        let state = manager.detectStartupState()
        XCTAssertEqual(state, .newWallet)
    }

    func testDetectStartupStateReady() throws {
        try mockStorage.storeMnemonic(testMnemonic)
        let dbPath = tempDirURL.appendingPathComponent("ldk_node_data.sqlite")
        try "fake db".write(to: dbPath, atomically: true, encoding: .utf8)

        let state = manager.detectStartupState()
        XCTAssertEqual(state, .ready)
    }

    func testDetectStartupStateSeedOnlyMismatch() throws {
        try mockStorage.storeMnemonic(testMnemonic)
        let state = manager.detectStartupState()
        XCTAssertEqual(state, .seedOnlyMismatch)
    }

    func testDetectStartupStateDbOnlyMismatch() throws {
        let dbPath = tempDirURL.appendingPathComponent("ldk_node_data.sqlite")
        try "fake db".write(to: dbPath, atomically: true, encoding: .utf8)

        let state = manager.detectStartupState()
        XCTAssertEqual(state, .dbOnlyMismatch)
    }

    func testDetectStartupStateStorageError() {
        mockStorage.mockLoadError = WalletKeychainError.accessDenied(errSecAuthFailed)
        let state = manager.detectStartupState()
        guard case .storageError = state else {
            XCTFail("Expected storageError, got \(state)")
            return
        }
    }

    func testDetectStartupStateSeedStorageMismatch() throws {
        try mockStorage.storeMnemonic(testMnemonic)
        let seedPhrasePath = tempDirURL.appendingPathComponent("seed_phrase")
        try otherMnemonic.write(to: seedPhrasePath, atomically: true, encoding: .utf8)

        let state = manager.detectStartupState()
        XCTAssertEqual(state, .seedStorageMismatch)
    }

    // MARK: - BIP-39 Validation & Restore Flow

    func testRestoreRejectsInvalidWordCountWithoutTouchingStorage() {
        var nodeStopped = false
        var persistenceWiped = false

        XCTAssertThrowsError(try manager.restoreMnemonic(
            "one two three four five",
            onStopNode: { nodeStopped = true },
            onWipePersistence: { persistenceWiped = true }
        )) { error in
            guard case WalletRestoreError.invalidMnemonic = error else {
                XCTFail("Expected invalidMnemonic, got \(error)")
                return
            }
        }

        XCTAssertFalse(nodeStopped)
        XCTAssertFalse(persistenceWiped)
        XCTAssertNil(mockStorage.mockPendingMnemonic)
        XCTAssertNil(mockStorage.mockMnemonic)
    }

    func testRestoreRejectsInvalidBip39WordWithoutTouchingStorage() {
        var nodeStopped = false
        var persistenceWiped = false
        let invalidWordMnemonic = "foo foo foo foo foo foo foo foo foo foo foo foo"

        XCTAssertThrowsError(try manager.restoreMnemonic(
            invalidWordMnemonic,
            onStopNode: { nodeStopped = true },
            onWipePersistence: { persistenceWiped = true }
        )) { error in
            guard case WalletRestoreError.invalidMnemonic = error else {
                XCTFail("Expected invalidMnemonic, got \(error)")
                return
            }
        }

        XCTAssertFalse(nodeStopped)
        XCTAssertFalse(persistenceWiped)
        XCTAssertNil(mockStorage.mockPendingMnemonic)
        XCTAssertNil(mockStorage.mockMnemonic)
    }

    func testRestoreRejectsInvalidBip39ChecksumWithoutTouchingStorage() {
        var nodeStopped = false
        var persistenceWiped = false
        // 12th word is 'abandon' instead of 'about', creating an invalid BIP-39 checksum
        let invalidChecksumMnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon"

        XCTAssertThrowsError(try manager.restoreMnemonic(
            invalidChecksumMnemonic,
            onStopNode: { nodeStopped = true },
            onWipePersistence: { persistenceWiped = true }
        )) { error in
            guard case WalletRestoreError.invalidMnemonic = error else {
                XCTFail("Expected invalidMnemonic, got \(error)")
                return
            }
        }

        XCTAssertFalse(nodeStopped)
        XCTAssertFalse(persistenceWiped)
        XCTAssertNil(mockStorage.mockPendingMnemonic)
        XCTAssertNil(mockStorage.mockMnemonic)
    }

    func testRestoreHappyPath() throws {
        try mockStorage.storeMnemonic(testMnemonic)
        var nodeStopped = false
        var persistenceWiped = false

        try manager.restoreMnemonic(
            otherMnemonic,
            onStopNode: { nodeStopped = true },
            onWipePersistence: {
                persistenceWiped = true
                try self.mockStorage.deleteMnemonic()
            }
        )

        XCTAssertTrue(nodeStopped)
        XCTAssertTrue(persistenceWiped)
        XCTAssertEqual(mockStorage.mockMnemonic, otherMnemonic)
        XCTAssertNil(mockStorage.mockPendingMnemonic)
        XCTAssertNil(UserDefaults(suiteName: testAppGroup)?.string(forKey: "restore_phase"))
        XCTAssertFalse(UserDefaults(suiteName: testAppGroup)?.bool(forKey: "restore_in_progress") ?? false)
    }

    func testRestoreWipeFailureRetainsPhase() throws {
        try mockStorage.storeMnemonic(testMnemonic)

        enum TestError: Error { case wipeFailed }

        XCTAssertThrowsError(try manager.restoreMnemonic(
            otherMnemonic,
            onStopNode: {},
            onWipePersistence: { throw TestError.wipeFailed }
        ))

        // Phase must be retained so recovery triggers on next launch
        XCTAssertEqual(
            UserDefaults(suiteName: testAppGroup)?.string(forKey: "restore_phase"),
            RestorePhase.pendingValidation.rawValue
        )
        XCTAssertEqual(mockStorage.mockPendingMnemonic, otherMnemonic)
    }

    // MARK: - Interrupted Recovery

    func testRecoveryFromPendingValidationPhase() throws {
        let ud = UserDefaults(suiteName: testAppGroup)
        ud?.set(RestorePhase.pendingValidation.rawValue, forKey: "restore_phase")
        try mockStorage.storePendingMnemonic(otherMnemonic)

        var persistenceWiped = false
        try manager.runRecoveryIfNeeded(onWipePersistence: {
            persistenceWiped = true
            try self.mockStorage.deleteMnemonic()
        })

        XCTAssertTrue(persistenceWiped)
        XCTAssertEqual(mockStorage.mockMnemonic, otherMnemonic)
        XCTAssertNil(mockStorage.mockPendingMnemonic)
        XCTAssertNil(ud?.string(forKey: "restore_phase"))
    }

    func testRecoveryFromOldPersistenceWipedPhase() throws {
        let ud = UserDefaults(suiteName: testAppGroup)
        ud?.set(RestorePhase.oldPersistenceWiped.rawValue, forKey: "restore_phase")
        try mockStorage.storePendingMnemonic(otherMnemonic)

        var persistenceWiped = false
        try manager.runRecoveryIfNeeded(onWipePersistence: {
            persistenceWiped = true
        })

        // In oldPersistenceWiped phase, wipe was already completed before crash
        XCTAssertFalse(persistenceWiped)
        XCTAssertEqual(mockStorage.mockMnemonic, otherMnemonic)
        XCTAssertNil(mockStorage.mockPendingMnemonic)
        XCTAssertNil(ud?.string(forKey: "restore_phase"))
    }
}

// MARK: - Mock Storage

private final class MockLifecycleMnemonicStorage: MnemonicStorageProtocol {
    var mockLoadError: Error?
    var mockMnemonic: String?
    var mockPendingMnemonic: String?

    func storeMnemonic(_ mnemonic: String) throws {
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
