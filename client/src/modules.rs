//! Встроенные опциональные модули клиента.
//!
//! Модуль выключен по умолчанию и включается пользователем в настройках;
//! выключенный модуль не рисуется в UI и не трогает аудиотракт.
//! Первый такой модуль — саундпад (`crate::soundpad`).

use crate::config::Config;

pub trait Module {
    /// Стабильный id для логов и конфига.
    fn id(&self) -> &'static str;
    /// Включён ли модуль по конфигу.
    fn enabled(cfg: &Config) -> bool
    where
        Self: Sized;
    /// Включить/выключить модуль в конфиге.
    fn set_enabled(cfg: &mut Config, on: bool)
    where
        Self: Sized;
}
