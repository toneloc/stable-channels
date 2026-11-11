import XCTest
@testable import StableChannels

/// Records submitted addresses (nil for removal) and fails when told to. Avoids real network.
private final class StubEmailSubmitter: EmailSubmitting, @unchecked Sendable {
    var submitted: [EmailAddress?] = []
    var error: Error?

    func submit(_ email: EmailAddress?) async throws {
        submitted.append(email)
        if let error { throw error }
    }
}

private final class FakeSavedEmailStore: SavedEmailStoring {
    var savedEmail: EmailAddress?
}

private final class FakeSigner: EmailRequestSigning {
    var isRunning = true
    var nodeId = "02" + String(repeating: "ab", count: 32)
    var signedMessages: [[UInt8]] = []

    func signMessage(_ message: [UInt8]) throws -> String {
        signedMessages.append(message)
        return "zbase32signature"
    }
}

final class EmailAddressTests: XCTestCase {
    func testValidAddress_isTrimmedAndLowercased() {
        XCTAssertEqual(EmailAddress("  Satoshi@GMX.com \n")?.value, "satoshi@gmx.com")
    }

    func testInvalidAddresses_areRejected() {
        let invalid = [
            "", "plain", "@example.com", "a@", "a@b", "a@b.c", "a@@b.com", "a@b@c.com",
            "a b@c.com", "a@.com", "a@b..com", "a@b.com.",
            String(repeating: "a", count: 65) + "@b.com",
            "a@" + String(repeating: "b", count: 250) + ".com"
        ]
        for raw in invalid {
            XCTAssertNil(EmailAddress(raw), raw)
        }
    }

    // Same rule as the server: plain ASCII address characters only, so nothing the server refuses gets sent.
    func testAddressesTheServerRefuses_areRejected() {
        let refused = [
            "a<b@example.com", "a>b@example.com", "\"a\"@example.com", "a@exa<mple.com", "a@example.com/x",
            "a@exa_mple.com", "a@-example.com", "a@example-.com", "a\\b@example.com", "ä@example.com",
            "a@exämple.com", ".a@example.com", "a.@example.com", "a..b@example.com"
        ]
        for raw in refused {
            XCTAssertNil(EmailAddress(raw), raw)
        }
        for raw in ["o'brien@example.com", "a_b-c+d@sub-domain.example.co", "x@xn--bcher-kva.example"] {
            XCTAssertNotNil(EmailAddress(raw), raw)
        }
    }

    func testLongestAllowedAddress_isAccepted() {
        let raw = "a@" + String(repeating: "b", count: EmailAddress.maxLength - 6) + ".com"
        XCTAssertEqual(raw.count, EmailAddress.maxLength)
        XCTAssertNotNil(EmailAddress(raw))
    }
}

final class EmailRegistrationEnvelopeTests: XCTestCase {
    func testPayload_isSortedCompactJSON() {
        let payload = EmailRegistrationEnvelope.payload(
            email: "you@example.com",
            nodeId: "02aa",
            timestamp: 1_762_790_000
        )
        XCTAssertEqual(
            payload,
            #"{"email":"you@example.com","node_id":"02aa","ts":1762790000,"type":"REGISTER_EMAIL_V1"}"#
        )
    }

    func testPayload_forRemoval_sendsEmptyEmail() {
        let payload = EmailRegistrationEnvelope.payload(email: "", nodeId: "02aa", timestamp: 1)
        XCTAssertEqual(payload, #"{"email":"","node_id":"02aa","ts":1,"type":"REGISTER_EMAIL_V1"}"#)
    }

    func testBody_wrapsPayloadAsString() throws {
        let body = try XCTUnwrap(EmailRegistrationEnvelope.body(payload: #"{"a":"b/c"}"#, signature: "sig"))
        XCTAssertEqual(String(data: body, encoding: .utf8), #"{"payload":"{\"a\":\"b/c\"}","signature":"sig"}"#)
    }
}

final class EmailSubmissionServiceTests: XCTestCase {
    private let timestamp: TimeInterval = 1_762_790_000

    override func setUp() {
        super.setUp()
        MockURLProtocol.requestHandler = nil
        MockURLProtocol.callCount = 0
    }

    override func tearDown() {
        MockURLProtocol.requestHandler = nil
        super.tearDown()
    }

    private func makeService(signer: FakeSigner) -> EmailSubmissionService {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [MockURLProtocol.self]
        return EmailSubmissionService(
            signer: signer,
            configuration: configuration,
            now: { [timestamp] in Date(timeIntervalSince1970: timestamp) }
        )
    }

    private static func body(of request: URLRequest) -> Data? {
        if let body = request.httpBody { return body }
        guard let stream = request.httpBodyStream else { return nil }
        stream.open()
        defer { stream.close() }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 1024)
        while stream.hasBytesAvailable {
            let read = stream.read(&buffer, maxLength: buffer.count)
            guard read > 0 else { break }
            data.append(buffer, count: read)
        }
        return data
    }

    private static func response(_ request: URLRequest, status: Int) -> (HTTPURLResponse, Data) {
        (HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil, headerFields: nil)!, Data())
    }

    func testSubmit_postsSignedEnvelope() async throws {
        let signer = FakeSigner()
        var captured: URLRequest?
        var capturedBody: Data?
        MockURLProtocol.requestHandler = { request in
            captured = request
            capturedBody = Self.body(of: request)
            return Self.response(request, status: 200)
        }

        try await makeService(signer: signer).submit(EmailAddress("You@Example.com"))

        let request = try XCTUnwrap(captured)
        XCTAssertEqual(request.url?.absoluteString, Constants.emailSubmitURL)
        XCTAssertEqual(request.httpMethod, "POST")
        XCTAssertEqual(request.value(forHTTPHeaderField: "Content-Type"), "application/json")

        let envelope = try XCTUnwrap(
            JSONSerialization.jsonObject(with: XCTUnwrap(capturedBody)) as? [String: String]
        )
        let expectedPayload = EmailRegistrationEnvelope.payload(
            email: "you@example.com",
            nodeId: signer.nodeId,
            timestamp: Int64(timestamp)
        )
        XCTAssertEqual(envelope["payload"], expectedPayload)
        XCTAssertEqual(envelope["signature"], "zbase32signature")
        XCTAssertEqual(signer.signedMessages, [Array(try XCTUnwrap(expectedPayload).utf8)])
    }

    func testSubmit_nil_signsEmptyEmail() async throws {
        let signer = FakeSigner()
        MockURLProtocol.requestHandler = { Self.response($0, status: 200) }

        try await makeService(signer: signer).submit(nil)

        let payload = String(decoding: try XCTUnwrap(signer.signedMessages.first), as: UTF8.self)
        XCTAssertTrue(payload.hasPrefix(#"{"email":"","#), payload)
    }

    func testSubmit_whenNodeNotReady_failsWithoutNetwork() async {
        let stopped = FakeSigner()
        stopped.isRunning = false
        let noId = FakeSigner()
        noId.nodeId = ""

        for signer in [stopped, noId] {
            do {
                try await makeService(signer: signer).submit(EmailAddress("a@b.com"))
                XCTFail("expected walletNotReady")
            } catch {
                XCTAssertEqual(error as? EmailSubmissionError, .walletNotReady)
            }
            XCTAssertTrue(signer.signedMessages.isEmpty)
        }
        XCTAssertEqual(MockURLProtocol.callCount, 0)
    }

    func testSubmit_non2xx_isRejected() async {
        MockURLProtocol.requestHandler = { Self.response($0, status: 401) }
        do {
            try await makeService(signer: FakeSigner()).submit(EmailAddress("a@b.com"))
            XCTFail("expected rejected")
        } catch {
            XCTAssertEqual(error as? EmailSubmissionError, .rejected(statusCode: 401))
        }
    }

    func testSubmit_networkFailure_isUnreachable() async {
        // No handler: the mock protocol fails the request with a URLError.
        do {
            try await makeService(signer: FakeSigner()).submit(EmailAddress("a@b.com"))
            XCTFail("expected unreachable")
        } catch {
            XCTAssertEqual(error as? EmailSubmissionError, .unreachable)
        }
    }
}

@MainActor
final class EmailSettingsModelTests: XCTestCase {
    private func makeModel(
        stub: StubEmailSubmitter = StubEmailSubmitter(),
        store: FakeSavedEmailStore = FakeSavedEmailStore(),
        onSaved: @escaping () -> Void = {}
    ) -> EmailSettingsModel {
        EmailSettingsModel(service: stub, store: store, onSaved: onSaved)
    }

    func testSave_submitsNormalizedEmailAndRemembersIt() async {
        let stub = StubEmailSubmitter()
        let store = FakeSavedEmailStore()
        let model = makeModel(stub: stub, store: store)
        model.input = " User@Example.com "

        await model.save()

        XCTAssertEqual(stub.submitted.map { $0?.value }, ["user@example.com"])
        XCTAssertEqual(model.status, .saved)
        XCTAssertEqual(model.savedEmail?.value, "user@example.com")
        XCTAssertEqual(store.savedEmail?.value, "user@example.com")
    }

    func testSave_withInvalidInput_doesNotSubmitAndShowsHint() async {
        let stub = StubEmailSubmitter()
        let model = makeModel(stub: stub)
        model.input = "not-an-email"

        XCTAssertFalse(model.canSave)
        XCTAssertFalse(model.showsInvalidHint)
        await model.save()

        XCTAssertTrue(stub.submitted.isEmpty)
        XCTAssertEqual(model.status, .idle)
        XCTAssertTrue(model.showsInvalidHint)
    }

    func testInvalidHint_afterLeavingField_onlyForNonEmptyInput() {
        let model = makeModel()
        model.fieldLostFocus()
        XCTAssertFalse(model.showsInvalidHint)

        model.input = "a@b"
        XCTAssertTrue(model.showsInvalidHint)

        model.input = "a@b.com"
        XCTAssertFalse(model.showsInvalidHint)
    }

    func testSave_failures_keepTheirReason() async {
        let stub = StubEmailSubmitter()
        let store = FakeSavedEmailStore()
        let model = makeModel(stub: stub, store: store)
        model.input = "user@example.com"

        stub.error = EmailSubmissionError.walletNotReady
        await model.save()
        XCTAssertEqual(model.status, .failed(.walletNotReady))

        stub.error = EmailSubmissionError.rejected(statusCode: 503)
        await model.save()
        XCTAssertEqual(model.failure, .rejected(statusCode: 503))

        stub.error = URLError(.timedOut)
        await model.save()
        XCTAssertEqual(model.failure, .unreachable)
        XCTAssertNil(store.savedEmail)
    }

    func testEditingAfterSave_resetsStatus() async {
        let model = makeModel()
        model.input = "user@example.com"
        await model.save()

        model.input = "other@example.com"

        XCTAssertEqual(model.status, .idle)
    }

    func testStartChanging_prefillsSavedAddress() {
        let store = FakeSavedEmailStore()
        store.savedEmail = EmailAddress("old@example.com")
        let model = makeModel(store: store)

        model.startChanging()

        XCTAssertTrue(model.isChanging)
        XCTAssertEqual(model.input, "old@example.com")
    }

    func testRemove_submitsNilAndForgetsAddress() async {
        let stub = StubEmailSubmitter()
        let store = FakeSavedEmailStore()
        store.savedEmail = EmailAddress("old@example.com")
        let model = makeModel(stub: stub, store: store)

        await model.remove()

        XCTAssertEqual(stub.submitted.count, 1)
        XCTAssertNil(stub.submitted.first ?? nil)
        XCTAssertNil(model.savedEmail)
        XCTAssertNil(store.savedEmail)
        XCTAssertEqual(model.status, .idle)
    }

    func testRemove_whenServerFails_keepsAddress() async {
        let stub = StubEmailSubmitter()
        stub.error = EmailSubmissionError.rejected(statusCode: 401)
        let store = FakeSavedEmailStore()
        store.savedEmail = EmailAddress("old@example.com")
        let model = makeModel(stub: stub, store: store)

        await model.remove()

        XCTAssertEqual(model.savedEmail?.value, "old@example.com")
        XCTAssertEqual(store.savedEmail?.value, "old@example.com")
        XCTAssertEqual(model.failure, .rejected(statusCode: 401))
    }

    func testOnSaved_firesOnlyOnSuccess() async {
        var savedCount = 0
        let ok = makeModel { savedCount += 1 }
        ok.input = "user@example.com"
        await ok.save()

        let failing = StubEmailSubmitter()
        failing.error = EmailSubmissionError.rejected(statusCode: 500)
        let bad = makeModel(stub: failing) { savedCount += 1 }
        bad.input = "user@example.com"
        await bad.save()

        XCTAssertEqual(savedCount, 1)
    }
}

final class EmailPromptPolicyTests: XCTestCase {
    private func shows(
        _ launches: Int,
        shown: Int = 0,
        lastShown: Int = 0,
        responded: Bool = false,
        locked: Bool = false
    ) -> Bool {
        EmailPromptPolicy.shouldShow(
            launches: launches,
            timesShown: shown,
            lastShownLaunch: lastShown,
            hasResponded: responded,
            isAppLocked: locked
        )
    }

    // Gaps of 3, 9 and 27 launches between showings: launches 3, 12 and 39, then the loop repeats.
    func testShows_afterGapsOf3_9_27_thenRepeats() {
        XCTAssertFalse(shows(2))
        XCTAssertTrue(shows(3))
        XCTAssertFalse(shows(11, shown: 1, lastShown: 3))
        XCTAssertTrue(shows(12, shown: 1, lastShown: 3))
        XCTAssertFalse(shows(38, shown: 2, lastShown: 12))
        XCTAssertTrue(shows(39, shown: 2, lastShown: 12))
        XCTAssertFalse(shows(41, shown: 3, lastShown: 39))
        XCTAssertTrue(shows(42, shown: 3, lastShown: 39))
        XCTAssertFalse(shows(50, shown: 4, lastShown: 42))
        XCTAssertTrue(shows(51, shown: 4, lastShown: 42))
        XCTAssertTrue(shows(78, shown: 5, lastShown: 51))
    }

    // A showing that was held back (app locked, another sheet open) starts its gap from when it did show.
    func testDelayedShow_countsTheNextGapFromWhenItShowed() {
        XCTAssertFalse(shows(28, shown: 1, lastShown: 20))
        XCTAssertTrue(shows(29, shown: 1, lastShown: 20))
    }

    func testNeverShows_afterAnswer() {
        XCTAssertFalse(shows(3, responded: true))
        XCTAssertFalse(shows(9, shown: 1, lastShown: 3, responded: true))
    }

    // The sheet draws above the App Access overlay, so a locked phone must never get the prompt.
    func testNeverShows_whileTheAppIsLocked() {
        XCTAssertFalse(shows(3, locked: true))
        XCTAssertFalse(shows(9, shown: 1, lastShown: 3, locked: true))
    }
}

final class EmailPromptStoreTests: XCTestCase {
    private func makeDefaults() -> UserDefaults {
        let name = "EmailPromptStoreTests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        addTeardownBlock { defaults.removePersistentDomain(forName: name) }
        return defaults
    }

    func testRecordLaunchOnce_countsOncePerProcess() {
        let defaults = makeDefaults()
        let store = EmailPromptStore(defaults: defaults)
        store.recordLaunchOnce()
        store.recordLaunchOnce()
        XCTAssertEqual(store.launches, 1)

        EmailPromptStore(defaults: defaults).recordLaunchOnce()
        XCTAssertEqual(EmailPromptStore(defaults: defaults).launches, 2)
    }

    // A restored wallet is a different node: the old address and answers must not carry over.
    func testReset_clearsEverythingForANewWallet() {
        let defaults = makeDefaults()
        let store = EmailPromptStore(defaults: defaults)
        store.recordLaunchOnce()
        store.recordShown()
        store.markResponded()
        store.savedEmail = EmailAddress("a@b.com")

        store.reset()

        let reopened = EmailPromptStore(defaults: defaults)
        XCTAssertEqual(reopened.launches, 0)
        XCTAssertEqual(reopened.timesShown, 0)
        XCTAssertEqual(reopened.lastShownLaunch, 0)
        XCTAssertFalse(reopened.hasResponded)
        XCTAssertNil(reopened.savedEmail)
    }

    func testRecordShown_countsAndRemembersTheLaunch() {
        let defaults = makeDefaults()
        let store = EmailPromptStore(defaults: defaults)
        store.recordLaunchOnce()
        store.recordShown()

        let reopened = EmailPromptStore(defaults: defaults)
        XCTAssertEqual(reopened.timesShown, 1)
        XCTAssertEqual(reopened.lastShownLaunch, 1)
    }

    func testMarkResponded_persists() {
        let defaults = makeDefaults()
        EmailPromptStore(defaults: defaults).markResponded()
        XCTAssertTrue(EmailPromptStore(defaults: defaults).hasResponded)
    }

    func testSavedEmail_persistsAndClears() {
        let defaults = makeDefaults()
        EmailPromptStore(defaults: defaults).savedEmail = EmailAddress("a@b.com")
        XCTAssertEqual(EmailPromptStore(defaults: defaults).savedEmail?.value, "a@b.com")

        EmailPromptStore(defaults: defaults).savedEmail = nil
        XCTAssertNil(EmailPromptStore(defaults: defaults).savedEmail)
    }
}
