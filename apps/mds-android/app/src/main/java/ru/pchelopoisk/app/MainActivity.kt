package ru.pchelopoisk.app

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.view.Gravity
import android.view.View
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
                HiveService.stop(this@MainActivity)
                HiveService.start(this@MainActivity)
            }
        }
        status = TextView(this).apply {
            text = "Проверяю версию приложения"
            setPadding(32, 28, 32, 16)
            textSize = 14f
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
                    placeBack(url)
                }

                override fun onPageFinished(view: WebView?, url: String?) {
                    placeBack(url)
                    if (resetHistory) {
                        resetHistory = false
                        view?.clearHistory()
                    }
                }
            }
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
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

    private fun showSearch() {
        searching = true
        gate.visibility = View.GONE
        status.visibility = View.GONE
        backToSearch.visibility = View.GONE
        web.visibility = View.VISIBLE
        web.loadUrl(TrustedNodes.hive())
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
        HiveService.start(this)
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
