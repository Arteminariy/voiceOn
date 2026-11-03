//! Автообновление из релизов GitHub.
//!
//! В фоне при запуске: узнаём последний релиз, скачиваем `SHA256SUMS` и его
//! подпись ed25519, проверяем подпись встроенным публичным ключом, качаем
//! бинарник своей платформы, сверяем sha256 и подменяем им себя. Пока идёт
//! звонок, ничего не перезапускаем: UI перезапустит приложение, когда звонка нет.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) use voiceon_core::release::parse_version;
use voiceon_core::release::{REPO, check_hash, verified_hash};

const MAX_BINARY: u64 = 64 * 1024 * 1024;

#[cfg(windows)]
const ASSET: &str = "voiceon-windows-x86_64.exe";
#[cfg(not(windows))]
const ASSET: &str = "voiceon-linux-x86_64";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Checking,
    UpToDate,
    Downloading(String),
    /// Новая версия уже подменила бинарник; нужен перезапуск.
    Installed(String),
    Failed(String),
}

pub type Shared = Arc<Mutex<Status>>;

/// Путь к себе, запомненный до подмены: после неё на Linux `current_exe()`
/// указывает на удалённый файл (`… (deleted)`).
static EXE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();

pub(crate) fn exe_path() -> Option<PathBuf> {
    EXE.get_or_init(|| std::env::current_exe().ok()).clone()
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Запускает проверку в фоне. `on_change` вызывается при смене статуса (перерисовка UI).
pub fn spawn(on_change: impl Fn() + Send + 'static) -> Shared {
    exe_path();
    let status: Shared = Arc::new(Mutex::new(Status::Checking));
    let st = status.clone();
    let _ = std::thread::Builder::new().name("voiceon-update".into()).spawn(move || {
        let set = |s: Status| {
            crate::log::info(&format!("обновление: {s:?}"));
            *st.lock().unwrap() = s;
            on_change();
        };
        if std::env::var_os("VOICEON_NO_UPDATE").is_some() {
            set(Status::UpToDate);
            return;
        }
        match check_and_install(&set) {
            Ok(None) => set(Status::UpToDate),
            Ok(Some(v)) => set(Status::Installed(v)),
            Err(e) => set(Status::Failed(e)),
        }
    });
    status
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .user_agent(format!("voiceon/{}", current_version()))
        .build()
        .new_agent()
}

fn get(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut resp = agent.get(url).call().map_err(|e| format!("{url}: {e}"))?;
    resp.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| format!("{url}: {e}"))
}

fn check_and_install(set: &dyn Fn(Status)) -> Result<Option<String>, String> {
    let agent = agent();
    let api = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body = get(&agent, &api, 1024 * 1024)?;
    let json: serde_json::Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    let tag = json["tag_name"].as_str().ok_or("в ответе GitHub нет tag_name")?.to_string();

    let (Some(latest), Some(current)) = (parse_version(&tag), parse_version(current_version()))
    else {
        return Err(format!("непонятная версия {tag}"));
    };
    if latest <= current {
        return Ok(None);
    }
    // Мы только что обновились до этого тега, а версия внутри всё ещё старая —
    // релиз собран неправильно; не уходим в цикл перезапусков.
    if std::env::var("VOICEON_UPDATED_TO").ok().as_deref() == Some(tag.as_str()) {
        return Err(format!("{tag} уже установлен, но версия внутри {}", current_version()));
    }
    set(Status::Downloading(tag.clone()));

    let base = format!("https://github.com/{REPO}/releases/download/{tag}");
    let sums = get(&agent, &format!("{base}/SHA256SUMS"), 64 * 1024)?;
    let sig = get(&agent, &format!("{base}/SHA256SUMS.sig"), 1024)?;
    let expected = verified_hash(&sums, &sig, ASSET)?;

    let bin = get(&agent, &format!("{base}/{ASSET}"), MAX_BINARY)?;
    check_hash(&bin, &expected)?;

    let exe = exe_path().ok_or("не знаю путь к себе")?;
    let tmp = staging_path(&exe);
    std::fs::write(&tmp, &bin).map_err(|e| format!("{}: {e}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    let res = self_replace::self_replace(&tmp).map_err(|e| format!("не удалось заменить себя: {e}"));
    let _ = std::fs::remove_file(&tmp);
    res?;
    Ok(Some(tag))
}

fn staging_path(exe: &std::path::Path) -> PathBuf {
    let dir = exe.parent().map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dir.join(".voiceon-update.tmp")
}

/// Перезапускает приложение с теми же аргументами (после установленного обновления).
/// `hidden` — окно сейчас спрятано в трей: новая копия тоже стартует спрятанной.
pub fn restart(tag: &str, hidden: bool) -> ! {
    crate::log::info(&format!("перезапуск после обновления до {tag}"));
    if let Some(exe) = exe_path() {
        let mut args: Vec<String> =
            std::env::args().skip(1).filter(|a| a != "--tray" && a != "--minimized").collect();
        if hidden {
            args.push("--tray".into());
        }
        let _ = std::process::Command::new(exe).args(args).env("VOICEON_UPDATED_TO", tag).spawn();
    }
    std::process::exit(0);
}
