package com.stablechannels.app

import android.app.Application
import android.app.NotificationChannel
import android.app.NotificationManager
import com.google.firebase.FirebaseApp
import com.stablechannels.app.push.StabilityWorkScheduler

class StableChannelsApp : Application() {

    companion object {
        const val STABILITY_CHANNEL_ID = "stability_processing"
        const val STABILITY_NOTIFICATION_ID = 1001
    }

    override fun onCreate() {
        super.onCreate()
        try {
            FirebaseApp.initializeApp(this)
        } catch (_: Exception) {
            // google-services.json missing — push notifications disabled
        }
        createNotificationChannel()
        StabilityWorkScheduler.schedule(this)
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
