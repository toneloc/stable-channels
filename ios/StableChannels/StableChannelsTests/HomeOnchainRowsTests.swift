import XCTest
@testable import StableChannels

// MARK: - Card rules

final class HomeOnchainRowsTests: XCTestCase {
    private func payment(
        id: Int64 = 1,
        type: String = "onchain",
        direction: String = "received",
        sats: UInt64 = 10000,
        txid: String? = "tx",
        confirmations: UInt32 = 0
    ) -> PaymentRecord {
        PaymentRecord(
            id: id, paymentId: nil, paymentType: type, direction: direction, amountMsat: sats * 1000,
            amountUSD: nil, btcPrice: nil, counterparty: nil, status: "pending", createdAt: id, feeMsat: 0,
            txid: txid, address: nil, confirmations: confirmations, txBlockHeight: nil
        )
    }

    private func send(id: Int64 = 1, sats: UInt64 = 5000, txid: String? = "send-tx") -> PaymentRecord {
        payment(id: id, direction: "sent", sats: sats, txid: txid)
    }

    // MARK: Visibility

    func testHiddenAtZeroBalanceWithNothingPending() {
        XCTAssertNil(HomeOnchainRows.card(.init()))
    }

    func testPendingReceiveAloneDoesNotShowCardAtZeroBalance() {
        XCTAssertNil(HomeOnchainRows.card(.init(pendingReceives: [payment()])))
    }

    func testPendingSendKeepsCardVisibleAtZeroBalance() throws {
        let card = try XCTUnwrap(HomeOnchainRows.card(.init(pendingSends: [send()])))
        XCTAssertEqual(card.rows, [])
        XCTAssertEqual(card.sendRows.map(\.kind), [.sending])
        XCTAssertEqual(card.sendRows.first?.sign, .outgoing)
        XCTAssertFalse(card.showsActivationHint)
    }

    func testSpliceKeepsCardVisibleAtZeroBalance() {
        XCTAssertNotNil(HomeOnchainRows.card(.init(isSweeping: true)))
    }

    // MARK: Sends

    func testOneSendingRowPerPendingSend() throws {
        let card = try XCTUnwrap(HomeOnchainRows.card(.init(
            onchainBalanceSats: 1000,
            spendableOnchainSats: 1000,
            pendingSends: [send(id: 1, sats: 100), send(id: 2, sats: 200)]
        )))
        XCTAssertEqual(card.sendRows.map(\.amountSats), [100, 200])
        XCTAssertTrue(card.sendRows.allSatisfy { $0.kind == .sending && $0.sign == .outgoing })
    }

    func testPendingSendWithZeroBalanceStillListsPendingReceives() throws {
        let card = try XCTUnwrap(HomeOnchainRows.card(.init(
            pendingReceives: [payment(id: 1)],
            pendingSends: [send(id: 2)]
        )))
        XCTAssertEqual(card.rows.map(\.kind), [.receiving])
    }

    func testUnconfirmedChangeFromOwnSendIsNotADeposit() throws {
        let card = try XCTUnwrap(HomeOnchainRows.card(.init(
            onchainBalanceSats: 4000,
            spendableOnchainSats: 0,
            hasReadyChannel: true,
            lastReceiveTxid: "old-rx",
            pendingSends: [send()]
        )))
        XCTAssertEqual(card.rows, [])
        XCTAssertEqual(card.sendRows.count, 1)
    }

    func testUnconfirmedBalanceWithoutSendFallsBackToDepositRow() throws {
        let card = try XCTUnwrap(HomeOnchainRows.card(.init(
            onchainBalanceSats: 4000,
            spendableOnchainSats: 0,
            lastReceiveTxid: "rx"
        )))
        XCTAssertEqual(card.rows, [HomeOnchainRow(kind: .depositFallback, txid: "rx")])
        XCTAssertTrue(card.showsActivationHint)
    }

    // MARK: Splice

    func testOutgoingSpliceReadsAsSendAndIncomingAsMove() throws {
        let outCard = try XCTUnwrap(HomeOnchainRows.card(.init(
            isSweeping: true,
            pendingSplice: PendingSplice(direction: "out", amountSats: 700, address: "bc1q")
        )))
        XCTAssertEqual(outCard.rows.first?.kind, .splice(isOutgoing: true))
        XCTAssertEqual(outCard.rows.first?.sign, .outgoing)
        XCTAssertEqual(outCard.rows.first?.amountSats, 700)

        let inCard = try XCTUnwrap(HomeOnchainRows.card(.init(
            onchainBalanceSats: 700,
            isSweeping: true,
            pendingSplice: PendingSplice(direction: "in", amountSats: 700, address: nil)
        )))
        XCTAssertEqual(inCard.rows.first?.kind, .splice(isOutgoing: false))
        XCTAssertEqual(inCard.rows.first?.sign, .neutral)
        XCTAssertNil(inCard.collapsedSummary)
    }

    func testSpliceRequiredConfirmationsComeFromPolicy() throws {
        let card = try XCTUnwrap(HomeOnchainRows.card(.init(isSweeping: true, spliceTxid: "splice")))
        XCTAssertEqual(card.rows.first?.requiredConfirmations, ConfirmationPolicy.spliceRequiredConfirmations)
        XCTAssertEqual(card.rows.first?.progress, 0)
    }

    func testSpliceWithPendingReceiveCollapsesToSummary() throws {
        let card = try XCTUnwrap(HomeOnchainRows.card(.init(
            onchainBalanceSats: 900,
            isSweeping: true,
            pendingSplice: PendingSplice(direction: "out", amountSats: 500, address: nil),
            pendingReceives: [payment(confirmations: 2)]
        )))
        XCTAssertEqual(
            card.collapsedSummary,
            .init(lead: .sendPending, receives: .progress(confirmations: 2, required: 6))
        )
        XCTAssertEqual(card.rows.map(\.kind), [.splice(isOutgoing: true), .receiving])
        XCTAssertNil(card.moveButton)
    }

    // MARK: Move + receives

    func testMoveAloneIsPlainAndNotExpandable() throws {
        let card = try XCTUnwrap(HomeOnchainRows.card(.init(
            onchainBalanceSats: 1000,
            spendableOnchainSats: 1000,
            hasReadyChannel: true
        )))
        XCTAssertEqual(card.moveButton, .plain)
        XCTAssertFalse(card.isExpandable)
        XCTAssertEqual(card.rows, [])
    }

    func testMoveWithPendingReceiveIsExpandableAndNamesSweptAmount() throws {
        let card = try XCTUnwrap(HomeOnchainRows.card(.init(
            onchainBalanceSats: 3000,
            spendableOnchainSats: 1000,
            hasReadyChannel: true,
            pendingReceives: [payment(id: 1, confirmations: 1)]
        )))
        XCTAssertTrue(card.isExpandable)
        XCTAssertEqual(
            card.collapsedSummary,
            .init(lead: .readyToMove, receives: .progress(confirmations: 1, required: 6))
        )
        XCTAssertEqual(card.moveButton, .amount(sats: 1000))
        XCTAssertEqual(card.rows.map(\.kind), [.receiving])
    }

    func testSummaryCountsMultipleDeposits() throws {
        let card = try XCTUnwrap(HomeOnchainRows.card(.init(
            onchainBalanceSats: 3000,
            spendableOnchainSats: 1000,
            hasReadyChannel: true,
            pendingReceives: [payment(id: 1), payment(id: 2), payment(id: 3)]
        )))
        XCTAssertEqual(card.collapsedSummary?.receives, .count(3))
        XCTAssertEqual(card.rows.count, 3)
    }

    // MARK: Receives

    func testOneReceiveRowPerPendingDepositInOrder() throws {
        let card = try XCTUnwrap(HomeOnchainRows.card(.init(
            onchainBalanceSats: 3000,
            pendingReceives: [payment(id: 1, sats: 100), payment(id: 2, sats: 200)]
        )))
        XCTAssertEqual(card.rows.map(\.amountSats), [100, 200])
        XCTAssertTrue(card.rows.allSatisfy { $0.kind == .receiving && $0.sign == .incoming })
    }

    func testPendingReceivesShownBesideConfirmedBalanceWithoutChannel() throws {
        let card = try XCTUnwrap(HomeOnchainRows.card(.init(
            onchainBalanceSats: 3000,
            spendableOnchainSats: 2000,
            pendingReceives: [payment()]
        )))
        XCTAssertEqual(card.rows.map(\.kind), [.receiving])
        XCTAssertTrue(card.showsActivationHint)
        XCTAssertNil(card.moveButton)
    }

    func testChannelClosingListsPendingReceivesBelowCloseRow() throws {
        let card = try XCTUnwrap(HomeOnchainRows.card(.init(
            onchainBalanceSats: 3000,
            isChannelClosing: true,
            lastCloseTxid: "",
            pendingReceives: [payment()]
        )))
        XCTAssertEqual(card.rows.map(\.kind), [.channelClosingNoLink, .receiving])
    }

    func testChannelOpeningKeepsFundingRow() throws {
        let card = try XCTUnwrap(HomeOnchainRows.card(.init(
            onchainBalanceSats: 3000,
            isOpeningChannel: true,
            fundingTxid: "fund",
            pendingReceives: [payment()]
        )))
        XCTAssertEqual(card.rows, [HomeOnchainRow(kind: .channelOpening, txid: "fund")])
    }

    // MARK: Row progress and caption

    func testProgressOnlyOnceTxidIsKnown() throws {
        let card = try XCTUnwrap(HomeOnchainRows.card(.init(
            onchainBalanceSats: 3000,
            pendingReceives: [payment(id: 1, txid: nil, confirmations: 0), payment(id: 2, txid: "", confirmations: 0)]
        )))
        XCTAssertEqual(card.rows.map(\.txid), [nil, nil])
        XCTAssertEqual(card.rows.map(\.progress), [nil, nil])
        XCTAssertEqual(card.rows.map(\.caption), [.awaitingTxid, .awaitingTxid])
    }

    func testProgressIsConfirmationsOverRequiredAndClamped() {
        var row = HomeOnchainRow(
            kind: .receiving,
            txid: "tx",
            amountSats: 1,
            confirmations: 3,
            requiredConfirmations: 6
        )
        XCTAssertEqual(row.progress, 0.5)
        XCTAssertEqual(row.caption, .progress(confirmations: 3, required: 6))
        row.confirmations = 9
        XCTAssertEqual(row.progress, 1)
        row.requiredConfirmations = 0
        XCTAssertNil(row.progress)
        XCTAssertEqual(row.caption, .amountOnly)
    }

    func testRowsWithoutAmountHaveNoCaptionOnceLinked() {
        XCTAssertEqual(HomeOnchainRow(kind: .channelClosing, txid: "close").caption, .none)
        XCTAssertEqual(HomeOnchainRow(kind: .channelClosingNoLink, txid: nil).caption, .none)
        XCTAssertEqual(HomeOnchainRow(kind: .depositFallback, txid: nil).caption, .awaitingTxid)
    }

    // MARK: Amount text

    func testAmountTextUsesUsdWhenPriceAvailable() {
        let text = HomeOnchainRows.amountText(sats: 100_000, btcPrice: 50000, showBTC: false, sign: .incoming)
        XCTAssertEqual(text, "+" + 50.0.usdFormatted)
    }

    func testAmountTextFallsBackToSpacedBtcWhenPriceMissingOrNegative() {
        let expected = "-\(UInt64(100_000).btcSpacedFormatted) BTC"
        XCTAssertEqual(
            HomeOnchainRows.amountText(sats: 100_000, btcPrice: 0, showBTC: false, sign: .outgoing),
            expected
        )
        XCTAssertEqual(
            HomeOnchainRows.amountText(sats: 100_000, btcPrice: -1, showBTC: false, sign: .outgoing),
            expected
        )
    }

    func testAmountTextHonorsBtcToggle() {
        XCTAssertEqual(
            HomeOnchainRows.amountText(sats: 100_000, btcPrice: 50000, showBTC: true),
            "\(UInt64(100_000).btcSpacedFormatted) BTC"
        )
    }
}

// MARK: - Repository queries

final class PendingOnchainPaymentsRepositoryTests: XCTestCase {
    private var service: DatabaseService!
    private var dataDir: URL!

    override func setUp() {
        super.setUp()
        dataDir = FileManager.default
            .temporaryDirectory
            .appendingPathComponent("PendingOnchainPaymentsRepositoryTests-\(UUID().uuidString)")
        try? FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        service = try? DatabaseService(dataDir: dataDir)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: dataDir)
        service = nil
        super.tearDown()
    }

    private func record(
        _ paymentId: String,
        type: String = "onchain",
        direction: String,
        status: String = "pending",
        createdAt: Int64
    ) throws {
        _ = try service.paymentRepo.recordPayment(
            paymentId: paymentId,
            paymentType: type,
            direction: direction,
            amountMsat: 1_000_000,
            amountUSD: nil,
            btcPrice: nil,
            counterparty: nil,
            status: status,
            txid: paymentId
        )
        try service.rawSQL.execute(
            "UPDATE payments SET created_at = ? WHERE payment_id = ?",
            params: [.integer(createdAt), .text(paymentId)]
        )
    }

    func testNoPendingRowsReturnsEmpty() {
        XCTAssertEqual(service.paymentRepo.pendingOnchainReceives(), [])
        XCTAssertEqual(service.paymentRepo.pendingOnchainSends(), [])
    }

    func testPendingSendsAreOldestFirstAndExcludeReceives() throws {
        try record("send-new", direction: "sent", createdAt: 200)
        try record("send-old", direction: "sent", createdAt: 100)
        try record("rx", direction: "received", createdAt: 150)
        XCTAssertEqual(service.paymentRepo.pendingOnchainSends().map(\.paymentId), ["send-old", "send-new"])
        XCTAssertEqual(service.paymentRepo.pendingOnchainReceives().map(\.paymentId), ["rx"])
    }

    func testConfirmedFailedAndNonOnchainRowsAreExcluded() throws {
        try record("done", direction: "sent", status: "completed", createdAt: 100)
        try record("failed", direction: "sent", status: "failed", createdAt: 110)
        try record("splice", type: "splice_out", direction: "sent", createdAt: 120)
        try record("ln", type: "lightning", direction: "received", createdAt: 130)
        try record("pending", direction: "sent", createdAt: 140)
        XCTAssertEqual(service.paymentRepo.pendingOnchainSends().map(\.paymentId), ["pending"])
        XCTAssertEqual(service.paymentRepo.pendingOnchainReceives(), [])
    }

    func testConfirmedSendLeavesTheList() throws {
        try record("send", direction: "sent", createdAt: 100)
        try service.paymentRepo.updatePaymentStatus(paymentId: "send", status: "completed")
        XCTAssertEqual(service.paymentRepo.pendingOnchainSends(), [])
    }

    func testNewDepositIsNotStarvedByOlderStuckRows() throws {
        for index in 0..<5 {
            try record("old-\(index)", direction: "received", createdAt: Int64(100 + index))
        }
        try record("new", direction: "received", createdAt: 1000)
        let rows = service.paymentRepo.pendingOnchainReceives(limit: 3)
        XCTAssertEqual(rows.map(\.paymentId), ["old-3", "old-4", "new"])
    }

    func testSameSecondTiesAreOrderedById() throws {
        try record("first", direction: "received", createdAt: 100)
        try record("second", direction: "received", createdAt: 100)
        XCTAssertEqual(service.paymentRepo.pendingOnchainReceives().map(\.paymentId), ["first", "second"])
    }

    func testDefaultLimitsMatchAndroid() {
        XCTAssertEqual(PaymentRepository.pendingOnchainReceivesLimit, 25)
        XCTAssertEqual(PaymentRepository.pendingOnchainSendsLimit, 10)
    }
}
