use mds_types::{Hit, QuantVector};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchRequest {
    pub text: String,
    pub vector: QuantVector,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub score: f32,
    pub via_words: bool,
    pub via_meaning: bool,
    #[serde(default)]
    pub adult: bool,
    #[serde(default)]
    pub volunteer: bool,
}

impl From<Hit> for SearchHit {
    fn from(hit: Hit) -> Self {
        Self {
            url: hit.url,
            title: hit.title,
            snippet: hit.snippet,
            score: hit.score,
            via_words: hit.via_words,
            via_meaning: hit.via_meaning,
            adult: hit.adult,
            volunteer: hit.volunteer,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardAnnounce {
    pub domain_hash: [u8; 16],
    pub semantic_bucket: u16,
    pub pages: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerHello {
    pub base: String,
    pub label: String,
    pub buckets: Vec<u16>,
    pub holds_all: bool,
    pub pages: u32,
    pub queue: u32,
    #[serde(default)]
    pub site: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerStatus {
    pub pages: u64,
    pub queue: u64,
    pub buckets: Vec<u16>,
    pub holds_all: bool,
    pub searches_per_minute: u64,
    pub avg_ms: u64,
    pub db_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrawlWork {
    pub urls: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushedPage {
    pub url: String,
    pub title: String,
    pub host: String,
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use mds_types::EMBED_DIMS;

    #[test]
    fn request_roundtrip() {
        let request = SearchRequest {
            text: "как лечить собаку".into(),
            vector: QuantVector {
                scale: 0.5,
                values: vec![1_i8; EMBED_DIMS],
            },
        };
        let bytes = serde_json::to_vec(&request).unwrap();
        let back: SearchRequest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back, request);
    }
}
