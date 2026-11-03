package io.github.arteminariy.voiceon

import android.app.Application
import io.github.arteminariy.voiceon.core.initLogging

class VoiceApp : Application() {
    override fun onCreate() {
        super.onCreate()
        // Журнал ядра — в logcat с тегом voiceon.
        initLogging()
    }
}
