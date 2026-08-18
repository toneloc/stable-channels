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
            appGroupIdentifier: testAppGroup,
            validator: { mnemonic in
                let words = mnemonic.split(whereSeparator: \.isWhitespace)
                guard [12, 15, 18, 21, 24].contains(words.count) else { return false }
                guard !mnemonic.contains("foo") else { return false }
                guard !mnemonic.hasSuffix("abandon abandon") else { return false }
                return true
            }
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

    func testRestoreRejectsInvalidWordCountWithoutTouchingStorage() async {
        var nodeStopped = false
        var persistenceWiped = false

        do {
            try await manager.restoreMnemonic(
                "one two three four five",
                onStopNode: { nodeStopped = true },
                onWipePersistence: { persistenceWiped = true }
            )
            XCTFail("Expected invalidMnemonic")
        } catch {
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

    func testRestoreRejectsInvalidBip39WordWithoutTouchingStorage() async {
        var nodeStopped = false
        var persistenceWiped = false
        let invalidWordMnemonic = "foo foo foo foo foo foo foo foo foo foo foo foo"

        do {
            try await manager.restoreMnemonic(
                invalidWordMnemonic,
                onStopNode: { nodeStopped = true },
                onWipePersistence: { persistenceWiped = true }
            )
            XCTFail("Expected invalidMnemonic")
        } catch {
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

    func testRestoreRejectsInvalidBip39ChecksumWithoutTouchingStorage() async {
        var nodeStopped = false
        var persistenceWiped = false
        // 12th word is 'abandon' instead of 'about', creating an invalid BIP-39 checksum
        let invalidChecksumMnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon"

        do {
            try await manager.restoreMnemonic(
                invalidChecksumMnemonic,
                onStopNode: { nodeStopped = true },
                onWipePersistence: { persistenceWiped = true }
            )
            XCTFail("Expected invalidMnemonic")
        } catch {
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

    func testRestoreHappyPath() async throws {
        try mockStorage.storeMnemonic(testMnemonic)
        var nodeStopped = false
        var persistenceWiped = false

        try await manager.restoreMnemonic(
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

    func testRestoreWipeFailureRetainsPhase() async throws {
        try mockStorage.storeMnemonic(testMnemonic)

        enum TestError: Error { case wipeFailed }

        do {
            try await manager.restoreMnemonic(
                otherMnemonic,
                onStopNode: {},
                onWipePersistence: { throw TestError.wipeFailed }
            )
            XCTFail("Expected wipe failure to propagate")
        } catch {
            // expected
        }

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

    func testRecoveryKeychainErrorPreservesRestorePhase() throws {
        let ud = UserDefaults(suiteName: testAppGroup)
        ud?.set(RestorePhase.pendingValidation.rawValue, forKey: "restore_phase")
        mockStorage.mockPendingLoadError = WalletKeychainError.accessDenied(errSecAuthFailed)

        XCTAssertThrowsError(try manager.runRecoveryIfNeeded(onWipePersistence: {}))

        // Must preserve restore_phase so recovery retries when Keychain access is restored
        XCTAssertEqual(ud?.string(forKey: "restore_phase"), RestorePhase.pendingValidation.rawValue)
    }

    func testRecoveryKeyNotFoundClearsDanglingPhaseWhenActiveSeedPresent() throws {
        let ud = UserDefaults(suiteName: testAppGroup)
        ud?.set(RestorePhase.pendingValidation.rawValue, forKey: "restore_phase")
        mockStorage.mockMnemonic = testMnemonic // Active seed exists

        try manager.runRecoveryIfNeeded(onWipePersistence: {})

        // Dangling phase should be cleared safely when active wallet is intact
        XCTAssertNil(ud?.string(forKey: "restore_phase"))
    }

    func testRecoveryWithRestoreMarkerFailsClosedWhenBothPendingAndActiveSeedsMissing() throws {
        let ud = UserDefaults(suiteName: testAppGroup)
        ud?.set(RestorePhase.pendingValidation.rawValue, forKey: "restore_phase")
        // Neither active nor pending seed exists in mockStorage

        XCTAssertThrowsError(try manager.runRecoveryIfNeeded(onWipePersistence: {})) { error in
            guard let restoreError = error as? WalletRestoreError,
                  case .recoveryFailed = restoreError else {
                XCTFail("Expected WalletRestoreError.recoveryFailed but got \(error)")
                return
            }
        }

        // Marker must be preserved so system fails closed and does not start as a fresh blank wallet
        XCTAssertEqual(ud?.string(forKey: "restore_phase"), RestorePhase.pendingValidation.rawValue)
    }

    func testRestoreRealWipePreservesPendingSeedAndPromotesSuccessfully() async throws {
        let ud = UserDefaults(suiteName: testAppGroup)
        mockStorage.mockMnemonic = testMnemonic

        var wipeExecuted = false
        // Simulate real persistence wipe: wipes files and active Keychain seed, but NOT pending seed
        try await manager.restoreMnemonic(
            otherMnemonic,
            onStopNode: {},
            onWipePersistence: {
                wipeExecuted = true
                try mockStorage.deleteMnemonic() // real wipe deletes active seed only
                XCTAssertEqual(
                    mockStorage.mockPendingMnemonic,
                    otherMnemonic,
                    "Pending seed must remain in Keychain during wipe!"
                )
            }
        )

        XCTAssertTrue(wipeExecuted)
        XCTAssertEqual(mockStorage.mockMnemonic, otherMnemonic)
        XCTAssertNil(mockStorage.mockPendingMnemonic)
        XCTAssertNil(ud?.string(forKey: "restore_phase"))
    }

    func testInterruptedRestoreAfterWipeRecoversFromPendingSeed() throws {
        let ud = UserDefaults(suiteName: testAppGroup)
        ud?.set(RestorePhase.oldPersistenceWiped.rawValue, forKey: "restore_phase")
        mockStorage.mockPendingMnemonic = otherMnemonic
        mockStorage.mockMnemonic = nil // Active seed was wiped before the crash

        var wipeCalled = false
        try manager.runRecoveryIfNeeded(onWipePersistence: {
            wipeCalled = true
        })

        // In oldPersistenceWiped phase, wipe is already complete; pending seed promotes to active
        XCTAssertFalse(wipeCalled)
        XCTAssertEqual(mockStorage.mockMnemonic, otherMnemonic)
        XCTAssertNil(mockStorage.mockPendingMnemonic)
        XCTAssertNil(ud?.string(forKey: "restore_phase"))
    }

    // MARK: - Markerless Recovery (pending Keychain slot is authoritative)

    func testMarkerlessPendingSeedWithoutActiveIsPromoted() throws {
        // The UserDefaults phase marker was lost (unflushed cache on a hard kill)
        // after the wipe: no marker, no active seed, but the pending seed survives.
        mockStorage.mockPendingMnemonic = otherMnemonic
        mockStorage.mockMnemonic = nil

        try manager.runRecoveryIfNeeded(onWipePersistence: {})

        XCTAssertEqual(mockStorage.mockMnemonic, otherMnemonic)
        XCTAssertNil(mockStorage.mockPendingMnemonic)
    }

    func testMarkerlessPendingSeedWithActiveIsCleared() throws {
        // The restore never reached the wipe: active seed intact, pending is
        // abandoned staging. It must be cleared, never promoted.
        mockStorage.mockMnemonic = testMnemonic
        mockStorage.mockPendingMnemonic = otherMnemonic

        try manager.runRecoveryIfNeeded(onWipePersistence: {})

        XCTAssertEqual(mockStorage.mockMnemonic, testMnemonic)
        XCTAssertNil(mockStorage.mockPendingMnemonic)
    }

    func testMarkerlessRecoveryFailsClosedOnKeychainError() {
        // An operational Keychain error must not be read as "no active seed" and
        // trigger promotion over a possibly-live wallet.
        mockStorage.mockPendingMnemonic = otherMnemonic
        mockStorage.mockLoadError = WalletKeychainError.accessDenied(errSecAuthFailed)

        XCTAssertThrowsError(try manager.runRecoveryIfNeeded(onWipePersistence: {}))
        XCTAssertEqual(mockStorage.mockPendingMnemonic, otherMnemonic)
    }
}

// MARK: - Mock Storage

private final class MockLifecycleMnemonicStorage: MnemonicStorageProtocol {
    var mockLoadError: Error?
    var mockPendingLoadError: Error?
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
        if let error = mockPendingLoadError {
            throw error
        }
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
