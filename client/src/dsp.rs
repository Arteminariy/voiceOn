//! Мелкая обработка сигнала: ресемплер, гейт передачи, лимитер, заглушка AEC.

/// Решает, передаём ли мы голос в этом 10-мс кадре.
/// Сейчас одна реализация — [`VadGate`]; push-to-talk встанет рядом.
pub trait TxGate: Send {
    /// `voice_prob` — вероятность речи от RNNoise (0..1) для 10-мс кадра.
    fn update(&mut self, voice_prob: f32) -> bool;
    fn set_threshold(&mut self, _threshold: f32) {}
}

/// Голосовая активация: порог по вероятности речи + удержание после фразы.
pub struct VadGate {
    threshold: f32,
    hangover_frames: u32,
    remaining: u32,
}

impl VadGate {
    /// `hangover_ms` — сколько держать гейт открытым после конца речи.
    pub fn new(threshold: f32, hangover_ms: u32) -> VadGate {
        VadGate { threshold, hangover_frames: hangover_ms / 10, remaining: 0 }
    }
}

impl TxGate for VadGate {
    fn update(&mut self, voice_prob: f32) -> bool {
        if voice_prob >= self.threshold {
            self.remaining = self.hangover_frames;
            true
        } else if self.remaining > 0 {
            self.remaining -= 1;
            true
        } else {
            false
        }
    }

    fn set_threshold(&mut self, threshold: f32) {
        self.threshold = threshold;
    }
}

/// Ступень обработки захваченного сигнала до шумодава.
/// Сюда при необходимости встанет эхоподавление (AEC).
pub trait Processor: Send {
    fn process(&mut self, frame: &mut [f32]);
}

pub struct Passthrough;

impl Processor for Passthrough {
    fn process(&mut self, _frame: &mut [f32]) {}
}

/// Потоковый линейный ресемплер (моно). Для голоса 44.1↔48 кГц достаточно.
pub struct Resampler {
    step: f64,
    pos: f64,
    last: f32,
}

impl Resampler {
    pub fn new(from_rate: u32, to_rate: u32) -> Resampler {
        Resampler { step: from_rate as f64 / to_rate as f64, pos: 0.0, last: 0.0 }
    }

    pub fn is_identity(&self) -> bool {
        self.step == 1.0
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if input.is_empty() {
            return;
        }
        if self.is_identity() {
            out.extend_from_slice(input);
            return;
        }
        let len = input.len() as f64;
        // pos — дробный индекс в input; -1 означает последний сэмпл прошлого блока.
        while self.pos < len - 1.0 {
            let i = self.pos.floor();
            let frac = (self.pos - i) as f32;
            let i = i as isize;
            let a = if i < 0 { self.last } else { input[i as usize] };
            let b = input[(i + 1) as usize];
            out.push(a + (b - a) * frac);
            self.pos += self.step;
        }
        self.pos -= len;
        self.last = input[input.len() - 1];
    }
}

/// Мягкий лимитер: линейный до 0.8, дальше плавно прижимает к 1.0.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    const KNEE: f32 = 0.8;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        let y = KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh();
        y.copysign(x)
    }
}

pub fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vad_gate_hangover() {
        let mut g = VadGate::new(0.5, 30);
        assert!(!g.update(0.1));
        assert!(g.update(0.9));
        assert!(g.update(0.0));
        assert!(g.update(0.0));
        assert!(g.update(0.0));
        assert!(!g.update(0.0));
    }

    #[test]
    fn resampler_rate_and_continuity() {
        let mut r = Resampler::new(44_100, 48_000);
        let mut out = Vec::new();
        // 1 секунда синуса 440 Гц блоками по 441.
        let sig: Vec<f32> =
            (0..44_100).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 44_100.0).sin()).collect();
        for chunk in sig.chunks(441) {
            r.process(chunk, &mut out);
        }
        assert!((out.len() as i64 - 48_000).abs() <= 2, "{}", out.len());
        // Без разрывов: соседние сэмплы синуса 440 Гц на 48 кГц отличаются < 0.06.
        let max_step = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        assert!(max_step < 0.06, "{max_step}");
    }

    #[test]
    fn resampler_identity() {
        let mut r = Resampler::new(48_000, 48_000);
        let mut out = Vec::new();
        r.process(&[1.0, 2.0], &mut out);
        assert_eq!(out, vec![1.0, 2.0]);
    }

    #[test]
    fn soft_clip_bounds() {
        assert_eq!(soft_clip(0.5), 0.5);
        assert!(soft_clip(3.0) <= 1.0 && soft_clip(3.0) > 0.95);
        assert!(soft_clip(-3.0) >= -1.0);
        assert!((soft_clip(0.81) - 0.81).abs() < 0.01);
    }
}
