package io.github.arteminariy.voiceon

import android.annotation.SuppressLint
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaRecorder
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.NoiseSuppressor
import android.os.Process
import io.github.arteminariy.voiceon.core.VoiceSession
import io.github.arteminariy.voiceon.core.logInfo

/**
 * Захват и воспроизведение. Кадры по 20 мс PCM16 48 кГц моно ходят в ядро и
 * обратно одним вызовом на кадр. Темп воспроизведения задаёт блокирующий
 * `AudioTrack.write` — часы звукового тракта, как на десктопе.
 */
class AudioEngine(private val session: VoiceSession, private val testTone: Boolean) {
    @Volatile
    private var running = true
    private val capture = Thread(::captureLoop, "voiceon-capture")
    private val playback = Thread(::playbackLoop, "voiceon-playback")

    fun start() {
        capture.start()
        playback.start()
    }

    fun stop() {
        running = false
        capture.join(1000)
        playback.join(1000)
    }

    @SuppressLint("MissingPermission") // RECORD_AUDIO проверяется до запуска звонка
    private fun captureLoop() {
        Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
        if (testTone) {
            toneLoop()
            return
        }
        val minBuf = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val rec = try {
            AudioRecord.Builder()
                .setAudioSource(MediaRecorder.AudioSource.VOICE_COMMUNICATION)
                .setAudioFormat(
                    AudioFormat.Builder()
                        .setSampleRate(RATE)
                        .setChannelMask(AudioFormat.CHANNEL_IN_MONO)
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .build(),
                )
                .setBufferSizeInBytes(maxOf(minBuf, FRAME_BYTES * 4))
                .build()
        } catch (e: Exception) {
            logInfo("микрофон: ${e.message}")
            return
        }
        if (rec.state != AudioRecord.STATE_INITIALIZED) {
            logInfo("микрофон: AudioRecord не инициализирован")
            rec.release()
            return
        }
        // Обычно VOICE_COMMUNICATION уже включает аппаратные AEC и шумодав; на всякий случай просим явно.
        val aec = if (AcousticEchoCanceler.isAvailable()) AcousticEchoCanceler.create(rec.audioSessionId) else null
        aec?.enabled = true
        val ns = if (NoiseSuppressor.isAvailable()) NoiseSuppressor.create(rec.audioSessionId) else null
        ns?.enabled = true
        logInfo("микрофон: буфер $minBuf байт, AEC=${aec?.enabled ?: false}, NS=${ns?.enabled ?: false}")
        rec.startRecording()
        val buf = ByteArray(FRAME_BYTES)
        try {
            while (running) {
                val n = rec.read(buf, 0, buf.size)
                if (n > 0) {
                    session.pushCapture(if (n == buf.size) buf else buf.copyOf(n))
                } else if (n < 0) {
                    logInfo("микрофон: ошибка чтения $n")
                    Thread.sleep(20)
                }
            }
        } finally {
            rec.stop()
            aec?.release()
            ns?.release()
            rec.release()
        }
    }

    /** Отладка: тон генерирует ядро, здесь только отсчитываем 20-мс кадры по часам. */
    private fun toneLoop() {
        val silent = ByteArray(FRAME_BYTES)
        var next = System.nanoTime()
        while (running) {
            session.pushCapture(silent)
            next += 20_000_000L
            val wait = next - System.nanoTime()
            if (wait > 0) Thread.sleep(wait / 1_000_000L, (wait % 1_000_000L).toInt())
        }
    }

    private fun playbackLoop() {
        Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
        val minBuf = AudioTrack.getMinBufferSize(RATE, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val track = try {
            AudioTrack.Builder()
                .setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                        .build(),
                )
                .setAudioFormat(
                    AudioFormat.Builder()
                        .setSampleRate(RATE)
                        .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .build(),
                )
                // ~40–60 мс в тракте: jitter buffer ядра и так держит запас.
                .setBufferSizeInBytes(maxOf(minBuf, FRAME_BYTES * 2))
                .setTransferMode(AudioTrack.MODE_STREAM)
                .build()
        } catch (e: Exception) {
            logInfo("вывод звука: ${e.message}")
            return
        }
        logInfo("вывод звука: буфер ${track.bufferSizeInFrames} кадров (мин. ${minBuf / 2})")
        track.play()
        try {
            while (running) {
                val pcm = session.pullPlayback()
                val n = track.write(pcm, 0, pcm.size)
                if (n < 0) {
                    logInfo("вывод звука: ошибка записи $n")
                    Thread.sleep(20)
                }
            }
        } finally {
            track.pause()
            track.flush()
            track.release()
        }
    }

    companion object {
        const val RATE = 48_000
        /** 20 мс: 960 сэмплов по 2 байта. */
        const val FRAME_BYTES = 1920
    }
}
