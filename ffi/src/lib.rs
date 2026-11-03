//! UniFFI-обёртка ядра для Android (Kotlin).
//!
//! Звук передаётся кадрами PCM16 little-endian, 48 кГц моно, как `ByteArray`:
//! `AudioRecord.read(ByteArray)` и `AudioTrack.write(ByteArray)` отдают и берут
//! ровно этот формат, так что на кадр приходится один вызов через FFI без
//! поэлементного копирования. Сеть крутится в собственном потоке ядра;
//! UI и уведомление опрашивают [`VoiceSession::snapshot`].
//!
//! Вызовы блокирующие (Argon2 и DNS в `connect`, Opus и RNNoise в
//! `push_capture`) — не звать из главного потока.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use voiceon_core::capture::TxPipeline;
use voiceon_core::dsp::rms;
use voiceon_core::mixer::{FRAME_SAMPLES, Mixer, SAMPLE_RATE};
use voiceon_core::net::{self, Link, Shared, Status};
use voiceon_proto::{Crypto, truncate_nick};

uniffi::setup_scaffolding!();

const BITRATE: i32 = 32_000;
const FEC_LOSS_PERC: i32 = 10;
const STATS_EVERY: Duration = Duration::from_secs(5);

/// Ошибка для Kotlin: `VoiceException.Failed`, текст — в `message`.
#[derive(Debug, uniffi::Error)]
#[uniffi(flat_error)]
pub enum VoiceError {
    Failed(String),
}

impl std::fmt::Display for VoiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VoiceError::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for VoiceError {}

impl From<String> for VoiceError {
    fn from(message: String) -> VoiceError {
        VoiceError::Failed(message)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CallStatus {
    Connecting,
    Connected,
    Reconnecting,
}

/// Параметры звонка (то, что десктоп берёт из конфига).
#[derive(Debug, Clone, uniffi::Record)]
pub struct CallConfig {
    pub server: String,
    pub nick: String,
    pub passphrase: String,
    pub vad_threshold: f32,
    pub denoise: bool,
    /// Громкость каждого участника по нику, 1.0 = 100%.
    pub volumes: HashMap<String, f32>,
    /// Только для отладки: вместо микрофона шлётся тон (секунду звучит, секунду
    /// молчит) мимо шумодава и VAD, как у десктопного бота.
    pub test_tone: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Peer {
    pub session_id: u32,
    pub nick: String,
    pub speaking: bool,
    pub volume: f32,
}

/// Снимок состояния звонка для UI и уведомления.
#[derive(Debug, Clone, uniffi::Record)]
pub struct CallSnapshot {
    pub status: CallStatus,
    pub rtt_ms: Option<f32>,
    pub loss_pct: f32,
    /// Остальные участники (без нас).
    pub peers: Vec<Peer>,
    /// RMS захваченного сигнала (0..1) за последние ~100 мс.
    pub mic_level: f32,
    /// Голос сейчас уходит в эфир.
    pub transmitting: bool,
    /// Вероятность речи по RNNoise — для полосы порога VAD.
    pub voice_prob: f32,
    pub tx_packets: u64,
    pub rx_packets: u64,
}

struct Capture {
    tx: TxPipeline,
    buf: Vec<f32>,
    tone_phase: f32,
    tone_samples: u64,
}

struct Playback {
    mixer: Mixer,
    frame: [f32; FRAME_SAMPLES],
    frames: u32,
    heard: u32,
    peak: f32,
    last_report: Instant,
}

#[derive(uniffi::Object)]
pub struct VoiceSession {
    shared: Arc<Shared>,
    link: Arc<Link>,
    net: Mutex<Option<JoinHandle<()>>>,
    capture: Mutex<Capture>,
    playback: Mutex<Playback>,
    test_tone: bool,
    captured_frames: AtomicU64,
}

#[uniffi::export]
impl VoiceSession {
    /// Подключается к серверу: запускает сетевой поток ядра. Звук начинает идти,
    /// когда платформа зовёт `push_capture` и `pull_playback`.
    #[uniffi::constructor]
    pub fn connect(config: CallConfig) -> Result<Arc<VoiceSession>, VoiceError> {
        init_logging();
        let nick = truncate_nick(config.nick.trim()).to_string();
        if nick.is_empty() {
            return Err("укажи ник".to_string().into());
        }
        if config.passphrase.is_empty() {
            return Err("укажи пароль комнаты".to_string().into());
        }
        let addr = net::resolve(&config.server)?;
        let crypto = Crypto::from_passphrase(&config.passphrase);
        let link = Arc::new(Link::new(crypto, addr).map_err(|e| format!("сокет: {e}"))?);
        let shared = Shared::new(config.volumes, config.vad_threshold, Arc::new(|| {}));
        shared.controls.denoise.store(config.denoise, Ordering::Relaxed);
        let tx = TxPipeline::new(link.clone(), shared.clone(), BITRATE, FEC_LOSS_PERC, config.test_tone)?;
        if config.test_tone {
            voiceon_core::log::info("отладка: вместо микрофона идёт тестовый тон");
        }
        let net = net::spawn(link.clone(), shared.clone(), nick);
        Ok(Arc::new(VoiceSession {
            shared,
            link,
            net: Mutex::new(Some(net)),
            capture: Mutex::new(Capture {
                tx,
                buf: Vec::with_capacity(FRAME_SAMPLES * 2),
                tone_phase: 0.0,
                tone_samples: 0,
            }),
            playback: Mutex::new(Playback {
                mixer: Mixer::new(),
                frame: [0.0; FRAME_SAMPLES],
                frames: 0,
                heard: 0,
                peak: 0.0,
                last_report: Instant::now(),
            }),
            test_tone: config.test_tone,
            captured_frames: AtomicU64::new(0),
        }))
    }

    /// Захваченный звук: PCM16 LE, 48 кГц моно, любой длины (обычно 10–20 мс).
    pub fn push_capture(&self, pcm: Vec<u8>) {
        let mut g = self.capture.lock().unwrap();
        let c = &mut *g;
        let n = pcm.len() / 2;
        self.captured_frames.fetch_add(1, Ordering::Relaxed);
        c.buf.clear();
        if self.test_tone {
            let on = (c.tone_samples / SAMPLE_RATE as u64) % 2 == 0;
            c.tone_samples += n as u64;
            if !on {
                return;
            }
            let step = 440.0 * std::f32::consts::TAU / SAMPLE_RATE as f32;
            for _ in 0..n {
                c.buf.push(0.3 * c.tone_phase.sin());
                c.tone_phase = (c.tone_phase + step) % std::f32::consts::TAU;
            }
        } else {
            c.buf.extend(pcm.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0));
        }
        let Capture { tx, buf, .. } = c;
        tx.push(buf);
    }

    /// Следующие 20 мс сведённого звука: PCM16 LE, 48 кГц моно, 960 сэмплов.
    /// Темп задают часы воспроизведения платформы (блокирующий `AudioTrack.write`).
    pub fn pull_playback(&self) -> Vec<u8> {
        let mut g = self.playback.lock().unwrap();
        let p = &mut *g;
        p.mixer.mix(&self.shared, &mut p.frame);
        let level = rms(&p.frame);
        p.frames += 1;
        if level > 0.01 {
            p.heard += 1;
        }
        p.peak = p.peak.max(level);
        if p.last_report.elapsed() >= STATS_EVERY {
            self.report(p);
        }
        let mut out = Vec::with_capacity(FRAME_SAMPLES * 2);
        for s in p.frame {
            out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        out
    }

    pub fn snapshot(&self) -> CallSnapshot {
        let now = Instant::now();
        let st = self.shared.state.lock().unwrap();
        let peers = st
            .roster
            .iter()
            .filter(|e| e.session_id != st.my_session)
            .map(|e| Peer {
                session_id: e.session_id,
                nick: e.nick.clone(),
                speaking: st.is_speaking(e.session_id, now),
                volume: st.volumes.get(&e.nick).copied().unwrap_or(1.0),
            })
            .collect();
        CallSnapshot {
            status: status(st.status),
            rtt_ms: st.rtt_ms,
            loss_pct: st.loss_pct,
            peers,
            mic_level: st.mic_level,
            transmitting: st.transmitting,
            voice_prob: st.voice_prob,
            tx_packets: self.shared.tx_packets.load(Ordering::Relaxed),
            rx_packets: self.shared.rx_packets.load(Ordering::Relaxed),
        }
    }

    pub fn set_mic_muted(&self, muted: bool) {
        self.shared.controls.mic_muted.store(muted, Ordering::Relaxed);
    }

    pub fn set_deafened(&self, deafened: bool) {
        self.shared.controls.deafened.store(deafened, Ordering::Relaxed);
    }

    pub fn set_denoise(&self, on: bool) {
        self.shared.controls.denoise.store(on, Ordering::Relaxed);
    }

    pub fn set_vad_threshold(&self, threshold: f32) {
        self.shared.controls.set_vad_threshold(threshold);
    }

    pub fn set_volume(&self, nick: String, volume: f32) {
        self.shared.state.lock().unwrap().volumes.insert(nick, volume.clamp(0.0, 2.0));
    }

    /// Громкости участников (выставленные в звонке) — чтобы сохранить в настройки.
    pub fn volumes(&self) -> HashMap<String, f32> {
        self.shared.state.lock().unwrap().volumes.clone()
    }

    /// Сменилась сеть: сокет пересоздаётся сразу, сессия та же.
    pub fn network_changed(&self) {
        self.link.network_changed();
    }

    /// Отключается (шлёт Bye) и ждёт сетевой поток. Повторный вызов ничего не делает.
    pub fn disconnect(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.net.lock().unwrap().take() {
            let _ = t.join();
            voiceon_core::log::info("отключено");
        }
    }
}

impl VoiceSession {
    fn report(&self, p: &mut Playback) {
        let secs = p.last_report.elapsed().as_secs_f32();
        p.last_report = Instant::now();
        let st = self.shared.state.lock().unwrap();
        let roster: Vec<&str> = st.roster.iter().map(|e| e.nick.as_str()).collect();
        voiceon_core::log::info(&format!(
            "статистика: статус={:?} rtt={} потери={:.0}% участники={roster:?} воспроизведение {:.0} к/с, со звуком {}/{}, пик {:.2}; захват {:.0} к/с; пакетов отправлено {}, принято {}",
            st.status,
            st.rtt_ms.map(|r| format!("{r:.0}мс")).unwrap_or("-".into()),
            st.loss_pct,
            p.frames as f32 / secs,
            p.heard,
            p.frames,
            p.peak,
            self.captured_frames.swap(0, Ordering::Relaxed) as f32 / secs,
            self.shared.tx_packets.load(Ordering::Relaxed),
            self.shared.rx_packets.load(Ordering::Relaxed),
        ));
        p.frames = 0;
        p.heard = 0;
        p.peak = 0.0;
    }
}

impl Drop for VoiceSession {
    fn drop(&mut self) {
        self.disconnect();
    }
}

fn status(s: Status) -> CallStatus {
    match s {
        Status::Connecting => CallStatus::Connecting,
        Status::Connected => CallStatus::Connected,
        Status::Reconnecting => CallStatus::Reconnecting,
    }
}

/// Направляет журнал ядра в logcat (тег `voiceon`) или в stderr на хосте.
#[uniffi::export]
pub fn init_logging() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| voiceon_core::log::set_sink(write_log));
}

#[cfg(target_os = "android")]
fn write_log(msg: &str) {
    use std::ffi::{CString, c_char, c_int};
    #[link(name = "log")]
    unsafe extern "C" {
        fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
    }
    const ANDROID_LOG_INFO: c_int = 4;
    let text = CString::new(msg.replace('\0', " ")).unwrap_or_default();
    // SAFETY: обе строки — валидные C-строки, живущие до конца вызова.
    unsafe {
        __android_log_write(ANDROID_LOG_INFO, c"voiceon".as_ptr(), text.as_ptr());
    }
}

#[cfg(not(target_os = "android"))]
fn write_log(msg: &str) {
    eprintln!("voiceon: {msg}");
}

/// Пишет строку в журнал ядра (чтобы события платформы шли рядом с событиями ядра).
#[uniffi::export]
pub fn log_info(message: String) {
    voiceon_core::log::info(&message);
}

/// Версия ядра — она же версия приложения (общая версия workspace).
#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Репозиторий релизов на GitHub (`владелец/имя`).
#[uniffi::export]
pub fn release_repo() -> String {
    voiceon_core::release::REPO.to_string()
}

/// Тег `latest` новее версии `current`.
#[uniffi::export]
pub fn is_newer_version(latest: String, current: String) -> bool {
    voiceon_core::release::is_newer(&latest, &current)
}

/// Проверяет подпись `SHA256SUMS` ключом релизов и sha256 файла `asset`, скачанного в `path`.
#[uniffi::export]
pub fn verify_release(sums: Vec<u8>, signature: Vec<u8>, asset: String, path: String) -> Result<(), VoiceError> {
    voiceon_core::release::verify_file(&sums, &signature, &asset, std::path::Path::new(&path)).map_err(VoiceError::from)
}
