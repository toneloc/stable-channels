package com.stablechannels.app.services

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Environment
import android.widget.Toast
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
            File(dir, "app_debug.log"),
            File(dir, "app_debug.old.log"),
            File(dir, "audit_log.txt"),
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
            AppLogger.e("LogExporter", "Failed to zip logs", e)
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
            AppLogger.e("LogExporter", "Failed to get URI for zip file", e)
            return
        }

        val intent = Intent(Intent.ACTION_SEND).apply {
            type = "application/zip"
            putExtra(Intent.EXTRA_STREAM, uri)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }

        context.startActivity(Intent.createChooser(intent, "Share Logs"))
    }

    fun downloadLogs(context: Context) {
        val zipFile = createZipFile(context)
        if (zipFile == null) {
            Toast.makeText(context, "No logs available", Toast.LENGTH_SHORT).show()
            return
        }
        try {
            val downloadsDir = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
            val destFile = File(downloadsDir, "stable_channels_logs.zip")
            zipFile.inputStream().use { input ->
                destFile.outputStream().use { output ->
                    input.copyTo(output)
                }
            }
            Toast.makeText(context, "Saved to Downloads", Toast.LENGTH_LONG).show()
        } catch (e: Exception) {
            AppLogger.e("LogExporter", "Failed to save logs to Downloads", e)
            Toast.makeText(context, "Failed to save logs", Toast.LENGTH_SHORT).show()
        }
    }
}
