package com.stablechannels.app.ui.transfer

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.AppState
import com.stablechannels.app.services.WalletErrorMessages
import com.stablechannels.app.ui.components.DoneToolbarButton
import com.stablechannels.app.ui.components.InkButton
import com.stablechannels.app.ui.components.NeutralOutlinedButton
import com.stablechannels.app.ui.components.NeutralTextButton
import com.stablechannels.app.ui.home.FundWalletScreen
import com.stablechannels.app.ui.home.generateQRCode
import com.stablechannels.app.ui.theme.LocalDarkTheme
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.InputSanitizer
import com.stablechannels.app.util.btcSpacedFormatted
import com.stablechannels.app.util.usdFormatted
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

internal const val RECEIVE_WAITING_FOR_PRICE =
    "Waiting for a current BTC price. A USD invoice needs one so you receive the full amount."
internal const val RECEIVE_OFFLINE_NO_PRICE =
    "You're offline. A USD invoice needs a current BTC price, so reconnect to create one."
internal const val RECEIVE_PRICE_OUT_OF_DATE =
    "The BTC price is out of date. Try again in a moment."

/**
 * Sats for a USD invoice, or null without a trusted price. An amount too large to send to the node
 * in msat is also null, since `sats * 1000` would wrap to a different amount.
 */
internal fun receiveInvoiceSats(usd: Double, accountingPrice: Double): Long? =
    accountingSatsFromUSD(usd, accountingPrice)?.takeIf { it <= Long.MAX_VALUE / 1000 }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ReceiveScreen(appState: AppState, onDismiss: () -> Unit) {
    var amountUSD by remember { mutableStateOf("") }
    var invoice by remember { mutableStateOf<String?>(null) }
    var invoiceAmountSats by remember { mutableStateOf<Long?>(null) }
    var isGenerating by remember { mutableStateOf(false) }
    var isCopied by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var showOnChain by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val clipboardManager = LocalClipboardManager.current
    val btcPrice by appState.priceService.currentPrice.collectAsState()
    // A USD invoice is priced only with a fresh, trusted price, as Send and Buy do. The display
    // price above can be an old one kept after failed refreshes.
    val trustedPrice by appState.priceService.accountingPrice.collectAsState()
    val priceUpdatedAt by appState.priceService.lastUpdate.collectAsState()
    val isOnline by appState.isOnline.collectAsState()

    val hasChannel = appState.nodeService.channels.any { it.isChannelReady }

    val enteredUSD = amountUSD.toDoubleOrNull() ?: 0.0
    val enteredSats = receiveInvoiceSats(enteredUSD, trustedPrice) ?: 0L

    // A fresh price clears the out-of-date message, so the user can just tap again.
    LaunchedEffect(priceUpdatedAt) { if (error == RECEIVE_PRICE_OUT_OF_DATE) error = null }

    if (showOnChain) {
        FundWalletScreen(appState, onBack = { showOnChain = false })
        return
    }

    Column(
        modifier =
            Modifier.fillMaxWidth()
                .navigationBarsPadding()
                .imePadding()
                .verticalScroll(rememberScrollState())
                .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        // Toolbar (Done button, centered title, top-right Onchain button)
        Box(modifier = Modifier.fillMaxWidth().height(56.dp)) {
            DoneToolbarButton(
                onClick = onDismiss,
                modifier = Modifier.align(Alignment.CenterStart),
            )
            Text(
                text = "Receive",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.align(Alignment.Center),
            )
            NeutralTextButton(
                onClick = { showOnChain = true },
                modifier = Modifier.align(Alignment.CenterEnd),
                colors =
                    ButtonDefaults.textButtonColors(
                        containerColor =
                            if (LocalDarkTheme.current) {
                                MaterialTheme.colorScheme.surfaceVariant
                            } else {
                                Color(0xFFE5E5EA)
                            },
                        contentColor = MaterialTheme.colorScheme.onSurface,
                    ),
                shape = RoundedCornerShape(20.dp),
                contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp),
            ) {
                Text("Onchain", style = MaterialTheme.typography.bodyMedium)
            }
        }
        Spacer(Modifier.height(16.dp))

        if (invoice != null) {
            val inv = invoice!!

            // Amount summary
            if (invoiceAmountSats != null && invoiceAmountSats!! > 0) {
                if (btcPrice > 0) {
                    val usd = invoiceAmountSats!!.toDouble() / Constants.SATS_IN_BTC * btcPrice
                    Text(
                        usd.usdFormatted(),
                        style = MaterialTheme.typography.headlineMedium,
                        fontWeight = androidx.compose.ui.text.font.FontWeight.Bold,
                    )
                }
                Text(
                    invoiceAmountSats!!.btcSpacedFormatted() + " BTC",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(12.dp))
            }

            val qrBitmap = remember(inv) { generateQRCode(inv.uppercase()) }
            if (qrBitmap != null) {
                Image(
                    bitmap = qrBitmap.asImageBitmap(),
                    contentDescription = "QR Code",
                    modifier = Modifier.size(200.dp),
                )
            }
            Spacer(Modifier.height(12.dp))
            Text(
                text = inv.take(30) + "..." + inv.takeLast(10),
                fontFamily = FontFamily.Monospace,
                style = MaterialTheme.typography.bodySmall,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(12.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                InkButton(
                    onClick = {
                        clipboardManager.setText(AnnotatedString(inv))
                        isCopied = true
                    }
                ) {
                    Text(if (isCopied) "Copied!" else "Copy")
                }
                NeutralOutlinedButton(
                    onClick = {
                        invoice = null
                        invoiceAmountSats = null
                        isCopied = false
                    }
                ) {
                    Text("New Invoice")
                }
            }
        } else {
            if (!hasChannel) {
                Card(
                    colors =
                        CardDefaults.cardColors(
                            containerColor = MaterialTheme.colorScheme.primaryContainer
                        ),
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text(
                        "Receive a payment over Lightning to activate your account.",
                        modifier = Modifier.padding(12.dp),
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
                Spacer(Modifier.height(12.dp))
                Text(
                    "$${Constants.MAX_CHANNEL_USD.toInt()} Maximum",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(16.dp))
            }

            Text(
                "Amount (USD)",
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(12.dp))

            Row(
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("$", fontSize = 44.sp, fontWeight = FontWeight.Bold)
                Spacer(Modifier.width(2.dp))
                BasicTextField(
                    value = amountUSD,
                    onValueChange = { amountUSD = InputSanitizer.decimal(it) },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                    textStyle =
                        TextStyle(
                            fontSize = 44.sp,
                            fontWeight = FontWeight.Bold,
                            color = MaterialTheme.colorScheme.onSurface,
                            textAlign = TextAlign.Start,
                        ),
                    singleLine = true,
                    cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                    modifier = Modifier.width(IntrinsicSize.Min),
                    decorationBox = { innerTextField ->
                        Box(contentAlignment = Alignment.CenterStart) {
                            if (amountUSD.isEmpty()) {
                                Text(
                                    text = "0.00",
                                    style =
                                        TextStyle(
                                            fontSize = 44.sp,
                                            fontWeight = FontWeight.Bold,
                                            color =
                                                MaterialTheme.colorScheme.onSurfaceVariant.copy(
                                                    alpha = 0.5f
                                                ),
                                            textAlign = TextAlign.Start,
                                        ),
                                )
                            }
                            innerTextField()
                        }
                    },
                )
            }

            if (enteredSats > 0) {
                Spacer(Modifier.height(4.dp))
                Text(
                    "${enteredSats.btcSpacedFormatted()} BTC",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            if (trustedPrice <= 0.0) {
                Spacer(Modifier.height(8.dp))
                Text(
                    if (isOnline) RECEIVE_WAITING_FOR_PRICE else RECEIVE_OFFLINE_NO_PRICE,
                    color = MaterialTheme.colorScheme.tertiary,
                    style = MaterialTheme.typography.bodySmall,
                    textAlign = TextAlign.Center,
                )
            }

            if (!hasChannel && enteredUSD > Constants.MAX_CHANNEL_USD) {
                Spacer(Modifier.height(4.dp))
                Text(
                    "Amount exceeds $${Constants.MAX_CHANNEL_USD.toInt()} channel limit",
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall,
                )
            }

            error?.let {
                Spacer(Modifier.height(8.dp))
                Text(
                    it,
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall,
                )
            }

            Spacer(Modifier.height(16.dp))
            InkButton(
                onClick = {
                    // Price the invoice with the trusted price as it is now, since the one on
                    // screen can go stale while the sheet is open.
                    val sats =
                        receiveInvoiceSats(
                            enteredUSD,
                            appState.priceService.currentAccountingPrice(),
                        )
                    if (sats == null) {
                        error = RECEIVE_PRICE_OUT_OF_DATE
                        return@InkButton
                    }
                    isGenerating = true
                    error = null
                    scope.launch(Dispatchers.IO) {
                        try {
                            val inv =
                                if (!hasChannel) {
                                    appState.nodeService.receiveViaJitChannel(
                                        sats * 1000,
                                        "Stable Channels",
                                    )
                                } else {
                                    appState.nodeService.receivePayment(
                                        sats * 1000,
                                        "Stable Channels",
                                    )
                                }
                            invoiceAmountSats = sats
                            invoice = inv.toString()
                            appState.isWaitingForPayment = true
                        } catch (e: Exception) {
                            error =
                                WalletErrorMessages.operation(
                                    e,
                                    "The invoice could not be created. Try again later.",
                                )
                        }
                        isGenerating = false
                    }
                },
                enabled =
                    !isGenerating &&
                        enteredSats > 0 &&
                        (hasChannel || enteredUSD <= Constants.MAX_CHANNEL_USD),
                modifier = Modifier.fillMaxWidth(),
            ) {
                if (isGenerating) CircularProgressIndicator(Modifier.size(20.dp))
                else Text("Generate Invoice")
            }

            if (hasChannel) {
                Spacer(Modifier.height(8.dp))
                NeutralOutlinedButton(
                    onClick = {
                        isGenerating = true
                        error = null
                        scope.launch(Dispatchers.IO) {
                            try {
                                val inv =
                                    appState.nodeService.receiveVariablePayment("Stable Channels")
                                invoiceAmountSats = null
                                invoice = inv.toString()
                                appState.isWaitingForPayment = true
                            } catch (e: Exception) {
                                error =
                                    WalletErrorMessages.operation(
                                        e,
                                        "The invoice could not be created. Try again later.",
                                    )
                            }
                            isGenerating = false
                        }
                    },
                    enabled = !isGenerating,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text("Any Amount")
                }
            }
        }
    }
}
