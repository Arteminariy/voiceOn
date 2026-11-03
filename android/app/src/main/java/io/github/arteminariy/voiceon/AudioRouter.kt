package io.github.arteminariy.voiceon

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import io.github.arteminariy.voiceon.core.logInfo

/**
 * Режим звонка и маршрут звука: подключённая проводная или Bluetooth-гарнитура
 * берётся сама, иначе разговорный динамик; «Громкая связь» — основной динамик.
 * На API 31+ — setCommunicationDevice, раньше — speakerphone и SCO.
 */
class AudioRouter(context: Context) {
    private val am = context.getSystemService(AudioManager::class.java)
    private val handler = Handler(Looper.getMainLooper())
    private var speaker = false
    private var scoStarted = false
    private var focus: AudioFocusRequest? = null

    /** Гарнитуру подключили или отключили посреди звонка — выбираем маршрут заново. */
    private val deviceCallback = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(added: Array<out AudioDeviceInfo>) = apply()
        override fun onAudioDevicesRemoved(removed: Array<out AudioDeviceInfo>) = apply()
    }

    fun start(speaker: Boolean) {
        this.speaker = speaker
        val attrs = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
            .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
            .build()
        focus = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
            .setAudioAttributes(attrs)
            .setOnAudioFocusChangeListener { }
            .build()
            .also { am.requestAudioFocus(it) }
        // В этом режиме телефон включает аппаратные эхоподавление и шумодав.
        am.mode = AudioManager.MODE_IN_COMMUNICATION
        am.registerAudioDeviceCallback(deviceCallback, handler)
        apply()
    }

    fun setSpeaker(on: Boolean) {
        if (speaker == on) return
        speaker = on
        logInfo("звук: громкая связь ${if (on) "вкл" else "выкл"}")
        apply()
    }

    fun stop() {
        am.unregisterAudioDeviceCallback(deviceCallback)
        if (Build.VERSION.SDK_INT >= 31) {
            am.clearCommunicationDevice()
        } else {
            @Suppress("DEPRECATION")
            am.isSpeakerphoneOn = false
            stopSco()
        }
        am.mode = AudioManager.MODE_NORMAL
        focus?.let { am.abandonAudioFocusRequest(it) }
        focus = null
    }

    private fun apply() {
        if (Build.VERSION.SDK_INT >= 31) applyModern() else applyLegacy()
    }

    private fun applyModern() {
        if (Build.VERSION.SDK_INT < 31) return
        val devices = am.availableCommunicationDevices
        fun find(vararg types: Int): AudioDeviceInfo? = types.asSequence().mapNotNull { t -> devices.firstOrNull { it.type == t } }.firstOrNull()
        val target = if (speaker) {
            find(AudioDeviceInfo.TYPE_BUILTIN_SPEAKER)
        } else {
            find(
                AudioDeviceInfo.TYPE_BLE_HEADSET,
                AudioDeviceInfo.TYPE_BLUETOOTH_SCO,
                AudioDeviceInfo.TYPE_WIRED_HEADSET,
                AudioDeviceInfo.TYPE_WIRED_HEADPHONES,
                AudioDeviceInfo.TYPE_USB_HEADSET,
                AudioDeviceInfo.TYPE_BUILTIN_EARPIECE,
            )
        }
        if (target == null) {
            am.clearCommunicationDevice()
            logInfo("звук: маршрут по умолчанию")
            return
        }
        if (am.communicationDevice?.id != target.id) {
            val ok = am.setCommunicationDevice(target)
            logInfo("звук: маршрут ${typeName(target.type)} ${target.productName} ($ok)")
        }
    }

    @Suppress("DEPRECATION")
    private fun applyLegacy() {
        if (speaker) {
            stopSco()
            am.isSpeakerphoneOn = true
            logInfo("звук: громкая связь")
            return
        }
        am.isSpeakerphoneOn = false
        val bt = am.getDevices(AudioManager.GET_DEVICES_OUTPUTS).any { it.type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO }
        if (bt && !scoStarted) {
            am.startBluetoothSco()
            am.isBluetoothScoOn = true
            scoStarted = true
            logInfo("звук: Bluetooth-гарнитура (SCO)")
        } else if (!bt) {
            stopSco()
            logInfo("звук: разговорный динамик или проводная гарнитура")
        }
    }

    @Suppress("DEPRECATION")
    private fun stopSco() {
        if (scoStarted) {
            am.isBluetoothScoOn = false
            am.stopBluetoothSco()
            scoStarted = false
        }
    }

    private fun typeName(type: Int) = when (type) {
        AudioDeviceInfo.TYPE_BUILTIN_SPEAKER -> "динамик"
        AudioDeviceInfo.TYPE_BUILTIN_EARPIECE -> "разговорный динамик"
        AudioDeviceInfo.TYPE_BLUETOOTH_SCO -> "Bluetooth"
        AudioDeviceInfo.TYPE_BLE_HEADSET -> "Bluetooth LE"
        AudioDeviceInfo.TYPE_WIRED_HEADSET, AudioDeviceInfo.TYPE_WIRED_HEADPHONES -> "проводная гарнитура"
        AudioDeviceInfo.TYPE_USB_HEADSET -> "USB-гарнитура"
        else -> "тип $type"
    }
}
