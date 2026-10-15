use std::fs;
use std::path::Path;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MAX_BYTES: usize = 80 * 1024 * 1024;

/// Публичная половина ключа издателя. Секрет лежит только у того, кто выпускает версию.
pub const PINNED_PUBLIC_HEX: &str = "bd478e03392a527df2c3e2deebe3485aaa5f87c74bda63cdffc520ae7369f668";

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("ключ обновления не читается")]
    BadKey,
    #[error("подпись обновления не сходится")]
    BadSignature,
    #[error("хеш обновления не сходится")]
    HashMismatch,
    #[error("версия в имени файла недопустима")]
    BadVersion,
    #[error("файл обновления больше допустимого размера")]
    TooLarge,
    #[error("файл: {0}")]
    Io(#[from] std::io::Error),
    #[error("описание обновления: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub version: String,
    pub sha256: String,
    pub signature: String,
    pub size: u64,
}

pub fn generate_keypair() -> (String, String) {
    let signing = SigningKey::generate(&mut rand_core::OsRng);
    let public = hex_encode(&signing.verifying_key().to_bytes());
    (hex_encode(&signing.to_bytes()), public)
}

pub fn public_from_secret(secret_hex: &str) -> Result<String, UpdateError> {
    let secret = decode_key(secret_hex).ok_or(UpdateError::BadKey)?;
    let signing = SigningKey::from_bytes(&secret);
    Ok(hex_encode(&signing.verifying_key().to_bytes()))
}

pub fn sign_release(secret_hex: &str, version: &str, bytes: &[u8]) -> Result<ReleaseManifest, UpdateError> {
    if !version_name_ok(version) {
        return Err(UpdateError::BadVersion);
    }
    if bytes.len() > MAX_BYTES {
        return Err(UpdateError::TooLarge);
    }
    let secret = decode_key(secret_hex).ok_or(UpdateError::BadKey)?;
    let signing = SigningKey::from_bytes(&secret);
    let sha256 = sha256_hex(bytes);
    let signature = signing.sign(&signed_message(version, &sha256));
    Ok(ReleaseManifest {
        version: version.to_string(),
        sha256,
        signature: hex_encode(&signature.to_bytes()),
        size: bytes.len() as u64,
    })
}

pub fn verify_release(manifest: &ReleaseManifest, bytes: &[u8], public_hex: &str) -> Result<(), UpdateError> {
    if bytes.len() > MAX_BYTES || manifest.size != bytes.len() as u64 {
        return Err(UpdateError::TooLarge);
    }
    let actual = sha256_hex(bytes);
    if !actual.eq_ignore_ascii_case(&manifest.sha256) {
        return Err(UpdateError::HashMismatch);
    }
    let public = decode_key(public_hex).ok_or(UpdateError::BadKey)?;
    let verifying = VerifyingKey::from_bytes(&public).map_err(|_| UpdateError::BadKey)?;
    let signature = decode_signature(&manifest.signature).ok_or(UpdateError::BadSignature)?;
    verifying
        .verify(&signed_message(&manifest.version, &manifest.sha256), &signature)
        .map_err(|_| UpdateError::BadSignature)
}

pub fn verify_pinned(manifest: &ReleaseManifest, bytes: &[u8]) -> Result<(), UpdateError> {
    verify_release(manifest, bytes, PINNED_PUBLIC_HEX)
}

pub fn should_replace(running_version: &str, running_hash: &str, manifest: &ReleaseManifest) -> bool {
    if !running_hash.is_empty() && running_hash.eq_ignore_ascii_case(&manifest.sha256) {
        return false;
    }
    is_newer(&manifest.version, running_version)
}

pub fn is_newer(offered: &str, running: &str) -> bool {
    version_parts(offered) > version_parts(running)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex_encode(&Sha256::digest(bytes))
}

pub fn hash_file(path: &Path) -> Result<String, UpdateError> {
    let bytes = fs::read(path)?;
    Ok(sha256_hex(&bytes))
}

pub fn running_hash() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| hash_file(&path).ok())
        .unwrap_or_default()
}

pub fn save_release(dir: &Path, manifest: &ReleaseManifest, bytes: &[u8]) -> Result<(), UpdateError> {
    verify_pinned(manifest, bytes)?;
    if !version_name_ok(&manifest.version) {
        return Err(UpdateError::BadVersion);
    }
    fs::create_dir_all(dir)?;
    fs::write(dir.join(format!("{}.bin", manifest.version)), bytes)?;
    fs::write(
        dir.join(format!("{}.json", manifest.version)),
        serde_json::to_vec_pretty(manifest)?,
    )?;
    Ok(())
}

pub fn load_newest(dir: &Path) -> Result<Option<(ReleaseManifest, Vec<u8>)>, UpdateError> {
    if !dir.is_dir() {
        return Ok(None);
    }
    let mut best: Option<(ReleaseManifest, Vec<u8>)> = None;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(version) = name.strip_suffix(".json") else {
            continue;
        };
        if !version_name_ok(version) {
            continue;
        }
        let Ok(manifest) = fs::read(entry.path()).and_then(|bytes| {
            serde_json::from_slice::<ReleaseManifest>(&bytes).map_err(|err| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, err)
            })
        }) else {
            continue;
        };
        if manifest.version != version {
            continue;
        }
        let Ok(bytes) = fs::read(dir.join(format!("{version}.bin"))) else {
            continue;
        };
        if verify_pinned(&manifest, &bytes).is_err() {
            continue;
        }
        let replace = best
            .as_ref()
            .map(|(current, _)| is_newer(&manifest.version, &current.version))
            .unwrap_or(true);
        if replace {
            best = Some((manifest, bytes));
        }
    }
    Ok(best)
}

pub fn stage_and_relaunch(bytes: &[u8]) -> Result<(), UpdateError> {
    if bytes.len() > MAX_BYTES {
        return Err(UpdateError::TooLarge);
    }
    let current = std::env::current_exe()?;
    let dir = current.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "нет каталога программы")
    })?;
    let staged = dir.join("mds-next.exe");
    fs::write(&staged, bytes)?;
    let args = std::env::args().skip(1).map(|arg| quote_cmd(&arg)).collect::<Vec<_>>();
    let arg_line = args.join(" ");
    let current_q = quote_cmd(&current.display().to_string());
    let staged_q = quote_cmd(&staged.display().to_string());
    if cfg!(windows) {
        let cmd = format!(
            "ping -n 4 127.0.0.1 >nul & copy /Y {staged_q} {current_q} & start \"\" {current_q} {arg_line}"
        );
        std::process::Command::new("cmd").arg("/C").arg(cmd).spawn()?;
    } else {
        let cmd = format!("sleep 2; cp -f {staged_q} {current_q}; {current_q} {arg_line}");
        std::process::Command::new("sh").arg("-c").arg(cmd).spawn()?;
    }
    Ok(())
}

fn signed_message(version: &str, sha256_hex: &str) -> Vec<u8> {
    format!("mds-release-v1\n{version}\n{sha256_hex}\n").into_bytes()
}

fn version_parts(version: &str) -> [u64; 4] {
    let mut parts = [0_u64; 4];
    for (index, part) in version.split('.').take(4).enumerate() {
        let digits: String = part.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        parts[index] = digits.parse().unwrap_or(0);
    }
    parts
}

fn version_name_ok(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 32
        && version
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_')
}

fn decode_key(hex_text: &str) -> Option<[u8; 32]> {
    let bytes = decode_hex(hex_text.trim())?;
    if bytes.len() != 32 {
        return None;
    }
    let mut out = [0_u8; 32];
    out.copy_from_slice(&bytes);
    Some(out)
}

fn decode_signature(hex_text: &str) -> Option<Signature> {
    let bytes = decode_hex(hex_text.trim())?;
    Signature::from_slice(&bytes).ok()
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if text.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let high = hex_value(bytes[index])?;
        let low = hex_value(bytes[index + 1])?;
        out.push((high << 4) | low);
        index += 2;
    }
    Some(out)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn quote_cmd(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}

pub fn zip_stored(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let name_bytes = name.as_bytes();
        let crc = crc32(data);
        let offset = out.len() as u32;
        out.extend_from_slice(&0x04034b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);

        central.extend_from_slice(&0x02014b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
    }
    let central_start = out.len() as u32;
    let central_size = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x06054b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_release_verifies_and_rejects_tampering() {
        let (secret, public) = generate_keypair();
        let bytes = b"mds-binary-v2";
        let manifest = sign_release(&secret, "0.2.0", bytes).unwrap();
        verify_release(&manifest, bytes, &public).unwrap();
        let mut broken = manifest.clone();
        broken.sha256 = "ab".repeat(32);
        assert!(verify_release(&broken, bytes, &public).is_err());
        assert!(verify_release(&manifest, b"other", &public).is_err());
        assert!(should_replace("0.1.0", "aaa", &manifest));
        assert!(!should_replace("0.2.0", &manifest.sha256, &manifest));
        assert!(!should_replace("0.9.0", "other", &manifest));
    }

    #[test]
    fn pinned_publisher_key_is_set() {
        assert_eq!(PINNED_PUBLIC_HEX.len(), 64);
        assert!(PINNED_PUBLIC_HEX.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert_ne!(&PINNED_PUBLIC_HEX[..8], "00000000");
    }

    #[test]
    fn foreign_signature_is_not_loaded() {
        let (secret, _) = generate_keypair();
        let dir = std::env::temp_dir().join(format!("mds-release-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let bytes = b"new-node";
        let manifest = sign_release(&secret, "0.2.0", bytes).unwrap();
        assert!(save_release(&dir, &manifest, bytes).is_err());
        fs::write(dir.join("0.2.0.bin"), bytes).unwrap();
        fs::write(dir.join("0.2.0.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
        let loaded = load_newest(&dir).unwrap();
        let _ = fs::remove_dir_all(&dir);
        assert!(loaded.is_none());
    }

    #[test]
    fn stored_zip_contains_the_node_files() {
        let zip = zip_stored(&[
            ("mds.exe", b"node"),
            ("onnxruntime.dll", b"ort"),
            ("urls.txt", b"https://example.com/\n"),
        ]);
        assert!(zip.windows(4).any(|w| w == [0x50, 0x4b, 0x03, 0x04]));
        let text = String::from_utf8_lossy(&zip);
        assert!(text.contains("mds.exe"));
        assert!(text.contains("onnxruntime.dll"));
        assert!(text.contains("urls.txt"));
    }
}
