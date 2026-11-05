//! Саундпад: инстансы звуков для подмешивания в передачу и локальный мониторинг.
//!
//! Одно нажатие = новый инстанс (как в Discord), инстансы наложением микшуются.
//! Очереди эфира и мониторинга независимы: часы захвата и воспроизведения разные,
//! и каждая тянет сэмплы в своём темпе.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::dsp::soft_clip;

/// Сколько одновременных инстансов разрешено на слот; лишний вытесняет старейший.
pub const MAX_INSTANCES_PER_SLOT: usize = 4;

struct Instance {
    slot: usize,
    samples: Arc<Vec<f32>>,
    pos: usize,
}

pub struct Pad {
    /// Очередь для передающего тракта (`TxPipeline`).
    tx: Mutex<Vec<Instance>>,
    /// Очередь для локального мониторинга (поток воспроизведения).
    monitor: Mutex<Vec<Instance>>,
    /// Громкость звуков (f32 в битах), единая для эфира и мониторинга.
    gain: AtomicU32,
}

impl Default for Pad {
    fn default() -> Pad {
        Pad::new()
    }
}

impl Pad {
    pub fn new() -> Pad {
        Pad {
            tx: Mutex::new(Vec::new()),
            monitor: Mutex::new(Vec::new()),
            gain: AtomicU32::new(1.0f32.to_bits()),
        }
    }

    /// Добавить инстанс в обе очереди; если у слота уже MAX_INSTANCES_PER_SLOT —
    /// вытеснить старейший инстанс слота.
    pub fn play(&self, slot: usize, samples: Arc<Vec<f32>>) {
        for q in [&self.tx, &self.monitor] {
            let mut q = q.lock().unwrap();
            if q.iter().filter(|i| i.slot == slot).count() >= MAX_INSTANCES_PER_SLOT
                && let Some(p) = q.iter().position(|i| i.slot == slot)
            {
                q.remove(p);
            }
            q.push(Instance { slot, samples: samples.clone(), pos: 0 });
        }
    }

    pub fn stop_all(&self) {
        self.tx.lock().unwrap().clear();
        self.monitor.lock().unwrap().clear();
    }

    pub fn set_gain(&self, gain: f32) {
        self.gain.store(gain.to_bits(), Ordering::Relaxed);
    }

    pub fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }

    /// Есть ли активные инстансы слота (подсветка кнопки в UI).
    pub fn slot_active(&self, slot: usize) -> bool {
        [&self.tx, &self.monitor]
            .iter()
            .any(|q| q.lock().unwrap().iter().any(|i| i.slot == slot))
    }

    /// Играет ли хоть один инстанс (перерисовка UI, таймер простоя).
    pub fn any_active(&self) -> bool {
        !self.tx.lock().unwrap().is_empty() || !self.monitor.lock().unwrap().is_empty()
    }

    /// Подмешать до `out.len()` сэмплов в передачу (+=, gain, soft_clip в конце),
    /// продвинуть инстансы, выкинуть доигранные. `discard = true`: дренировать
    /// без подмешивания (дефен/нет сети — таймлайн не встаёт).
    /// Возвращает true, если были активные инстансы.
    pub fn mix_tx(&self, out: &mut [f32], discard: bool) -> bool {
        mix(&self.tx, self.gain(), out, discard)
    }

    /// То же для локального мониторинга.
    pub fn mix_monitor(&self, out: &mut [f32], discard: bool) {
        mix(&self.monitor, self.gain(), out, discard);
    }
}

fn mix(q: &Mutex<Vec<Instance>>, gain: f32, out: &mut [f32], discard: bool) -> bool {
    let mut q = q.lock().unwrap();
    if q.is_empty() {
        return false;
    }
    for inst in q.iter_mut() {
        let n = (inst.samples.len() - inst.pos).min(out.len());
        if !discard {
            for (o, &s) in out.iter_mut().zip(&inst.samples[inst.pos..inst.pos + n]) {
                *o += s * gain;
            }
        }
        inst.pos += n;
    }
    q.retain(|i| i.pos < i.samples.len());
    drop(q);
    if !discard {
        for s in out.iter_mut() {
            *s = soft_clip(*s);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn clip(value: f32, len: usize) -> Arc<Vec<f32>> {
        Arc::new(vec![value; len])
    }

    #[test]
    fn play_mixes_into_both_sinks() {
        let pad = Pad::new();
        pad.play(0, clip(0.5, 960));
        let mut a = [0.0; 960];
        let mut b = [0.0; 960];
        assert!(pad.mix_tx(&mut a, false));
        pad.mix_monitor(&mut b, false);
        assert!(a[0] > 0.0);
        assert_eq!(a[0], b[0]);
    }

    #[test]
    fn stealing_caps_instances_per_slot() {
        let pad = Pad::new();
        for k in 1..=5usize {
            pad.play(0, clip(0.1 * k as f32, 960));
        }
        let tx = pad.tx.lock().unwrap();
        assert_eq!(tx.len(), MAX_INSTANCES_PER_SLOT);
        // Самый старый (0.1) вытеснен, первым стоит второй клип.
        assert_eq!(tx[0].samples[0], 0.2);
    }

    #[test]
    fn gain_applied() {
        let pad = Pad::new();
        pad.set_gain(0.5);
        pad.play(0, clip(0.5, 960));
        let mut out = [0.0; 960];
        pad.mix_tx(&mut out, false);
        assert!((out[0] - 0.25).abs() < 0.01, "{}", out[0]);
    }

    #[test]
    fn stop_all_clears_and_deactivates() {
        let pad = Pad::new();
        pad.play(0, clip(0.5, 960));
        pad.stop_all();
        let mut out = [0.0; 960];
        assert!(!pad.mix_tx(&mut out, false));
        assert!(!pad.slot_active(0));
    }

    #[test]
    fn slot_active_toggles() {
        let pad = Pad::new();
        pad.play(2, clip(0.5, 100));
        assert!(pad.slot_active(2));
        let mut out = [0.0; 200];
        pad.mix_tx(&mut out, false);
        pad.mix_monitor(&mut out, false);
        assert!(!pad.slot_active(2));
    }

    #[test]
    fn discard_drains_without_mixing() {
        let pad = Pad::new();
        pad.play(0, clip(0.5, 960));
        let mut out = [0.0; 960];
        assert!(pad.mix_tx(&mut out, true));
        assert!(out.iter().all(|&s| s == 0.0));
        assert!(!pad.mix_tx(&mut out, false));
    }

    #[test]
    fn any_active_toggles() {
        let pad = Pad::new();
        assert!(!pad.any_active());
        pad.play(1, clip(0.5, 100));
        assert!(pad.any_active());
        let mut out = [0.0; 100];
        pad.mix_tx(&mut out, false);
        assert!(pad.any_active()); // мониторинг ещё играет
        pad.mix_monitor(&mut out, false);
        assert!(!pad.any_active());
    }

    #[test]
    fn gain_change_applies_live() {
        let pad = Pad::new();
        pad.play(0, clip(0.5, 1920));
        let mut out = [0.0; 960];
        pad.mix_tx(&mut out, false);
        assert!((out[0] - 0.5).abs() < 1e-6, "{}", out[0]);
        pad.set_gain(0.5);
        let mut out2 = [0.0; 960];
        pad.mix_tx(&mut out2, false);
        assert!((out2[0] - 0.25).abs() < 0.01, "{}", out2[0]);
    }
}
