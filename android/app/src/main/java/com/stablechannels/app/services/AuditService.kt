package com.stablechannels.app.services

import org.json.JSONObject
import java.io.File
import java.time.Instant

object AuditService {
    private var logFile: File? = null

    fun setLogPath(path: String) {
        logFile = File(path)
        logFile?.parentFile?.mkdirs()
    }

    fun log(event: String, data: Map<String, Any?> = emptyMap()) {
        val file = logFile ?: return
        try {
            val entry = JSONObject().apply {
                put("ts", Instant.now().toString())
                put("event", event)
                put("data", JSONObject(data))
            }
            val line = entry.toString() + "\n"
            file.appendText(line)
        } catch (_: Exception) {
            // Audit logging should never crash the app
        }
    }
}
