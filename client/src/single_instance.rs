//! Одна копия приложения на пользователя: первая занимает локальный порт,
//! вторая стучится в него («покажи окно») и выходит. Без зависимостей и
//! одинаково на Windows и Linux.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use eframe::egui;

const PORT: u16 = 47823;
const HELLO: &[u8] = b"voiceon-show\n";

static LISTENER: Mutex<Option<TcpListener>> = Mutex::new(None);
static SHOW: AtomicBool = AtomicBool::new(false);

pub enum Acquire {
    /// Мы первая копия (или проверить нельзя — тогда тоже работаем).
    First,
    /// Уже запущена другая копия, ей отправлена просьба показать окно.
    AlreadyRunning,
}

fn addr() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, PORT))
}

pub fn acquire() -> Acquire {
    // После автообновления или самоустановки старая копия ещё выходит —
    // даём ей несколько секунд освободить порт.
    let handover = std::env::var_os("VOICEON_UPDATED_TO").is_some()
        || std::env::var_os("VOICEON_INSTALLED").is_some();
    let attempts = if handover { 50 } else { 1 };
    for i in 0..attempts {
        match TcpListener::bind(addr()) {
            Ok(l) => {
                *LISTENER.lock().unwrap() = Some(l);
                return Acquire::First;
            }
            Err(_) if i + 1 < attempts => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => {}
        }
    }
    match TcpStream::connect_timeout(&addr(), Duration::from_millis(500)) {
        Ok(mut s) => {
            let _ = s.write_all(HELLO);
            crate::log::info("уже запущена другая копия — попросил её показать окно");
            Acquire::AlreadyRunning
        }
        // Порт занят чем-то посторонним: защиты нет, но работать не мешаем.
        Err(_) => Acquire::First,
    }
}

/// Слушает просьбы других копий; вызывается, когда есть контекст egui.
pub fn serve(ctx: &egui::Context) {
    let Some(listener) = LISTENER.lock().unwrap().take() else { return };
    let ctx = ctx.clone();
    let _ = std::thread::Builder::new().name("voiceon-instance".into()).spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
            let mut buf = [0u8; 32];
            let n = s.read(&mut buf).unwrap_or(0);
            if &buf[..n] == HELLO {
                SHOW.store(true, Ordering::Relaxed);
                ctx.request_repaint();
            }
        }
    });
}

/// `true`, если другая копия просила показать окно (сбрасывает флаг).
pub fn take_show_request() -> bool {
    SHOW.swap(false, Ordering::Relaxed)
}
