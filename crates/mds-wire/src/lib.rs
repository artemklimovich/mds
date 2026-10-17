use mds_types::{Hit, QuantVector};
use serde::{Deserialize, Serialize};

pub mod network;
pub mod v1;

pub use network::{HiveDoor, NetworkMap, NodeKey, PlaceEntry, PlaceRef, ShardNote, PROTOCOL_VERSION};
pub use v1::{
    search_compatible, SearchAnswerV1, SearchQueryV1, ShardAckV1, ShardCopy, ShardDeltaV1,
    ShardManifestV1, WireEnvelope,
};

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
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub distance_km: Option<f32>,
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default)]
    pub lon: Option<f64>,
    #[serde(default)]
    pub kind: u32,
    #[serde(default)]
    pub media_url: Option<String>,
    #[serde(default)]
    pub feed_url: Option<String>,
    #[serde(default)]
    pub news_feed: Option<String>,
    #[serde(default)]
    pub price: Option<String>,
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
            region: hit.region,
            place: hit.place,
            distance_km: hit.distance_km,
            lat: hit.lat,
            lon: hit.lon,
            kind: hit.kind,
            media_url: hit.media_url,
            feed_url: hit.feed_url,
            news_feed: hit.news_feed,
            price: hit.price,
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
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrawlWork {
    pub urls: Vec<String>,
    /// Сколько секунд приложению ждать до следующего захода. Старые приложения поле пропускают.
    #[serde(default)]
    pub pause_secs: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushedPage {
    pub url: String,
    pub title: String,
    pub host: String,
    pub text: String,
    /// До 20 абсолютных ссылок того же сайта. Старая пчела поле не шлёт.
    #[serde(default)]
    pub links: Vec<String>,
    /// Отпечаток полного текста до обрезки. Ноль значит, что его не прислали.
    #[serde(default)]
    pub simhash: u64,
    /// Слова ссылки и адрес, на который она ведёт. Старая пчела поле не шлёт.
    #[serde(default)]
    pub anchors: Vec<PushedAnchor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushedAnchor {
    pub url: String,
    #[serde(default)]
    pub text: String,
}

/// Страница в снимке шарда. Вектор считает улей, который её принимает.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardLeaf {
    pub url: String,
    pub host: String,
    pub title: String,
    pub text: String,
    pub simhash: u64,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardBlockV1 {
    pub from_revision: u64,
    pub to_revision: u64,
    pub pages: Vec<ShardLeaf>,
    pub block_sha256: String,
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
