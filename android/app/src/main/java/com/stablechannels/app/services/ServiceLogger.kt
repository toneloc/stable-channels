package com.stablechannels.app.services

import android.content.Context
import android.util.Log
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Lightweight file-based logger for the stability background service.
 * Appends timestamped entries to a single log file capped at 1 MB.
 * When the cap is exceeded, the oldest half of entries are evicted.
 */
object ServiceLogger {
    private const val TAG = "ServiceLogger"
    private const val MAX_FILE_SIZE = 1_048_576L // 1 MB
    private const val LOG_FILE_NAME = "stability_service.log"

    private val dateFormat = SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.US)

    /**
     * Append a timestamped log entry.
     * @param event Short event name (e.g. START, WAKELOCK_ACQUIRED, PROCESSING_SUCCESS)
     * @param details Optional extra context
     */
    fun log(context: Context, event: String, details: String? = null) {
        try {
            val file = getLogFile(context)
            val timestamp = dateFormat.format(Date())
            val entry = if (details != null) {
                "[$timestamp] $event $details\n"
            } else {
                "[$timestamp] $event\n"
            }

            // Evict oldest entries if file exceeds cap
            if (file.exists() && file.length() + entry.toByteArray().size > MAX_FILE_SIZE) {
                trimFile(file)
            }

            file.appendText(entry)
        } catch (e: Exception) {
            Log.w(TAG, "Failed to write log entry", e)
        }
    }

    /**
     * Read the full log file contents.
     */
    fun readLogs(context: Context): String {
        return try {
            val file = getLogFile(context)
            if (file.exists()) file.readText() else ""
        } catch (e: Exception) {
            Log.w(TAG, "Failed to read logs", e)
            ""
        }
    }

    private fun getLogFile(context: Context): File {
        return File(context.filesDir, LOG_FILE_NAME)
    }

    /**
     * Drop the oldest half of lines to make room for new entries.
     */
    private fun trimFile(file: File) {
        try {
            val lines = file.readLines()
            val keepFrom = lines.size / 2
            val remaining = lines.subList(keepFrom, lines.size)
            file.writeText(remaining.joinToString("\n") + "\n")
        } catch (e: Exception) {
            Log.w(TAG, "Failed to trim log file", e)
        }
    }
}
