//! voiceon-server — тупой relay (SFU): проверяет тег пакета и пересылает
//! голос остальным участникам комнаты теми же байтами, без перешифровки.

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use serde::Deserialize;
use voiceon_proto::{
    Crypto, DEFAULT_PORT, Header, Kind, MAX_PACKET, ReplayWindow, RosterEntry, encode_roster,
    truncate_nick,
};

const PEER_TIMEOUT: Duration = Duration::from_secs(15);
const ROSTER_INTERVAL: Duration = Duration::from_secs(5);
const MAX_PEERS: usize = 32;

#[derive(Deserialize)]
struct Config {
    #[serde(default = "default_bind")]
    bind: String,
    passphrase: String,
}

fn default_bind() -> String {
    format!("0.0.0.0:{DEFAULT_PORT}")
}

struct Peer {
    addr: SocketAddr,
    room_id: u16,
    nick: String,
    last_seen: Instant,
    replay: ReplayWindow,
}

struct Server {
    sock: UdpSocket,
    crypto: Crypto,
    /// Собственная сессия сервера: он тоже шифрует (Welcome/Pong/Roster),
    /// поэтому ему нужен свой случайный session_id и свой seq.
    session_id: u32,
    seq: u64,
    peers: HashMap<u32, Peer>,
}

impl Server {
    fn send(&mut self, to: SocketAddr, kind: Kind, room_id: u16, payload: &[u8]) {
        self.seq += 1;
        let h = Header { kind, room_id, session_id: self.session_id, seq: self.seq };
        let pkt = self.crypto.seal(&h, payload);
        let _ = self.sock.send_to(&pkt, to);
    }

    fn broadcast_roster(&mut self, room_id: u16) {
        let mut entries: Vec<RosterEntry> = self
            .peers
            .iter()
            .filter(|(_, p)| p.room_id == room_id)
            .map(|(&session_id, p)| RosterEntry { session_id, nick: p.nick.clone() })
            .collect();
        entries.sort_by(|a, b| a.nick.cmp(&b.nick));
        let payload = encode_roster(&entries);
        let targets: Vec<SocketAddr> =
            self.peers.values().filter(|p| p.room_id == room_id).map(|p| p.addr).collect();
        for addr in targets {
            self.send(addr, Kind::Roster, room_id, &payload);
        }
    }

    fn rooms(&self) -> Vec<u16> {
        let mut r: Vec<u16> = self.peers.values().map(|p| p.room_id).collect();
        r.sort_unstable();
        r.dedup();
        r
    }

    fn handle(&mut self, pkt: &[u8], from: SocketAddr) {
        // Без валидного тега пакет молча дропается: для сканера порт мёртв.
        let Some((h, payload)) = self.crypto.open(pkt) else { return };
        if h.session_id == self.session_id {
            return;
        }

        if let Some(peer) = self.peers.get_mut(&h.session_id) {
            if !peer.replay.accept(h.seq) {
                return;
            }
            // Роуминг: адрес меняется только после проверки тега и seq.
            if peer.addr != from {
                eprintln!("[{}] {} сменил адрес {} -> {}", h.session_id, peer.nick, peer.addr, from);
                peer.addr = from;
            }
            peer.last_seen = Instant::now();
        } else if h.kind != Kind::Hello {
            // Незнакомая сессия (например, сервер перезапускался) — пусть представится.
            if matches!(h.kind, Kind::Ping | Kind::Voice) {
                self.send(from, Kind::Rejoin, h.room_id, &[]);
            }
            return;
        }

        match h.kind {
            Kind::Hello => self.on_hello(h, &payload, from),
            Kind::Ping => self.send(from, Kind::Pong, h.room_id, &payload),
            Kind::Voice => {
                for (&sid, p) in &self.peers {
                    if sid != h.session_id && p.room_id == h.room_id {
                        let _ = self.sock.send_to(pkt, p.addr);
                    }
                }
            }
            Kind::Bye => {
                if let Some(p) = self.peers.remove(&h.session_id) {
                    eprintln!("[{}] {} вышел", h.session_id, p.nick);
                    self.broadcast_roster(p.room_id);
                }
            }
            // Серверные типы от клиента игнорируем.
            Kind::Welcome | Kind::Pong | Kind::Roster | Kind::Rejoin => {}
        }
    }

    fn on_hello(&mut self, h: Header, payload: &[u8], from: SocketAddr) {
        let nick = truncate_nick(&String::from_utf8_lossy(payload)).to_string();
        let is_new = !self.peers.contains_key(&h.session_id);
        if is_new {
            // Переподключение того же клиента: новая сессия с того же IP с тем же ником
            // вытесняет старую, иначе она висела бы «призраком» до таймаута.
            let stale: Vec<u32> = self
                .peers
                .iter()
                .filter(|(_, p)| p.addr.ip() == from.ip() && p.nick == nick && p.room_id == h.room_id)
                .map(|(&sid, _)| sid)
                .collect();
            for sid in stale {
                if let Some(p) = self.peers.remove(&sid) {
                    eprintln!("[{sid}] {} заменён новой сессией", p.nick);
                }
            }
            if self.peers.len() >= MAX_PEERS {
                return;
            }
            let mut replay = ReplayWindow::new();
            replay.accept(h.seq);
            eprintln!("[{}] {} вошёл с {}", h.session_id, nick, from);
            self.peers.insert(
                h.session_id,
                Peer { addr: from, room_id: h.room_id, nick, last_seen: Instant::now(), replay },
            );
        } else if let Some(p) = self.peers.get_mut(&h.session_id) {
            p.nick = nick;
        }
        self.send(from, Kind::Welcome, h.room_id, &[]);
        self.broadcast_roster(h.room_id);
    }

    fn expire(&mut self) {
        let now = Instant::now();
        let mut changed = Vec::new();
        self.peers.retain(|sid, p| {
            let alive = now.duration_since(p.last_seen) < PEER_TIMEOUT;
            if !alive {
                eprintln!("[{sid}] {} отвалился по таймауту", p.nick);
                changed.push(p.room_id);
            }
            alive
        });
        changed.sort_unstable();
        changed.dedup();
        for room in changed {
            self.broadcast_roster(room);
        }
    }
}

fn random_u32() -> u32 {
    let mut b = [0u8; 4];
    getrandom::getrandom(&mut b).expect("нет источника случайности");
    u32::from_be_bytes(b)
}

fn load_config() -> Result<Config, String> {
    let mut args = std::env::args().skip(1);
    let mut path = String::from("/etc/voiceon/server.toml");
    while let Some(a) = args.next() {
        match a.as_str() {
            "-c" | "--config" => path = args.next().ok_or("после --config нужен путь")?,
            "-h" | "--help" => {
                println!("voiceon-server [--config /etc/voiceon/server.toml]");
                std::process::exit(0);
            }
            other => return Err(format!("неизвестный аргумент: {other}")),
        }
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
    let cfg: Config = toml::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    if cfg.passphrase.is_empty() {
        return Err("passphrase пустая".into());
    }
    Ok(cfg)
}

fn main() {
    let cfg = match load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ошибка конфига: {e}");
            std::process::exit(2);
        }
    };
    let sock = UdpSocket::bind(&cfg.bind).unwrap_or_else(|e| {
        eprintln!("не удалось занять {}: {e}", cfg.bind);
        std::process::exit(1);
    });
    sock.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
    let mut srv = Server {
        sock,
        crypto: Crypto::from_passphrase(&cfg.passphrase),
        session_id: random_u32(),
        seq: 0,
        peers: HashMap::new(),
    };
    eprintln!("voiceon-server слушает {}", cfg.bind);

    let mut buf = [0u8; MAX_PACKET];
    let mut last_roster = Instant::now();
    let mut last_expire = Instant::now();
    loop {
        match srv.sock.recv_from(&mut buf) {
            Ok((n, from)) => srv.handle(&buf[..n], from),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            // ICMP port unreachable и прочее — не повод падать.
            Err(_) => {}
        }
        let now = Instant::now();
        if now.duration_since(last_expire) >= Duration::from_secs(1) {
            last_expire = now;
            srv.expire();
        }
        // Периодический roster заменяет подтверждения: состояние идемпотентно.
        if now.duration_since(last_roster) >= ROSTER_INTERVAL {
            last_roster = now;
            for room in srv.rooms() {
                srv.broadcast_roster(room);
            }
        }
    }
}
