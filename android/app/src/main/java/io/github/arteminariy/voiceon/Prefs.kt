package io.github.arteminariy.voiceon

import android.content.Context
import org.json.JSONObject

/** Настройки клиента — те же поля, что в десктопном config.toml. */
class Prefs(context: Context) {
    private val sp = context.getSharedPreferences("voiceon", Context.MODE_PRIVATE)

    var server: String
        get() = sp.getString("server", "") ?: ""
        set(v) = sp.edit().putString("server", v).apply()

    var nick: String
        get() = sp.getString("nick", "") ?: ""
        set(v) = sp.edit().putString("nick", v).apply()

    var passphrase: String
        get() = sp.getString("passphrase", "") ?: ""
        set(v) = sp.edit().putString("passphrase", v).apply()

    /** Порог вероятности речи для VAD (0..1). */
    var vadThreshold: Float
        get() = sp.getFloat("vad_threshold", 0.6f)
        set(v) = sp.edit().putFloat("vad_threshold", v).apply()

    /** Шумодав RNNoise. */
    var denoise: Boolean
        get() = sp.getBoolean("denoise", true)
        set(v) = sp.edit().putBoolean("denoise", v).apply()

    /** Громкая связь (динамик вместо разговорного). */
    var speaker: Boolean
        get() = sp.getBoolean("speaker", false)
        set(v) = sp.edit().putBoolean("speaker", v).apply()

    /** Громкость каждого участника по нику, 1.0 = 100%. */
    var volumes: Map<String, Float>
        get() {
            val json = runCatching { JSONObject(sp.getString("volumes", "{}") ?: "{}") }.getOrElse { JSONObject() }
            return json.keys().asSequence().associateWith { json.optDouble(it, 1.0).toFloat() }
        }
        set(v) {
            val json = JSONObject()
            v.forEach { (k, x) -> json.put(k, x.toDouble()) }
            sp.edit().putString("volumes", json.toString()).apply()
        }

    /** Только для отладочной сборки: тестовый тон вместо микрофона. */
    var testTone: Boolean
        get() = BuildConfig.DEBUG && sp.getBoolean("test_tone", false)
        set(v) = sp.edit().putBoolean("test_tone", v && BuildConfig.DEBUG).apply()
}
