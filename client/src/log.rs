//! Простой журнал в файл рядом с конфигом: `voiceon.log` (до ~1 МБ, потом
//! переименовывается в `voiceon.log.old`). Нужен, чтобы разбирать обрывы.

use std::io::Write;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static LOCK: Mutex<()> = Mutex::new(());
const MAX_SIZE: u64 = 1024 * 1024;

pub fn path() -> Option<std::path::PathBuf> {
    crate::config::path().map(|p| p.with_file_name("voiceon.log"))
}

pub fn info(msg: &str) {
    let Some(p) = path() else { return };
    let _g = LOCK.lock();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&p).map(|m| m.len() > MAX_SIZE).unwrap_or(false) {
        let _ = std::fs::rename(&p, p.with_extension("log.old"));
    }
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // Время в UTC без зависимостей: ЧЧ:ММ:СС и день от эпохи достаточно для разбора.
    let (h, m, s) = ((ts / 3600) % 24, (ts / 60) % 60, ts % 60);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "[d{} {h:02}:{m:02}:{s:02}Z] {msg}", ts / 86400);
    }
    #[cfg(debug_assertions)]
    eprintln!("{msg}");
}
