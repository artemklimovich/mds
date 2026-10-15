use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const EMBED_DIMS: usize = 384;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VectorError {
    #[error("ожидалось {EMBED_DIMS} измерений, получено {0}")]
    BadDims(usize),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuantVector {
    pub scale: f32,
    pub values: Vec<i8>,
}

impl QuantVector {
    pub fn is_empty_meaning(&self) -> bool {
        self.values.iter().all(|v| *v == 0)
    }

    pub fn dequantize(&self) -> Vec<f32> {
        self.values
            .iter()
            .map(|v| (*v as f32) * self.scale / 127.0)
            .collect()
    }
}

pub fn quantize(values: &[f32]) -> Result<QuantVector, VectorError> {
    if values.len() != EMBED_DIMS {
        return Err(VectorError::BadDims(values.len()));
    }
    let max = values.iter().map(|v| v.abs()).fold(0.0_f32, f32::max);
    let scale = if max < 1e-8 { 1.0 } else { max };
    let quantized = values
        .iter()
        .map(|v| {
            let q = (v / scale * 127.0).round().clamp(-127.0, 127.0);
            q as i8
        })
        .collect();
    Ok(QuantVector {
        scale,
        values: quantized,
    })
}

pub fn cosine(query: &[f32], stored: &QuantVector) -> f32 {
    if query.len() != stored.values.len() || query.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0_f32;
    let mut left = 0.0_f32;
    let mut right = 0.0_f32;
    for (q, stored_q) in query.iter().zip(&stored.values) {
        let value = (*stored_q as f32) * stored.scale / 127.0;
        dot += q * value;
        left += q * q;
        right += value * value;
    }
    if left <= 1e-12 || right <= 1e-12 {
        return 0.0;
    }
    dot / (left.sqrt() * right.sqrt())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeId(pub [u8; 16]);

impl NodeId {
    pub fn local() -> Self {
        let hash = blake_like(b"mds-local-node");
        let mut bytes = [0_u8; 16];
        bytes.copy_from_slice(&hash[..16]);
        Self(bytes)
    }
}

fn blake_like(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0_u8; 32];
    let mut state: u64 = 0x6d6473;
    for (i, byte) in bytes.iter().enumerate() {
        state = state
            .wrapping_mul(0x100000001b3)
            .wrapping_add(*byte as u64)
            .wrapping_add(i as u64);
        out[i % 32] ^= state as u8;
        out[(i + 7) % 32] = out[(i + 7) % 32].wrapping_add((state >> 8) as u8);
    }
    out
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub score: f32,
    pub via_words: bool,
    pub via_meaning: bool,
    pub adult: bool,
    #[serde(default)]
    pub volunteer: bool,
}

#[derive(Clone, Debug)]
pub struct StoredPage {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub host: String,
    pub text: String,
}

pub fn snippet(text: &str, query: &str) -> String {
    let lower = text.to_lowercase();
    let needle = query
        .split(|c: char| !c.is_alphanumeric())
        .find(|token| token.chars().count() >= 3)
        .map(|token| token.to_lowercase());
    if let Some(needle) = needle {
        if let Some(pos) = lower.find(&needle) {
            return window(text, pos, 70, 150);
        }
    }
    truncate_chars(text, 220)
}

fn window(text: &str, byte_pos: usize, before: usize, after: usize) -> String {
    let start = floor_char(text, byte_pos.saturating_sub(before));
    let end = ceil_char(text, (byte_pos + after).min(text.len()));
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.push_str(text[start..end].trim());
    if end < text.len() {
        out.push('…');
    }
    out
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let end = trimmed
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(trimmed.len());
    format!("{}…", trimmed[..end].trim_end())
}

fn floor_char(text: &str, mut index: usize) -> usize {
    if index > text.len() {
        index = text.len();
    }
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char(text: &str, mut index: usize) -> usize {
    if index > text.len() {
        index = text.len();
    }
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_roundtrip_is_almost_parallel() {
        let mut values = vec![0.0_f32; EMBED_DIMS];
        for (i, value) in values.iter_mut().enumerate() {
            *value = ((i % 9) as f32 - 4.0) / 8.0;
        }
        let stored = quantize(&values).unwrap();
        let score = cosine(&values, &stored);
        assert!(score > 0.98, "cosine {score}");
    }

    #[test]
    fn quantize_rejects_wrong_size() {
        assert!(quantize(&[0.1, 0.2]).is_err());
    }

    #[test]
    fn snippet_finds_cyrillic_token() {
        let text = "Короткий абзац. Дальше крепёж MDSBOLT42 для каркаса склада и ещё текст.";
        let snippet = snippet(text, "MDSBOLT42");
        assert!(snippet.contains("MDSBOLT42"));
    }
}
