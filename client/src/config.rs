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
