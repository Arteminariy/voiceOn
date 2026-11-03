//! Бот-тестер без звуковых устройств: шлёт тон и печатает, что слышит.
//!
//! `voiceon --bot --server HOST:PORT --pass ФРАЗА [--nick bot] [--tone 440] [--seconds 10]`

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use voiceon_proto::Crypto;

use crate::capture::TxPipeline;
use crate::dsp::rms;
use crate::mixer::{FRAME_SAMPLES, Mixer, SAMPLE_RATE};
use crate::net::{self, Link, Shared};

pub fn run(args: &[String]) -> Result<(), String> {
    let mut server = String::new();
    let mut pass = String::new();
    let mut nick = "bot".to_string();
    let mut tone = 440.0f32;
    let mut seconds = 10u64;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().ok_or(format!("после {a} нужно значение"));
        match a.as_str() {
            "--bot" => {}
            "--server" => server = val()?,
            "--pass" => pass = val()?,
            "--nick" => nick = val()?,
            "--tone" => tone = val()?.parse().map_err(|_| "--tone: число")?,
            "--seconds" => seconds = val()?.parse().map_err(|_| "--seconds: число")?,
            other => return Err(format!("неизвестный аргумент {other}")),
        }
    }
    let addr = net::resolve(&server)?;
    let link = Arc::new(
        Link::new(Crypto::from_passphrase(&pass), addr).map_err(|e| e.to_string())?,
    );
    let shared = Shared::new(Default::default(), 0.6, Arc::new(|| {}));
    let mut tx = TxPipeline::new(link.clone(), shared.clone(), 32_000, 10, true)?;
    let net_thread = net::spawn(link.clone(), shared.clone(), nick.clone());

    let mut mixer = Mixer::new();
    let mut out = [0.0f32; FRAME_SAMPLES];
    let mut phase = 0.0f32;
    let mut tone_buf = vec![0.0f32; FRAME_SAMPLES];
    let start = Instant::now();
    let mut next_tick = start;
    let mut last_report = start;
    let (mut heard_frames, mut total_frames) = (0u32, 0u32);
    let mut peak = 0.0f32;

    while start.elapsed() < Duration::from_secs(seconds) {
        // Тон: 1 с звучит, 1 с тишина — проверяем и паузы VAD/jitter buffer.
        let on = tone > 0.0 && (start.elapsed().as_secs() % 2 == 0);
        for s in tone_buf.iter_mut() {
            *s = if on { 0.3 * phase.sin() } else { 0.0 };
            phase = (phase + tone * std::f32::consts::TAU / SAMPLE_RATE as f32) % std::f32::consts::TAU;
        }
        if on {
            tx.push(&tone_buf);
        }

        mixer.mix(&shared, &mut out);
        let level = rms(&out);
        total_frames += 1;
        if level > 0.01 {
            heard_frames += 1;
        }
        peak = peak.max(level);

        if last_report.elapsed() >= Duration::from_secs(1) {
            last_report = Instant::now();
            let st = shared.state.lock().unwrap();
            let roster: Vec<&str> = st.roster.iter().map(|e| e.nick.as_str()).collect();
            let jitter: Vec<String> = shared
                .jitter
                .lock()
                .unwrap()
                .values()
                .map(|j| format!("{:.1}мс/цель {}", j.jitter_ms(), j.target_frames()))
                .collect();
            println!(
                "[{nick}] статус={:?} rtt={} потери={:.0}% участники={:?} слышно {}/{} кадров, пик {:.2}, джиттер {:?}",
                st.status,
                st.rtt_ms.map(|r| format!("{r:.1}мс")).unwrap_or("-".into()),
                st.loss_pct,
                roster,
                heard_frames,
                total_frames,
                peak,
                jitter
            );
            heard_frames = 0;
            total_frames = 0;
            peak = 0.0;
        }

        next_tick += Duration::from_millis(20);
        if let Some(d) = next_tick.checked_duration_since(Instant::now()) {
            std::thread::sleep(d);
        }
    }
    shared.stop.store(true, Ordering::Relaxed);
    let _ = net_thread.join();
    Ok(())
}
