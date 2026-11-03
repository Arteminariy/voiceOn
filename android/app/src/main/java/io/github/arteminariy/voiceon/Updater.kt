package io.github.arteminariy.voiceon

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.net.Uri
import android.provider.Settings
import io.github.arteminariy.voiceon.core.coreVersion
import io.github.arteminariy.voiceon.core.isNewerVersion
import io.github.arteminariy.voiceon.core.logInfo
import io.github.arteminariy.voiceon.core.releaseRepo
import io.github.arteminariy.voiceon.core.verifyRelease
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import org.json.JSONObject

/**
 * Обновление из релизов GitHub: при запуске смотрим последний релиз; если в нём
 * есть APK новее нашего, на экране входа появляется «Обновить». По нажатию
 * качаем `SHA256SUMS`, его подпись и APK, проверяем подпись и sha256 в ядре
 * (тот же ключ, что у десктопа) и отдаём APK системному установщику.
 */
object Updater {
    const val ASSET = "voiceon-android.apk"

    sealed interface State {
        data object Idle : State
        data class Available(val tag: String) : State
        data class Downloading(val tag: String) : State
        /** Нужно разрешить установку из этого источника в настройках. */
        data class NeedPermission(val tag: String) : State
        data class Installing(val tag: String) : State
        data class Failed(val tag: String, val message: String) : State
    }

    private val _state = MutableStateFlow<State>(State.Idle)
    val state: StateFlow<State> = _state

    @Volatile
    private var checked = false

    /** Проверка при запуске приложения (не во время звонка). */
    fun check() {
        if (checked) return
        checked = true
        Thread({
            try {
                val json = JSONObject(String(get("https://api.github.com/repos/${releaseRepo()}/releases/latest", 1 shl 20)))
                val tag = json.getString("tag_name")
                val assets = json.optJSONArray("assets")
                val hasApk = assets != null && (0 until assets.length()).any { assets.getJSONObject(it).optString("name") == ASSET }
                logInfo("обновление: последний релиз $tag, APK ${if (hasApk) "есть" else "нет"}, у нас ${coreVersion()}")
                if (hasApk && isNewerVersion(tag, coreVersion())) _state.value = State.Available(tag)
            } catch (e: Exception) {
                logInfo("обновление: проверка не удалась: $e")
            }
        }, "voiceon-update").start()
    }

    /** «Обновить»: скачать, проверить и запустить установку. */
    fun update(context: Context) {
        val tag = when (val s = _state.value) {
            is State.Available -> s.tag
            is State.NeedPermission -> s.tag
            is State.Failed -> s.tag
            else -> return
        }
        val app = context.applicationContext
        val apk = File(app.cacheDir, "update/$ASSET")
        if (_state.value is State.NeedPermission && apk.isFile) {
            Thread({ installOrFail(context, tag, apk) }, "voiceon-update").start()
            return
        }
        _state.value = State.Downloading(tag)
        Thread({
            try {
                val base = "https://github.com/${releaseRepo()}/releases/download/$tag"
                val sums = get("$base/SHA256SUMS", 64 shl 10)
                val sig = get("$base/SHA256SUMS.sig", 1 shl 10)
                apk.parentFile?.mkdirs()
                download("$base/$ASSET", apk)
                verifyRelease(sums, sig, ASSET, apk.path)
                logInfo("обновление: $tag скачано, подпись и sha256 сошлись")
                installOrFail(context, tag, apk)
            } catch (e: Exception) {
                apk.delete()
                logInfo("обновление: $e")
                _state.value = State.Failed(tag, e.message ?: e.toString())
            }
        }, "voiceon-update").start()
    }

    private fun installOrFail(context: Context, tag: String, apk: File) {
        try {
            install(context, tag, apk)
        } catch (e: Exception) {
            logInfo("обновление: установка: $e")
            _state.value = State.Failed(tag, e.message ?: e.toString())
        }
    }

    private fun install(context: Context, tag: String, apk: File) {
        val app = context.applicationContext
        if (!app.packageManager.canRequestPackageInstalls()) {
            _state.value = State.NeedPermission(tag)
            context.startActivity(
                Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${app.packageName}"))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
            return
        }
        _state.value = State.Installing(tag)
        val installer = app.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL)
        params.setAppPackageName(app.packageName)
        val id = installer.createSession(params)
        installer.openSession(id).use { session ->
            session.openWrite(ASSET, 0, apk.length()).use { out ->
                apk.inputStream().use { it.copyTo(out) }
                session.fsync(out)
            }
            val intent = Intent(app, InstallReceiver::class.java).putExtra(InstallReceiver.EXTRA_TAG, tag)
            // Система дописывает в интент статус и окно подтверждения — нужен изменяемый PendingIntent.
            val pending = PendingIntent.getBroadcast(
                app, id, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE,
            )
            session.commit(pending.intentSender)
        }
    }

    internal fun installFailed(tag: String, message: String) {
        _state.value = State.Failed(tag, message)
    }

    private fun open(url: String): HttpURLConnection {
        val c = URL(url).openConnection() as HttpURLConnection
        c.connectTimeout = 15_000
        c.readTimeout = 30_000
        c.instanceFollowRedirects = true
        c.setRequestProperty("User-Agent", "voiceon-android/${coreVersion()}")
        if (c.responseCode !in 200..299) {
            val code = c.responseCode
            c.disconnect()
            throw java.io.IOException("$url: HTTP $code")
        }
        return c
    }

    private fun get(url: String, limit: Int): ByteArray {
        val c = open(url)
        try {
            val out = java.io.ByteArrayOutputStream()
            c.inputStream.use { input ->
                val buf = ByteArray(16 shl 10)
                while (true) {
                    val n = input.read(buf)
                    if (n < 0) break
                    out.write(buf, 0, n)
                    if (out.size() > limit) throw java.io.IOException("$url: слишком большой ответ")
                }
            }
            return out.toByteArray()
        } finally {
            c.disconnect()
        }
    }

    private fun download(url: String, to: File) {
        val c = open(url)
        try {
            c.inputStream.use { input -> to.outputStream().use { input.copyTo(it) } }
        } finally {
            c.disconnect()
        }
    }
}
