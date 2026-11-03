package io.github.arteminariy.voiceon

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.Network
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.PowerManager
import io.github.arteminariy.voiceon.core.CallConfig
import io.github.arteminariy.voiceon.core.CallStatus
import io.github.arteminariy.voiceon.core.VoiceException
import io.github.arteminariy.voiceon.core.VoiceSession
import io.github.arteminariy.voiceon.core.logInfo

/**
 * Звонок в фоне: foreground-сервис с типом microphone держит сессию ядра,
 * звук, маршрут, слежение за сетью и неснимаемое уведомление с кнопками.
 */
class CallService : Service() {
    private val main = Handler(Looper.getMainLooper())
    private var session: VoiceSession? = null
    private var audio: AudioEngine? = null
    private var router: AudioRouter? = null
    private var netCallback: ConnectivityManager.NetworkCallback? = null
    private var wakeLock: PowerManager.WakeLock? = null
    private var connecting = false
    private var lastNotification: String? = null
    private var title = "voiceOn: подключение…"

    private val ticker = object : Runnable {
        override fun run() {
            updateNotification()
            main.postDelayed(this, 1000)
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_CONNECT -> connect()
            ACTION_TOGGLE_MIC -> Call.setMicMuted(!Call.state.value.micMuted)
            ACTION_HANGUP -> hangup(null)
            else -> if (session == null && !connecting) stopSelf()
        }
        return START_NOT_STICKY
    }

    private fun connect() {
        // Каждый startForegroundService обязан закончиться startForeground, даже повторный.
        startInForeground()
        if (session != null || connecting) {
            return
        }
        connecting = true
        val prefs = Prefs(this)
        val config = CallConfig(
            server = prefs.server,
            nick = prefs.nick,
            passphrase = prefs.passphrase,
            vadThreshold = prefs.vadThreshold,
            denoise = prefs.denoise,
            volumes = prefs.volumes,
            testTone = prefs.testTone,
        )
        // Argon2 и DNS блокируют — не в главном потоке.
        Thread({
            val result = try {
                Result.success(VoiceSession.connect(config))
            } catch (e: VoiceException) {
                Result.failure(e)
            }
            main.post {
                connecting = false
                result.fold(::started, { hangup(it.message ?: "ошибка подключения") })
            }
        }, "voiceon-connect").start()
    }

    private fun started(s: VoiceSession) {
        val state = Call.state.value
        if (state.phase == Call.Phase.Idle) {
            // Пока подключались, нажали «Отключиться».
            Thread { s.disconnect() }.start()
            stopSelf()
            return
        }
        session = s
        s.setMicMuted(state.micMuted)
        s.setDeafened(state.deafened)
        router = AudioRouter(this).also { it.start(state.speaker) }
        audio = AudioEngine(s, Prefs(this).testTone).also { it.start() }
        watchNetwork(s)
        wakeLock = getSystemService(PowerManager::class.java)
            .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "voiceon:call")
            .also { it.acquire() }
        Call.onControlsChanged = {
            router?.setSpeaker(Call.state.value.speaker)
            updateNotification()
        }
        Call.started(s)
        main.post(ticker)
        logInfo("звонок начат")
    }

    /** Смена сети по умолчанию (Wi-Fi ↔ LTE): ядро сразу пересоздаёт сокет. */
    private fun watchNetwork(s: VoiceSession) {
        val cm = getSystemService(ConnectivityManager::class.java)
        val cb = object : ConnectivityManager.NetworkCallback() {
            private var current: Network? = null
            private var seen = false

            override fun onAvailable(network: Network) {
                val changed = seen && network != current
                seen = true
                current = network
                if (changed) {
                    logInfo("сеть по умолчанию сменилась: $network")
                    s.networkChanged()
                }
            }

            override fun onLost(network: Network) {
                if (network == current) {
                    logInfo("сеть потеряна: $network")
                    current = null
                }
            }
        }
        cm.registerDefaultNetworkCallback(cb)
        netCallback = cb
    }

    private fun hangup(error: String?) {
        main.removeCallbacks(ticker)
        Call.onControlsChanged = null
        netCallback?.let { runCatching { getSystemService(ConnectivityManager::class.java).unregisterNetworkCallback(it) } }
        netCallback = null
        audio?.stop()
        audio = null
        router?.stop()
        router = null
        session?.let { s ->
            // Громкости, выставленные во время звонка, сохраняем.
            Prefs(this).volumes = s.volumes()
            Thread({ s.disconnect() }, "voiceon-bye").start()
        }
        session = null
        wakeLock?.takeIf { it.isHeld }?.release()
        wakeLock = null
        Call.ended(error)
        if (error != null) logInfo("звонок не удался: $error")
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() {
        if (session != null) hangup(null)
        super.onDestroy()
    }

    private fun startInForeground() {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL, "Звонок", NotificationManager.IMPORTANCE_LOW).apply {
                setShowBadge(false)
            },
        )
        val n = buildNotification(title)
        if (Build.VERSION.SDK_INT >= 30) {
            startForeground(NOTIFICATION_ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE)
        } else {
            startForeground(NOTIFICATION_ID, n)
        }
    }

    private fun updateNotification() {
        val s = session ?: return
        val snap = s.snapshot()
        title = when (snap.status) {
            CallStatus.CONNECTING -> "voiceOn: подключение…"
            CallStatus.RECONNECTING -> "voiceOn: переподключение…"
            CallStatus.CONNECTED -> "voiceOn: в звонке · ${participants(snap.peers.size + 1)}"
        }
        val key = "$title|${Call.state.value.micMuted}"
        if (key == lastNotification) return
        lastNotification = key
        getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, buildNotification(title))
    }

    private fun buildNotification(title: String): Notification {
        val muted = Call.state.value.micMuted
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), flags)
        fun action(action: String, code: Int) =
            PendingIntent.getService(this, code, Intent(this, CallService::class.java).setAction(action), flags)
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(if (muted) "Микрофон выключен" else "Микрофон включён")
            .setContentIntent(open)
            .setOngoing(true)
            .setCategory(Notification.CATEGORY_CALL)
            .setOnlyAlertOnce(true)
            .addAction(
                Notification.Action.Builder(
                    null,
                    if (muted) "Микрофон вкл" else "Микрофон выкл",
                    action(ACTION_TOGGLE_MIC, 1),
                ).build(),
            )
            .addAction(Notification.Action.Builder(null, "Отключиться", action(ACTION_HANGUP, 2)).build())
            .build()
    }

    companion object {
        const val ACTION_CONNECT = "io.github.arteminariy.voiceon.CONNECT"
        const val ACTION_TOGGLE_MIC = "io.github.arteminariy.voiceon.TOGGLE_MIC"
        const val ACTION_HANGUP = "io.github.arteminariy.voiceon.HANGUP"
        private const val CHANNEL = "call"
        private const val NOTIFICATION_ID = 1

        /** «1 участник», «3 участника», «5 участников». */
        fun participants(n: Int): String {
            val word = when {
                n % 100 in 11..14 -> "участников"
                n % 10 == 1 -> "участник"
                n % 10 in 2..4 -> "участника"
                else -> "участников"
            }
            return "$n $word"
        }
    }
}
