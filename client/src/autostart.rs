//! Запуск вместе с системой (спрятанным в трей, `--tray`).
//!
//! - Windows: значение `voiceOn` в `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
//! - Linux: `~/.config/autostart/voiceon.desktop`.

/// Включает или выключает автозапуск. Включение заново прописывает путь к exe.
pub fn set(enabled: bool) -> Result<(), String> {
    imp::set(enabled)
}

#[cfg(target_os = "linux")]
mod imp {
    use std::path::PathBuf;

    fn entry_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("autostart").join("voiceon.desktop"))
    }

    pub fn set(enabled: bool) -> Result<(), String> {
        let path = entry_path().ok_or("не найден ~/.config")?;
        if !enabled {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", path.display())),
                _ => Ok(()),
            };
        }
        let exe = crate::install::launch_exe().ok_or("не знаю путь к себе")?;
        let mut text = crate::install::desktop_entry(&exe, "--tray");
        text.push_str("X-GNOME-Autostart-enabled=true\n");
        if std::fs::read_to_string(&path).is_ok_and(|t| t == text) {
            return Ok(());
        }
        crate::install::write_file(&path, text.as_bytes())
    }
}

#[cfg(windows)]
mod imp {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE};

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const NAME: &str = "voiceOn";

    pub fn set(enabled: bool) -> Result<(), String> {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey_with_flags(RUN, KEY_SET_VALUE | KEY_QUERY_VALUE)
            .map_err(|e| format!("реестр: {e}"))?;
        if !enabled {
            return match key.delete_value(NAME) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("реестр: {e}")),
                _ => Ok(()),
            };
        }
        let exe = crate::install::launch_exe().ok_or("не знаю путь к себе")?;
        let cmd = format!("\"{}\" --tray", exe.display());
        if key.get_value::<String, _>(NAME).is_ok_and(|v| v == cmd) {
            return Ok(());
        }
        key.set_value(NAME, &cmd).map_err(|e| format!("реестр: {e}"))
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    pub fn set(_enabled: bool) -> Result<(), String> {
        Err("автозапуск не поддерживается на этой системе".into())
    }
}
