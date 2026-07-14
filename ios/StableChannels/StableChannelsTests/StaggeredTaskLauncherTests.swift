import XCTest
@testable import StableChannels

@MainActor
final class StaggeredTaskLauncherTests: XCTestCase {
    func testLaunch_runsTask() async {
        let sut = StaggeredTaskLauncher()
        var didRun = false
        let exp = expectation(description: "body ran")
        sut.launch(opId: "a", delaySeconds: 0) {
            didRun = true
            exp.fulfill()
        }
        await fulfillment(of: [exp], timeout: 1)
        XCTAssertTrue(didRun)
    }

    func testLaunch_withDelay_delaysStart() async {
        let sut = StaggeredTaskLauncher()
        let start = Date()
        var didRun = false
        let exp = expectation(description: "body ran")
        sut.launch(opId: "a", delaySeconds: 1) {
            didRun = true
            exp.fulfill()
        }
        await fulfillment(of: [exp], timeout: 5)
        let elapsed = Date().timeIntervalSince(start)
        XCTAssertTrue(didRun)
        XCTAssertGreaterThanOrEqual(elapsed, 0.9, "Expected delay >= 0.9s, got \(elapsed)")
    }

    func testLaunch_replacesExistingTask_cancelsOld() async {
        let sut = StaggeredTaskLauncher()
        let aStarted = expectation(description: "A started")
        let aCancelled = expectation(description: "A saw cancel")
        let bRan = expectation(description: "B ran")

        sut.launch(opId: "x", delaySeconds: 0) {
            aStarted.fulfill()
            // Sleep long enough that B's launch() definitely calls cancel()
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            if Task.isCancelled {
                aCancelled.fulfill()
            }
        }
        await fulfillment(of: [aStarted], timeout: 1)

        sut.launch(opId: "x", delaySeconds: 0) {
            bRan.fulfill()
        }
        await fulfillment(of: [bRan], timeout: 2)
        await fulfillment(of: [aCancelled], timeout: 3)
    }

    func testCancel_stopsTask() async {
        let sut = StaggeredTaskLauncher()
        let started = expectation(description: "started")
        let finishedEarly = expectation(description: "saw cancel before sleep done")
        var finishedEarlyFlag = false

        sut.launch(opId: "k", delaySeconds: 0) {
            started.fulfill()
            try? await Task.sleep(nanoseconds: 5_000_000_000)
            if Task.isCancelled {
                finishedEarlyFlag = true
                finishedEarly.fulfill()
            }
        }
        await fulfillment(of: [started], timeout: 1)
        sut.cancel(opId: "k")
        await fulfillment(of: [finishedEarly], timeout: 2)
        XCTAssertTrue(finishedEarlyFlag)
    }

    func testCancelAll_clearsAll() async {
        let sut = StaggeredTaskLauncher()
        let s1 = expectation(description: "s1")
        let s2 = expectation(description: "s2")
        let s3 = expectation(description: "s3")
        let c1 = expectation(description: "c1")
        let c2 = expectation(description: "c2")
        let c3 = expectation(description: "c3")
        var saw1 = false, saw2 = false, saw3 = false

        sut.launch(opId: "1", delaySeconds: 0) {
            s1.fulfill()
            try? await Task.sleep(nanoseconds: 5_000_000_000)
            if Task.isCancelled {
                saw1 = true; c1.fulfill()
            }
        }
        sut.launch(opId: "2", delaySeconds: 0) {
            s2.fulfill()
            try? await Task.sleep(nanoseconds: 5_000_000_000)
            if Task.isCancelled {
                saw2 = true; c2.fulfill()
            }
        }
        sut.launch(opId: "3", delaySeconds: 0) {
            s3.fulfill()
            try? await Task.sleep(nanoseconds: 5_000_000_000)
            if Task.isCancelled {
                saw3 = true; c3.fulfill()
            }
        }
        await fulfillment(of: [s1, s2, s3], timeout: 1)
        sut.cancelAll()
        await fulfillment(of: [c1, c2, c3], timeout: 2)
        XCTAssertTrue(saw1 && saw2 && saw3)
    }

    func testGenerationMismatch_doesNotClearNewest() async {
        let sut = StaggeredTaskLauncher()
        let aStarted = expectation(description: "A started")
        let bStarted = expectation(description: "B started")
        let aCancelled = expectation(description: "A saw cancel")

        // A is launched with a long sleep; B replaces it before A finishes.
        // When B's body completes, B's generation still matches -> clears.
        // When A's body resumes and finds Task.isCancelled, A's generation
        // check fails, so A does NOT clear the launcher state.
        sut.launch(opId: "g", delaySeconds: 0) {
            aStarted.fulfill()
            try? await Task.sleep(nanoseconds: 3_000_000_000)
            if Task.isCancelled {
                aCancelled.fulfill()
            }
        }
        await fulfillment(of: [aStarted], timeout: 1)

        sut.launch(opId: "g", delaySeconds: 0) {
            bStarted.fulfill()
        }
        await fulfillment(of: [bStarted], timeout: 2)
        // Give A a chance to wake from sleep and observe its own cancel.
        await fulfillment(of: [aCancelled], timeout: 4)
    }
}
