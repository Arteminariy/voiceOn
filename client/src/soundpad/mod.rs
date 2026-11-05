//! Модуль «Саундпад»: импорт, хранение и проигрывание звуков.

pub mod import;
pub mod preview;
pub mod ui;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use voiceon_core::net::Shared;

use crate::config::Config;
use crate::modules::Module;
use crate::soundpad::preview::Preview;

/// Слотов ровно столько — решение из спеки.
pub const SLOT_COUNT: usize = 5;

pub struct Slot {
    pub name: String,
    pub samples: Arc<Vec<f32>>,
}

/// Открытый диалог обрезки; логика — в ui (задача 9 плана).
pub struct TrimState {
    pub source: Vec<f32>,
    pub peaks: Vec<(f32, f32)>,
    pub name: String,
    pub start: f32,
    pub end: f32,
    pub target_slot: usize,
    pub busy: bool,
    pub error: Option<String>,
    /// Канал фонового декода: один ответ — (сэмплы 48 кГц моно, пики волны).
    pub rx: Option<std::sync::mpsc::Receiver<Result<(Vec<f32>, Vec<(f32, f32)>), String>>>,
}

pub struct Soundpad {
    pub slots: [Option<Slot>; SLOT_COUNT],
    pub drawer_open: bool,
    pub trim: Option<TrimState>,
    pub preview: Option<Preview>,
    /// Какой слот играет через предпрослушивание (подсветка на экране входа).
    pub preview_slot: Option<usize>,
}

impl Default for Soundpad {
    fn default() -> Soundpad {
        Soundpad {
            slots: std::array::from_fn(|_| None),
            drawer_open: false,
            trim: None,
            preview: None,
            preview_slot: None,
        }
    }
}

impl Soundpad {
    /// Читает слоты из конфига, грузит `slot_N.wav` из dir; битые и лишние пропускает.
    pub fn load(cfg: &Config, dir: &Path) -> Soundpad {
        let mut slots: [Option<Slot>; SLOT_COUNT] = std::array::from_fn(|_| None);
        for sc in &cfg.soundpad.slots {
            if sc.slot >= SLOT_COUNT {
                continue;
            }
            let path = dir.join(format!("slot_{}.wav", sc.slot));
            match import::load_wav(&path) {
                Ok(samples) if !samples.is_empty() => {
                    slots[sc.slot] = Some(Slot { name: sc.name.clone(), samples: Arc::new(samples) });
                }
                _ => {
                    crate::log::info(&format!("саундпад: слот {} пропущен ({})", sc.slot, path.display()));
                }
            }
        }
        Soundpad { slots, drawer_open: false, trim: None, preview: None, preview_slot: None }
    }

    pub fn remove(&mut self, cfg: &mut Config, dir: &Path, slot: usize) {
        if slot >= SLOT_COUNT {
            return;
        }
        self.slots[slot] = None;
        let _ = std::fs::remove_file(dir.join(format!("slot_{slot}.wav")));
        cfg.soundpad.slots.retain(|s| s.slot != slot);
        let _ = cfg.save();
    }

    /// Завершает диалог обрезки: режет source из `trim`, нормализует, пишет WAV, обновляет слот.
    pub fn import_into(&mut self, cfg: &mut Config, dir: &Path, slot: usize, name: String) -> Result<(), String> {
        if slot >= SLOT_COUNT {
            return Err("нет такого слота".into());
        }
        let Some(trim) = &self.trim else {
            return Err("нет открытого импорта".into());
        };
        let clip = import::prepare_clip(&trim.source, trim.start, trim.end);
        if clip.is_empty() {
            return Err("пустой кусок".into());
        }
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        import::save_wav(&clip, &dir.join(format!("slot_{slot}.wav")))?;
        self.slots[slot] = Some(Slot { name: name.clone(), samples: Arc::new(clip) });
        cfg.soundpad.slots.retain(|s| s.slot != slot);
        cfg.soundpad.slots.push(crate::config::SlotConfig { slot, name });
        cfg.save()
    }

    /// Эфир + локальный мониторинг (в звонке); при дефене и без слота — no-op.
    pub fn play(&self, shared: &Shared, slot: usize) {
        if shared.controls.deafened.load(Ordering::Relaxed) {
            return;
        }
        if let Some(s) = self.slots.get(slot).and_then(|s| s.as_ref()) {
            shared.pad.play(slot, s.samples.clone());
        }
    }

    /// Только локально (экран входа): играет через предпрослушивание.
    pub fn play_local(&mut self, cfg: &Config, slot: usize) {
        if let Some(s) = self.slots.get(slot).and_then(|s| s.as_ref()) {
            let len = s.samples.len();
            match Preview::play(cfg.output_device.as_deref(), s.samples.clone(), 0, len, cfg.soundpad.volume) {
                Ok(p) => {
                    self.preview = Some(p);
                    self.preview_slot = Some(slot);
                }
                Err(e) => crate::log::info(&format!("саундпад: {e}")),
            }
        }
    }

    /// Глушит всё: очереди ядра (в звонке) и локальное предпрослушивание.
    pub fn stop_all(&mut self, shared: Option<&Shared>) {
        if let Some(sh) = shared {
            sh.pad.stop_all();
        }
        self.preview = None;
    }
}

impl Module for Soundpad {
    fn id(&self) -> &'static str {
        "soundpad"
    }

    fn enabled(cfg: &Config) -> bool {
        cfg.soundpad.enabled
    }

    fn set_enabled(cfg: &mut Config, on: bool) {
        cfg.soundpad.enabled = on;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("voiceon-test-{}-{}", name, std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn load_skips_missing_and_garbage_slots() {
        let d = tmpdir("load");
        import::save_wav(&vec![0.1f32; 960], &d.join("slot_0.wav")).unwrap();
        std::fs::write(d.join("slot_1.wav"), b"musor").unwrap();
        let mut cfg = Config::default();
        cfg.soundpad.slots = vec![
            crate::config::SlotConfig { slot: 0, name: "а".into() },
            crate::config::SlotConfig { slot: 1, name: "б".into() },
            crate::config::SlotConfig { slot: 2, name: "нет файла".into() },
        ];
        let pad = Soundpad::load(&cfg, &d);
        assert!(pad.slots[0].is_some());
        assert_eq!(pad.slots[0].as_ref().unwrap().name, "а");
        assert!(pad.slots[1].is_none());
        assert!(pad.slots[2].is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn module_flag_roundtrip() {
        let mut cfg = Config::default();
        assert!(!Soundpad::enabled(&cfg));
        Soundpad::set_enabled(&mut cfg, true);
        assert!(Soundpad::enabled(&cfg));
        assert!(cfg.soundpad.enabled);
    }
}
