//! Журнал ядра. Куда писать, решает приложение: десктоп — в файл рядом с
//! конфигом, Android — в logcat. Пока приёмник не задан, сообщения теряются.

use std::sync::OnceLock;

type Sink = Box<dyn Fn(&str) + Send + Sync>;

static SINK: OnceLock<Sink> = OnceLock::new();

/// Задаёт приёмник журнала; повторные вызовы игнорируются.
pub fn set_sink(sink: impl Fn(&str) + Send + Sync + 'static) {
    let _ = SINK.set(Box::new(sink));
}

pub fn info(msg: &str) {
    if let Some(sink) = SINK.get() {
        sink(msg);
    }
}
