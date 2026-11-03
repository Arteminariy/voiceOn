//! Самоустановка при первом запуске: копируем себя в постоянное место, создаём
//! ярлык в меню и перезапускаемся оттуда. Дальше всё запускается из установленной
//! копии, и автообновление подменяет именно её.
//!
//! - Windows: `%LOCALAPPDATA%\Programs\voiceOn\voiceon.exe` + ярлык в меню «Пуск».
//! - Linux: `~/.local/share/voiceon/voiceon`, иконка в `hicolor` и `voiceon.desktop`.
//!
//! Без прав администратора. Пропускается при `VOICEON_NO_INSTALL`, в режиме бота и
//! при запуске из `target/` (сборки cargo). Ошибка установки не мешает работе.

use std::path::{Path, PathBuf};

/// Метка версии внутри бинарника: по ней узнаём версию установленной копии, не запуская её
/// (старые версии не понимают `--version` и открыли бы окно).
const MARKER_PREFIX: &str = "VOICEON-BUILD-VERSION:";
#[used]
static BUILD_MARKER: &str = concat!("VOICEON-BUILD-VERSION:", env!("CARGO_PKG_VERSION"), ";");

pub enum Outcome {
    /// Установленная копия запущена — этому процессу пора выйти.
    Relaunched,
    /// Работаем дальше; `Some` — что пошло не так (для заметки в окне).
    Continue(Option<String>),
}

pub fn version() -> &'static str {
    &BUILD_MARKER[MARKER_PREFIX.len()..BUILD_MARKER.len() - 1]
}

/// Куда ставится бинарник.
pub fn installed_exe() -> Option<PathBuf> {
    #[cfg(windows)]
    return dirs::data_local_dir().map(|d| d.join("Programs").join("voiceOn").join("voiceon.exe"));
    #[cfg(not(windows))]
    return dirs::data_local_dir().map(|d| d.join("voiceon").join("voiceon"));
}

/// Путь для автозапуска: установленная копия, если она есть, иначе текущий файл.
pub fn launch_exe() -> Option<PathBuf> {
    installed_exe().filter(|p| p.is_file()).or_else(crate::update::exe_path)
}

pub fn run(args: &[String]) -> Outcome {
    match try_run(args) {
        Ok(true) => Outcome::Relaunched,
        Ok(false) => Outcome::Continue(None),
        Err(e) => {
            crate::log::info(&format!("установка: {e}"));
            Outcome::Continue(Some(e))
        }
    }
}

fn try_run(args: &[String]) -> Result<bool, String> {
    if std::env::var_os("VOICEON_NO_INSTALL").is_some_and(|v| !v.is_empty() && v != "0") {
        return Ok(false);
    }
    let Some(exe) = crate::update::exe_path() else { return Ok(false) };
    if exe.ancestors().any(|a| a.file_name().is_some_and(|n| n == "target")) {
        return Ok(false); // cargo run / сборка разработчика
    }
    let target = installed_exe().ok_or("не найден каталог для установки")?;
    if is_installed_copy(&exe, &target) {
        // Уже установленная копия: только чиним ярлык, если его удалили.
        if let Err(e) = launcher(&target, false) {
            crate::log::info(&format!("ярлык: {e}"));
        }
        return Ok(false);
    }

    copy_self(&exe, &target)?;
    if let Err(e) = launcher(&target, true) {
        crate::log::info(&format!("ярлык: {e}"));
    }
    crate::log::info(&format!("запуск установленной копии {}", target.display()));
    std::process::Command::new(&target)
        .args(args)
        .env("VOICEON_INSTALLED", "1")
        .spawn()
        .map_err(|e| format!("не удалось запустить {}: {e}", target.display()))?;
    Ok(true)
}

fn is_installed_copy(exe: &Path, target: &Path) -> bool {
    // Перезапущенная после установки копия: защита от петли, если пути не сравнились.
    if std::env::var_os("VOICEON_INSTALLED").is_some() {
        return true;
    }
    match (exe.canonicalize(), target.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Версия из метки внутри файла; `None` — метки нет (версии до 0.2.2) или файл не читается.
fn file_version(bytes: &[u8]) -> Option<(u64, u64, u64)> {
    let pre = MARKER_PREFIX.as_bytes();
    let mut i = 0;
    while let Some(off) = find(&bytes[i..], pre) {
        let start = i + off + pre.len();
        // Сама строка-префикс тоже лежит в бинарнике; подходит только «префикс + x.y.z;».
        let tail = &bytes[start..bytes.len().min(start + 32)];
        if let Some(end) = tail.iter().position(|&b| b == b';') {
            let v = &tail[..end];
            if !v.is_empty()
                && v.iter().all(|b| b.is_ascii_digit() || *b == b'.')
                && let Some(v) = std::str::from_utf8(v).ok().and_then(crate::update::parse_version)
            {
                return Some(v);
            }
        }
        i = start;
    }
    None
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    let first = needle[0];
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        let off = hay[i..hay.len() - needle.len() + 1].iter().position(|&b| b == first)?;
        i += off;
        if &hay[i..i + needle.len()] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn copy_self(exe: &Path, target: &Path) -> Result<(), String> {
    let me = std::fs::read(exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    if let Ok(old) = std::fs::read(target) {
        if old == me {
            return Ok(());
        }
        let ours = crate::update::parse_version(version());
        if let (Some(theirs), Some(ours)) = (file_version(&old), ours)
            && theirs > ours
        {
            // Автообновление уже поставило версию новее — не откатываем её.
            crate::log::info(&format!("установлена версия новее ({theirs:?}), не перезаписываю"));
            return Ok(());
        }
    }
    let dir = target.parent().ok_or("плохой путь установки")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = dir.join(".voiceon-install.tmp");
    std::fs::write(&tmp, &me).map_err(|e| format!("{}: {e}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    // Windows не даёт перезаписать запущенный exe, но даёт его переименовать.
    // Старую копию удалить нельзя, пока она запущена, — уберётся при следующей установке.
    #[cfg(windows)]
    let old = target.with_extension("old.exe");
    #[cfg(windows)]
    if target.exists() {
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::rename(target, &old);
    }
    std::fs::rename(&tmp, target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        #[cfg(windows)]
        if !target.exists() {
            let _ = std::fs::rename(&old, target);
        }
        format!("{}: {e}", target.display())
    })?;
    crate::log::info(&format!("установлено в {}", target.display()));
    Ok(())
}

// ---------- ярлык в меню ----------

#[cfg(target_os = "linux")]
fn launcher(exe: &Path, force: bool) -> Result<(), String> {
    let data = dirs::data_local_dir().ok_or("нет ~/.local/share")?;
    let icon = data.join("icons/hicolor/256x256/apps/voiceon.png");
    let desktop = data.join("applications/voiceon.desktop");
    let entry = desktop_entry(exe, "");
    let fresh = std::fs::read_to_string(&desktop).is_ok_and(|t| t == entry) && icon.is_file();
    if fresh && !force {
        return Ok(());
    }
    write_file(&icon, include_bytes!("../assets/icon.png"))?;
    write_file(&desktop, entry.as_bytes())?;
    refresh_menu(&data.join("applications"));
    Ok(())
}

/// Ярлык `.desktop`: и для меню, и для автозапуска (`args` = `--tray`).
#[cfg(target_os = "linux")]
pub fn desktop_entry(exe: &Path, args: &str) -> String {
    let path = exe.to_string_lossy();
    // Спецификация: путь с пробелами и спецсимволами — в двойных кавычках с экранированием.
    let exec = if path.chars().any(|c| c.is_whitespace() || "\"'\\$`".contains(c)) {
        let escaped: String = path.chars().flat_map(|c| {
            let esc = matches!(c, '"' | '`' | '$' | '\\');
            esc.then_some('\\').into_iter().chain(std::iter::once(c))
        }).collect();
        format!("\"{escaped}\"")
    } else {
        path.into_owned()
    };
    let exec = exec.replace('%', "%%");
    let exec = if args.is_empty() { exec } else { format!("{exec} {args}") };
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=voiceOn\n\
         Comment=Голосовой чат\n\
         Exec={exec}\n\
         Icon=voiceon\n\
         Terminal=false\n\
         Categories=Network;AudioVideo;\n\
         StartupWMClass=voiceon\n\
         StartupNotify=true\n"
    )
}

#[cfg(target_os = "linux")]
fn refresh_menu(apps: &Path) {
    use std::process::{Command, Stdio};
    // Обновление кэшей меню — по желанию: если утилит нет или они упали, не важно.
    // Запускаем сразу (процесс после установки тут же выходит, а дети доработают сами),
    // ждём их в отдельном потоке, чтобы не копить зомби в работающей копии.
    let children: Vec<_> = [("update-desktop-database", Some(apps)), ("kbuildsycoca6", None)]
        .into_iter()
        .filter_map(|(cmd, arg)| {
            Command::new(cmd)
                .args(arg)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .ok()
        })
        .collect();
    let _ = std::thread::Builder::new().name("voiceon-menu".into()).spawn(move || {
        for mut c in children {
            let _ = c.wait();
        }
    });
}

#[cfg(windows)]
fn launcher(exe: &Path, force: bool) -> Result<(), String> {
    let lnk = dirs::data_dir()
        .ok_or("нет %APPDATA%")?
        .join(r"Microsoft\Windows\Start Menu\Programs\voiceOn.lnk");
    if lnk.is_file() && !force {
        return Ok(());
    }
    if let Some(dir) = lnk.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut link = mslnk::ShellLink::new(exe).map_err(|e| format!("ярлык: {e}"))?;
    link.set_name(Some("voiceOn".into()));
    link.set_icon_location(Some(exe.to_string_lossy().into_owned()));
    link.create_lnk(&lnk).map_err(|e| format!("{}: {e}", lnk.display()))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn launcher(_exe: &Path, _force: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "linux")]
pub(crate) fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_version_matches_package() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
        assert_eq!(file_version(BUILD_MARKER.as_bytes()), crate::update::parse_version(version()));
    }

    #[test]
    fn marker_scan_skips_bare_prefix() {
        let mut blob = b"junk VOICEON-BUILD-VERSION:\x00\x01 more ".to_vec();
        blob.extend_from_slice(b"VOICEON-BUILD-VERSION:VOICEON-BUILD-VERSION:1.2.3; tail");
        assert_eq!(file_version(&blob), Some((1, 2, 3)));
        assert_eq!(file_version(b"no marker here"), None);
        assert_eq!(file_version(b"VOICEON-BUILD-VERSION:"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn desktop_entry_quotes_paths() {
        let e = desktop_entry(Path::new("/home/u/.local/share/voiceon/voiceon"), "--tray");
        assert!(e.contains("Exec=/home/u/.local/share/voiceon/voiceon --tray\n"));
        let e = desktop_entry(Path::new("/home/my user/v\"o"), "");
        assert!(e.contains("Exec=\"/home/my user/v\\\"o\"\n"));
        assert!(e.contains("StartupWMClass=voiceon\n"));
    }
}
