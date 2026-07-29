package com.stablechannels.app.services

import android.Manifest
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.util.Log
import android.widget.Toast
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import com.stablechannels.app.util.Constants
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

object LogExporter {
    private fun createZipFile(context: Context): File? {
        val dir = Constants.userDataDir(context)
        val logsToZip = listOf(
            File(dir, "audit_log.txt"),
            File(dir, "ldk_node.log"),
            File(dir, "logs/ldk_node.log")
        ).filter { it.exists() && it.length() > 0 }

        if (logsToZip.isEmpty()) return null

        val cacheLogsDir = File(context.cacheDir, "logs")
        cacheLogsDir.mkdirs()
        val zipFile = File(cacheLogsDir, "stable_channels_logs.zip")
        try {
            ZipOutputStream(FileOutputStream(zipFile)).use { zos ->
                for (file in logsToZip) {
                    FileInputStream(file).use { fis ->
                        val entry = ZipEntry(file.name)
                        zos.putNextEntry(entry)
                        fis.copyTo(zos)
                        zos.closeEntry()
                    }
                }
            }
        } catch (e: Exception) {
            Log.e("LogExporter", "Failed to zip logs", e)
            return null
        }
        return zipFile
    }

    fun shareLogs(context: Context) {
        val zipFile = createZipFile(context) ?: return
        val uri: Uri = try {
            FileProvider.getUriForFile(
                context,
                "${context.packageName}.fileprovider",
                zipFile
            )
        } catch (e: Exception) {
            Log.e("LogExporter", "Failed to get URI for zip file", e)
            return
        }

        val intent = Intent(Intent.ACTION_SEND).apply {
            type = "application/zip"
            putExtra(Intent.EXTRA_STREAM, uri)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }

        // Suppress the node stop/resync for this in-app background cycle so returning
        // from the share sheet doesn't visibly refresh the UI.
        com.stablechannels.app.AppState.suppressNextBackgroundCycle = true
        context.startActivity(Intent.createChooser(intent, "Share Logs"))
    }

    fun downloadLogs(context: Context) {
        val zipFile = createZipFile(context)
        if (zipFile == null) {
            Toast.makeText(context, "No logs available", Toast.LENGTH_SHORT).show()
            return
        }
        // Scoped storage (API 29+) disallows writing directly into the public Downloads
        // directory; go through MediaStore.Downloads instead, which needs no permission there.
        val saved = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            saveToDownloadsViaMediaStore(context, zipFile)
        } else {
            saveToDownloadsLegacy(context, zipFile)
        }
        if (saved) {
            Toast.makeText(context, "Saved to Downloads", Toast.LENGTH_LONG).show()
        } else {
            Toast.makeText(context, "Failed to save logs", Toast.LENGTH_SHORT).show()
        }
    }

    private fun saveToDownloadsViaMediaStore(context: Context, zipFile: File): Boolean {
        return try {
            val resolver = context.contentResolver
            val values = ContentValues().apply {
                put(MediaStore.MediaColumns.DISPLAY_NAME, "stable_channels_logs.zip")
                put(MediaStore.MediaColumns.MIME_TYPE, "application/zip")
                put(MediaStore.MediaColumns.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS)
            }
            val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
                ?: return false
            val wrote = resolver.openOutputStream(uri)?.use { output ->
                zipFile.inputStream().use { input -> input.copyTo(output) }
            }
            wrote != null
        } catch (e: Exception) {
            Log.e("LogExporter", "Failed to save logs via MediaStore", e)
            false
        }
    }

    @Suppress("DEPRECATION")
    private fun saveToDownloadsLegacy(context: Context, zipFile: File): Boolean {
        if (ContextCompat.checkSelfPermission(
                context,
                Manifest.permission.WRITE_EXTERNAL_STORAGE
            ) != PackageManager.PERMISSION_GRANTED
        ) {
            Log.e("LogExporter", "WRITE_EXTERNAL_STORAGE not granted; cannot save to Downloads")
            return false
        }
        return try {
            val downloadsDir = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
            val destFile = File(downloadsDir, "stable_channels_logs.zip")
            zipFile.inputStream().use { input ->
                destFile.outputStream().use { output ->
                    input.copyTo(output)
                }
            }
            true
        } catch (e: Exception) {
            Log.e("LogExporter", "Failed to save logs to Downloads", e)
            false
        }
    }
}
