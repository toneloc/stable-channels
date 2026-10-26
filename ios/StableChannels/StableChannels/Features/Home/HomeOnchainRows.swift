import Foundation

/// One status row in the Home Onchain Account card.
struct HomeOnchainRow: Equatable {
    enum Kind: Equatable {
        case receiving
        case sending
        /// A splice in flight. Outgoing splices read as sends, incoming ones as moves.
        case splice(isOutgoing: Bool)
        case channelClosing
        case channelClosingNoLink
        case channelOpening
        /// Unconfirmed balance with no deposit row to explain it.
        case depositFallback
    }

    enum Sign: String, Equatable {
        case incoming = "+"
        case outgoing = "-"
        case neutral = ""
    }

    /// What the line under the label shows.
    enum Caption: Equatable {
        case progress(confirmations: Int, required: Int)
        case awaitingTxid
        case amountOnly
        case none
    }

    let kind: Kind
    let txid: String?
    var amountSats: UInt64?
    var sign: Sign = .neutral
    var confirmations: Int?
    var requiredConfirmations: Int?

    /// Confirmation fraction, only once a txid is known: before that nothing is counting yet,
    /// and an empty ring would read as stalled rather than not yet detected.
    var progress: Double? {
        guard txid != nil, let confirmations, let requiredConfirmations, requiredConfirmations > 0 else { return nil }
        return min(max(Double(confirmations) / Double(requiredConfirmations), 0), 1)
    }

    var caption: Caption {
        if progress != nil, let confirmations, let requiredConfirmations {
            return .progress(confirmations: confirmations, required: requiredConfirmations)
        }
        // The no-link close label already says it is waiting on confirmation.
        if txid == nil, kind != .channelClosingNoLink { return .awaitingTxid }
        return amountSats == nil ? .none : .amountOnly
    }
}

/// Everything the Onchain Account card renders, decided from wallet state.
struct HomeOnchainCard: Equatable {
    enum MoveButton: Equatable {
        case plain
        /// Names the exact amount Move sweeps, so it is unambiguous beside a pending deposit.
        case amount(sats: UInt64)
    }

    /// One-line gist shown while a busy card is collapsed.
    struct Summary: Equatable {
        enum Lead: Equatable {
            case readyToMove
            case movePending
            case sendPending
        }

        enum Receives: Equatable {
            case none
            case progress(confirmations: Int, required: Int)
            case count(Int)
        }

        let lead: Lead
        let receives: Receives
    }

    /// Non-nil when the card collapses to `summary` and expands to `moveButton` + `rows`.
    var collapsedSummary: Summary?
    var moveButton: MoveButton?
    var rows: [HomeOnchainRow] = []
    var showsActivationHint = false
    /// Always visible, below the expandable section.
    var sendRows: [HomeOnchainRow] = []

    var isExpandable: Bool { collapsedSummary != nil }
}

/// Builds the Home Onchain Account card. Pure: wallet state in, card (or nil when hidden) out.
enum HomeOnchainRows {
    struct Inputs {
        var onchainBalanceSats: UInt64 = 0
        var spendableOnchainSats: UInt64 = 0
        var hasReadyChannel = false
        var isSweeping = false
        var spliceTxid: String?
        var pendingSplice: PendingSplice?
        var isChannelClosing = false
        var lastCloseTxid: String?
        var isOpeningChannel = false
        var fundingTxid: String?
        var lastReceiveTxid: String?
        var pendingReceives: [PaymentRecord] = []
        var pendingSends: [PaymentRecord] = []
    }

    static func card(_ inputs: Inputs) -> HomeOnchainCard? {
        // A send or splice still confirming keeps the card up after the balance reaches zero.
        guard inputs.onchainBalanceSats > 0 || !inputs.pendingSends.isEmpty || inputs.isSweeping else { return nil }

        let receives = inputs.pendingReceives.map(receiveRow)
        var card = HomeOnchainCard(sendRows: inputs.pendingSends.map(sendRow))

        if inputs.isSweeping {
            let isOutgoing = inputs.pendingSplice?.direction == "out"
            card.rows = [spliceRow(inputs, isOutgoing: isOutgoing)]
            if !receives.isEmpty {
                // Splice and a new deposit both in flight: show both rather than hide the deposit.
                card.collapsedSummary = .init(
                    lead: isOutgoing ? .sendPending : .movePending,
                    receives: receivesSummary(inputs.pendingReceives)
                )
                card.rows += receives
            }
        } else if inputs.isChannelClosing {
            let closeTxid = nonEmpty(inputs.lastCloseTxid)
            card.rows = [HomeOnchainRow(
                kind: closeTxid == nil ? .channelClosingNoLink : .channelClosing,
                txid: closeTxid
            )]
                + receives
        } else if inputs.hasReadyChannel && inputs.spendableOnchainSats > 0 {
            if receives.isEmpty {
                card.moveButton = .plain
            } else {
                card.collapsedSummary = .init(lead: .readyToMove, receives: receivesSummary(inputs.pendingReceives))
                card.moveButton = .amount(sats: inputs.spendableOnchainSats)
                card.rows = receives
            }
        } else if inputs.onchainBalanceSats == 0 {
            // Only a pending send keeps the card up; show any deposit still confirming.
            card.rows = receives
        } else if inputs.spendableOnchainSats == 0 {
            if inputs.isOpeningChannel, let fundingTxid = nonEmpty(inputs.fundingTxid) {
                card.rows = [HomeOnchainRow(kind: .channelOpening, txid: fundingTxid)]
            } else if !receives.isEmpty {
                card.rows = receives
            } else if inputs.pendingSends.isEmpty {
                // Unconfirmed change from our own send is not a deposit.
                card.rows = [HomeOnchainRow(kind: .depositFallback, txid: nonEmpty(inputs.lastReceiveTxid))]
            }
            card.showsActivationHint = !inputs.hasReadyChannel
        } else {
            // Confirmed funds without a channel; a newer deposit can still be confirming beside them.
            card.rows = receives
            card.showsActivationHint = true
        }
        return card
    }

    /// Signed amount: USD at `btcPrice`, or spaced BTC when asked for or when the price is unknown.
    static func amountText(sats: UInt64, btcPrice: Double, showBTC: Bool,
                           sign: HomeOnchainRow.Sign = .neutral) -> String {
        let amount = showBTC || btcPrice <= 0
            ? "\(sats.btcSpacedFormatted) BTC"
            : BitcoinUnits.usd(sats: sats, price: btcPrice).usdFormatted
        return sign.rawValue + amount
    }

    private static func receiveRow(_ payment: PaymentRecord) -> HomeOnchainRow {
        HomeOnchainRow(
            kind: .receiving,
            txid: nonEmpty(payment.txid),
            amountSats: payment.amountSats,
            sign: .incoming,
            confirmations: Int(payment.confirmations),
            requiredConfirmations: ConfirmationPolicy.requiredConfirmations(for: payment.paymentType)
        )
    }

    private static func sendRow(_ payment: PaymentRecord) -> HomeOnchainRow {
        HomeOnchainRow(
            kind: .sending,
            txid: nonEmpty(payment.txid),
            amountSats: payment.amountSats,
            sign: .outgoing,
            confirmations: Int(payment.confirmations),
            requiredConfirmations: ConfirmationPolicy.requiredConfirmations(for: payment.paymentType)
        )
    }

    private static func spliceRow(_ inputs: Inputs, isOutgoing: Bool) -> HomeOnchainRow {
        HomeOnchainRow(
            kind: .splice(isOutgoing: isOutgoing),
            txid: nonEmpty(inputs.spliceTxid),
            amountSats: inputs.pendingSplice?.amountSats,
            sign: isOutgoing ? .outgoing : .neutral,
            confirmations: 0,
            requiredConfirmations: ConfirmationPolicy
                .requiredConfirmations(for: isOutgoing ? "splice_out" : "splice_in")
        )
    }

    private static func receivesSummary(_ receives: [PaymentRecord]) -> HomeOnchainCard.Summary.Receives {
        switch receives.count {
        case 0:
            return .none
        case 1:
            let receive = receives[0]
            return .progress(
                confirmations: Int(receive.confirmations),
                required: ConfirmationPolicy.requiredConfirmations(for: receive.paymentType)
            )
        default:
            return .count(receives.count)
        }
    }

    private static func nonEmpty(_ txid: String?) -> String? {
        guard let txid, !txid.isEmpty else { return nil }
        return txid
    }
}
