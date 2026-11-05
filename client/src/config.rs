//! Конфиг клиента: `~/.config/voiceon/config.toml` или `%APPDATA%\voiceon\config.toml`.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `host:port` сервера.
    pub server: String,
    pub nick: String,
    /// Парольная фраза комнаты (хранится открытым текстом).
    pub passphrase: String,
    /// Имя устройства захвата; `None` — системное по умолчанию.
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    /// Порог вероятности речи для VAD (0..1).
    pub vad_threshold: f32,
    /// Шумодав RNNoise включён.
    pub denoise: bool,
    /// Громкость каждого участника по нику, 1.0 = 100%.
    pub volumes: HashMap<String, f32>,
    pub bitrate: i32,
    pub fec_loss_perc: i32,
    /// Запускать вместе с системой (спрятанным в трей).
    pub autostart: bool,
    /// Модуль «Саундпад».
    pub soundpad: SoundpadConfig,
}

/// Настройки саундпада: модуль выключен по умолчанию.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundpadConfig {
    pub enabled: bool,
    /// Громкость звуков, 1.0 = 100%.
    pub volume: f32,
    /// Привязка слотов к файлам `sounds/slot_{slot}.wav`.
    pub slots: Vec<SlotConfig>,
}

impl Default for SoundpadConfig {
    fn default() -> SoundpadConfig {
        SoundpadConfig { enabled: false, volume: 1.0, slots: Vec::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotConfig {
    pub slot: usize,
    pub name: String,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            server: String::new(),
            nick: std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default(),
            passphrase: String::new(),
            input_device: None,
            output_device: None,
            vad_threshold: 0.6,
            denoise: true,
            volumes: HashMap::new(),
            bitrate: 32_000,
            fec_loss_perc: 10,
            autostart: false,
            soundpad: SoundpadConfig::default(),
        }
    }
}

pub fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("voiceon").join("config.toml"))
}

impl Config {
    pub fn load() -> Config {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let p = path().ok_or("не найден каталог конфигов")?;
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&p, text).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_config_without_soundpad_parses_with_defaults() {
        let cfg: Config = toml::from_str(r#"server = "x""#).unwrap();
        assert!(!cfg.soundpad.enabled);
        assert_eq!(cfg.soundpad.volume, 1.0);
        assert!(cfg.soundpad.slots.is_empty());
    }

    #[test]
    fn soundpad_roundtrip() {
        let mut cfg = Config::default();
        cfg.soundpad.enabled = true;
        cfg.soundpad.volume = 0.7;
        cfg.soundpad.slots = vec![
            SlotConfig { slot: 0, name: "бум".into() },
            SlotConfig { slot: 3, name: "вжух".into() },
        ];
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert!(back.soundpad.enabled);
        assert_eq!(back.soundpad.volume, 0.7);
        assert_eq!(back.soundpad.slots.len(), 2);
        assert_eq!(back.soundpad.slots[1].slot, 3);
        assert_eq!(back.soundpad.slots[1].name, "вжух");
    }
}
