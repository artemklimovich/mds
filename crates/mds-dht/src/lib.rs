use mds_types::{NodeId, EMBED_DIMS};

pub const REPLICATION: usize = 3;
const PLANES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShardKey {
    pub domain_hash: [u8; 16],
    pub semantic_bucket: u16,
}

#[derive(Clone, Debug)]
pub struct Lsh {
    planes: [[f32; EMBED_DIMS]; PLANES],
}

impl Lsh {
    pub fn new() -> Self {
        let mut state = 0x4D445300_u32;
        let mut planes = [[0.0_f32; EMBED_DIMS]; PLANES];
        for plane in &mut planes {
            for value in plane.iter_mut() {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                *value = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
            }
        }
        Self { planes }
    }

    pub fn bucket(&self, vector: &[f32]) -> u16 {
        let mut bits = 0_u16;
        for (i, plane) in self.planes.iter().enumerate() {
            let dot: f32 = plane
                .iter()
                .zip(vector.iter().chain(std::iter::repeat(&0.0)))
                .take(EMBED_DIMS)
                .map(|(p, v)| p * v)
                .sum();
            if dot >= 0.0 {
                bits |= 1 << i;
            }
        }
        bits
    }
}

impl Default for Lsh {
    fn default() -> Self {
        Self::new()
    }
}

pub fn host_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.trim().to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

pub fn domain_hash(host: &str) -> [u8; 16] {
    let host = host.trim().to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let digest = blake3::hash(host.as_bytes());
    let mut out = [0_u8; 16];
    out.copy_from_slice(&digest.as_bytes()[..16]);
    out
}

pub fn shard_key(host: &str, vector: &[f32], lsh: &Lsh) -> ShardKey {
    ShardKey {
        domain_hash: domain_hash(host),
        semantic_bucket: lsh.bucket(vector),
    }
}

#[derive(Clone, Debug)]
pub struct LocalDirectory {
    node: NodeId,
}

impl LocalDirectory {
    pub fn single() -> Self {
        Self {
            node: NodeId::local(),
        }
    }

    pub fn locate(&self, _key: &ShardKey) -> Vec<NodeId> {
        vec![self.node.clone()]
    }

    pub fn missing_replicas(&self, key: &ShardKey) -> usize {
        REPLICATION.saturating_sub(self.locate(key).len())
    }
}

#[derive(Clone, Debug)]
pub struct PeerCard {
    pub base: String,
    pub label: String,
    pub buckets: Vec<u16>,
    pub holds_all: bool,
    pub latency_ms: u64,
    pub alive: bool,
    pub reachable: bool,
    pub pages: u32,
    pub queue: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Swarm {
    peers: Vec<PeerCard>,
}

impl Swarm {
    pub fn upsert(&mut self, mut card: PeerCard) {
        card.base = normalize_base(&card.base);
        let key = if card.base.is_empty() {
            card.label.clone()
        } else {
            card.base.clone()
        };
        if key.is_empty() {
            return;
        }
        if let Some(existing) = self.peers.iter_mut().find(|peer| peer_key(peer) == key) {
            existing.buckets = card.buckets;
            existing.holds_all = card.holds_all;
            existing.pages = card.pages;
            existing.queue = card.queue;
            existing.reachable = card.reachable;
            existing.alive = card.alive;
            if !card.label.is_empty() {
                existing.label = card.label;
            }
            if card.latency_ms > 0 {
                existing.latency_ms = card.latency_ms;
            }
            return;
        }
        self.peers.push(card);
    }

    pub fn note_latency(&mut self, base: &str, latency_ms: u64, alive: bool) {
        let base = normalize_base(base);
        if let Some(peer) = self.peers.iter_mut().find(|peer| peer.base == base) {
            peer.latency_ms = latency_ms;
            peer.alive = alive;
        }
    }

    /// Кого спросить по бакету: не больше трёх живых держателей, сначала тех, кто отвечал быстрее.
    /// Медленная реплика (дольше самой быстрой втрое и больше чем на 400 мс) в этот круг не входит,
    /// если есть хотя бы ещё одна живая. Пока никто не объявил бакет, спрашиваются все доступные соседи.
    pub fn ask(&self, bucket: u16) -> Vec<String> {
        let reachable: Vec<&PeerCard> = self
            .peers
            .iter()
            .filter(|peer| peer.reachable && !peer.base.is_empty())
            .collect();
        let announced = reachable
            .iter()
            .any(|peer| peer.holds_all || peer.buckets.contains(&bucket));
        let mut chosen: Vec<&PeerCard> = reachable
            .into_iter()
            .filter(|peer| {
                if announced {
                    peer.holds_all || peer.buckets.contains(&bucket)
                } else {
                    true
                }
            })
            .collect();
        chosen.sort_by_key(|peer| (!peer.alive, peer.latency_ms));
        if let Some(fast) = chosen.iter().find(|peer| peer.alive).map(|peer| peer.latency_ms) {
            let cutoff = fast.saturating_mul(3).max(fast.saturating_add(400));
            let alive = chosen.iter().filter(|peer| peer.alive).count();
            if alive > 1 {
                chosen.retain(|peer| !peer.alive || peer.latency_ms <= cutoff);
            }
        }
        chosen
            .into_iter()
            .take(REPLICATION)
            .map(|peer| peer.base.clone())
            .collect()
    }

    pub fn cards(&self) -> &[PeerCard] {
        &self.peers
    }
}

fn peer_key(peer: &PeerCard) -> String {
    if peer.base.is_empty() {
        peer.label.clone()
    } else {
        peer.base.clone()
    }
}

fn normalize_base(base: &str) -> String {
    base.trim().trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mds_types::EMBED_DIMS;

    #[test]
    fn bucket_is_stable() {
        let lsh = Lsh::new();
        let mut vector = vec![0.0_f32; EMBED_DIMS];
        for (i, value) in vector.iter_mut().enumerate() {
            *value = if i % 2 == 0 { 0.2 } else { -0.1 };
        }
        assert_eq!(lsh.bucket(&vector), lsh.bucket(&vector));
    }

    #[test]
    fn opposite_vector_changes_bucket() {
        let lsh = Lsh::new();
        let vector = vec![0.3_f32; EMBED_DIMS];
        let opposite: Vec<f32> = vector.iter().map(|v| -v).collect();
        assert_ne!(lsh.bucket(&vector), lsh.bucket(&opposite));
    }

    #[test]
    fn www_host_hashes_like_bare_host() {
        assert_eq!(domain_hash("www.farm.example"), domain_hash("farm.example"));
        assert_eq!(
            host_of("https://www.farm.example/milk").as_deref(),
            Some("farm.example")
        );
    }

    #[test]
    fn single_node_keeps_replication_deficit() {
        let dir = LocalDirectory::single();
        let key = ShardKey {
            domain_hash: [1; 16],
            semantic_bucket: 7,
        };
        assert_eq!(dir.locate(&key).len(), 1);
        assert_eq!(dir.missing_replicas(&key), REPLICATION - 1);
        assert_eq!(REPLICATION, 3);
    }

    fn card(base: &str, buckets: &[u16], latency_ms: u64) -> PeerCard {
        PeerCard {
            base: base.into(),
            label: base.into(),
            buckets: buckets.to_vec(),
            holds_all: false,
            latency_ms,
            alive: true,
            reachable: true,
            pages: 1,
            queue: 0,
        }
    }

    #[test]
    fn ask_picks_fast_holders_of_the_bucket() {
        let mut swarm = Swarm::default();
        swarm.upsert(card("http://slow:8080", &[7, 8], 900));
        swarm.upsert(card("http://fast:8080", &[7], 40));
        swarm.upsert(card("http://other:8080", &[3], 20));
        swarm.upsert(card("http://mid:8080", &[7], 80));
        let asked = swarm.ask(7);
        assert_eq!(
            asked,
            vec![
                "http://fast:8080".to_string(),
                "http://mid:8080".to_string(),
            ]
        );
    }

    #[test]
    fn ask_falls_back_to_every_peer_before_announcements() {
        let mut swarm = Swarm::default();
        swarm.upsert(card("http://a:8080", &[], 10));
        swarm.upsert(card("http://b:8080", &[], 20));
        assert_eq!(swarm.ask(7).len(), 2);
    }
}
