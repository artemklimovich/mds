use std::path::{Path, PathBuf};

use fastembed::{
    EmbeddingModel, InitOptions, InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles,
    UserDefinedEmbeddingModel,
};
use mds_types::EMBED_DIMS;
use mds_update::{load_model_manifest, ModelManifest};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmbedError {
    #[error("модель эмбеддингов: {0}")]
    Model(String),
    #[error("у вектора {got} измерений, нужно {EMBED_DIMS}")]
    Dims { got: usize },
    #[error("модель не вернула вектор")]
    Empty,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub struct Embedder {
    model: TextEmbedding,
}

impl Embedder {
    pub fn open(cache_dir: &Path) -> Result<Self, EmbedError> {
        std::fs::create_dir_all(cache_dir)?;
        locate_runtime(cache_dir)?;
        let swarm = cache_dir.join("swarm");
        if swarm.join("manifest.json").is_file() {
            let manifest = load_model_manifest(&swarm).map_err(|err| {
                EmbedError::Model(format!(
                    "модель роя в {} не сошлась с подписью: {err}",
                    swarm.display()
                ))
            })?;
            return open_swarm(&swarm, &manifest);
        }
        if legacy_onnx(cache_dir) {
            return open_legacy(cache_dir);
        }
        Err(EmbedError::Model(
            "нет модели роя. Полная нода забирает её с улья в data/models/swarm. Hugging Face для новой ноды не используется".into(),
        ))
    }

    pub fn embed_query(&mut self, text: &str) -> Result<Vec<f32>, EmbedError> {
        self.embed(&format!("query: {}", clip(text)))
    }

    pub fn embed_passage(&mut self, text: &str) -> Result<Vec<f32>, EmbedError> {
        self.embed(&format!("passage: {}", clip(text)))
    }

    pub fn embed_passages(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let inputs: Vec<String> = texts
            .iter()
            .map(|text| format!("passage: {}", clip(text)))
            .collect();
        let vectors = self
            .model
            .embed(inputs, Some(16))
            .map_err(|err| EmbedError::Model(err.to_string()))?;
        for vector in &vectors {
            if vector.len() != EMBED_DIMS {
                return Err(EmbedError::Dims { got: vector.len() });
            }
        }
        Ok(vectors)
    }

    fn embed(&mut self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let mut vectors = self
            .model
            .embed(vec![text], None)
            .map_err(|err| EmbedError::Model(err.to_string()))?;
        let vector = vectors.pop().ok_or(EmbedError::Empty)?;
        if vector.len() != EMBED_DIMS {
            return Err(EmbedError::Dims { got: vector.len() });
        }
        Ok(vector)
    }
}

fn open_swarm(dir: &Path, manifest: &ModelManifest) -> Result<Embedder, EmbedError> {
    let read = |name: &str| -> Result<Vec<u8>, EmbedError> {
        if !manifest.files.iter().any(|file| file.name == name) {
            return Err(EmbedError::Model(format!("в манифесте нет {name}")));
        }
        std::fs::read(dir.join(name)).map_err(EmbedError::from)
    };
    let model = UserDefinedEmbeddingModel::new(
        read("model.onnx")?,
        TokenizerFiles {
            tokenizer_file: read("tokenizer.json")?,
            config_file: read("config.json")?,
            special_tokens_map_file: read("special_tokens_map.json")?,
            tokenizer_config_file: read("tokenizer_config.json")?,
        },
    )
    .with_pooling(Pooling::Mean);
    let model = TextEmbedding::try_new_from_user_defined(model, InitOptionsUserDefined::default())
        .map_err(|err| EmbedError::Model(err.to_string()))?;
    Ok(Embedder { model })
}

fn open_legacy(cache_dir: &Path) -> Result<Embedder, EmbedError> {
    let previous = std::env::var_os("HF_HUB_OFFLINE");
    std::env::set_var("HF_HUB_OFFLINE", "1");
    let model = TextEmbedding::try_new(
        InitOptions::new(EmbeddingModel::MultilingualE5Small)
            .with_cache_dir(cache_dir.to_path_buf())
            .with_show_download_progress(false),
    );
    match previous {
        Some(value) => std::env::set_var("HF_HUB_OFFLINE", value),
        None => std::env::remove_var("HF_HUB_OFFLINE"),
    }
    let model = model.map_err(|err| EmbedError::Model(err.to_string()))?;
    Ok(Embedder { model })
}

fn legacy_onnx(cache_dir: &Path) -> bool {
    let mut stack = vec![cache_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name().and_then(|name| name.to_str()) == Some("swarm") {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.file_name().and_then(|name| name.to_str()) == Some("model.onnx")
                && path.metadata().map(|meta| meta.len() > 1_000_000).unwrap_or(false)
            {
                return true;
            }
        }
    }
    false
}

fn locate_runtime(cache_dir: &Path) -> Result<(), EmbedError> {
    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        return Ok(());
    }
    let mut candidates = vec![
        cache_dir.join("onnxruntime.dll"),
        cache_dir.join("libonnxruntime.so"),
        PathBuf::from("data/onnx/onnxruntime.dll"),
        PathBuf::from("data/onnx/libonnxruntime.so"),
    ];
    if let Some(parent) = cache_dir.parent() {
        candidates.push(parent.join("onnx").join("onnxruntime.dll"));
        candidates.push(parent.join("onnx").join("libonnxruntime.so"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("onnxruntime.dll"));
            candidates.push(dir.join("libonnxruntime.so"));
        }
    }
    let Some(path) = candidates.into_iter().find(|path| path.exists()) else {
        return Err(EmbedError::Model(
            "нет библиотеки модели. Положите onnxruntime.dll или libonnxruntime.so сборки 1.20 рядом с программой или задайте ORT_DYLIB_PATH".into(),
        ));
    };
    std::env::set_var("ORT_DYLIB_PATH", &path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_directory_does_not_fetch_a_model() {
        let dir = std::env::temp_dir().join(format!("mds-embed-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let err = match Embedder::open(&dir) {
            Ok(_) => panic!("пустой каталог не должен открыть модель"),
            Err(err) => err,
        };
        let text = err.to_string();
        assert!(
            text.contains("улья") || text.contains("библиотеки модели"),
            "{text}"
        );
        assert!(!dir.join("swarm").join("model.onnx").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

fn clip(text: &str) -> &str {
    const MAX: usize = 1200;
    if text.len() <= MAX {
        return text;
    }
    let mut end = MAX;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].trim()
}
