package com.stablechannels.app.ui.scanner

import android.Manifest
import android.content.Intent
import android.net.Uri
import android.provider.Settings
import android.view.HapticFeedbackConstants
import android.view.ViewGroup
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.ClipOp
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.clipPath
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.foundation.Canvas
import androidx.core.content.ContextCompat
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.common.InputImage
import com.stablechannels.app.util.QRCodeUtils
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Full-screen QR code scanner using CameraX and ML Kit barcode detection.
 *
 * @param onResult Called with the decoded and cleaned payment string on successful scan
 * @param onCancel Called when the user taps cancel without scanning
 */
@Composable
fun QRScannerScreen(
    onResult: (String) -> Unit,
    onCancel: () -> Unit
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val view = LocalView.current

    // Single-detection guard
    val hasDetected = remember { AtomicBoolean(false) }

    // Permission state
    var permissionGranted by remember { mutableStateOf(false) }
    var permissionDenied by remember { mutableStateOf(false) }

    // Check initial permission state
    LaunchedEffect(Unit) {
        val result = ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA)
        permissionGranted = result == android.content.pm.PackageManager.PERMISSION_GRANTED
    }

    // Permission launcher
    val permissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { granted ->
        if (granted) {
            permissionGranted = true
            permissionDenied = false
        } else {
            permissionDenied = true
        }
    }

    // Request permission if not granted and not yet denied
    LaunchedEffect(permissionGranted, permissionDenied) {
        if (!permissionGranted && !permissionDenied) {
            permissionLauncher.launch(Manifest.permission.CAMERA)
        }
    }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(Color.Black)
    ) {
        when {
            permissionGranted -> {
                // Camera preview with barcode analysis
                CameraPreviewWithAnalysis(
                    hasDetected = hasDetected,
                    onBarcodeDetected = { rawValue ->
                        // Strip URI prefix and query params
                        val cleaned = QRCodeUtils.stripUriPrefix(rawValue)

                        // Trigger haptic feedback
                        view.performHapticFeedback(HapticFeedbackConstants.CONFIRM)

                        onResult(cleaned)
                    }
                )

                // Viewfinder square overlay
                Canvas(modifier = Modifier.fillMaxSize()) {
                    val squareSize = size.minDimension * 0.75f
                    val left = (size.width - squareSize) / 2f
                    val top = (size.height - squareSize) / 2f
                    val cornerRadius = 24f
                    val cornerLength = squareSize * 0.15f
                    val strokeWidth = 4f

                    // Semi-transparent overlay outside the square
                    val cutoutPath = Path().apply {
                        addRoundRect(RoundRect(
                            rect = Rect(left, top, left + squareSize, top + squareSize),
                            cornerRadius = CornerRadius(cornerRadius, cornerRadius)
                        ))
                    }
                    clipPath(cutoutPath, clipOp = ClipOp.Difference) {
                        drawRect(Color.Black.copy(alpha = 0.5f))
                    }

                    // Corner brackets (white)
                    val bracketColor = Color.White
                    // Top-left
                    drawLine(bracketColor, Offset(left, top + cornerRadius), Offset(left, top + cornerLength), strokeWidth)
                    drawLine(bracketColor, Offset(left + cornerRadius, top), Offset(left + cornerLength, top), strokeWidth)
                    // Top-right
                    drawLine(bracketColor, Offset(left + squareSize, top + cornerRadius), Offset(left + squareSize, top + cornerLength), strokeWidth)
                    drawLine(bracketColor, Offset(left + squareSize - cornerRadius, top), Offset(left + squareSize - cornerLength, top), strokeWidth)
                    // Bottom-left
                    drawLine(bracketColor, Offset(left, top + squareSize - cornerRadius), Offset(left, top + squareSize - cornerLength), strokeWidth)
                    drawLine(bracketColor, Offset(left + cornerRadius, top + squareSize), Offset(left + cornerLength, top + squareSize), strokeWidth)
                    // Bottom-right
                    drawLine(bracketColor, Offset(left + squareSize, top + squareSize - cornerRadius), Offset(left + squareSize, top + squareSize - cornerLength), strokeWidth)
                    drawLine(bracketColor, Offset(left + squareSize - cornerRadius, top + squareSize), Offset(left + squareSize - cornerLength, top + squareSize), strokeWidth)
                }

                // Hint text at bottom
                Text(
                    text = "Scan invoice, offer, or Bitcoin address",
                    color = Color.White,
                    style = MaterialTheme.typography.bodyMedium,
                    textAlign = TextAlign.Center,
                    modifier = Modifier
                        .align(Alignment.BottomCenter)
                        .padding(bottom = 100.dp)
                        .padding(horizontal = 32.dp)
                )
            }

            permissionDenied -> {
                // Permission denied - show settings prompt
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(32.dp),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally
                ) {
                    Text(
                        text = "Camera access needed",
                        style = MaterialTheme.typography.headlineSmall,
                        color = Color.White,
                        textAlign = TextAlign.Center
                    )
                    Spacer(Modifier.height(12.dp))
                    Text(
                        text = "Camera access is needed to scan QR codes. Please enable it in Settings.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = Color.White.copy(alpha = 0.7f),
                        textAlign = TextAlign.Center
                    )
                    Spacer(Modifier.height(24.dp))
                    Button(
                        onClick = {
                            val intent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS).apply {
                                data = Uri.fromParts("package", context.packageName, null)
                            }
                            context.startActivity(intent)
                        }
                    ) {
                        Text("Open Settings")
                    }
                }
            }

            else -> {
                // Waiting for permission response
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center
                ) {
                    CircularProgressIndicator(color = Color.White)
                }
            }
        }

        // Cancel button - always visible
        IconButton(
            onClick = onCancel,
            modifier = Modifier
                .align(Alignment.TopStart)
                .padding(16.dp)
                .statusBarsPadding()
        ) {
            Icon(
                imageVector = Icons.Default.Close,
                contentDescription = "Cancel",
                tint = Color.White,
                modifier = Modifier.size(28.dp)
            )
        }
    }
}

@Composable
private fun CameraPreviewWithAnalysis(
    hasDetected: AtomicBoolean,
    onBarcodeDetected: (String) -> Unit
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current

    // ML Kit barcode scanner options - QR codes only
    val scannerOptions = remember {
        BarcodeScannerOptions.Builder()
            .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
            .build()
    }
    val barcodeScanner = remember { BarcodeScanning.getClient(scannerOptions) }

    AndroidView(
        factory = { ctx ->
            val previewView = PreviewView(ctx).apply {
                layoutParams = ViewGroup.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.MATCH_PARENT
                )
                scaleType = PreviewView.ScaleType.FILL_CENTER
            }

            val cameraProviderFuture = ProcessCameraProvider.getInstance(ctx)
            cameraProviderFuture.addListener({
                val cameraProvider = cameraProviderFuture.get()

                val preview = Preview.Builder().build().also {
                    it.surfaceProvider = previewView.surfaceProvider
                }

                val imageAnalysis = ImageAnalysis.Builder()
                    .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                    .build()

                imageAnalysis.setAnalyzer(ContextCompat.getMainExecutor(ctx)) { imageProxy ->
                    processImageProxy(imageProxy, barcodeScanner, hasDetected, onBarcodeDetected)
                }

                val cameraSelector = CameraSelector.DEFAULT_BACK_CAMERA

                try {
                    cameraProvider.unbindAll()
                    cameraProvider.bindToLifecycle(
                        lifecycleOwner,
                        cameraSelector,
                        preview,
                        imageAnalysis
                    )
                } catch (_: Exception) {
                    // Camera binding failed - silently handle
                }
            }, ContextCompat.getMainExecutor(ctx))

            previewView
        },
        modifier = Modifier.fillMaxSize()
    )
}

@androidx.annotation.OptIn(androidx.camera.core.ExperimentalGetImage::class)
private fun processImageProxy(
    imageProxy: ImageProxy,
    barcodeScanner: com.google.mlkit.vision.barcode.BarcodeScanner,
    hasDetected: AtomicBoolean,
    onBarcodeDetected: (String) -> Unit
) {
    // Skip if already detected
    if (hasDetected.get()) {
        imageProxy.close()
        return
    }

    val mediaImage = imageProxy.image
    if (mediaImage == null) {
        imageProxy.close()
        return
    }

    val inputImage = InputImage.fromMediaImage(mediaImage, imageProxy.imageInfo.rotationDegrees)

    barcodeScanner.process(inputImage)
        .addOnSuccessListener { barcodes ->
            for (barcode in barcodes) {
                val rawValue = barcode.rawValue
                if (rawValue != null && hasDetected.compareAndSet(false, true)) {
                    onBarcodeDetected(rawValue)
                    break
                }
            }
        }
        .addOnCompleteListener {
            imageProxy.close()
        }
}
