use std::path::{Path, PathBuf};

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use mds_types::EMBED_DIMS;
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
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::MultilingualE5Small)
                .with_cache_dir(cache_dir.to_path_buf())
                .with_show_download_progress(true),
        )
        .map_err(|err| EmbedError::Model(err.to_string()))?;
        Ok(Self { model })
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

fn locate_runtime(cache_dir: &Path) -> Result<(), EmbedError> {
    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        return Ok(());
    }
    let mut candidates = vec![
        cache_dir.join("onnxruntime.dll"),
        PathBuf::from("data/onnx/onnxruntime.dll"),
    ];
    if let Some(parent) = cache_dir.parent() {
        candidates.push(parent.join("onnx").join("onnxruntime.dll"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("onnxruntime.dll"));
        }
    }
    let Some(path) = candidates.into_iter().find(|path| path.exists()) else {
        return Err(EmbedError::Model(
            "нет onnxruntime.dll. Положите сборку 1.20 в data/onnx или задайте ORT_DYLIB_PATH".into(),
        ));
    };
    std::env::set_var("ORT_DYLIB_PATH", &path);
    Ok(())
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
