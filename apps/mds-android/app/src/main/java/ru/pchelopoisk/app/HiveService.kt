package ru.pchelopoisk.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder

object HiveStatus {
    @Volatile
    var listener: ((String) -> Unit)? = null

    fun say(text: String) {
        listener?.invoke(text)
    }
}

class HiveService : Service() {
    private val worker = ShellWorker()

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == STOP) {
            worker.stop()
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
            return START_NOT_STICKY
        }
        val built = note("Пчела читает страницы для улья.")
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(NOTE_ID, built, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            startForeground(NOTE_ID, built)
        }
        worker.start(this) { text ->
            HiveStatus.say(text)
            val manager = getSystemService(NotificationManager::class.java)
            manager.notify(NOTE_ID, note(text))
        }
        return START_STICKY
    }

    override fun onDestroy() {
        worker.stop()
        super.onDestroy()
    }

    private fun note(text: String): Notification {
        val manager = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= 26) {
            val channel = NotificationChannel(CHANNEL, "Помощь улью", NotificationManager.IMPORTANCE_LOW)
            manager.createNotificationChannel(channel)
        }
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val stop = PendingIntent.getService(
            this,
            1,
            Intent(this, HiveService::class.java).setAction(STOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val builder = if (Build.VERSION.SDK_INT >= 26) {
            Notification.Builder(this, CHANNEL)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
        }
        return builder
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setContentTitle("Пчелопоиск")
            .setContentText(text)
            .setContentIntent(open)
            .setOngoing(true)
            .addAction(Notification.Action(0, "Остановить", stop))
            .build()
    }

    companion object {
        private const val CHANNEL = "pchela-help"
        private const val NOTE_ID = 7
        const val STOP = "ru.pchelopoisk.app.STOP_HELP"

        fun start(context: Context) {
            val intent = Intent(context, HiveService::class.java)
            if (Build.VERSION.SDK_INT >= 26) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, HiveService::class.java))
        }
    }
}
