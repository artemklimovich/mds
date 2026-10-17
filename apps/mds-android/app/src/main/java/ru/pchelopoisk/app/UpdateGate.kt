package ru.pchelopoisk.app

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

object UpdateGate {
    fun newer(): String? {
        val offered = offeredVersion() ?: return null
        return if (isNewer(offered, TrustedNodes.VERSION)) offered else null
    }

    fun apkAddress(): String = TrustedNodes.hive() + "/download/pchela-android.apk"

    fun download(context: Context): File? {
        val file = File(context.cacheDir, "pchela-update.apk")
        val connection = open(apkAddress()) ?: return null
        return try {
            if (connection.responseCode != 200) return null
            connection.inputStream.use { input ->
                file.outputStream().use { output ->
                    val buffer = ByteArray(8192)
                    var read = input.read(buffer)
                    var total = 0L
                    while (read >= 0 && total < 80_000_000) {
                        output.write(buffer, 0, read)
                        total += read
                        read = input.read(buffer)
                    }
                }
            }
            if (file.length() < 1000) null else file
        } catch (_: Exception) {
            null
        } finally {
            connection.disconnect()
        }
    }

    fun install(context: Context, file: File) {
        if (Build.VERSION.SDK_INT >= 26 && !context.packageManager.canRequestPackageInstalls()) {
            val settings = Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${context.packageName}"))
            settings.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            context.startActivity(settings)
            return
        }
        val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
        val intent = Intent(Intent.ACTION_VIEW)
            .setDataAndType(uri, "application/vnd.android.package-archive")
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        context.startActivity(intent)
    }

    private fun offeredVersion(): String? {
        val connection = open(TrustedNodes.hive() + "/shell/version") ?: return null
        return try {
            if (connection.responseCode != 200) return null
            val body = connection.inputStream.bufferedReader().readText()
            JSONObject(body).optString("version").trim().ifEmpty { null }
        } catch (_: Exception) {
            null
        } finally {
            connection.disconnect()
        }
    }

    private fun open(address: String): HttpURLConnection? {
        return try {
            (URL(address).openConnection() as HttpURLConnection).apply {
                connectTimeout = 8000
                readTimeout = 20000
                instanceFollowRedirects = true
                setRequestProperty("User-Agent", "MDS-Bot")
            }
        } catch (_: Exception) {
            null
        }
    }

    private fun isNewer(offered: String, current: String): Boolean {
        val left = offered.split('.').map { it.toIntOrNull() ?: 0 }
        val right = current.split('.').map { it.toIntOrNull() ?: 0 }
        val count = maxOf(left.size, right.size)
        for (index in 0 until count) {
            val gap = left.getOrElse(index) { 0 } - right.getOrElse(index) { 0 }
            if (gap > 0) return true
            if (gap < 0) return false
        }
        return false
    }
}
