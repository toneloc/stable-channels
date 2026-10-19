package com.stablechannels.app.domain.send

/** The chain a wallet runs on, independent of the node library's own type. */
enum class BitcoinNetwork {
    BITCOIN,
    TESTNET,
    SIGNET,
    REGTEST,
}
