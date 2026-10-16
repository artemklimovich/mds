package ru.pchelopoisk.app

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.ActivityCompat
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.Executors

class MainActivity : AppCompatActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            ActivityCompat.requestPermissions(this, arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
        val web = WebView(this)
        web.webViewClient = WebViewClient()
        setContentView(web)
        web.loadUrl(TrustedNodes.hive())
        Executors.newSingleThreadExecutor().execute { lookForUpdate() }
    }

    private fun lookForUpdate() {
        val connection = (URL(TrustedNodes.hive() + "/peer/notice").openConnection() as HttpURLConnection).apply {
            connectTimeout = 15000
            readTimeout = 15000
            instanceFollowRedirects = true
        }
        val version = try {
            if (connection.responseCode != 200) return
            connection.inputStream.bufferedReader().readText().trim().trim('"')
        } catch (_: Exception) {
            return
        } finally {
            connection.disconnect()
        }
        if (version.isEmpty() || version == BuildConfigVersion.NAME) return
        runOnUiThread { notifyUpdate(version) }
    }

    private fun notifyUpdate(version: String) {
        val manager = getSystemService(NotificationManager::class.java)
        val channel = "update"
        if (Build.VERSION.SDK_INT >= 26) {
            manager.createNotificationChannel(
                NotificationChannel(channel, "Обновление", NotificationManager.IMPORTANCE_DEFAULT)
            )
        }
        val notice = NotificationCompat.Builder(this, channel)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle("Пчелопоиск")
            .setContentText("Вышла версия $version. Нужно обновиться.")
            .setAutoCancel(true)
            .build()
        manager.notify(1, notice)
    }
}

private object BuildConfigVersion {
    const val NAME = "0.1.0"
}
