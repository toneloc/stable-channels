package com.stablechannels.app

import android.app.Application
import android.app.NotificationChannel
import android.app.NotificationManager
import com.google.firebase.FirebaseApp

class StableChannelsApp : Application() {

    companion object {
        const val STABILITY_CHANNEL_ID = "stability_processing"
        const val STABILITY_NOTIFICATION_ID = 1001
    }

    override fun onCreate() {
        super.onCreate()
        // E2E endpoint overrides — no-op in release builds and when no
        // test_config.json is present. Must run before anything reads Constants.
        com.stablechannels.app.util.TestOverrides.init(this)
        try {
            FirebaseApp.initializeApp(this)
        } catch (_: Exception) {
            // google-services.json missing — push notifications disabled
        }
        createNotificationChannel()
    }

    private fun createNotificationChannel() {
        val channel = NotificationChannel(
            STABILITY_CHANNEL_ID,
            "Stability Processing",
            NotificationManager.IMPORTANCE_LOW
        ).apply {
            description = "Background stability payment processing"
        }
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(channel)
    }
}
