//! Протокол voiceOn: заголовок пакета, шифрование PSK + ChaCha20-Poly1305,
//! окно защиты от повторов и полезные нагрузки управляющих сообщений.
//!
//! Формат пакета на проводе:
//! ```text
//! | version u8 | kind u8 | room_id u16 | session_id u32 | seq u64 | ciphertext ... | tag 16 |
//! ```
//! Заголовок (16 байт) идёт открытым текстом и аутентифицируется как AAD.
//! Nonce = `session_id (4, BE) ‖ seq (8, BE)`: каждая сторона генерирует свой
//! случайный `session_id` при каждом подключении и ведёт монотонный `seq`,
//! поэтому пара (ключ, nonce) не повторяется.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

pub const VERSION: u8 = 1;
pub const HEADER_LEN: usize = 16;
pub const TAG_LEN: usize = 16;
/// Максимальный размер датаграммы, который мы отправляем и принимаем.
pub const MAX_PACKET: usize = 1400;
pub const MAX_NICK_LEN: usize = 32;
/// Комната по умолчанию; поле зарезервировано под будущие каналы.
pub const DEFAULT_ROOM: u16 = 0;
pub const DEFAULT_PORT: u16 = 47800;

/// Фиксированная соль приложения: клиент и сервер должны вывести один ключ
/// из одной фразы, поэтому соль не случайная.
const KDF_SALT: &[u8] = b"voiceon-psk-v1-salt";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    /// C→S: вход в комнату, полезная нагрузка — ник (UTF-8).
    Hello = 1,
    /// S→C: подтверждение Hello.
    Welcome = 2,
    /// C→S: keepalive + замер RTT, полезная нагрузка — 8 байт метки времени клиента.
    Ping = 3,
    /// S→C: эхо Ping.
    Pong = 4,
    /// S→C: список участников, см. [`encode_roster`].
    Roster = 5,
    /// C→S→C: голосовой фрейм, см. [`encode_voice`].
    Voice = 6,
    /// C→S: выход из комнаты.
    Bye = 7,
    /// S→C: сервер не знает эту сессию, нужно заново прислать Hello.
    Rejoin = 8,
}

impl Kind {
    pub fn from_u8(v: u8) -> Option<Kind> {
        Some(match v {
            1 => Kind::Hello,
            2 => Kind::Welcome,
            3 => Kind::Ping,
            4 => Kind::Pong,
            5 => Kind::Roster,
            6 => Kind::Voice,
            7 => Kind::Bye,
            8 => Kind::Rejoin,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub kind: Kind,
    pub room_id: u16,
    pub session_id: u32,
    pub seq: u64,
}

impl Header {
    pub fn write(&self, out: &mut [u8; HEADER_LEN]) {
        out[0] = VERSION;
        out[1] = self.kind as u8;
        out[2..4].copy_from_slice(&self.room_id.to_be_bytes());
        out[4..8].copy_from_slice(&self.session_id.to_be_bytes());
        out[8..16].copy_from_slice(&self.seq.to_be_bytes());
    }

    pub fn parse(buf: &[u8]) -> Option<Header> {
        if buf.len() < HEADER_LEN + TAG_LEN || buf[0] != VERSION {
            return None;
        }
        Some(Header {
            kind: Kind::from_u8(buf[1])?,
            room_id: u16::from_be_bytes([buf[2], buf[3]]),
            session_id: u32::from_be_bytes(buf[4..8].try_into().ok()?),
            seq: u64::from_be_bytes(buf[8..16].try_into().ok()?),
        })
    }

    fn nonce(&self) -> [u8; 12] {
        let mut n = [0u8; 12];
        n[..4].copy_from_slice(&self.session_id.to_be_bytes());
        n[4..].copy_from_slice(&self.seq.to_be_bytes());
        n
    }
}

/// Симметричный ключ комнаты, выведенный из парольной фразы.
#[derive(Clone)]
pub struct Crypto {
    cipher: ChaCha20Poly1305,
}

impl Crypto {
    /// Выводит ключ через Argon2id (параметры по умолчанию, ~20–50 мс).
    pub fn from_passphrase(pass: &str) -> Crypto {
        let mut key = [0u8; 32];
        argon2::Argon2::default()
            .hash_password_into(pass.as_bytes(), KDF_SALT, &mut key)
            .expect("argon2 с параметрами по умолчанию не может упасть");
        Crypto::from_key(key)
    }

    pub fn from_key(key: [u8; 32]) -> Crypto {
        Crypto { cipher: ChaCha20Poly1305::new(Key::from_slice(&key)) }
    }

    /// Собирает зашифрованный пакет.
    pub fn seal(&self, header: &Header, payload: &[u8]) -> Vec<u8> {
        let mut hdr = [0u8; HEADER_LEN];
        header.write(&mut hdr);
        let nonce = header.nonce();
        let ct = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: payload, aad: &hdr })
            .expect("шифрование в памяти не падает");
        let mut out = Vec::with_capacity(HEADER_LEN + ct.len());
        out.extend_from_slice(&hdr);
        out.extend_from_slice(&ct);
        out
    }

    /// Проверяет и расшифровывает пакет. `None` — чужой, битый или подделанный пакет.
    pub fn open(&self, packet: &[u8]) -> Option<(Header, Vec<u8>)> {
        let header = Header::parse(packet)?;
        let nonce = header.nonce();
        let plain = self
            .cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload { msg: &packet[HEADER_LEN..], aad: &packet[..HEADER_LEN] },
            )
            .ok()?;
        Some((header, plain))
    }
}

/// Окно защиты от повторов (как в WireGuard/DTLS): принимаем только `seq`
/// новее максимального или попадающие в последние 64 и ещё не виденные.
#[derive(Debug, Clone, Default)]
pub struct ReplayWindow {
    max: Option<u64>,
    bitmap: u64,
}

impl ReplayWindow {
    pub fn new() -> ReplayWindow {
        ReplayWindow::default()
    }

    /// Возвращает `true` и запоминает `seq`, если пакет свежий.
    /// Вызывать только после успешной проверки тега.
    pub fn accept(&mut self, seq: u64) -> bool {
        let Some(max) = self.max else {
            self.max = Some(seq);
            self.bitmap = 1;
            return true;
        };
        if seq > max {
            let shift = seq - max;
            self.bitmap = if shift >= 64 { 0 } else { self.bitmap << shift };
            self.bitmap |= 1;
            self.max = Some(seq);
            true
        } else {
            let back = max - seq;
            if back >= 64 {
                return false;
            }
            let bit = 1u64 << back;
            if self.bitmap & bit != 0 {
                return false;
            }
            self.bitmap |= bit;
            true
        }
    }
}

/// Голос: `frame_no u32 BE ‖ opus`. `frame_no` растёт на 1 каждые 20 мс захвата,
/// даже когда VAD молчит, поэтому разрыв в номерах означает паузу.
pub fn encode_voice(frame_no: u32, opus: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + opus.len());
    v.extend_from_slice(&frame_no.to_be_bytes());
    v.extend_from_slice(opus);
    v
}

pub fn decode_voice(payload: &[u8]) -> Option<(u32, &[u8])> {
    if payload.len() < 5 {
        return None;
    }
    Some((u32::from_be_bytes(payload[..4].try_into().ok()?), &payload[4..]))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    pub session_id: u32,
    pub nick: String,
}

/// Roster: `count u8 ‖ (session_id u32 BE ‖ len u8 ‖ nick)*`.
pub fn encode_roster(entries: &[RosterEntry]) -> Vec<u8> {
    let mut v = vec![entries.len().min(255) as u8];
    for e in entries.iter().take(255) {
        let nick = truncate_nick(&e.nick);
        v.extend_from_slice(&e.session_id.to_be_bytes());
        v.push(nick.len() as u8);
        v.extend_from_slice(nick.as_bytes());
    }
    v
}

pub fn decode_roster(payload: &[u8]) -> Option<Vec<RosterEntry>> {
    let (&count, mut rest) = payload.split_first()?;
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        if rest.len() < 5 {
            return None;
        }
        let session_id = u32::from_be_bytes(rest[..4].try_into().ok()?);
        let len = rest[4] as usize;
        rest = &rest[5..];
        if rest.len() < len {
            return None;
        }
        let nick = String::from_utf8_lossy(&rest[..len]).into_owned();
        rest = &rest[len..];
        out.push(RosterEntry { session_id, nick });
    }
    Some(out)
}

/// Обрезает ник до [`MAX_NICK_LEN`] байт по границе символа.
pub fn truncate_nick(nick: &str) -> &str {
    let nick = nick.trim();
    if nick.len() <= MAX_NICK_LEN {
        return nick;
    }
    let mut end = MAX_NICK_LEN;
    while !nick.is_char_boundary(end) {
        end -= 1;
    }
    &nick[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(kind: Kind, session_id: u32, seq: u64) -> Header {
        Header { kind, room_id: DEFAULT_ROOM, session_id, seq }
    }

    #[test]
    fn seal_open_roundtrip() {
        let c = Crypto::from_key([7; 32]);
        let h = hdr(Kind::Voice, 0xdead_beef, 42);
        let pkt = c.seal(&h, b"hello");
        assert_eq!(pkt.len(), HEADER_LEN + 5 + TAG_LEN);
        let (h2, p) = c.open(&pkt).unwrap();
        assert_eq!(h, h2);
        assert_eq!(p, b"hello");
    }

    #[test]
    fn passphrase_derives_same_key() {
        let a = Crypto::from_passphrase("катка");
        let b = Crypto::from_passphrase("катка");
        let pkt = a.seal(&hdr(Kind::Ping, 1, 1), b"x");
        assert!(b.open(&pkt).is_some());
        let other = Crypto::from_passphrase("другая");
        assert!(other.open(&pkt).is_none());
    }

    #[test]
    fn tampered_header_or_body_rejected() {
        let c = Crypto::from_key([1; 32]);
        let pkt = c.seal(&hdr(Kind::Voice, 5, 9), b"payload");
        for i in 0..pkt.len() {
            let mut bad = pkt.clone();
            bad[i] ^= 0x01;
            assert!(c.open(&bad).is_none(), "byte {i} flip accepted");
        }
        assert!(c.open(&pkt[..pkt.len() - 1]).is_none());
        assert!(c.open(&[]).is_none());
    }

    #[test]
    fn replay_window() {
        let mut w = ReplayWindow::new();
        assert!(w.accept(10));
        assert!(!w.accept(10));
        assert!(w.accept(8));
        assert!(!w.accept(8));
        assert!(w.accept(11));
        assert!(w.accept(100));
        assert!(!w.accept(36)); // ровно 64 назад — вне окна
        assert!(w.accept(37));
        assert!(!w.accept(37));
        assert!(w.accept(1000));
        assert!(!w.accept(100));
    }

    #[test]
    fn roster_roundtrip() {
        let r = vec![
            RosterEntry { session_id: 1, nick: "Тёма".into() },
            RosterEntry { session_id: u32::MAX, nick: String::new() },
        ];
        assert_eq!(decode_roster(&encode_roster(&r)).unwrap(), r);
        assert!(decode_roster(&[3, 0, 0]).is_none());
    }

    #[test]
    fn voice_roundtrip() {
        let p = encode_voice(77, &[1, 2, 3]);
        assert_eq!(decode_voice(&p), Some((77, &[1u8, 2, 3][..])));
        assert!(decode_voice(&[0, 0, 0, 1]).is_none());
    }

    #[test]
    fn nick_truncation_respects_utf8() {
        let long = "я".repeat(40); // 80 байт
        let t = truncate_nick(&long);
        assert!(t.len() <= MAX_NICK_LEN);
        assert_eq!(t.chars().count(), 16);
    }
}
