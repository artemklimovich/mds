use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceRef {
    pub country: String,
    pub region: String,
    pub city: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiveDoor {
    pub base: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardNote {
    pub shard: String,
    pub holders: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceEntry {
    pub place: PlaceRef,
    pub hives: Vec<HiveDoor>,
    pub shards: Vec<ShardNote>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeKey {
    pub node_id: String,
    pub public_hex: String,
    pub role: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkMap {
    pub version: u64,
    pub places: Vec<PlaceEntry>,
    pub keys: Vec<NodeKey>,
    pub signature: String,
}

impl NetworkMap {
    pub fn hives_for(&self, region: &str) -> Vec<&str> {
        let want = fold(region);
        self.places
            .iter()
            .find(|entry| {
                let names = [
                    entry.place.region.as_str(),
                    entry.place.city.as_str(),
                    entry.place.country.as_str(),
                ];
                names.iter().any(|name| fold(name) == want)
            })
            .map(|entry| {
                entry
                    .hives
                    .iter()
                    .take(3)
                    .map(|door| door.base.as_str())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn hive_for(&self, region: &str) -> Option<&str> {
        self.hives_for(region).into_iter().next()
    }

    pub fn key_of(&self, node_id: &str) -> Option<&NodeKey> {
        self.keys.iter().find(|key| key.node_id == node_id)
    }
}

fn fold(value: &str) -> String {
    value.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hive_is_found_by_region_or_city() {
        let map = NetworkMap {
            version: 1,
            places: vec![PlaceEntry {
                place: PlaceRef {
                    country: "Россия".into(),
                    region: "Татарстан".into(),
                    city: "Казань".into(),
                },
                hives: vec![HiveDoor {
                    base: "https://kazan.example".into(),
                }],
                shards: Vec::new(),
            }],
            keys: Vec::new(),
            signature: String::new(),
        };
        assert_eq!(map.hive_for("Татарстан"), Some("https://kazan.example"));
        assert_eq!(map.hive_for("казань"), Some("https://kazan.example"));
        assert_eq!(map.hive_for("Москва"), None);
    }
}
