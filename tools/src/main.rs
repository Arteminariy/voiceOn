//! Подпись релизов voiceOn.
//!
//! `voiceon-release keygen` — создаёт ключ в `~/.config/voiceon-release.key` (0600)
//! и печатает публичный ключ для `client/src/update.rs`.
//! `voiceon-release sign FILE` — пишет `FILE.sig` (hex подписи ed25519).
//! `voiceon-release pubkey` — печатает публичный ключ существующего ключа.

use std::path::PathBuf;

use ed25519_dalek::{Signer, SigningKey};

fn key_path() -> PathBuf {
    let home = std::env::var_os("HOME").expect("нет HOME");
    PathBuf::from(home).join(".config").join("voiceon-release.key")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn load_key() -> Result<SigningKey, String> {
    let p = key_path();
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let text = text.trim();
    if text.len() != 64 {
        return Err("ключ повреждён".into());
    }
    let mut b = [0u8; 32];
    for (i, chunk) in text.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).map_err(|e| e.to_string())?;
        b[i] = u8::from_str_radix(s, 16).map_err(|e| e.to_string())?;
    }
    Ok(SigningKey::from_bytes(&b))
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("keygen") => {
            let p = key_path();
            if p.exists() {
                return Err(format!("{} уже есть — не перезаписываю", p.display()));
            }
            let mut seed = [0u8; 32];
            getrandom::getrandom(&mut seed).map_err(|e| e.to_string())?;
            let key = SigningKey::from_bytes(&seed);
            std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&p, hex(&seed)).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))
                    .map_err(|e| e.to_string())?;
            }
            println!("ключ: {}", p.display());
            println!("публичный ключ: {}", hex(key.verifying_key().as_bytes()));
            Ok(())
        }
        Some("pubkey") => {
            println!("{}", hex(load_key()?.verifying_key().as_bytes()));
            Ok(())
        }
        Some("sign") => {
            let file = args.get(1).ok_or("sign: укажи файл")?;
            let data = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
            let sig = load_key()?.sign(&data);
            std::fs::write(format!("{file}.sig"), hex(&sig.to_bytes())).map_err(|e| e.to_string())?;
            println!("подписано: {file}.sig");
            Ok(())
        }
        _ => Err("использование: voiceon-release keygen | pubkey | sign FILE".into()),
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
