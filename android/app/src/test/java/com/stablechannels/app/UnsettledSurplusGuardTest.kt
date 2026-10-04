package com.stablechannels.app

import com.stablechannels.app.models.Bitcoin
import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.models.USD
import java.util.Date
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class UnsettledSurplusGuardTest {
    private lateinit var appState: AppState

    @Before
    fun setUp() {
        appState = AppState(RuntimeEnvironment.getApplication())
    }

    @Suppress("UNCHECKED_CAST")
    private fun setChannel(sc: StableChannel) {
        val field = AppState::class.java.getDeclaredField("_stableChannel")
        field.isAccessible = true
        (field.get(appState) as MutableStateFlow<StableChannel>).value = sc
    }

    @Suppress("UNCHECKED_CAST")
    private fun setTrustedPrice(price: Double) {
        val priceService = appState.priceService
        val priceField =
            priceService.javaClass.getDeclaredField("_currentPrice").apply { isAccessible = true }
        (priceField.get(priceService) as MutableStateFlow<Double>).value = price
        val updateField =
            priceService.javaClass.getDeclaredField("_lastUpdate").apply { isAccessible = true }
        (updateField.get(priceService) as MutableStateFlow<Date>).value = Date()
    }

    /** $10 target backed by 11,000 sats: at $100k/BTC the $1 excess is owed to the LSP (#322). */
    private fun surplusChannel() =
        StableChannel(
            channelId = "ab".repeat(32),
            userChannelId = "41",
            isStableReceiver = true,
            expectedUSD = USD(10.0),
            backingSats = 11_000,
            stableReceiverBTC = Bitcoin(11_000), // no native balance at all
        )

    @Test
    fun guardBlocksASpendThatExhaustsTheTargetIntoTheSurplus() {
        setChannel(surplusChannel())
        setTrustedPrice(100_000.0)

        // 10,500 sats with no native balance: $10 covers the target, the extra $0.50 eats
        // the LSP's surplus, so the spend must wait for settlement.
        val thrown =
            assertThrows(IllegalStateException::class.java) {
                appState.ensureNoUnsettledSurplus(10_500_000)
            }
        assertTrue(thrown.message!!.contains("to the LSP is still settling"))
    }

    @Test
    fun guardAllowsASpendWithinTheStableTarget() {
        setChannel(surplusChannel())
        setTrustedPrice(100_000.0)

        // 9,000 sats with no native balance spends into backing, but only shrinks the $10
        // target to $1 — the surplus owed to the LSP is untouched, so the spend is fine.
        appState.ensureNoUnsettledSurplus(9_000_000)
    }

    @Test
    fun guardAllowsASpendCoveredByTheNativeBalance() {
        setChannel(surplusChannel().copy(stableReceiverBTC = Bitcoin(20_000)))
        setTrustedPrice(100_000.0)

        // 9,000 sats are native (receiver - backing); a 5,000 sat spend never touches backing.
        appState.ensureNoUnsettledSurplus(5_000_000)
    }

    @Test
    fun guardFailsOpenWithoutATrustedPrice() {
        setChannel(surplusChannel())
        // PriceService starts empty and stale — same fail-open rule as the stability timer.
        appState.ensureNoUnsettledSurplus(10_500_000)
    }

    @Test
    fun guardAllowsTheSpendWhenNoStabilityPaymentIsDue() {
        // Below par at $80k: the receiver checks only, nothing is owed to the LSP.
        setChannel(surplusChannel())
        setTrustedPrice(80_000.0)
        appState.ensureNoUnsettledSurplus(10_500_000)
    }
}
