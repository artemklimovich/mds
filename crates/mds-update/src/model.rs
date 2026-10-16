use std::fs;
use std::path::Path;

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::{
    decode_key, decode_signature, hex_encode, hash_path, UpdateError, PINNED_PUBLIC_HEX,
};

pub const SWARM_MODEL_FILES: [&str; 5] = [
    "model.onnx",
    "tokenizer.json",
    "config.json",
    "tokenizer_config.json",
    "special_tokens_map.json",
];

pub const MODEL_DIMS: u32 = 384;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelFile {
    pub name: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelManifest {
    pub name: String,
    pub dims: u32,
    pub files: Vec<ModelFile>,
    pub signature: String,
}

pub fn sign_model(secret_hex: &str, dir: &Path, name: &str) -> Result<ModelManifest, UpdateError> {
    if !model_name_ok(name) {
        return Err(UpdateError::BadVersion);
    }
    let mut files = Vec::with_capacity(SWARM_MODEL_FILES.len());
    for file_name in SWARM_MODEL_FILES {
        let (sha256, size) = hash_path(&dir.join(file_name))?;
        files.push(ModelFile {
            name: file_name.to_string(),
            sha256,
            size,
        });
    }
    let secret = decode_key(secret_hex).ok_or(UpdateError::BadKey)?;
    let signing = SigningKey::from_bytes(&secret);
    let signature = signing.sign(&model_message(name, MODEL_DIMS, &files));
    let manifest = ModelManifest {
        name: name.to_string(),
        dims: MODEL_DIMS,
        files,
        signature: hex_encode(&signature.to_bytes()),
    };
    fs::write(dir.join("manifest.json"), serde_json::to_vec_pretty(&manifest)?)?;
    Ok(manifest)
}

pub fn verify_model(manifest: &ModelManifest) -> Result<(), UpdateError> {
    verify_model_with(manifest, PINNED_PUBLIC_HEX)
}

pub fn verify_model_with(manifest: &ModelManifest, public_hex: &str) -> Result<(), UpdateError> {
    if manifest.dims != MODEL_DIMS || !model_name_ok(&manifest.name) {
        return Err(UpdateError::BadVersion);
    }
    if manifest.files.len() != SWARM_MODEL_FILES.len() {
        return Err(UpdateError::BadVersion);
    }
    let mut seen = [false; SWARM_MODEL_FILES.len()];
    for file in &manifest.files {
        let Some(index) = SWARM_MODEL_FILES.iter().position(|name| *name == file.name) else {
            return Err(UpdateError::BadVersion);
        };
        if seen[index] || !sha_ok(&file.sha256) {
            return Err(UpdateError::BadVersion);
        }
        seen[index] = true;
    }
    if seen.iter().any(|flag| !flag) {
        return Err(UpdateError::BadVersion);
    }
    let public = decode_key(public_hex).ok_or(UpdateError::BadKey)?;
    let verifying = VerifyingKey::from_bytes(&public).map_err(|_| UpdateError::BadKey)?;
    let signature = decode_signature(&manifest.signature).ok_or(UpdateError::BadSignature)?;
    verifying
        .verify(&model_message(&manifest.name, manifest.dims, &manifest.files), &signature)
        .map_err(|_| UpdateError::BadSignature)
}

/// Подпись и состав манифеста, без чтения самих весов.
pub fn read_model_manifest(dir: &Path) -> Result<ModelManifest, UpdateError> {
    let bytes = fs::read(dir.join("manifest.json"))?;
    let manifest: ModelManifest = serde_json::from_slice(&bytes)?;
    verify_model(&manifest)?;
    Ok(manifest)
}

/// Подпись и контрольные суммы файлов на диске.
pub fn load_model_manifest(dir: &Path) -> Result<ModelManifest, UpdateError> {
    let manifest = read_model_manifest(dir)?;
    for file in &manifest.files {
        if !model_file_ready(dir, file) {
            return Err(UpdateError::HashMismatch);
        }
    }
    Ok(manifest)
}

pub fn model_file_ready(dir: &Path, file: &ModelFile) -> bool {
    if !SWARM_MODEL_FILES.contains(&file.name.as_str()) {
        return false;
    }
    match hash_path(&dir.join(&file.name)) {
        Ok((sha, size)) => size == file.size && sha.eq_ignore_ascii_case(&file.sha256),
        Err(_) => false,
    }
}

fn model_message(name: &str, dims: u32, files: &[ModelFile]) -> Vec<u8> {
    let mut ordered: Vec<&ModelFile> = files.iter().collect();
    ordered.sort_by(|left, right| left.name.cmp(&right.name));
    let mut text = format!("mds-model-v1\n{name}\n{dims}\n");
    for file in ordered {
        text.push_str(&format!(
            "{} {} {}\n",
            file.name,
            file.sha256.to_ascii_lowercase(),
            file.size
        ));
    }
    text.into_bytes()
}

fn model_name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_')
}

fn sha_ok(text: &str) -> bool {
    text.len() == 64 && text.chars().all(|ch| ch.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate_keypair;

    #[test]
    fn signed_model_rejects_a_swapped_file() {
        let dir = std::env::temp_dir().join(format!("mds-model-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for name in SWARM_MODEL_FILES {
            fs::write(dir.join(name), format!("вес {name}")).unwrap();
        }
        let (secret, public) = generate_keypair();
        let manifest = sign_model(&secret, &dir, "multilingual-e5-small").unwrap();
        verify_model_with(&manifest, &public).unwrap();
        assert!(model_file_ready(&dir, &manifest.files[0]));
        fs::write(dir.join("model.onnx"), b"other").unwrap();
        assert!(!model_file_ready(&dir, &manifest.files[0]));
        let mut broken = manifest.clone();
        broken.files[0].sha256 = "ab".repeat(32);
        assert!(verify_model_with(&broken, &public).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
