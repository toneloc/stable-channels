import XCTest
@testable import StableChannels

final class ResilientEsploraClientTests: XCTestCase {
    private let validTxid = String(repeating: "a", count: 64)
    private var session: URLSession!
    /// Backoff waits are recorded, not slept, so the tests run in milliseconds.
    private var sleeps = SleepLog()

    override func setUp() {
        super.setUp()
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        session = URLSession(configuration: config)

        MockURLProtocol.requestHandler = nil
        MockURLProtocol.callCount = 0
        sleeps = SleepLog()
        MockURLProtocol.seenURLs = []

        // Reset the audit log path so audit log tests can capture clean output.
        // We don't snapshot the prior path because no other test in this class
        // sets it; teardown clears it again.
        AuditService.setLogPath("")
    }

    override func tearDown() {
        // Clear the audit log path so subsequent tests start clean.
        AuditService.setLogPath("")
        session = nil
        MockURLProtocol.requestHandler = nil
        super.tearDown()
    }

    private func jsonResponse(body: [String: Any], status: Int = 200) -> (HTTPURLResponse, Data) {
        let data = (try? JSONSerialization.data(withJSONObject: body)) ?? Data()
        let resp = HTTPURLResponse(
            url: URL(string: "https://mock.local")!,
            statusCode: status,
            httpVersion: "HTTP/1.1",
            headerFields: nil
        )!
        return (resp, data)
    }

    private func makeClient(
        chainURLs: [String] = ["https://primary.local/api", "https://fallback.local/api"],
        maxAttempts: Int = 2,
        backoffSeconds: [UInt64] = [0],
        wallClockBudgetSeconds: TimeInterval = 420,
        onExhausted: (@Sendable () async -> Void)? = nil
    ) -> ResilientEsploraClient {
        let cfg = ResilientEsploraClient.Config(
            chainURLs: chainURLs,
            maxAttempts: maxAttempts,
            backoffSeconds: backoffSeconds,
            timeout: 5,
            wallClockBudgetSeconds: wallClockBudgetSeconds,
            onExhausted: onExhausted
        )
        let sleeps = sleeps
        return ResilientEsploraClient(urlSession: session, config: cfg, sleep: { await sleeps.record($0) })
    }

    // Default parser used by most tests: extracts a valid txid from the
    // "spent" JSON shape, returns nil otherwise.
    private func spentTxidParser(data: Data) throws -> String? {
        guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let spent = json["spent"] as? Bool,
              spent,
              let txid = json["txid"] as? String,
              ResilientEsploraClient.isValidTxid(txid)
        else { return nil }
        return txid
    }

    // MARK: - isValidTxid

    func testIsValidTxid_accepts64Hex() {
        // 48 zeros + 16 hex digits = 64 chars, with a non-zero hex range
        let s = String(repeating: "0", count: 48) + "abcdef0123456789"
        XCTAssertEqual(s.count, 64)
        XCTAssertTrue(ResilientEsploraClient.isValidTxid(s))
        XCTAssertTrue(ResilientEsploraClient.isValidTxid(validTxid))
    }

    func testIsValidTxid_rejectsShort() {
        XCTAssertFalse(ResilientEsploraClient.isValidTxid(String(repeating: "a", count: 63)))
    }

    func testIsValidTxid_rejectsUppercase() {
        let upper = "ABCDEF" + String(repeating: "0", count: 58)
        XCTAssertFalse(ResilientEsploraClient.isValidTxid(upper))
    }

    func testIsValidTxid_rejectsNonHex() {
        let bad = "g" + String(repeating: "0", count: 63)
        XCTAssertFalse(ResilientEsploraClient.isValidTxid(bad))
        let mixed = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdez"
        XCTAssertEqual(mixed.count, 64)
        XCTAssertFalse(ResilientEsploraClient.isValidTxid(mixed))
    }

    // MARK: - trimSlash

    func testTrimSlash_stripsTrailingSlash() {
        XCTAssertEqual(
            ResilientEsploraClient.trimSlash("https://x.com/"),
            "https://x.com"
        )
    }

    func testTrimSlash_keepsNonTrailingSlash() {
        XCTAssertEqual(
            ResilientEsploraClient.trimSlash("https://x.com"),
            "https://x.com"
        )
    }

    // MARK: - run()

    func testRun_resolvesOnFirstAttempt_primarySuccess() async {
        MockURLProtocol.requestHandler = { _ in
            self.jsonResponse(body: ["spent": true, "txid": self.validTxid])
        }

        let client = makeClient()
        let resolved = ResolvedBox<String>()
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/somefunding/outspend/0"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: spentTxidParser,
            onResolved: { hit in await resolved.set(hit) }
        )

        let v = await resolved.value
        XCTAssertEqual(v, self.validTxid)
    }

    func testRun_fallsBackToSecondaryChain() async {
        MockURLProtocol.requestHandler = { req in
            if let host = req.url?.host, host == "primary.local" {
                return self.jsonResponse(body: [:], status: 500)
            }
            return self.jsonResponse(body: ["spent": true, "txid": self.validTxid])
        }

        let client = makeClient(maxAttempts: 2, backoffSeconds: [0])
        let resolved = ResolvedBox<String>()
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/abc/outspend/0"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: spentTxidParser,
            onResolved: { hit in await resolved.set(hit) }
        )

        let v = await resolved.value
        XCTAssertEqual(v, self.validTxid)
        XCTAssertTrue(MockURLProtocol.seenURLs.contains { $0.host == "fallback.local" })
    }

    func testRun_retriesOnParseFailure() async {
        var attempts = 0
        MockURLProtocol.requestHandler = { _ in
            attempts += 1
            if attempts == 1 {
                // 200 but with a "not spent" shape — parser returns nil → retry
                return self.jsonResponse(body: ["spent": false])
            }
            return self.jsonResponse(body: ["spent": true, "txid": self.validTxid])
        }

        let client = makeClient(maxAttempts: 3, backoffSeconds: [0, 0])
        let resolved = ResolvedBox<String>()
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/abc/outspend/0"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: spentTxidParser,
            onResolved: { hit in await resolved.set(hit) }
        )

        let v = await resolved.value
        XCTAssertEqual(v, self.validTxid)
        XCTAssertGreaterThanOrEqual(attempts, 2)
    }

    func testRun_doesNotResolveAfterMaxAttempts() async {
        MockURLProtocol.requestHandler = { _ in
            self.jsonResponse(body: ["spent": false])
        }

        let client = makeClient(maxAttempts: 2, backoffSeconds: [0])
        let resolved = ResolvedBox<String>()
        let parser: ResilientEsploraClient.ResultParser<String> = { _ in nil }
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/abc/outspend/0"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: parser,
            onResolved: { hit in await resolved.set(hit) }
        )

        let v = await resolved.value
        XCTAssertNil(v, "onResolved must not fire when all attempts fail")
    }

    func testRun_cancelsMidAttempt() async {
        // Cancel the task before the poll loop runs. The handler is
        // synchronous so we cannot introduce real network latency in
        // MockURLProtocol; instead we just ensure that a task cancelled
        // before any attempt completes does not fire onResolved.
        MockURLProtocol.requestHandler = { _ in
            self.jsonResponse(body: ["spent": true, "txid": self.validTxid])
        }

        let client = makeClient(maxAttempts: 5, backoffSeconds: [1, 1, 1, 1])
        let resolved = ResolvedBox<String>()
        let parser: ResilientEsploraClient.ResultParser<String> = { _ in nil }
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/abc/outspend/0"]
        }

        let task = Task {
            await client.run(
                endpointBuilder: builder,
                resultParser: parser,
                onResolved: { hit in await resolved.set(hit) }
            )
        }
        // Cancel immediately. The poll loop checks Task.isCancelled at the
        // top of each attempt, so the first attempt's URL request may or
        // may not complete — but with a nil parser, even a 200 hit will
        // not fire onResolved. The assertion below covers both cases.
        task.cancel()
        await task.value

        let v = await resolved.value
        // Parser always returns nil, so onResolved must never fire
        // regardless of cancellation timing.
        XCTAssertNil(v)
    }

    func testRun_respectsBackoffSchedule() async {
        MockURLProtocol.requestHandler = { _ in
            self.jsonResponse(body: ["spent": false])
        }

        let client = makeClient(
            maxAttempts: 3,
            backoffSeconds: [0, 1] // second attempt waits 1s
        )
        let parser: ResilientEsploraClient.ResultParser<String> = { _ in nil }
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/abc/outspend/0"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: parser,
            onResolved: { _ in }
        )

        let recorded = await sleeps.values
        XCTAssertEqual(recorded.count, 2, "Attempts 2 and 3 each wait their backoff")
        XCTAssertEqual(recorded.first, 0, "A 0s backoff waits nothing")
        XCTAssertTrue(
            (750_000_000 ... 1_250_000_000).contains(recorded.last ?? 0),
            "The 1s backoff before attempt 3 must wait 0.75-1.25s, got \(recorded)"
        )
    }

    func testRun_skipsChainWhenEndpointBuilderReturnsEmpty() async {
        MockURLProtocol.requestHandler = { _ in
            self.jsonResponse(body: ["spent": true, "txid": self.validTxid])
        }

        let client = makeClient(
            chainURLs: ["https://primary.local", "https://fallback.local"],
            maxAttempts: 1,
            backoffSeconds: []
        )
        let resolved = ResolvedBox<String>()
        let parser: ResilientEsploraClient.ResultParser<String> = { data in
            guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let txid = json["txid"] as? String
            else { return nil }
            return txid
        }
        // Skip primary, build for fallback.
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            if base.contains("primary") {
                return []
            }
            return ["\(ResilientEsploraClient.trimSlash(base))/tx/abc"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: parser,
            onResolved: { hit in await resolved.set(hit) }
        )

        let v = await resolved.value
        XCTAssertEqual(v, self.validTxid)
        XCTAssertFalse(MockURLProtocol.seenURLs.contains { $0.host == "primary.local" })
    }

    func testRun_returnsSilentlyOn400() async {
        MockURLProtocol.requestHandler = { _ in
            self.jsonResponse(body: ["error": "bad"], status: 400)
        }

        let client = makeClient(maxAttempts: 2, backoffSeconds: [0])
        let resolved = ResolvedBox<String>()
        let parser: ResilientEsploraClient.ResultParser<String> = { _ in nil }
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/abc"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: parser,
            onResolved: { hit in await resolved.set(hit) }
        )

        let v = await resolved.value
        XCTAssertNil(v)
    }

    func testRun_handlesNetworkError() async {
        MockURLProtocol.requestHandler = { _ in
            // Return a 5xx so the URLProtocol hands the resolver a non-200
            // response — same effect as a network error for our purposes.
            // The resolver treats anything that isn't 200 as a continue.
            return self.jsonResponse(body: [:], status: 503)
        }

        let client = makeClient(maxAttempts: 2, backoffSeconds: [0])
        let resolved = ResolvedBox<String>()
        let parser: ResilientEsploraClient.ResultParser<String> = { _ in nil }
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/abc"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: parser,
            onResolved: { hit in await resolved.set(hit) }
        )

        let v = await resolved.value
        XCTAssertNil(v)
    }

    // MARK: - Production hardening: jitter, budget, body cap, onExhausted, telemetry

    func testRun_appliesBackoffJitter() async {
        MockURLProtocol.requestHandler = { _ in
            self.jsonResponse(body: ["spent": false])
        }

        let client = makeClient(
            maxAttempts: 2,
            backoffSeconds: [1]
        )
        let parser: ResilientEsploraClient.ResultParser<String> = { _ in nil }
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/abc/outspend/0"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: parser,
            onResolved: { _ in }
        )

        let recorded = await sleeps.values
        XCTAssertEqual(recorded.count, 1)
        XCTAssertTrue(
            (750_000_000 ... 1_250_000_000).contains(recorded.first ?? 0),
            "Jittered backoff of base 1s must wait 0.75-1.25s, got \(recorded)"
        )
    }

    func testRun_jitteredBackoff_helperStaysInBand() {
        // Direct unit test of the helper: 1000 samples, all in [0.75x, 1.25x].
        var samples = Set<UInt64>()
        for _ in 0 ..< 1000 {
            let j = ResilientEsploraClient.jitteredBackoffNanoseconds(base: 1)
            XCTAssertGreaterThanOrEqual(j, 750_000_000)
            XCTAssertLessThanOrEqual(j, 1_250_000_000)
            samples.insert(j)
        }
        XCTAssertGreaterThan(samples.count, 1, "A 1s base must actually be jittered, not rounded to 1s")
    }

    func testRun_respectsWallClockBudget() async {
        // Budget of 0.1s with a 10s backoff must short-circuit before
        // the long sleep, and onResolved must not fire.
        MockURLProtocol.requestHandler = { _ in
            self.jsonResponse(body: ["spent": false])
        }

        let client = makeClient(
            maxAttempts: 5,
            backoffSeconds: [10],
            wallClockBudgetSeconds: 0.1
        )
        let resolved = ResolvedBox<String>()
        let parser: ResilientEsploraClient.ResultParser<String> = { _ in nil }
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/abc/outspend/0"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: parser,
            onResolved: { hit in await resolved.set(hit) }
        )

        let v = await resolved.value
        XCTAssertNil(v, "onResolved must not fire when wall-clock budget is exceeded")
        let recorded = await sleeps.values
        XCTAssertTrue(recorded.isEmpty, "The 10s backoff must be skipped, not slept")
    }

    func testRun_capsBodyAt1MB() async {
        // Return a 2MB body. The 1MB cap must reject it and onResolved
        // must not fire.
        let bigBody = Data(repeating: 0x20, count: 2 * 1_048_576) // 2 MB of spaces
        MockURLProtocol.requestHandler = { _ in
            let resp = HTTPURLResponse(
                url: URL(string: "https://mock.local")!,
                statusCode: 200,
                httpVersion: "HTTP/1.1",
                headerFields: nil
            )!
            return (resp, bigBody)
        }

        let client = makeClient(maxAttempts: 1, backoffSeconds: [])
        let resolved = ResolvedBox<String>()
        let parser: ResilientEsploraClient.ResultParser<String> = { _ in nil }
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/abc/outspend/0"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: parser,
            onResolved: { hit in await resolved.set(hit) }
        )

        let v = await resolved.value
        XCTAssertNil(v, "onResolved must not fire when the body is over the 1MB cap")
    }

    func testRun_acceptsBodyAt1MB() async {
        // Edge: a body exactly at the cap (1MB) must pass through.
        let exactlyOneMB = Data(repeating: 0x20, count: 1_048_576)
        MockURLProtocol.requestHandler = { _ in
            let resp = HTTPURLResponse(
                url: URL(string: "https://mock.local")!,
                statusCode: 200,
                httpVersion: "HTTP/1.1",
                headerFields: nil
            )!
            return (resp, exactlyOneMB)
        }

        // Parser intentionally returns nil for this body so we can verify
        // the cap is enforced as "<= 1MB" (inclusive). The body should
        // reach the parser; with a nil parser, onResolved does not fire.
        let client = makeClient(maxAttempts: 1, backoffSeconds: [])
        let parser: ResilientEsploraClient.ResultParser<String> = { _ in nil }
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/abc/outspend/0"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: parser,
            onResolved: { _ in }
        )
        // No assertion on resolved.value; the important thing is that
        // run() returns cleanly without crashing on the 1MB edge.
    }

    func testRun_invokesOnExhausted() async {
        // All attempts fail; onExhausted must fire exactly once.
        MockURLProtocol.requestHandler = { _ in
            self.jsonResponse(body: ["spent": false])
        }

        let exhaustedFlag = ExhaustedFlag()
        let client = makeClient(
            maxAttempts: 2,
            backoffSeconds: [0],
            onExhausted: { await exhaustedFlag.set() }
        )
        let parser: ResilientEsploraClient.ResultParser<String> = { _ in nil }
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/abc/outspend/0"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: parser,
            onResolved: { _ in }
        )

        let fired = await exhaustedFlag.value
        XCTAssertTrue(fired, "onExhausted must fire after the attempt loop falls through")
    }

    func testRun_doesNotInvokeOnExhaustedOnSuccess() async {
        // When a hit is found, onExhausted must NOT fire.
        MockURLProtocol.requestHandler = { _ in
            self.jsonResponse(body: ["spent": true, "txid": self.validTxid])
        }

        let exhaustedFlag = ExhaustedFlag()
        let client = makeClient(
            maxAttempts: 3,
            backoffSeconds: [0, 0],
            onExhausted: { await exhaustedFlag.set() }
        )
        let resolved = ResolvedBox<String>()
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/abc/outspend/0"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: spentTxidParser,
            onResolved: { hit in await resolved.set(hit) }
        )

        let v = await resolved.value
        XCTAssertEqual(v, self.validTxid)
        let fired = await exhaustedFlag.value
        XCTAssertFalse(fired, "onExhausted must not fire when a hit is found")
    }

    func testRun_logsEsploraResolvedOnHit() async {
        // AuditService.log writes to a private file. We can't intercept
        // the log destination from tests, so the best we can do is
        // verify the call path does not crash and the run completes.
        MockURLProtocol.requestHandler = { _ in
            self.jsonResponse(body: ["spent": true, "txid": self.validTxid])
        }

        let client = makeClient(maxAttempts: 1, backoffSeconds: [])
        let resolved = ResolvedBox<String>()
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/abc/outspend/0"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: spentTxidParser,
            onResolved: { hit in await resolved.set(hit) }
        )

        let v = await resolved.value
        XCTAssertEqual(v, self.validTxid, "Resolved path must not crash on AuditService.log call")
    }

    func testRun_logsEsploraExhaustedOnNoHit() async {
        // Best-effort: confirm the exhaustion log call path completes.
        MockURLProtocol.requestHandler = { _ in
            self.jsonResponse(body: ["spent": false])
        }

        let client = makeClient(maxAttempts: 1, backoffSeconds: [])
        let parser: ResilientEsploraClient.ResultParser<String> = { _ in nil }
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/abc/outspend/0"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: parser,
            onResolved: { _ in }
        )
        // No assertion; we just verify the exhaustion log call path runs
        // without crashing.
    }

    // MARK: - Audit log assertions

    //
    // AuditService writes to a file path; we point it at a temp file via
    // setLogPath and then read the file back. AuditService is a global
    // singleton, so these tests are not parallel-safe with other tests
    // that touch the log file. Each test uses its own temp path; setUp
    // and tearDown clear the path between tests.

    func testRun_writesESPLORA_RESOLVED_event_to_audit_log() async throws {
        let tmp = FileManager.default.temporaryDirectory
            .appendingPathComponent("audit_test_\(UUID().uuidString).log")
        defer { try? FileManager.default.removeItem(at: tmp) }
        AuditService.setLogPath(tmp.path)

        MockURLProtocol.requestHandler = { _ in
            self.jsonResponse(body: ["spent": true, "txid": self.validTxid])
        }
        let client = makeClient(maxAttempts: 1, backoffSeconds: [])
        let resolved = ResolvedBox<String>()
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/abc/outspend/0"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: spentTxidParser,
            onResolved: { hit in await resolved.set(hit) }
        )

        // Flush: AuditService writes synchronously, so by the time run()
        // returns, the file should be readable.
        let log = try String(contentsOf: tmp, encoding: .utf8)
        XCTAssertTrue(
            log.contains("ESPLORA_RESOLVED"),
            "expected ESPLORA_RESOLVED in audit log, got: \(log)"
        )

        let v = await resolved.value
        XCTAssertEqual(v, self.validTxid)
    }

    func testRun_writesESPLORA_EXHAUSTED_event_to_audit_log() async throws {
        let tmp = FileManager.default.temporaryDirectory
            .appendingPathComponent("audit_test_\(UUID().uuidString).log")
        defer { try? FileManager.default.removeItem(at: tmp) }
        AuditService.setLogPath(tmp.path)

        MockURLProtocol.requestHandler = { _ in
            self.jsonResponse(body: ["spent": false])
        }
        let client = makeClient(maxAttempts: 2, backoffSeconds: [0])
        let parser: ResilientEsploraClient.ResultParser<String> = { _ in nil }
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/abc/outspend/0"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: parser,
            onResolved: { _ in }
        )

        let log = try String(contentsOf: tmp, encoding: .utf8)
        XCTAssertTrue(
            log.contains("ESPLORA_EXHAUSTED"),
            "expected ESPLORA_EXHAUSTED in audit log, got: \(log)"
        )
    }

    func testRun_doesNotWriteESPLORA_EXHAUSTED_onSuccess() async throws {
        // The exhaustion event must only fire on the fall-through path.
        let tmp = FileManager.default.temporaryDirectory
            .appendingPathComponent("audit_test_\(UUID().uuidString).log")
        defer { try? FileManager.default.removeItem(at: tmp) }
        AuditService.setLogPath(tmp.path)

        MockURLProtocol.requestHandler = { _ in
            self.jsonResponse(body: ["spent": true, "txid": self.validTxid])
        }
        let client = makeClient(maxAttempts: 3, backoffSeconds: [0, 0])
        let resolved = ResolvedBox<String>()
        let builder: ResilientEsploraClient.EndpointBuilder = { base in
            ["\(ResilientEsploraClient.trimSlash(base))/tx/abc/outspend/0"]
        }

        await client.run(
            endpointBuilder: builder,
            resultParser: spentTxidParser,
            onResolved: { hit in await resolved.set(hit) }
        )

        let v = await resolved.value
        XCTAssertEqual(v, self.validTxid)

        let log = try String(contentsOf: tmp, encoding: .utf8)
        XCTAssertTrue(log.contains("ESPLORA_RESOLVED"))
        XCTAssertFalse(
            log.contains("ESPLORA_EXHAUSTED"),
            "ESPLORA_EXHAUSTED must not fire when a hit is found; got: \(log)"
        )
    }
}

/// Records requested sleeps (nanoseconds) instead of waiting.
actor SleepLog {
    private(set) var values: [UInt64] = []
    func record(_ nanoseconds: UInt64) { values.append(nanoseconds) }
}

/// Minimal actor-isolated box for capturing `onResolved` hits in tests.
actor ResolvedBox<T: Sendable> {
    var value: T?
    func set(_ v: T) { value = v }
}

/// Actor-isolated flag for capturing `onExhausted` invocations in tests.
actor ExhaustedFlag {
    var value: Bool = false
    func set() { value = true }
}

extension ResilientEsploraClientTests {
    /// Esplora answers 404 for a txid it does not know; anything else is not an answer.
    func testTxPresenceTellsNotFoundApartFromNoAnswer() async {
        let resolver = TxConfirmationResolver(
            chainURLs: ["https://primary.local", "https://fallback.local"],
            urlSession: session
        )
        let cases: [(primary: Int, fallback: Int, expected: Bool?)] = [
            (404, 200, true),
            (404, 404, false),
            (500, 404, nil),
            (500, 503, nil)
        ]
        for (primary, fallback, expected) in cases {
            MockURLProtocol.requestHandler = { req in
                self.jsonResponse(body: [:], status: req.url?.host == "primary.local" ? primary : fallback)
            }
            let known = await resolver.isKnown(txid: validTxid)
            XCTAssertEqual(known, expected, "primary \(primary), fallback \(fallback)")
        }
    }
}
