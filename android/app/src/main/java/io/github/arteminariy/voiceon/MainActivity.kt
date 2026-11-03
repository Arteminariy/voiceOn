package io.github.arteminariy.voiceon

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.getValue
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.arteminariy.voiceon.core.logInfo
import io.github.arteminariy.voiceon.ui.App
import io.github.arteminariy.voiceon.ui.VoiceTheme

class MainActivity : ComponentActivity() {
    private val permissions = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { granted ->
        if (granted[Manifest.permission.RECORD_AUDIO] == true || hasPermission(Manifest.permission.RECORD_AUDIO)) {
            Call.connect(this)
        } else {
            logInfo("нет разрешения на микрофон")
            Call.ended("Нужен доступ к микрофону: разреши его в настройках приложения")
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        handleDebugExtras(intent)
        // Обновления проверяем при запуске, но не посреди звонка.
        if (Call.state.value.phase == Call.Phase.Idle) Updater.check()
        setContent {
            val call by Call.state.collectAsStateWithLifecycle()
            val update by Updater.state.collectAsStateWithLifecycle()
            VoiceTheme {
                App(call, update, onConnect = ::connect)
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleDebugExtras(intent)
    }

    /** Отладочная сборка: `am start … --ez test_tone true` включает тон вместо микрофона. */
    private fun handleDebugExtras(intent: Intent?) {
        if (BuildConfig.DEBUG && intent?.hasExtra("test_tone") == true) {
            Prefs(this).testTone = intent.getBooleanExtra("test_tone", false)
            logInfo("отладка: тестовый тон ${Prefs(this).testTone}")
        }
    }

    private fun connect() {
        val needed = buildList {
            add(Manifest.permission.RECORD_AUDIO)
            if (Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
            if (Build.VERSION.SDK_INT >= 31) add(Manifest.permission.BLUETOOTH_CONNECT)
        }.filterNot(::hasPermission)
        if (needed.isEmpty()) Call.connect(this) else permissions.launch(needed.toTypedArray())
    }

    private fun hasPermission(p: String) = ContextCompat.checkSelfPermission(this, p) == PackageManager.PERMISSION_GRANTED
}
