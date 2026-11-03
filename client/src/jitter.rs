//! Адаптивный jitter buffer на одного участника.
//!
//! Пакеты кладёт сетевой поток (`push`), забирает поток воспроизведения
//! (`pop`) раз в 20 мс по часам звуковой карты. Целевая задержка 40–120 мс
//! подстраивается под наблюдаемый джиттер (оценка из RFC 3550).

use std::collections::BTreeMap;
use std::time::Instant;

pub const FRAME_MS: f32 = 20.0;
const MIN_TARGET_MS: f32 = 40.0;
const MAX_TARGET_MS: f32 = 120.0;
/// Сколько подряд пропусков на пустом буфере считаем концом фразы.
const END_OF_SPURT_MISSES: u32 = 3;
/// На сколько фреймов буфер может превысить цель, прежде чем мы начнём
/// выкидывать старые фреймы, чтобы не копить задержку.
const MAX_EXCESS_FRAMES: u32 = 3;
const MAX_BUFFERED: usize = 50;

#[derive(Debug, PartialEq, Eq)]
pub enum Frame {
    /// Обычный пакет.
    Packet(Vec<u8>),
    /// Пакет потерян, но следующий есть: восстанавливаем из его FEC.
    Fec(Vec<u8>),
    /// Пакет потерян, FEC нет: маскировка потерь декодером (PLC).
    Lost,
    /// Участник молчит.
    Silent,
}

pub struct JitterBuffer {
    packets: BTreeMap<u32, Vec<u8>>,
    playing: bool,
    /// Следующий фрейм к воспроизведению (валиден, если что-то уже играло).
    next: Option<u32>,
    misses: u32,
    jitter_ms: f32,
    prev_transit: Option<f32>,
    epoch: Instant,
    pub last_arrival: Instant,
}

impl JitterBuffer {
    pub fn new(now: Instant) -> JitterBuffer {
        JitterBuffer {
            packets: BTreeMap::new(),
            playing: false,
            next: None,
            misses: 0,
            jitter_ms: 0.0,
            prev_transit: None,
            epoch: now,
            last_arrival: now,
        }
    }

    pub fn target_frames(&self) -> u32 {
        let ms = (FRAME_MS + 3.0 * self.jitter_ms).clamp(MIN_TARGET_MS, MAX_TARGET_MS);
        (ms / FRAME_MS).ceil() as u32
    }

    pub fn jitter_ms(&self) -> f32 {
        self.jitter_ms
    }

    pub fn push(&mut self, frame_no: u32, data: Vec<u8>, now: Instant) {
        self.last_arrival = now;
        let arrival = now.duration_since(self.epoch).as_secs_f32() * 1000.0;
        let transit = arrival - frame_no as f32 * FRAME_MS;
        if let Some(prev) = self.prev_transit {
            let d = (transit - prev).abs();
            // Паузы VAD дают огромный скачок transit — это не джиттер.
            if d < 500.0 {
                self.jitter_ms += (d - self.jitter_ms) / 16.0;
            }
        }
        self.prev_transit = Some(transit);

        if let Some(next) = self.next {
            if frame_no < next && (self.playing || next - frame_no < 50) {
                return; // опоздал
            }
        }
        self.packets.insert(frame_no, data);
        while self.packets.len() > MAX_BUFFERED {
            self.packets.pop_first();
        }
    }

    fn span(&self, from: u32) -> u32 {
        match self.packets.last_key_value() {
            Some((&last, _)) if last >= from => last - from + 1,
            _ => 0,
        }
    }

    pub fn pop(&mut self) -> Frame {
        if !self.playing {
            let Some((&first, _)) = self.packets.first_key_value() else {
                return Frame::Silent;
            };
            if self.span(first) < self.target_frames() {
                return Frame::Silent;
            }
            self.playing = true;
            self.misses = 0;
            self.next = Some(first);
        }

        let mut next = self.next.expect("playing подразумевает next");

        // Длинная дыра впереди — это новая фраза после паузы, перезапускаемся.
        if let Some((&first, _)) = self.packets.first_key_value() {
            if first > next + 10 {
                self.playing = false;
                return self.pop();
            }
        }

        // Буфер разросся (дрейф часов, всплеск сети) — догоняем.
        let target = self.target_frames();
        while self.span(next) > target + MAX_EXCESS_FRAMES {
            self.packets.remove(&next);
            next += 1;
        }

        let frame = if let Some(data) = self.packets.remove(&next) {
            self.misses = 0;
            Frame::Packet(data)
        } else if let Some(data) = self.packets.get(&(next + 1)) {
            self.misses = 0;
            Frame::Fec(data.clone())
        } else if self.packets.is_empty() {
            self.misses += 1;
            if self.misses >= END_OF_SPURT_MISSES {
                self.playing = false;
                self.next = Some(next + 1);
                return Frame::Silent;
            }
            Frame::Lost
        } else {
            self.misses += 1;
            Frame::Lost
        };
        self.next = Some(next + 1);
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn pkt(n: u32) -> Vec<u8> {
        vec![n as u8]
    }

    /// Пакеты приходят ровно каждые 20 мс, начиная с frame `start`.
    fn feed(jb: &mut JitterBuffer, t0: Instant, start: u32, frames: std::ops::Range<u32>) {
        for n in frames {
            let at = t0 + Duration::from_millis(((n - start) * 20) as u64);
            jb.push(n, pkt(n), at);
        }
    }

    #[test]
    fn buffers_to_target_then_plays_in_order() {
        let t0 = Instant::now();
        let mut jb = JitterBuffer::new(t0);
        assert_eq!(jb.target_frames(), 2);
        jb.push(100, pkt(100), t0);
        assert_eq!(jb.pop(), Frame::Silent);
        jb.push(101, pkt(101), t0 + Duration::from_millis(20));
        assert_eq!(jb.pop(), Frame::Packet(pkt(100)));
        assert_eq!(jb.pop(), Frame::Packet(pkt(101)));
    }

    #[test]
    fn single_loss_uses_fec_from_next_packet() {
        let t0 = Instant::now();
        let mut jb = JitterBuffer::new(t0);
        feed(&mut jb, t0, 0, 0..2);
        feed(&mut jb, t0, 0, 3..5);
        assert_eq!(jb.pop(), Frame::Packet(pkt(0)));
        assert_eq!(jb.pop(), Frame::Packet(pkt(1)));
        assert_eq!(jb.pop(), Frame::Fec(pkt(3)));
        assert_eq!(jb.pop(), Frame::Packet(pkt(3)));
    }

    #[test]
    fn double_loss_uses_plc_then_fec() {
        let t0 = Instant::now();
        let mut jb = JitterBuffer::new(t0);
        feed(&mut jb, t0, 0, 0..2);
        jb.pop();
        jb.pop();
        feed(&mut jb, t0, 0, 4..6);
        assert_eq!(jb.pop(), Frame::Lost);
        assert_eq!(jb.pop(), Frame::Fec(pkt(4)));
        assert_eq!(jb.pop(), Frame::Packet(pkt(4)));
    }

    #[test]
    fn end_of_spurt_goes_silent_and_late_packets_dropped() {
        let t0 = Instant::now();
        let mut jb = JitterBuffer::new(t0);
        feed(&mut jb, t0, 0, 0..2);
        jb.pop();
        jb.pop();
        assert_eq!(jb.pop(), Frame::Lost);
        assert_eq!(jb.pop(), Frame::Lost);
        assert_eq!(jb.pop(), Frame::Silent);
        jb.push(1, pkt(1), t0);
        assert_eq!(jb.pop(), Frame::Silent);
    }

    #[test]
    fn new_spurt_after_pause_restarts_buffering() {
        let t0 = Instant::now();
        let mut jb = JitterBuffer::new(t0);
        feed(&mut jb, t0, 0, 0..3);
        for _ in 0..3 {
            jb.pop();
        }
        // Пауза 2 секунды: следующая фраза начинается с frame 103.
        let t1 = t0 + Duration::from_secs(2);
        feed(&mut jb, t1, 103, 103..105);
        assert_eq!(jb.pop(), Frame::Packet(pkt(103)));
        assert_eq!(jb.pop(), Frame::Packet(pkt(104)));
    }

    #[test]
    fn overfull_buffer_catches_up() {
        let t0 = Instant::now();
        let mut jb = JitterBuffer::new(t0);
        feed(&mut jb, t0, 0, 0..20);
        let first = jb.pop();
        // Цель 2 фрейма + запас 3: из 20 фреймов играть начнём не с нулевого.
        match first {
            Frame::Packet(p) => assert!(p[0] >= 15, "играет {:?}", p),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn jitter_raises_target() {
        let t0 = Instant::now();
        let mut jb = JitterBuffer::new(t0);
        for n in 0..200u32 {
            let wobble = if n % 2 == 0 { 0 } else { 40 };
            jb.push(n, pkt(n), t0 + Duration::from_millis((n * 20 + wobble) as u64));
        }
        assert!(jb.jitter_ms() > 20.0, "{}", jb.jitter_ms());
        assert!(jb.target_frames() >= 4);
    }
}
