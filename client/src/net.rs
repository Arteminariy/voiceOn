//! Сетевая часть клиента: UDP-сокет, сессия, keepalive, роуминг и переподключение.

use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use voiceon_proto::{
    Crypto, DEFAULT_PORT, DEFAULT_ROOM, Header, Kind, MAX_PACKET, ReplayWindow, RosterEntry,
    decode_roster, decode_voice,
};

use crate::jitter::JitterBuffer;

const PING_INTERVAL: Duration = Duration::from_secs(1);
const HELLO_RETRY: Duration = Duration::from_millis(500);
/// Нет понгов дольше этого — полное переподключение с новой сессией.
const RECONNECT_AFTER: Duration = Duration::from_secs(5);
const SPEAKING_HOLD: Duration = Duration::from_millis(250);
const LOSS_WINDOW: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Connecting,
    Connected,
    Reconnecting,
}

pub struct State {
    pub status: Status,
    pub rtt_ms: Option<f32>,
    pub loss_pct: f32,
    pub my_session: u32,
    pub roster: Vec<RosterEntry>,
    pub last_voice: HashMap<u32, Instant>,
    pub volumes: HashMap<String, f32>,
    pub mic_level: f32,
    pub transmitting: bool,
    pub audio_error: Option<String>,
}

impl State {
    pub fn is_speaking(&self, session: u32, now: Instant) -> bool {
        self.last_voice.get(&session).is_some_and(|t| now.duration_since(*t) < SPEAKING_HOLD)
    }

    pub fn nick_of(&self, session: u32) -> Option<&str> {
        self.roster.iter().find(|e| e.session_id == session).map(|e| e.nick.as_str())
    }
}

/// Управление, которое UI меняет на лету.
pub struct Controls {
    pub mic_muted: AtomicBool,
    pub deafened: AtomicBool,
    vad_threshold: AtomicU32,
}

impl Controls {
    pub fn vad_threshold(&self) -> f32 {
        f32::from_bits(self.vad_threshold.load(Ordering::Relaxed))
    }
    pub fn set_vad_threshold(&self, v: f32) {
        self.vad_threshold.store(v.to_bits(), Ordering::Relaxed);
    }
}

pub type Repaint = Arc<dyn Fn() + Send + Sync>;

pub struct Shared {
    pub state: Mutex<State>,
    pub jitter: Mutex<HashMap<u32, JitterBuffer>>,
    pub controls: Controls,
    pub repaint: Repaint,
    pub stop: AtomicBool,
}

impl Shared {
    pub fn new(volumes: HashMap<String, f32>, vad_threshold: f32, repaint: Repaint) -> Arc<Shared> {
        Arc::new(Shared {
            state: Mutex::new(State {
                status: Status::Connecting,
                rtt_ms: None,
                loss_pct: 0.0,
                my_session: 0,
                roster: Vec::new(),
                last_voice: HashMap::new(),
                volumes,
                mic_level: 0.0,
                transmitting: false,
                audio_error: None,
            }),
            jitter: Mutex::new(HashMap::new()),
            controls: Controls {
                mic_muted: AtomicBool::new(false),
                deafened: AtomicBool::new(false),
                vad_threshold: AtomicU32::new(vad_threshold.to_bits()),
            },
            repaint,
            stop: AtomicBool::new(false),
        })
    }
}

struct LinkInner {
    sock: Arc<UdpSocket>,
    session_id: u32,
    seq: u64,
}

/// Отправляющая сторона сессии; общая для сетевого потока и потока захвата.
pub struct Link {
    crypto: Crypto,
    server: SocketAddr,
    inner: Mutex<LinkInner>,
    /// Ошибка отправки (сменилась сеть) — сетевой поток пересоздаст сокет.
    socket_broken: AtomicBool,
}

pub fn random_u32() -> u32 {
    let mut b = [0u8; 4];
    getrandom::getrandom(&mut b).expect("нет источника случайности");
    // 0 оставляем как «нет сессии».
    u32::from_be_bytes(b).max(1)
}

fn new_socket(server: SocketAddr) -> std::io::Result<UdpSocket> {
    let bind: SocketAddr = if server.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }.parse().unwrap();
    let s = UdpSocket::bind(bind)?;
    s.set_read_timeout(Some(Duration::from_millis(50)))?;
    Ok(s)
}

/// `host`, `host:port` → адрес; IPv4 в приоритете.
pub fn resolve(server: &str) -> Result<SocketAddr, String> {
    let mut s = server.trim();
    // Люди вставляют адрес как ссылку — отрезаем схему и хвостовой слэш.
    for scheme in ["https://", "http://", "udp://"] {
        if let Some(rest) = s.strip_prefix(scheme) {
            s = rest;
        }
    }
    let s = s.trim_end_matches('/');
    if s.is_empty() {
        return Err("не указан сервер".into());
    }
    let with_port = if s.parse::<SocketAddr>().is_ok() || s.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) {
        s.to_string()
    } else {
        format!("{s}:{DEFAULT_PORT}")
    };
    let addrs: Vec<SocketAddr> =
        with_port.to_socket_addrs().map_err(|e| format!("{s}: {e}"))?.collect();
    addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or(addrs.first())
        .copied()
        .ok_or_else(|| format!("{s}: адрес не найден"))
}

impl Link {
    pub fn new(crypto: Crypto, server: SocketAddr) -> std::io::Result<Link> {
        Ok(Link {
            crypto,
            server,
            inner: Mutex::new(LinkInner {
                sock: Arc::new(new_socket(server)?),
                session_id: random_u32(),
                seq: 0,
            }),
            socket_broken: AtomicBool::new(false),
        })
    }

    pub fn session_id(&self) -> u32 {
        self.inner.lock().unwrap().session_id
    }

    fn socket(&self) -> Arc<UdpSocket> {
        self.inner.lock().unwrap().sock.clone()
    }

    pub fn send(&self, kind: Kind, payload: &[u8]) {
        let (pkt, sock) = {
            let mut g = self.inner.lock().unwrap();
            g.seq += 1;
            let h = Header { kind, room_id: DEFAULT_ROOM, session_id: g.session_id, seq: g.seq };
            (self.crypto.seal(&h, payload), g.sock.clone())
        };
        if sock.send_to(&pkt, self.server).is_err() {
            self.socket_broken.store(true, Ordering::Relaxed);
        }
    }

    /// Новый сокет, та же сессия: сервер увидит новый адрес и обновит его.
    fn replace_socket(&self) {
        if let Ok(s) = new_socket(self.server) {
            self.inner.lock().unwrap().sock = Arc::new(s);
        }
    }

    /// Полное переподключение: новый сокет и новая сессия (seq с нуля безопасен,
    /// потому что nonce включает новый session_id).
    fn renew(&self) {
        let sock = new_socket(self.server).ok();
        let mut g = self.inner.lock().unwrap();
        if let Some(s) = sock {
            g.sock = Arc::new(s);
        }
        g.session_id = random_u32();
        g.seq = 0;
    }

    pub fn bye(&self) {
        for _ in 0..3 {
            self.send(Kind::Bye, &[]);
        }
    }
}

pub fn spawn(link: Arc<Link>, shared: Arc<Shared>, nick: String) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("voiceon-net".into())
        .spawn(move || run(link, shared, nick))
        .expect("не удалось запустить сетевой поток")
}

fn set_status(shared: &Shared, status: Status, session: u32) {
    let mut st = shared.state.lock().unwrap();
    if st.status != status || st.my_session != session {
        st.status = status;
        st.my_session = session;
        if status != Status::Connected {
            st.rtt_ms = None;
        }
        drop(st);
        (shared.repaint)();
    }
}

fn run(link: Arc<Link>, shared: Arc<Shared>, nick: String) {
    let epoch = Instant::now();
    let mut welcomed = false;
    let mut last_hello = Instant::now() - HELLO_RETRY;
    let mut last_ping = Instant::now();
    let mut last_pong = Instant::now();
    let mut last_cleanup = Instant::now();
    let mut pings: VecDeque<(u64, Instant, bool)> = VecDeque::new();
    let mut replay: HashMap<u32, (ReplayWindow, Instant)> = HashMap::new();
    let mut buf = [0u8; MAX_PACKET];

    set_status(&shared, Status::Connecting, link.session_id());

    while !shared.stop.load(Ordering::Relaxed) {
        let now = Instant::now();

        if link.socket_broken.swap(false, Ordering::Relaxed) {
            link.replace_socket();
        }

        // Долго нет ответа сервера — новая сессия.
        if now.duration_since(last_pong) > RECONNECT_AFTER {
            link.bye();
            link.renew();
            welcomed = false;
            last_pong = now;
            pings.clear();
            set_status(&shared, Status::Reconnecting, link.session_id());
        }

        if !welcomed && now.duration_since(last_hello) >= HELLO_RETRY {
            link.send(Kind::Hello, nick.as_bytes());
            last_hello = now;
        }

        // Keepalive раз в секунду: пока VAD молчит, только он сообщает
        // серверу наш актуальный адрес (роуминг при включении VPN).
        if now.duration_since(last_ping) >= PING_INTERVAL {
            last_ping = now;
            let ts = now.duration_since(epoch).as_micros() as u64;
            link.send(Kind::Ping, &ts.to_be_bytes());
            pings.push_back((ts, now, false));
            while pings.len() > LOSS_WINDOW {
                pings.pop_front();
            }
            let settled: Vec<_> =
                pings.iter().filter(|p| now.duration_since(p.1) > Duration::from_secs(2)).collect();
            if !settled.is_empty() {
                let lost = settled.iter().filter(|p| !p.2).count();
                shared.state.lock().unwrap().loss_pct = lost as f32 * 100.0 / settled.len() as f32;
            }
        }

        if now.duration_since(last_cleanup) >= Duration::from_secs(5) {
            last_cleanup = now;
            replay.retain(|_, (_, t)| now.duration_since(*t) < Duration::from_secs(60));
            let roster: Vec<u32> =
                shared.state.lock().unwrap().roster.iter().map(|e| e.session_id).collect();
            shared.jitter.lock().unwrap().retain(|sid, jb| {
                roster.contains(sid) || now.duration_since(jb.last_arrival) < Duration::from_secs(10)
            });
        }

        let sock = link.socket();
        let (n, from) = match sock.recv_from(&mut buf) {
            Ok(v) => v,
            Err(_) => continue, // таймаут 50 мс или ICMP — просто следующая итерация
        };
        if from != link.server {
            continue;
        }
        let Some((h, payload)) = link.crypto.open(&buf[..n]) else { continue };
        let now = Instant::now();
        let my = link.session_id();
        if h.session_id == my {
            continue;
        }
        let entry = replay.entry(h.session_id).or_insert_with(|| (ReplayWindow::new(), now));
        if !entry.0.accept(h.seq) {
            continue;
        }
        entry.1 = now;

        match h.kind {
            Kind::Welcome => {
                welcomed = true;
                last_pong = now;
                set_status(&shared, Status::Connected, my);
            }
            Kind::Pong => {
                last_pong = now;
                if payload.len() == 8 {
                    let ts = u64::from_be_bytes(payload[..8].try_into().unwrap());
                    if let Some(p) = pings.iter_mut().find(|p| p.0 == ts) {
                        p.2 = true;
                        let rtt = now.duration_since(p.1).as_secs_f32() * 1000.0;
                        let mut st = shared.state.lock().unwrap();
                        st.rtt_ms = Some(match st.rtt_ms {
                            Some(prev) => prev * 0.7 + rtt * 0.3,
                            None => rtt,
                        });
                        drop(st);
                        (shared.repaint)();
                    }
                }
            }
            Kind::Rejoin => {
                welcomed = false;
                last_hello = now - HELLO_RETRY;
            }
            Kind::Roster => {
                if let Some(roster) = decode_roster(&payload) {
                    let mut st = shared.state.lock().unwrap();
                    if st.roster != roster {
                        st.roster = roster;
                        drop(st);
                        (shared.repaint)();
                    }
                }
            }
            Kind::Voice => {
                if let Some((frame_no, opus)) = decode_voice(&payload) {
                    shared
                        .jitter
                        .lock()
                        .unwrap()
                        .entry(h.session_id)
                        .or_insert_with(|| JitterBuffer::new(now))
                        .push(frame_no, opus.to_vec(), now);
                    let mut st = shared.state.lock().unwrap();
                    let was = st.is_speaking(h.session_id, now);
                    st.last_voice.insert(h.session_id, now);
                    drop(st);
                    if !was {
                        (shared.repaint)();
                    }
                }
            }
            Kind::Hello | Kind::Ping | Kind::Bye => {}
        }
    }
    link.bye();
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn resolve_strips_scheme_and_adds_default_port() {
        let a = resolve("https://127.0.0.1:123/").unwrap();
        assert_eq!(a.to_string(), "127.0.0.1:123");
        let b = resolve(" 127.0.0.1 ").unwrap();
        assert_eq!(b.port(), voiceon_proto::DEFAULT_PORT);
        assert!(resolve("").is_err());
    }
}
