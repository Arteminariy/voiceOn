//! Ядро клиента voiceOn без звуковых устройств и окна.
//!
//! Звук в ядро приходит и уходит кадрами 48 кГц моно: захват толкает сэмплы в
//! [`capture::TxPipeline`], воспроизведение по своим часам забирает 20-мс кадры
//! из [`mixer::Mixer`]. Сеть живёт в отдельном потоке ([`net::spawn`]).

pub mod capture;
pub mod dsp;
pub mod jitter;
pub mod log;
pub mod mixer;
pub mod net;
pub mod release;
