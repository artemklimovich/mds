use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use mds_update::open_wire;
use mds_wire::{search_compatible, NetworkMap, WireEnvelope, PROTOCOL_VERSION};

pub struct Nonces {
    seen: Mutex<HashMap<String, u64>>,
}

impl Default for Nonces {
    fn default() -> Self {
        Self {
            seen: Mutex::new(HashMap::new()),
        }
    }
}

impl Nonces {
    pub fn accept(
        &self,
        envelope: &WireEnvelope,
        body: &[u8],
        map: &NetworkMap,
        model: &str,
        index_generation: u64,
    ) -> bool {
        if envelope.protocol_version != PROTOCOL_VERSION || !envelope.version_ok() {
            return false;
        }
        if envelope.index_generation != index_generation {
            return false;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|span| span.as_secs())
            .unwrap_or(0);
        if envelope.sent_at.abs_diff(now) > 300 {
            return false;
        }
        if !search_compatible(model, &envelope.model_generation) {
            return false;
        }
        let Some(key) = map.key_of(&envelope.node_id) else {
            return false;
        };
        if key.role != "full" {
            return false;
        }
        if open_wire(envelope, body, &key.public_hex).is_err() {
            return false;
        }
        let mut seen = self.seen.lock().unwrap_or_else(|poison| poison.into_inner());
        seen.retain(|_, at| now.saturating_sub(*at) < 600);
        if seen.contains_key(&envelope.nonce) {
            return false;
        }
        seen.insert(envelope.nonce.clone(), now);
        true
    }
}

pub fn pack_block(bytes: &[u8], cap: usize) -> Option<Vec<u8>> {
    if bytes.is_empty() || bytes.len() > cap {
        return None;
    }
    let packed = zstd::encode_all(std::io::Cursor::new(bytes), 3).ok()?;
    if packed.len() > cap {
        return None;
    }
    Some(packed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_block_roundtrip_and_cap() {
        let packed = pack_block("страница".as_bytes(), 1024).unwrap();
        assert_eq!(unpack_block(&packed, 1024).unwrap(), "страница".as_bytes());
        assert!(pack_block("страница".as_bytes(), 4).is_none());
    }
}

pub fn unpack_block(bytes: &[u8], cap: usize) -> Option<Vec<u8>> {
    if bytes.len() > cap {
        return None;
    }
    let plain = zstd::decode_all(std::io::Cursor::new(bytes)).ok()?;
    if plain.len() > cap {
        return None;
    }
    Some(plain)
}
