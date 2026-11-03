//! Конвейер передачи: 48 кГц моно → [AEC-заглушка] → RNNoise → VAD → Opus → сеть.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use nnnoiseless::DenoiseState;
use voiceon_proto::{Kind, encode_voice};

use crate::dsp::{Passthrough, Processor, TxGate, VadGate, rms};
use crate::mixer::{FRAME_SAMPLES, SAMPLE_RATE};
use crate::net::{Link, Shared};

const RNN_FRAME: usize = DenoiseState::FRAME_SIZE; // 480 = 10 мс
const HANGOVER_MS: u32 = 300;

pub struct TxPipeline {
    link: Arc<Link>,
    shared: Arc<Shared>,
    pre: Box<dyn Processor>,
    denoise: Box<DenoiseState<'static>>,
    gate: Box<dyn TxGate>,
    encoder: opus::Encoder,
    /// Накопитель входа до 10-мс кадра RNNoise.
    pending: Vec<f32>,
    rnn_out: [f32; RNN_FRAME],
    /// Текущий 20-мс кадр (после шумодава) и предыдущий — для pre-roll.
    frame: Vec<f32>,
    prev_frame: Vec<f32>,
    frame_open: bool,
    was_sending: bool,
    frame_no: u32,
    level_acc: f32,
    level_n: u32,
    packet: Vec<u8>,
    /// Без шумодава и VAD — для бота-тестера.
    raw: bool,
}

impl TxPipeline {
    pub fn new(
        link: Arc<Link>,
        shared: Arc<Shared>,
        bitrate: i32,
        fec_loss_perc: i32,
        raw: bool,
    ) -> Result<TxPipeline, String> {
        let mut encoder =
            opus::Encoder::new(SAMPLE_RATE, opus::Channels::Mono, opus::Application::Voip)
                .map_err(|e| format!("opus: {e}"))?;
        let _ = encoder.set_bitrate(opus::Bitrate::Bits(bitrate.clamp(8_000, 128_000)));
        let _ = encoder.set_vbr(true);
        let _ = encoder.set_inband_fec(true);
        let _ = encoder.set_packet_loss_perc(fec_loss_perc.clamp(0, 100));
        let _ = encoder.set_signal(opus::Signal::Voice);
        let threshold = shared.controls.vad_threshold();
        Ok(TxPipeline {
            link,
            shared,
            pre: Box::new(Passthrough),
            denoise: DenoiseState::new(),
            gate: Box::new(VadGate::new(threshold, HANGOVER_MS)),
            encoder,
            pending: Vec::with_capacity(RNN_FRAME * 2),
            rnn_out: [0.0; RNN_FRAME],
            frame: Vec::with_capacity(FRAME_SAMPLES),
            prev_frame: vec![0.0; FRAME_SAMPLES],
            frame_open: false,
            was_sending: false,
            frame_no: 0,
            level_acc: 0.0,
            level_n: 0,
            packet: vec![0u8; 1275],
            raw,
        })
    }

    /// Принимает моно-сэмплы 48 кГц в диапазоне ±1.0 любыми порциями.
    pub fn push(&mut self, samples: &[f32]) {
        for &s in samples {
            self.pending.push(s);
            if self.pending.len() == RNN_FRAME {
                self.process_rnn_frame();
                self.pending.clear();
            }
        }
    }

    fn process_rnn_frame(&mut self) {
        let open = if self.raw {
            self.rnn_out.copy_from_slice(&self.pending);
            true
        } else {
            self.pre.process(&mut self.pending);
            // RNNoise ждёт сэмплы в масштабе i16.
            for s in self.pending.iter_mut() {
                *s *= 32768.0;
            }
            let prob = self.denoise.process_frame(&mut self.rnn_out, &self.pending);
            for s in self.rnn_out.iter_mut() {
                *s /= 32768.0;
            }
            self.gate.set_threshold(self.shared.controls.vad_threshold());
            self.gate.update(prob)
        };
        self.frame_open |= open;
        self.frame.extend_from_slice(&self.rnn_out);
        self.level_acc = self.level_acc.max(rms(&self.rnn_out));
        self.level_n += 1;

        if self.frame.len() >= FRAME_SAMPLES {
            self.finish_frame();
        }
    }

    fn finish_frame(&mut self) {
        self.frame_no = self.frame_no.wrapping_add(1);
        let muted = self.shared.controls.mic_muted.load(Ordering::Relaxed)
            || self.shared.controls.deafened.load(Ordering::Relaxed);
        let send = self.frame_open && !muted;

        if send {
            // Pre-roll: на открытии гейта шлём и предыдущий кадр, чтобы не съесть начало слова.
            if !self.was_sending && !self.raw {
                let prev = std::mem::take(&mut self.prev_frame);
                self.encode_and_send(&prev, self.frame_no.wrapping_sub(1));
                self.prev_frame = prev;
            }
            let frame = std::mem::take(&mut self.frame);
            self.encode_and_send(&frame, self.frame_no);
            self.frame = frame;
        }

        if self.level_n >= 5 || send != self.was_sending {
            let mut st = self.shared.state.lock().unwrap();
            let changed = st.transmitting != send;
            st.transmitting = send;
            st.mic_level = if muted { 0.0 } else { self.level_acc };
            drop(st);
            self.level_acc = 0.0;
            self.level_n = 0;
            if changed {
                (self.shared.repaint)();
            }
        }

        self.was_sending = send;
        self.frame_open = false;
        std::mem::swap(&mut self.prev_frame, &mut self.frame);
        self.frame.clear();
    }

    fn encode_and_send(&mut self, frame: &[f32], frame_no: u32) {
        if frame.len() != FRAME_SAMPLES {
            return;
        }
        if let Ok(n) = self.encoder.encode_float(frame, &mut self.packet) {
            let payload = encode_voice(frame_no, &self.packet[..n]);
            self.link.send(Kind::Voice, &payload);
        }
    }
}
