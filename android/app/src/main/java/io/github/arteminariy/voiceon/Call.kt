package io.github.arteminariy.voiceon

import android.content.Context
import android.content.Intent
import androidx.core.content.ContextCompat
import io.github.arteminariy.voiceon.core.VoiceSession
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update

/**
 * Состояние звонка, общее для UI и [CallService]. Сам звонок (сессия ядра, звук,
 * сеть) живёт в сервисе, чтобы продолжаться в фоне и с выключенным экраном.
 */
object Call {
    enum class Phase { Idle, Connecting, InCall }

    data class State(
        val phase: Phase = Phase.Idle,
        val error: String? = null,
        val micMuted: Boolean = false,
        val deafened: Boolean = false,
        val speaker: Boolean = false,
    )

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state

    /** Сессия ядра, пока идёт звонок; UI опрашивает её снимок. */
    @Volatile
    var session: VoiceSession? = null
        internal set

    /** Сервис подписывается, чтобы обновить уведомление и маршрут звука. */
    internal var onControlsChanged: (() -> Unit)? = null

    fun connect(context: Context) {
        val speaker = Prefs(context).speaker
        _state.update { it.copy(phase = Phase.Connecting, error = null, speaker = speaker) }
        ContextCompat.startForegroundService(
            context,
            Intent(context, CallService::class.java).setAction(CallService.ACTION_CONNECT),
        )
    }

    fun hangup(context: Context) {
        context.startService(Intent(context, CallService::class.java).setAction(CallService.ACTION_HANGUP))
    }

    fun setMicMuted(muted: Boolean) {
        _state.update { it.copy(micMuted = muted) }
        session?.setMicMuted(muted)
        onControlsChanged?.invoke()
    }

    fun setDeafened(deafened: Boolean) {
        _state.update { it.copy(deafened = deafened) }
        session?.setDeafened(deafened)
        onControlsChanged?.invoke()
    }

    fun setSpeaker(context: Context, on: Boolean) {
        Prefs(context).speaker = on
        _state.update { it.copy(speaker = on) }
        onControlsChanged?.invoke()
    }

    internal fun started(session: VoiceSession) {
        this.session = session
        _state.update { it.copy(phase = Phase.InCall, error = null) }
    }

    internal fun ended(error: String? = null) {
        session = null
        _state.update { State(error = error, speaker = it.speaker) }
    }
}
