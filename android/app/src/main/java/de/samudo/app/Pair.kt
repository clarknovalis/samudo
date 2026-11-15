package de.samudo.app

import android.content.Context
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning

/**
 * Reads the QR code the web UI shows under "Handy koppeln" (ADR 0031), with
 * the code scanner of the Play services, as catastr's app: it brings its
 * own camera screen, so the app asks for no camera permission. What the
 * code means, samu-mobile reads; a scan given up says nothing.
 */
fun scanPairingCode(context: Context, onCode: (String) -> Unit, onError: (String) -> Unit) {
    GmsBarcodeScanning.getClient(context, OPTIONS)
        .startScan()
        .addOnSuccessListener { barcode ->
            val text = barcode.rawValue
            if (text == null) onError(tr("Der QR-Code enthält keinen Text.")) else onCode(text)
        }
        // No Play services, or the scanner's module not there yet and no
        // network to fetch it.
        .addOnFailureListener { onError(tr("Der Scanner startet nicht: {message}", "message" to it.message)) }
}

private val OPTIONS = GmsBarcodeScannerOptions.Builder()
    .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
    .build()
