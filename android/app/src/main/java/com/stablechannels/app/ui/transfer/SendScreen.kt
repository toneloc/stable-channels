package com.stablechannels.app.ui.transfer

import android.content.ContextWrapper
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Link
import androidx.compose.material.icons.filled.PhotoLibrary
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material.icons.filled.QuestionMark
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material3.*
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.fragment.app.FragmentActivity
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import com.stablechannels.app.AppState
import com.stablechannels.app.services.AppAccessPreferencesManager
import com.stablechannels.app.services.BiometricService
import com.stablechannels.app.ui.scanner.QRScannerScreen
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.QRCodeUtils
import com.stablechannels.app.util.btcSpacedFormatted
import com.stablechannels.app.util.usdFormatted
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.lightningdevkit.ldknode.Bolt11Invoice
import org.lightningdevkit.ldknode.Offer

enum class InputType { BOLT11, BOLT12, ONCHAIN, UNKNOWN }

@Composable
fun SendScreen(appState: AppState, onDismiss: () -> Unit) {
    var input by remember { mutableStateOf("") }
    var amountUSDStr by remember { mutableStateOf("") }
    var isSending by remember { mutableStateOf(false) }
    var result by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var showScanner by remember { mutableStateOf(false) }
    var isExtractingQR by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val activity = context.findActivity()
    val btcPrice by appState.priceService.currentPrice.collectAsState()
    val lightningSats by appState.lightningBalanceSats.collectAsState()
    val spendableOnchainSats by appState.spendableOnchainSats.collectAsState()

    val inputType = remember(input) {
        val lower = input.trim().lowercase()
        when {
            lower.startsWith("lnbc") || lower.startsWith("lntb") || lower.startsWith("lnts") -> InputType.BOLT11
            lower.startsWith("lno") -> InputType.BOLT12
            lower.startsWith("bc1") || lower.startsWith("1") || lower.startsWith("3") || lower.startsWith("tb1") -> InputType.ONCHAIN
            else -> InputType.UNKNOWN
        }
    }

    val parsedBolt11Msat = remember(input) {
        if (inputType != InputType.BOLT11) null
        else try { Bolt11Invoice.fromStr(input.trim()).amountMilliSatoshis()?.toLong() } catch (_: Exception) { null }
    }

    val isAmountlessBolt11 = inputType == InputType.BOLT11 && parsedBolt11Msat == null && input.isNotBlank()

    val enteredUSD = amountUSDStr.toDoubleOrNull() ?: 0.0
    val manualAmountMsat: Long = run {
        if (btcPrice <= 0 || enteredUSD <= 0) return@run 0L
        (enteredUSD / btcPrice * Constants.SATS_IN_BTC * 1000).toLong()
    }
    val manualAmountSats = manualAmountMsat / 1000

    val needsAmount = when {
        inputType == InputType.BOLT11 && !isAmountlessBolt11 -> false
        else -> manualAmountMsat == 0L
    }

    val displaySats: Long = when (inputType) {
        InputType.BOLT11 -> if ((parsedBolt11Msat ?: 0) > 0) (parsedBolt11Msat ?: 0) / 1000 else manualAmountSats
        InputType.BOLT12, InputType.ONCHAIN -> manualAmountSats
        InputType.UNKNOWN -> 0
    }

    val displayUSD = if (btcPrice > 0 && displaySats > 0) (displaySats.toDouble() / Constants.SATS_IN_BTC) * btcPrice else null

    // Photo picker launcher for QR extraction (Task 7.4)
    val photoPickerLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.PickVisualMedia()
    ) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult

        isExtractingQR = true
        try {
            val inputImage = InputImage.fromFilePath(context, uri)
            val options = BarcodeScannerOptions.Builder()
                .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
                .build()
            val scanner = BarcodeScanning.getClient(options)

            scanner.process(inputImage)
                .addOnSuccessListener { barcodes ->
                    isExtractingQR = false
                    // Find first valid payment string
                    val validPayload = barcodes
                        .mapNotNull { it.rawValue }
                        .map { QRCodeUtils.stripUriPrefix(it) }
                        .firstOrNull { QRCodeUtils.isValidPaymentString(it) }

                    if (validPayload != null) {
                        input = validPayload
                    } else {
                        Toast.makeText(
                            context,
                            "No Lightning invoice or Bitcoin address QR code was found",
                            Toast.LENGTH_LONG
                        ).show()
                    }
                }
                .addOnFailureListener {
                    isExtractingQR = false
                    Toast.makeText(
                        context,
                        "No Lightning invoice or Bitcoin address QR code was found",
                        Toast.LENGTH_LONG
                    ).show()
                }
        } catch (_: Exception) {
            isExtractingQR = false
            Toast.makeText(
                context,
                "No Lightning invoice or Bitcoin address QR code was found",
                Toast.LENGTH_LONG
            ).show()
        }
    }

    // Show full-screen scanner overlay (Task 7.5)
    if (showScanner) {
        QRScannerScreen(
            onResult = { decoded ->
                input = decoded
                showScanner = false
            },
            onCancel = {
                showScanner = false
            }
        )
        return
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        // Header row
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(56.dp)
        ) {
            if (result == null) {
                TextButton(
                    onClick = onDismiss,
                    modifier = Modifier.align(Alignment.CenterStart),
                    colors = ButtonDefaults.textButtonColors(
                        containerColor = if (isSystemInDarkTheme()) {
                            MaterialTheme.colorScheme.surfaceVariant
                        } else {
                            Color(0xFFE5E5EA)
                        },
                        contentColor = MaterialTheme.colorScheme.primary
                    ),
                    shape = RoundedCornerShape(20.dp),
                    contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp)
                ) {
                    Text("Cancel", style = MaterialTheme.typography.bodyMedium)
                }
            }
            Text(
                text = "Send",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.align(Alignment.Center)
            )
            if (result == null) {
                Row(
                    modifier = Modifier
                        .align(Alignment.CenterEnd)
                        .background(
                            color = if (isSystemInDarkTheme()) {
                                MaterialTheme.colorScheme.surfaceVariant
                            } else {
                                Color(0xFFE5E5EA)
                            },
                            shape = RoundedCornerShape(20.dp)
                        )
                        .padding(horizontal = 4.dp, vertical = 2.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    // Photo library button
                    IconButton(
                        onClick = {
                            photoPickerLauncher.launch(
                                PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)
                            )
                        },
                        modifier = Modifier.size(36.dp)
                    ) {
                        Icon(
                            imageVector = Icons.Default.PhotoLibrary,
                            contentDescription = "Import from photo library",
                            tint = MaterialTheme.colorScheme.primary,
                            modifier = Modifier.size(18.dp)
                        )
                    }

                    // Divider
                    Box(
                        modifier = Modifier
                            .width(0.5.dp)
                            .height(20.dp)
                            .background(
                                color = if (isSystemInDarkTheme()) {
                                    Color(0xFF38383A)
                                } else {
                                    Color(0xFFC7C7CC)
                                }
                            )
                    )

                    // QR Scanner button
                    IconButton(
                        onClick = { showScanner = true },
                        modifier = Modifier.size(36.dp)
                    ) {
                        Icon(
                            imageVector = Icons.Default.QrCodeScanner,
                            contentDescription = "Scan QR code",
                            tint = MaterialTheme.colorScheme.primary,
                            modifier = Modifier.size(18.dp)
                        )
                    }
                }
            }
        }

        Spacer(Modifier.height(16.dp))

        if (result != null) {
            Spacer(Modifier.height(40.dp))
            Icon(
                imageVector = Icons.Filled.CheckCircle,
                contentDescription = "Success",
                tint = Color(0xFF10B981),
                modifier = Modifier.size(64.dp)
            )
            Spacer(Modifier.height(16.dp))
            Text("Sent!", style = MaterialTheme.typography.headlineMedium, fontWeight = FontWeight.Bold)
            Spacer(Modifier.height(12.dp))
            Card(
                colors = CardDefaults.cardColors(
                    containerColor = if (isSystemInDarkTheme()) {
                        MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.3f)
                    } else {
                        Color(0xFFF2F2F7)
                    }
                ),
                shape = RoundedCornerShape(12.dp),
                modifier = Modifier.fillMaxWidth().padding(horizontal = 8.dp)
            ) {
                Text(
                    text = result!!,
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.Medium,
                    modifier = Modifier.padding(16.dp),
                    textAlign = TextAlign.Center
                )
            }
            Spacer(Modifier.weight(1f))
            Button(
                onClick = onDismiss
            ) {
                Text("Done")
            }
        } else {
            // Loading indicator during photo QR extraction
            if (isExtractingQR) {
                Spacer(Modifier.height(8.dp))
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.Center,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                    Spacer(Modifier.width(8.dp))
                    Text("Extracting QR code...", style = MaterialTheme.typography.bodySmall)
                }
                Spacer(Modifier.height(8.dp))
            }

            OutlinedTextField(
                value = input,
                onValueChange = { input = it },
                label = { Text("Lightning invoice or Onchain address") },
                modifier = Modifier.fillMaxWidth(),
                minLines = 2
            )

            // Color-coded input type indicator (Task 7.5)
            if (inputType != InputType.UNKNOWN) {
                Spacer(Modifier.height(4.dp))
                InputTypeIndicator(inputType)
            }

            // Bolt11 with amount — show USD and BTC
            if (inputType == InputType.BOLT11 && (parsedBolt11Msat ?: 0) > 0) {
                Spacer(Modifier.height(8.dp))
                displayUSD?.let {
                    Text(it.usdFormatted(), style = MaterialTheme.typography.titleMedium, fontWeight = androidx.compose.ui.text.font.FontWeight.Bold)
                }
                Text(
                    displaySats.btcSpacedFormatted(),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }

            // Amount input (USD) — for amountless bolt11, bolt12, onchain
            if (isAmountlessBolt11 || inputType == InputType.BOLT12 || inputType == InputType.ONCHAIN) {
                Spacer(Modifier.height(16.dp))
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text("Amount (USD)", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    TextButton(
                        onClick = {
                            val hasChannel = appState.nodeService.channels.any { it.isChannelReady }
                            val maxSats = if (inputType == InputType.ONCHAIN) {
                                if (hasChannel) lightningSats else spendableOnchainSats
                            } else {
                                lightningSats
                            }
                            val maxUSD = (maxSats.toDouble() / Constants.SATS_IN_BTC) * btcPrice
                            amountUSDStr = String.format(java.util.Locale.US, "%.2f", maxUSD)
                        },
                        contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp)
                    ) {
                        Text("Send Max", style = MaterialTheme.typography.labelMedium)
                    }
                }
                Spacer(Modifier.height(12.dp))

                Row(
                    horizontalArrangement = Arrangement.Center,
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text("$", fontSize = 44.sp, fontWeight = FontWeight.Bold)
                    Spacer(Modifier.width(2.dp))
                    BasicTextField(
                        value = amountUSDStr,
                        onValueChange = { amountUSDStr = it.filter { c -> c.isDigit() || c == '.' } },
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                        textStyle = TextStyle(
                            fontSize = 44.sp,
                            fontWeight = FontWeight.Bold,
                            color = MaterialTheme.colorScheme.onSurface,
                            textAlign = TextAlign.Start
                        ),
                        singleLine = true,
                        cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                        modifier = Modifier.width(IntrinsicSize.Min),
                        decorationBox = { innerTextField ->
                            Box(contentAlignment = Alignment.CenterStart) {
                                if (amountUSDStr.isEmpty()) {
                                    Text(
                                        text = "0.00",
                                        style = TextStyle(
                                            fontSize = 44.sp,
                                            fontWeight = FontWeight.Bold,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f),
                                            textAlign = TextAlign.Start
                                        )
                                    )
                                }
                                innerTextField()
                            }
                        }
                    )
                }

                if (manualAmountSats > 0) {
                    Spacer(Modifier.height(4.dp))
                    Text(
                        manualAmountSats.btcSpacedFormatted(),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }

            error?.let {
                Spacer(Modifier.height(8.dp))
                Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
            }

            Spacer(Modifier.height(16.dp))
            Spacer(Modifier.weight(1f))
            Button(
                onClick = {
                    isSending = true
                    error = null
                    scope.launch {
                        // Auth gate: check if authentication is required
                        val isOnChain = inputType == InputType.ONCHAIN
                        val requiresAuth = AppAccessPreferencesManager.shouldRequireAuth(context, isOnChain)

                        if (requiresAuth && activity != null) {
                            val reason = if (isOnChain) {
                                "Confirm on-chain withdrawal"
                            } else {
                                "Confirm payment of $displaySats sats"
                            }
                            val authResult = BiometricService.authenticate(activity, reason)
                            if (authResult != BiometricService.AuthResult.SUCCESS) {
                                error = "Authentication required to send"
                                isSending = false
                                return@launch
                            }
                        } else if (requiresAuth) {
                            // Activity not available (e.g., inside bottom sheet) — block send
                            error = "Authentication required to send"
                            isSending = false
                            return@launch
                        }

                        // Auth succeeded (or not required), proceed with send on IO thread
                        withContext(Dispatchers.IO) {
                            try {
                                appState.ensureLSPConnected()
                                val trimmed = input.trim()
                                val price = btcPrice
                                when (inputType) {
                                    InputType.BOLT11 -> {
                                        val invoice = Bolt11Invoice.fromStr(trimmed)
                                        val invoiceMsat = invoice.amountMilliSatoshis()?.toLong() ?: 0L
                                        val paymentId: String
                                        val actualMsat: Long
                                        if (invoiceMsat > 0) {
                                            paymentId = appState.nodeService.sendPayment(invoice)
                                            actualMsat = invoiceMsat
                                        } else {
                                            actualMsat = manualAmountMsat
                                            paymentId = appState.nodeService.sendPaymentUsingAmount(invoice, actualMsat)
                                        }
                                        appState.databaseService?.recordPayment(
                                            paymentId = paymentId, paymentType = "lightning",
                                            direction = "sent", amountMsat = actualMsat,
                                            amountUSD = if (price > 0) (actualMsat.toDouble() / 1000.0 / Constants.SATS_IN_BTC) * price else null,
                                            btcPrice = if (price > 0) price else null
                                        )
                                        result = "Payment sent"
                                    }
                                    InputType.BOLT12 -> {
                                        val sats = manualAmountSats
                                        if (sats <= 0) throw Exception("Enter amount")
                                        val offer = Offer.fromStr(trimmed)
                                        val paymentId = appState.nodeService.sendBolt12UsingAmount(offer, sats * 1000)
                                        appState.databaseService?.recordPayment(
                                            paymentId = paymentId, paymentType = "bolt12",
                                            direction = "sent", amountMsat = sats * 1000,
                                            amountUSD = if (price > 0) (sats.toDouble() / Constants.SATS_IN_BTC) * price else null,
                                            btcPrice = if (price > 0) price else null
                                        )
                                        result = "Bolt12 payment sent"
                                    }
                                    InputType.ONCHAIN -> {
                                        val sats = manualAmountSats
                                        if (sats <= 0) throw Exception("Enter amount")
                                        val hasChannel = appState.nodeService.channels.any { it.isChannelReady }
                                        if (hasChannel) {
                                            if (appState.isSpliceInFlight) throw Exception("A splice is already in progress — try again shortly")
                                            val sc = appState.stableChannel.value
                                            appState.pendingSplice = com.stablechannels.app.models.PendingSplice("out", sats, trimmed)
                                            appState.nodeService.spliceOut(sc.userChannelId, sc.counterparty, trimmed, sats)
                                            result = "Splice-out initiated"
                                        } else {
                                            val txid = appState.nodeService.sendOnchain(trimmed, sats)
                                            appState.databaseService?.recordPayment(
                                                paymentId = null, paymentType = "onchain",
                                                direction = "sent", amountMsat = sats * 1000,
                                                amountUSD = if (price > 0) (sats.toDouble() / Constants.SATS_IN_BTC) * price else null,
                                                btcPrice = if (price > 0) price else null,
                                                txid = txid, address = trimmed
                                            )
                                            result = "On-chain tx sent: $txid"
                                        }
                                    }
                                    InputType.UNKNOWN -> throw Exception("Enter a valid invoice, offer, or address")
                                }
                            } catch (e: Exception) {
                                error = e.message ?: "Send failed"
                            }
                            isSending = false
                        }
                    }
                },
                enabled = !isSending && input.isNotBlank() && !needsAmount,
                modifier = Modifier.fillMaxWidth()
            ) {
                if (isSending) CircularProgressIndicator(Modifier.size(20.dp))
                else Text("Send")
            }
        }
    }
}

/**
 * Displays a color-coded icon + label for the detected input type.
 * - Blue ⚡ + "Lightning Invoice" for Bolt11
 * - Purple ⚡ + "Lightning Offer" for Bolt12
 * - Orange 🔗 + "Bitcoin Address" for on-chain
 * - Gray ? + "Unrecognized format" for unknown
 */
@Composable
private fun InputTypeIndicator(inputType: InputType) {
    val (icon, label, tint) = when (inputType) {
        InputType.BOLT11 -> Triple(
            Icons.Default.Link,
            "Lightning Invoice",
            Color(0xFF2196F3) // Blue
        )
        InputType.BOLT12 -> Triple(
            Icons.Default.Link,
            "Lightning Offer",
            Color(0xFF9C27B0) // Purple
        )
        InputType.ONCHAIN -> Triple(
            Icons.Default.Link,
            "Bitcoin Address",
            Color(0xFFFF9800) // Orange
        )
        InputType.UNKNOWN -> Triple(
            Icons.Default.QuestionMark,
            "Unrecognized format",
            Color.Gray
        )
    }

    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.Start,
        modifier = Modifier.fillMaxWidth()
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            tint = tint,
            modifier = Modifier.size(16.dp)
        )
        Spacer(Modifier.width(4.dp))
        Text(
            text = label,
            style = MaterialTheme.typography.labelSmall,
            color = tint
        )
    }
}


/**
 * Walks up the Context wrapper chain to find the hosting FragmentActivity.
 * Works even inside ModalBottomSheet where LocalContext is a ContextThemeWrapper.
 */
private fun android.content.Context.findActivity(): FragmentActivity? {
    var ctx = this
    while (ctx is ContextWrapper) {
        if (ctx is FragmentActivity) return ctx
        ctx = ctx.baseContext
    }
    return null
}
