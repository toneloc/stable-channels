package com.stablechannels.app.services

import android.content.Context
import android.util.Log
import com.stablechannels.app.BuildConfig
import com.stablechannels.app.util.Constants
import java.io.File
import java.time.Instant
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter

object AppLogger {
    enum class Level { DEBUG, INFO, WARN, ERROR }

    private var logFile: File? = null
    private var oldLogFile: File? = null
    private val MAX_FILE_SIZE = 5 * 1024 * 1024L // 5MB

    private val formatter = DateTimeFormatter.ofPattern("yyyy-MM-dd'T'HH:mm:ss.SSS'Z'").withZone(ZoneOffset.UTC)

    // Matches 12 to 24 lowercase alphabetic words separated by a single space.
    // This is a naive but effective filter for BIP39 seed phrases.
    private val seedRegex = Regex("(?<=\\b|\\s)(?:[a-z]+\\s){11,23}[a-z]+(?=\\b|\\s)")

    var minLevel = if (BuildConfig.DEBUG) Level.DEBUG else Level.INFO

    fun init(context: Context) {
        val dir = Constants.userDataDir(context)
        dir.mkdirs()
        logFile = File(dir, "app_debug.log")
        oldLogFile = File(dir, "app_debug.old.log")

        val defaultHandler = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, throwable ->
            e("UncaughtException", "App crashed", throwable)
            defaultHandler?.uncaughtException(thread, throwable)
        }
        
        i("AppLogger", "Logger initialized. Min level: $minLevel")
    }

    fun d(tag: String, message: String, throwable: Throwable? = null) {
        log(Level.DEBUG, tag, message, throwable)
    }

    fun i(tag: String, message: String, throwable: Throwable? = null) {
        log(Level.INFO, tag, message, throwable)
    }

    fun w(tag: String, message: String, throwable: Throwable? = null) {
        log(Level.WARN, tag, message, throwable)
    }

    fun e(tag: String, message: String, throwable: Throwable? = null) {
        log(Level.ERROR, tag, message, throwable)
    }

    private fun log(level: Level, tag: String, message: String, throwable: Throwable?) {
        if (level < minLevel) return

        var finalMessage = message
        if (throwable != null) {
            finalMessage += "\n" + Log.getStackTraceString(throwable)
        }

        // Redact any potential seed phrases
        finalMessage = redact(finalMessage)

        val timestamp = formatter.format(Instant.now())
        val logLine = "[$timestamp] [${level.name}] [$tag] $finalMessage\n"

        // Write to Android Logcat as well
        when (level) {
            Level.DEBUG -> Log.d(tag, finalMessage)
            Level.INFO -> Log.i(tag, finalMessage)
            Level.WARN -> Log.w(tag, finalMessage)
            Level.ERROR -> Log.e(tag, finalMessage)
        }

        writeToFile(logLine)
    }

    private fun redact(input: String): String {
        return seedRegex.replace(input, "[REDACTED_SEED]")
    }

    @Synchronized
    private fun writeToFile(line: String) {
        val file = logFile ?: return
        try {
            if (file.exists() && file.length() > MAX_FILE_SIZE) {
                rotateLogs()
            }
            file.appendText(line)
        } catch (_: Exception) {
            // Ignore file write errors so we don't crash
        }
    }

    private fun rotateLogs() {
        try {
            val old = oldLogFile ?: return
            val file = logFile ?: return
            if (old.exists()) {
                old.delete()
            }
            file.renameTo(old)
        } catch (_: Exception) {}
    }
}
