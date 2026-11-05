//! Конвейер передачи: 48 кГц моно → [AEC-заглушка] → RNNoise → VAD → Opus → сеть.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use nnnoiseless::DenoiseState;
use voiceon_proto::{Kind, encode_voice};

use crate::dsp::{Passthrough, Processor, TxGate, VadGate, rms, soft_clip};
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
    /// Максимум вероятности речи за текущий 20-мс кадр.
    prob_acc: f32,
    packet: Vec<u8>,
    /// Буфер кадра саундпада: сюда миксуются инстансы перед отправкой.
    pad_buf: [f32; FRAME_SAMPLES],
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
            prob_acc: 0.0,
            packet: vec![0u8; 1275],
            pad_buf: [0.0; FRAME_SAMPLES],
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
            self.prob_acc = self.prob_acc.max(prob);
            // Шумодав выключен: RNNoise нужен только для VAD, шлём исходные сэмплы.
            if !self.shared.controls.denoise.load(Ordering::Relaxed) {
                for (o, i) in self.rnn_out.iter_mut().zip(&self.pending) {
                    *o = *i / 32768.0;
                }
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
        let mic_muted = self.shared.controls.mic_muted.load(Ordering::Relaxed);
        let deafened = self.shared.controls.deafened.load(Ordering::Relaxed);
        let muted = mic_muted || deafened;
        let connected = self.shared.controls.connected.load(Ordering::Relaxed);
        let voice_send = self.frame_open && !muted && connected;
        // Саундпад идёт мимо мьюта микрофона, но не мимо дефена и дисконнекта.
        // При дефене/дисконнекте очередь дренируется молча, чтобы таймлайн не вставал.
        let pad_active = self.shared.pad.mix_tx(&mut self.pad_buf, deafened || !connected);
        let pad_send = pad_active && connected && !deafened;
        let send = voice_send || pad_send;

        if send {
            // Pre-roll: на открытии гейта шлём и предыдущий кадр, чтобы не съесть начало слова.
            if voice_send && !self.was_sending && !self.raw {
                let prev = std::mem::take(&mut self.prev_frame);
                self.encode_and_send(&prev, self.frame_no.wrapping_sub(1));
                self.prev_frame = prev;
            }
            let mut frame = std::mem::take(&mut self.frame);
            if !voice_send {
                frame.fill(0.0); // микрофон молчит/замьючен — шлём только саундпад
            }
            if pad_send {
                for (o, &p) in frame.iter_mut().zip(self.pad_buf.iter()) {
                    *o = soft_clip(*o + p);
                }
            }
            self.encode_and_send(&frame, self.frame_no);
            self.frame = frame;
        }
        self.pad_buf = [0.0; FRAME_SAMPLES];

        if self.level_n >= 5 || send != self.was_sending {
            let mut st = self.shared.state.lock().unwrap();
            let changed = st.transmitting != send;
            st.transmitting = send;
            st.mic_level = if muted { 0.0 } else { self.level_acc };
            st.voice_prob = self.prob_acc;
            drop(st);
            self.level_acc = 0.0;
            self.prob_acc = 0.0;
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
            self.shared.tx_packets.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{Link, Shared};
    use std::collections::HashMap;
    use std::net::UdpSocket;
    use std::sync::Arc;
    use std::time::Duration;
    use voiceon_proto::Crypto;

    /// Приёмный сокет + конвейер, шлющий в него через настоящий Link.
    fn loopback() -> (UdpSocket, Crypto, TxPipeline, Arc<Shared>) {
        let recv = UdpSocket::bind("127.0.0.1:0").unwrap();
        recv.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
        let link =
            Arc::new(Link::new(Crypto::from_passphrase("t"), recv.local_addr().unwrap()).unwrap());
        let shared = Shared::new(HashMap::new(), 0.6, Arc::new(|| {}));
        let tx = TxPipeline::new(link, shared.clone(), 32_000, 10, false).unwrap();
        (recv, Crypto::from_passphrase("t"), tx, shared)
    }

    fn recv_voice(recv: &UdpSocket, crypto: &Crypto) -> bool {
        let mut buf = [0u8; 2048];
        match recv.recv(&mut buf) {
            Ok(n) => crypto.open(&buf[..n]).is_some_and(|(h, _)| matches!(h.kind, Kind::Voice)),
            Err(_) => false,
        }
    }

    /// Тишина в микрофон: гейт закрыт, голос не отправляется.
    fn push_silence(tx: &mut TxPipeline) {
        tx.push(&[0.0; 960]);
        tx.push(&[0.0; 960]);
    }

    #[test]
    fn pad_forces_send_when_gate_closed() {
        let (recv, crypto, mut tx, shared) = loopback();
        shared.controls.connected.store(true, Ordering::Relaxed);
        shared.pad.play(0, Arc::new(vec![0.5; 960]));
        push_silence(&mut tx);
        assert!(recv_voice(&recv, &crypto));
    }

    #[test]
    fn mic_mute_does_not_block_pad() {
        let (recv, crypto, mut tx, shared) = loopback();
        shared.controls.connected.store(true, Ordering::Relaxed);
        shared.controls.mic_muted.store(true, Ordering::Relaxed);
        shared.pad.play(0, Arc::new(vec![0.5; 960]));
        push_silence(&mut tx);
        assert!(recv_voice(&recv, &crypto));
    }

    #[test]
    fn deafen_blocks_pad() {
        let (recv, crypto, mut tx, shared) = loopback();
        shared.controls.connected.store(true, Ordering::Relaxed);
        shared.controls.deafened.store(true, Ordering::Relaxed);
        shared.pad.play(0, Arc::new(vec![0.5; 960]));
        push_silence(&mut tx);
        assert!(!recv_voice(&recv, &crypto));
    }

    #[test]
    fn disconnected_blocks_pad() {
        let (recv, crypto, mut tx, shared) = loopback();
        shared.pad.play(0, Arc::new(vec![0.5; 960]));
        push_silence(&mut tx);
        assert!(!recv_voice(&recv, &crypto));
    }
}
