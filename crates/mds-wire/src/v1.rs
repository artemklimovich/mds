use serde::{Deserialize, Serialize};

use crate::network::PROTOCOL_VERSION;
use crate::SearchHit;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireEnvelope {
    pub protocol_version: u32,
    pub node_id: String,
    pub request_id: String,
    pub sent_at: u64,
    pub nonce: String,
    pub model_generation: String,
    pub index_generation: u64,
    pub kind: String,
    pub body_sha256: String,
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchQueryV1 {
    pub text: String,
    pub vector: Vec<i8>,
    pub scale: f32,
    pub tab: String,
    pub region: String,
    pub top_k: u16,
    pub hop_limit: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchAnswerV1 {
    pub hits: Vec<SearchHit>,
    pub compatible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardManifestV1 {
    pub shard: String,
    pub generation: u64,
    pub revision: u64,
    pub pages: u64,
    pub root_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardDeltaV1 {
    pub shard: String,
    pub generation: u64,
    pub from_revision: u64,
    pub to_revision: u64,
    pub block_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardAckV1 {
    pub shard: String,
    pub generation: u64,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShardCopy {
    Active,
    CatchingUp,
    Stale,
}

impl WireEnvelope {
    pub fn version_ok(&self) -> bool {
        self.protocol_version == PROTOCOL_VERSION
    }
}

pub fn search_compatible(local_model: &str, offered_model: &str) -> bool {
    !local_model.is_empty() && local_model == offered_model
}
