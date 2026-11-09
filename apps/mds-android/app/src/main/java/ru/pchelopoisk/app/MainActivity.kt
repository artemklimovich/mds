package ru.pchelopoisk.app

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.PowerManager
import android.provider.Settings
import android.view.Gravity
import android.view.View
import android.webkit.GeolocationPermissions
import android.webkit.RenderProcessGoneDetail
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AppCompatActivity
import java.util.concurrent.Executors

class MainActivity : AppCompatActivity() {
    private val pool = Executors.newSingleThreadExecutor()
    private lateinit var status: TextView
    private lateinit var backToSearch: Button
    private lateinit var web: WebView
    private lateinit var gate: LinearLayout
    private var searching = false
    private var resetHistory = false
    private var geoOrigin: String? = null
    private var geoCallback: GeolocationPermissions.Callback? = null
    private var lastCrawl = "Обход ещё не ходил к улью."
    private var opening = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(Color.WHITE)
        }
        val pace = Button(this).apply {
            text = paceLabel()
            setOnClickListener {
                val prefs = getSharedPreferences("pchela", MODE_PRIVATE)
                val next = when (prefs.getString("pace", "hive")) {
                    "hive" -> "fast"
                    "fast" -> "server"
                    else -> "hive"
                }
                prefs.edit().putString("pace", next).apply()
                text = paceLabel()
                HiveService.start(this@MainActivity)
            }
        }
        status = TextView(this).apply {
            text = "Проверяю версию приложения"
            setPadding(24, 8, 24, 8)
            textSize = 13f
            maxLines = 3
            setTextColor(Color.parseColor("#4d5156"))
        }
        backToSearch = Button(this).apply {
            text = "К поиску"
            visibility = View.GONE
            setOnClickListener {
                resetHistory = true
                web.loadUrl(TrustedNodes.hive())
            }
        }
        web = WebView(this).apply {
            visibility = View.GONE
            webViewClient = object : WebViewClient() {
                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                    val target = request.url ?: return false
                    val scheme = target.scheme?.lowercase()
                    if (scheme != "http" && scheme != "https") {
                        openOutside(target)
                        return true
                    }
                    if (target.path.orEmpty().endsWith(".apk")) {
                        openOutside(target)
                        return true
                    }
                    return false
                }

                override fun onPageStarted(view: WebView?, url: String?, favicon: android.graphics.Bitmap?) {
                    opening = true
                    status.text = "Открываю страницу…"
                    placeBack(url)
                }

                override fun onPageFinished(view: WebView?, url: String?) {
                    opening = false
                    status.text = lastCrawl
                    placeBack(url)
                    if (resetHistory) {
                        resetHistory = false
                        view?.clearHistory()
                    }
                }

                override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
                    if (!request.isForMainFrame) return
                    opening = false
                    status.text = "Страница не открылась. Нажмите «К поиску»."
                }

                override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean {
                    opening = false
                    status.text = "Экран поиска сбросился, открываю снова."
                    view.loadUrl(TrustedNodes.hive())
                    return true
                }
            }
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            setBackgroundColor(Color.parseColor("#f6f1e7"))
            settings.setGeolocationEnabled(true)
            webChromeClient = object : WebChromeClient() {
                override fun onGeolocationPermissionsShowPrompt(
                    origin: String?,
                    callback: GeolocationPermissions.Callback,
                ) {
                    val fine = checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED
                    val coarse = checkSelfPermission(Manifest.permission.ACCESS_COARSE_LOCATION) == PackageManager.PERMISSION_GRANTED
                    if (fine || coarse) {
                        callback.invoke(origin, true, false)
                        return
                    }
                    geoOrigin = origin
                    geoCallback = callback
                    requestPermissions(
                        arrayOf(Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION),
                        2,
                    )
                }
            }
            layoutParams = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                0,
                1f,
            )
        }
        gate = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER
            visibility = View.GONE
            setPadding(48, 80, 48, 48)
            layoutParams = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                0,
                1f,
            )
        }
        root.addView(pace)
        root.addView(status)
        root.addView(backToSearch)
        root.addView(web)
        root.addView(gate)
        setContentView(root)
        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                if (searching && web.canGoBack()) web.goBack() else finish()
            }
        })
        pool.execute {
            val newer = try {
                UpdateGate.newer()
            } catch (_: Exception) {
                null
            }
            runOnUiThread {
                if (newer == null) showSearch() else showGate(newer)
            }
        }
    }

    private fun paceLabel(): String {
        val mode = getSharedPreferences("pchela", MODE_PRIVATE).getString("pace", "hive")
        return when (mode) {
            "fast" -> "Темп: быстрее"
            "server" -> "Темп: как сервер"
            else -> "Темп: обычный"
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == 1) {
            askBattery()
            return
        }
        if (requestCode != 2) return
        val ok = grantResults.any { it == PackageManager.PERMISSION_GRANTED }
        geoCallback?.invoke(geoOrigin, ok, false)
        geoCallback = null
        geoOrigin = null
    }

    override fun onStart() {
        super.onStart()
        HiveStatus.listener = { text ->
            runOnUiThread {
                lastCrawl = text
                if (searching && !opening) status.text = text
            }
        }
    }

    override fun onStop() {
        HiveStatus.listener = null
        super.onStop()
    }

    private fun showSearch() {
        searching = true
        gate.visibility = View.GONE
        status.visibility = View.VISIBLE
        status.text = lastCrawl
        backToSearch.visibility = View.GONE
        web.visibility = View.VISIBLE
        web.loadUrl(TrustedNodes.hive())
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        } else {
            askBattery()
        }
        HiveService.start(this)
    }

    private fun askBattery() {
        if (Build.VERSION.SDK_INT < 23) return
        val prefs = getSharedPreferences("pchela", MODE_PRIVATE)
        if (prefs.getBoolean("battery_asked", false)) return
        val power = getSystemService(PowerManager::class.java) ?: return
        if (power.isIgnoringBatteryOptimizations(packageName)) return
        prefs.edit().putBoolean("battery_asked", true).apply()
        try {
            startActivity(Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:$packageName")))
        } catch (_: Exception) {
        }
    }

    private fun showGate(version: String) {
        searching = false
        HiveService.stop(this)
        backToSearch.visibility = View.GONE
        web.visibility = View.GONE
        gate.visibility = View.VISIBLE
        gate.removeAllViews()
        status.visibility = View.VISIBLE
        status.text = "Нужно обновиться"
        val words = TextView(this).apply {
            text = "Вышла версия $version. Поиск откроется после установки."
            textSize = 18f
            setTextColor(Color.parseColor("#202124"))
            setPadding(0, 0, 0, 28)
        }
        val button = Button(this).apply {
            text = "Обновить"
            setOnClickListener {
                text = "Скачиваю"
                isEnabled = false
                pool.execute {
                    val file = UpdateGate.download(this@MainActivity)
                    runOnUiThread {
                        isEnabled = true
                        text = "Обновить"
                        if (file == null) {
                            words.text = "Файл обновления сейчас не скачался. Нажмите ещё раз."
                        } else {
                            UpdateGate.install(this@MainActivity, file)
                        }
                    }
                }
            }
        }
        gate.addView(words)
        gate.addView(button)
    }

    private fun placeBack(url: String?) {
        val uri = url?.let(Uri::parse)
        val outside = searching && uri != null && !isHive(uri)
        backToSearch.visibility = if (outside) View.VISIBLE else View.GONE
    }

    private fun isHive(uri: Uri): Boolean {
        val host = uri.host?.lowercase() ?: return false
        val hiveHost = Uri.parse(TrustedNodes.hive()).host?.lowercase() ?: return false
        return host == hiveHost
    }

    private fun openOutside(uri: Uri) {
        try {
            startActivity(Intent(Intent.ACTION_VIEW, uri))
        } catch (_: Exception) {
            status.visibility = View.VISIBLE
            status.text = "Эта ссылка на телефоне не открылась"
        }
    }
}
