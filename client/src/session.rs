//! Один звонок: сеть + звук, собранные вместе.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;

use voiceon_proto::{Crypto, truncate_nick};

use crate::audio::{self, AudioIo};
use crate::capture::TxPipeline;
use crate::config::Config;
use crate::net::{self, Link, Repaint, Shared};

pub struct Session {
    pub shared: Arc<Shared>,
    _link: Arc<Link>,
    net: Option<JoinHandle<()>>,
    audio: Option<AudioIo>,
}

impl Session {
    pub fn connect(cfg: &Config, repaint: Repaint) -> Result<Session, String> {
        let nick = truncate_nick(&cfg.nick).to_string();
        if nick.is_empty() {
            return Err("укажи ник".into());
        }
        if cfg.passphrase.is_empty() {
            return Err("укажи пароль комнаты".into());
        }
        let addr = net::resolve(&cfg.server)?;
        let crypto = Crypto::from_passphrase(&cfg.passphrase);
        let link = Arc::new(Link::new(crypto, addr).map_err(|e| format!("сокет: {e}"))?);
        let shared = Shared::new(cfg.volumes.clone(), cfg.vad_threshold, repaint);
        shared.controls.denoise.store(cfg.denoise, Ordering::Relaxed);
        shared.pad.set_gain(cfg.soundpad.volume);
        let tx = TxPipeline::new(link.clone(), shared.clone(), cfg.bitrate, cfg.fec_loss_perc, false)?;
        let audio = audio::start(
            shared.clone(),
            tx,
            cfg.input_device.as_deref(),
            cfg.output_device.as_deref(),
        )?;
        let net = net::spawn(link.clone(), shared.clone(), nick);
        Ok(Session { shared, _link: link, net: Some(net), audio: Some(audio) })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        drop(self.audio.take());
        if let Some(t) = self.net.take() {
            let _ = t.join(); // сетевой поток сам шлёт Bye на выходе
        }
    }
}
