package ru.pchelopoisk.app

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import java.nio.ByteBuffer
import java.nio.charset.Charset
import java.nio.charset.CodingErrorAction
import java.security.SecureRandom
import kotlin.concurrent.thread

class ShellWorker {
    @Volatile
    private var running = false
    private val generation = java.util.concurrent.atomic.AtomicInteger(0)

    fun start(context: Context, onStatus: (String) -> Unit) {
        if (running) return
        val ticket = generation.incrementAndGet()
        running = true
        val app = context.applicationContext
        thread(name = "pchela-shell") {
            while (running && generation.get() == ticket) {
                val pause = if (!online(app)) {
                    onStatus("Нет сети. Обход подождёт.")
                    30
                } else {
                    onStatus("Пчела читает страницы для улья.")
                    val (given, pause) = pull(app)
                    if (given > 0) onStatus("Улью отдано страниц: $given.")
                    pause
                }
                var waited = 0
                while (running && generation.get() == ticket && waited < pause) {
                    Thread.sleep(1000)
                    waited += 1
                }
            }
        }
    }

    fun stop() {
        running = false
        generation.incrementAndGet()
    }

    private fun pull(context: Context): Pair<Int, Int> {
        val id = shellId(context)
        val wish = paceWish(context)
        val work = open(TrustedNodes.hive() + "/shell/work", id, wish) ?: return 0 to 600
        val (urls, pause) = try {
            if (work.responseCode != 200) return 0 to 600
            val body = work.inputStream.bufferedReader().readText()
            val root = JSONObject(body)
            val list = root.optJSONArray("urls") ?: return 0 to 600
            val pause = root.optLong("pause_secs", 600).coerceIn(60, 3600).toInt()
            (0 until list.length()).map { list.getString(it) } to pause
        } catch (_: Exception) {
            return 0 to 600
        } finally {
            work.disconnect()
        }
        val pages = JSONArray()
        for (url in urls) {
            val page = readPage(url) ?: continue
            pages.put(page)
        }
        if (pages.length() == 0) return 0 to pause
        val post = open(TrustedNodes.hive() + "/shell/pages", id, wish) ?: return 0 to pause
        val given = try {
            post.requestMethod = "POST"
            post.doOutput = true
            post.setRequestProperty("Content-Type", "application/json; charset=utf-8")
            post.outputStream.use { it.write(pages.toString().toByteArray()) }
            if (post.responseCode in 200..299) pages.length() else 0
        } catch (_: Exception) {
            0
        } finally {
            post.disconnect()
        }
        return given to pause
    }

    private fun readPage(address: String): JSONObject? {
        val connection = (URL(address).openConnection() as HttpURLConnection).apply {
            connectTimeout = 8000
            readTimeout = 15000
            instanceFollowRedirects = false
            setRequestProperty("User-Agent", "MDS-Bot")
        }
        return try {
            if (connection.responseCode !in 200..299) return null
            if (connection.url.toString().trimEnd('/') != address.trimEnd('/')) return null
            val bytes = connection.inputStream.use { input ->
                val buffer = ByteArray(8192)
                val out = java.io.ByteArrayOutputStream()
                var read = input.read(buffer)
                var total = 0
                while (read >= 0 && total < 500_000) {
                    val take = minOf(read, 500_000 - total)
                    out.write(buffer, 0, take)
                    total += take
                    read = if (total >= 500_000) -1 else input.read(buffer)
                }
                out.toByteArray()
            }
            val type = connection.contentType
            val html = decodeHtml(bytes, type)
            val title = titleOf(html).ifEmpty { hostOf(address) }
            val text = textOf(html)
            if (text.length < 40) return null
            JSONObject()
                .put("url", address)
                .put("title", title.take(180))
                .put("host", hostOf(address))
                .put("text", text.take(20_000))
                .put("links", linksOf(html, address))
        } catch (_: Exception) {
            null
        } finally {
            connection.disconnect()
        }
    }

    private fun open(address: String, id: String, wish: String): HttpURLConnection? {
        return try {
            (URL(address).openConnection() as HttpURLConnection).apply {
                connectTimeout = 8000
                readTimeout = 20000
                setRequestProperty("User-Agent", "MDS-Bot")
                setRequestProperty("x-mds-shell", id)
                setRequestProperty("x-mds-wish", wish)
            }
        } catch (_: Exception) {
            null
        }
    }

    private fun paceWish(context: Context): String {
        val mode = context.getSharedPreferences("pchela", Context.MODE_PRIVATE).getString("pace", "hive")
        return when (mode) {
            "fast" -> "fast"
            "server" -> "server"
            else -> "hive"
        }
    }

    private fun shellId(context: Context): String {
        val prefs = context.getSharedPreferences("pchela", Context.MODE_PRIVATE)
        val saved = prefs.getString("shell", "").orEmpty()
        if (saved.length == 32 && saved.all { it.isDigit() || it.lowercaseChar() in 'a'..'f' }) return saved
        val bytes = ByteArray(16)
        SecureRandom().nextBytes(bytes)
        val fresh = bytes.joinToString("") { "%02x".format(it) }
        prefs.edit().putString("shell", fresh).apply()
        return fresh
    }

    private fun online(context: Context): Boolean {
        val manager = context.getSystemService(ConnectivityManager::class.java) ?: return false
        val network = manager.activeNetwork ?: return false
        val caps = manager.getNetworkCapabilities(network) ?: return false
        return caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
    }

    private fun decodeHtml(bytes: ByteArray, contentType: String?): String {
        val declared = charsetOf(contentType) ?: metaCharset(bytes)
        if (declared != null) {
            val name = when {
                declared.contains("1251") -> "windows-1251"
                declared.contains("koi8") -> "KOI8-R"
                declared.contains("utf") -> "UTF-8"
                else -> declared
            }
            return try {
                bytes.toString(Charset.forName(name))
            } catch (_: Exception) {
                bytes.toString(Charsets.UTF_8)
            }
        }
        return try {
            val decoder = Charsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
            decoder.decode(ByteBuffer.wrap(bytes)).toString()
        } catch (_: Exception) {
            try {
                bytes.toString(Charset.forName("windows-1251"))
            } catch (_: Exception) {
                bytes.toString(Charsets.UTF_8)
            }
        }
    }

    private fun charsetOf(contentType: String?): String? {
        val raw = contentType ?: return null
        val mark = raw.lowercase().substringAfter("charset=", "")
        if (mark.isEmpty()) return null
        return mark.substringBefore(';').trim().trim('"', '\'').ifEmpty { null }
    }

    private fun metaCharset(bytes: ByteArray): String? {
        val head = bytes.copyOfRange(0, minOf(bytes.size, 8192))
        val text = head.toString(Charsets.ISO_8859_1).lowercase()
        val mark = text.substringAfter("charset=", "")
        if (mark.isEmpty()) return null
        val token = mark.trimStart(' ', '"', '\'').takeWhile { it.isLetterOrDigit() || it == '-' || it == '_' }
        return token.ifEmpty { null }
    }

    private fun linksOf(html: String, page: String): JSONArray {
        val host = hostOf(page).removePrefix("www.")
        val found = JSONArray()
        val pattern = Regex("(?i)href\\s*=\\s*['\"]([^'\"]+)['\"]")
        for (match in pattern.findAll(html)) {
            if (found.length() >= 20) break
            val absolute = absoluteLink(page, match.groupValues[1]) ?: continue
            val linkHost = hostOf(absolute).removePrefix("www.")
            if (linkHost != host) continue
            if (!absolute.startsWith("http://") && !absolute.startsWith("https://")) continue
            found.put(absolute)
        }
        return found
    }

    private fun absoluteLink(page: String, raw: String): String? {
        val link = raw.trim()
        if (link.isEmpty() || link.startsWith("#") || link.startsWith("mailto:") || link.startsWith("javascript:")) return null
        if (link.startsWith("https://") || link.startsWith("http://")) return link
        val scheme = page.substringBefore("://", "https")
        val host = hostOf(page)
        if (link.startsWith("//")) return "$scheme:$link"
        if (link.startsWith("/")) return "$scheme://$host$link"
        val base = page.substringBefore('?').substringBefore('#')
        val slash = base.lastIndexOf('/')
        if (slash <= scheme.length + 2) return "$scheme://$host/$link"
        return base.substring(0, slash + 1) + link
    }

    private fun hostOf(address: String): String {
        return address.substringAfter("://").substringBefore('/').substringBefore(':').lowercase()
    }

    private fun titleOf(html: String): String {
        val match = Regex("(?is)<title[^>]*>(.*?)</title>").find(html) ?: return ""
        return plain(match.groupValues[1])
    }

    private fun textOf(html: String): String {
        val cut = html
            .replace(Regex("(?is)<script[^>]*>.*?</script>"), " ")
            .replace(Regex("(?is)<style[^>]*>.*?</style>"), " ")
        return plain(cut)
    }

    private fun plain(raw: String): String {
        return raw.replace(Regex("<[^>]+>"), " ").replace(Regex("\\s+"), " ").trim()
    }
}
