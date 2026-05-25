package com.stablechannels.app.push

import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import java.util.concurrent.TimeUnit

/**
 * Periodic worker that checks if the stability service has received a heartbeat
 * in the last 15 minutes. If not, it starts StabilityProcessingService with
 * direction="user_to_lsp" to ensure stability payments are not missed.
 */
class StabilityWorker(
    context: Context,
    params: WorkerParameters
) : CoroutineWorker(context, params) {

    companion object {
        private const val TAG = "StabilityWorker"
        private const val HEARTBEAT_KEY = "bg_last_heartbeat"
        private const val HEARTBEAT_MAX_AGE_MS = 15 * 60 * 1000L // 15 minutes
    }

    override suspend fun doWork(): Result {
        return try {
            val prefs = FCMService.getPrefs(applicationContext)
            val lastHeartbeat = prefs.getLong(HEARTBEAT_KEY, 0L)
            val now = System.currentTimeMillis()

            if (lastHeartbeat == 0L || (now - lastHeartbeat) > HEARTBEAT_MAX_AGE_MS) {
                Log.d(TAG, "No heartbeat in last 15 minutes, starting stability service")
                val intent = Intent(applicationContext, StabilityProcessingService::class.java).apply {
                    putExtra("direction", "user_to_lsp")
                }
                applicationContext.startForegroundService(intent)
            } else {
                Log.d(TAG, "Heartbeat recent (${(now - lastHeartbeat) / 1000}s ago), skipping")
            }

            Result.success()
        } catch (e: Exception) {
            Log.e(TAG, "StabilityWorker failed", e)
            Result.retry()
        }
    }
}

/**
 * Helper to register the periodic stability check with WorkManager.
 */
object StabilityWorkScheduler {
    private const val UNIQUE_WORK_NAME = "stability_check"

    fun schedule(context: Context) {
        val constraints = Constraints.Builder()
            .setRequiredNetworkType(NetworkType.CONNECTED)
            .build()

        val workRequest = PeriodicWorkRequestBuilder<StabilityWorker>(15, TimeUnit.MINUTES)
            .setConstraints(constraints)
            .build()

        WorkManager.getInstance(context).enqueueUniquePeriodicWork(
            UNIQUE_WORK_NAME,
            ExistingPeriodicWorkPolicy.KEEP,
            workRequest
        )
    }
}
