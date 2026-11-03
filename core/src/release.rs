//! Проверка релизов: подпись ed25519 над `SHA256SUMS` и sha256 скачанного файла.
//!
//! Общая для десктопного автообновления и Android: один публичный ключ, один
//! формат `SHA256SUMS` (вывод `sha256sum`), подпись — hex в `SHA256SUMS.sig`.

use std::io::Read;
use std::path::Path;

use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

/// Репозиторий с релизами на GitHub.
pub const REPO: &str = "Arteminariy/voiceOn";
/// Публичный ключ подписи релизов (`voiceon-release pubkey`).
pub const RELEASE_PUBKEY: &str = "6a0a4f0cb0f0781fbbbc3e012faecfb4a96ea567438dc2df1af400fe68889204";

/// `v1.2.3`, `1.2`, `0.3.0-beta` → (мажор, минор, патч).
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches('v');
    let mut it = s.split('.').map(|p| p.split(|c: char| !c.is_ascii_digit()).next().unwrap_or("").parse::<u64>());
    Some((it.next()?.ok()?, it.next()?.ok()?, it.next().unwrap_or(Ok(0)).ok()?))
}

/// Тег `latest` новее версии `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

pub fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    let s = s.trim();
    if s.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (i, c) in s.as_bytes().chunks(2).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok()?;
    }
    Some(out)
}

/// Проверяет подпись `SHA256SUMS` ключом релизов и возвращает ожидаемый sha256 файла `asset`.
pub fn verified_hash(sums: &[u8], sig_hex: &[u8], asset: &str) -> Result<[u8; 32], String> {
    let pk = VerifyingKey::from_bytes(&unhex::<32>(RELEASE_PUBKEY).ok_or("битый ключ")?)
        .map_err(|e| e.to_string())?;
    verified_hash_with(&pk, sums, sig_hex, asset)
}

fn verified_hash_with(pk: &VerifyingKey, sums: &[u8], sig_hex: &[u8], asset: &str) -> Result<[u8; 32], String> {
    let sig = unhex::<64>(std::str::from_utf8(sig_hex).map_err(|_| "битая подпись")?)
        .ok_or("битая подпись")?;
    pk.verify_strict(sums, &Signature::from_bytes(&sig))
        .map_err(|_| "подпись релиза не сошлась — обновление отклонено")?;
    let text = std::str::from_utf8(sums).map_err(|_| "битый SHA256SUMS")?;
    let line = text
        .lines()
        .find(|l| l.split_whitespace().nth(1).map(|n| n.trim_start_matches('*')) == Some(asset))
        .ok_or_else(|| format!("в релизе нет {asset}"))?;
    unhex::<32>(line.split_whitespace().next().unwrap_or("")).ok_or_else(|| "битый SHA256SUMS".into())
}

/// sha256 файла, читая его кусками.
pub fn sha256_file(path: &Path) -> Result<[u8; 32], String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().into())
}

/// Сверяет sha256 данных с ожидаемым.
pub fn check_hash(data: &[u8], expected: &[u8; 32]) -> Result<(), String> {
    let actual: [u8; 32] = Sha256::digest(data).into();
    if &actual == expected { Ok(()) } else { Err("sha256 скачанного файла не совпал".into()) }
}

/// Полная проверка скачанного файла: подпись `SHA256SUMS` и sha256 файла `asset` на диске.
pub fn verify_file(sums: &[u8], sig_hex: &[u8], asset: &str, path: &Path) -> Result<(), String> {
    let expected = verified_hash(sums, sig_hex, asset)?;
    if sha256_file(path)? == expected { Ok(()) } else { Err("sha256 скачанного файла не совпал".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn versions() {
        assert!(parse_version("v0.2.0") > parse_version("0.1.9"));
        assert_eq!(parse_version("v1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("v0.3.0-beta"), Some((0, 3, 0)));
        assert!(parse_version("garbage").is_none());
        assert!(is_newer("v0.4.0", "0.3.0"));
        assert!(!is_newer("v0.3.0", "0.3.0"));
        assert!(!is_newer("garbage", "0.3.0"));
    }

    #[test]
    fn rejects_bad_signature() {
        let sums = b"00  voiceon-linux-x86_64\n";
        let sig = "00".repeat(64);
        assert!(verified_hash(sums, sig.as_bytes(), "voiceon-linux-x86_64").is_err());
    }

    #[test]
    fn pubkey_parses() {
        assert!(VerifyingKey::from_bytes(&unhex::<32>(RELEASE_PUBKEY).unwrap()).is_ok());
    }

    #[test]
    fn good_signature_and_hash() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let apk = b"fake apk contents";
        let digest: [u8; 32] = Sha256::digest(apk).into();
        let sums = format!("{}  voiceon-linux-x86_64\n{}  voiceon-android.apk\n", "11".repeat(32), hex(&digest));
        let sig = hex(&key.sign(sums.as_bytes()).to_bytes());
        let pk = key.verifying_key();

        let expected = verified_hash_with(&pk, sums.as_bytes(), sig.as_bytes(), "voiceon-android.apk").unwrap();
        assert_eq!(expected, digest);
        assert!(check_hash(apk, &expected).is_ok());
        assert!(check_hash(b"tampered apk", &expected).is_err());
        // Файла нет в списке.
        assert!(verified_hash_with(&pk, sums.as_bytes(), sig.as_bytes(), "voiceon-ios.ipa").is_err());
        // Подделанный список: подпись больше не сходится.
        let forged = sums.replace(&hex(&digest), &"22".repeat(32));
        assert!(verified_hash_with(&pk, forged.as_bytes(), sig.as_bytes(), "voiceon-android.apk").is_err());
        // Чужой ключ (встроенный ключ релизов) эту подпись не принимает.
        assert!(verified_hash(sums.as_bytes(), sig.as_bytes(), "voiceon-android.apk").is_err());
    }

    #[test]
    fn file_hash_streams() {
        let dir = std::env::temp_dir().join(format!("voiceon-release-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("blob");
        let data: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
        std::fs::write(&p, &data).unwrap();
        let want: [u8; 32] = Sha256::digest(&data).into();
        assert_eq!(sha256_file(&p).unwrap(), want);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
