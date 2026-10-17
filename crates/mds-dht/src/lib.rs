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

/// Три разные машины одного края. Если машин меньше трёх, возвращаются все.
pub fn assign_holders(nodes: &[NodeId], key: &ShardKey) -> Vec<NodeId> {
    if nodes.is_empty() {
        return Vec::new();
    }
    let mut ranked: Vec<(String, NodeId)> = nodes
        .iter()
        .map(|node| {
            let mut hasher = blake3::Hasher::new();
            hasher.update(&node.0);
            hasher.update(&key.domain_hash);
            hasher.update(&key.semantic_bucket.to_le_bytes());
            (hasher.finalize().to_hex().to_string(), node.clone())
        })
        .collect();
    ranked.sort_by(|left, right| left.0.cmp(&right.0));
    ranked.dedup_by(|left, right| left.1.0 == right.1.0);
    ranked.into_iter().take(REPLICATION).map(|(_, node)| node).collect()
}

/// Имя, которому принадлежит домен, когда ульев несколько.
/// Пока список соседей пуст, домен никому отдельно не отдан.
pub fn keeps_domain(self_name: &str, peers: &[String], host: &str) -> bool {
    let self_name = normalize_base(self_name);
    if self_name.is_empty() {
        return true;
    }
    let mut names = vec![self_name.clone()];
    for peer in peers {
        let peer = normalize_base(peer);
        if peer.is_empty() || names.iter().any(|item| item == &peer) {
            continue;
        }
        names.push(peer);
    }
    if names.len() < 2 {
        return true;
    }
    domain_owner(&names, host).is_some_and(|owner| owner == &self_name)
}

fn domain_owner<'a>(names: &'a [String], host: &str) -> Option<&'a String> {
    let hash = domain_hash(host);
    let mut best: Option<(&String, String)> = None;
    for name in names {
        let mut hasher = blake3::Hasher::new();
        hasher.update(name.as_bytes());
        hasher.update(&hash);
        let score = hasher.finalize().to_hex().to_string();
        match &best {
            Some((_, have)) if score.as_str() >= have.as_str() => {}
            _ => best = Some((name, score)),
        }
    }
    best.map(|(name, _)| name)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyState {
    Active,
    CatchingUp,
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShardCopy {
    pub node_id: String,
    pub generation: u64,
    pub revision: u64,
    pub root_sha256: String,
    pub state: CopyState,
}

#[derive(Clone, Debug, Default)]
pub struct ShardBook {
    copies: Vec<ShardCopy>,
}

impl ShardBook {
    pub fn known(&self, node_id: &str) -> bool {
        self.copies.iter().any(|item| item.node_id == node_id)
    }

    pub fn note(&mut self, copy: ShardCopy) {
        if let Some(existing) = self.copies.iter_mut().find(|item| item.node_id == copy.node_id) {
            *existing = copy;
        } else {
            self.copies.push(copy);
        }
    }

    pub fn apply_delta(
        &mut self,
        node_id: &str,
        generation: u64,
        from_revision: u64,
        to_revision: u64,
        block_sha256: &str,
        expected_sha256: &str,
    ) -> bool {
        if block_sha256 != expected_sha256 || to_revision <= from_revision {
            return false;
        }
        let Some(copy) = self.copies.iter_mut().find(|item| item.node_id == node_id) else {
            return false;
        };
        if copy.generation != generation || copy.revision != from_revision {
            return false;
        }
        copy.revision = to_revision;
        copy.root_sha256 = expected_sha256.to_string();
        copy.state = CopyState::Active;
        true
    }

    pub fn active(&self) -> Vec<&ShardCopy> {
        self.copies
            .iter()
            .filter(|copy| copy.state == CopyState::Active)
            .collect()
    }
}

/// Активный набор: несколько ключевых, ульи своего края, другие копии своих шардов и дальние соседи.
pub fn neighbor_set(
    bootstrap: &[String],
    regional: &[String],
    shard_peers: &[String],
    distant: &[String],
) -> Vec<String> {
    let mut out = Vec::new();
    push_unique(&mut out, bootstrap, 3);
    push_unique(&mut out, regional, 3);
    push_unique(&mut out, shard_peers, 6);
    push_unique(&mut out, distant, 4);
    out
}

fn push_unique(out: &mut Vec<String>, source: &[String], limit: usize) {
    let mut taken = 0;
    for item in source {
        if taken >= limit {
            break;
        }
        if item.is_empty() || out.iter().any(|have| have == item) {
            continue;
        }
        out.push(item.clone());
        taken += 1;
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
    fn one_hive_keeps_every_domain_and_two_hives_split_them() {
        assert!(keeps_domain("https://a.example", &[], "shop.example"));
        let peers = vec!["https://b.example".into()];
        let mut only_a = 0;
        let mut only_b = 0;
        for n in 0..40 {
            let host = format!("site{n}.example");
            let a = keeps_domain("https://a.example", &peers, &host);
            let b = keeps_domain("https://b.example", &["https://a.example".into()], &host);
            assert!(a ^ b, "{host}");
            if a {
                only_a += 1;
            } else {
                only_b += 1;
            }
        }
        assert!(only_a > 0 && only_b > 0);
    }

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
    fn three_holders_are_stable_and_distinct() {
        let nodes: Vec<NodeId> = (0..8)
            .map(|n| NodeId([n, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]))
            .collect();
        let key = ShardKey {
            domain_hash: [9; 16],
            semantic_bucket: 4,
        };
        let first = assign_holders(&nodes, &key);
        let again = assign_holders(&nodes, &key);
        assert_eq!(first.len(), 3);
        assert_eq!(first, again);
        let mut ids: Vec<[u8; 16]> = first.iter().map(|node| node.0).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn delta_moves_revision_only_when_the_hash_matches() {
        let mut book = ShardBook::default();
        book.note(ShardCopy {
            node_id: "node-a".into(),
            generation: 1,
            revision: 4,
            root_sha256: "old".into(),
            state: CopyState::CatchingUp,
        });
        assert!(!book.apply_delta("node-a", 1, 4, 5, "bad", "good"));
        assert!(book.apply_delta("node-a", 1, 4, 5, "good", "good"));
        assert_eq!(book.active()[0].revision, 5);
    }

    #[test]
    fn neighbor_set_stays_inside_the_budget() {
        let bootstrap = vec!["a".into(), "b".into(), "c".into(), "d".into()];
        let regional = vec!["b".into(), "e".into(), "f".into(), "g".into()];
        let shards = vec!["h".into(), "i".into()];
        let distant = vec!["j".into(), "k".into(), "l".into(), "m".into(), "n".into()];
        let set = neighbor_set(&bootstrap, &regional, &shards, &distant);
        assert!(set.len() <= 16);
        assert_eq!(set.iter().filter(|item| item.as_str() == "b").count(), 1);
        assert!(set.contains(&"a".to_string()));
        assert!(!set.contains(&"d".to_string()));
        assert!(!set.contains(&"n".to_string()));
    }

    #[test]
    fn ask_falls_back_to_every_peer_before_announcements() {
        let mut swarm = Swarm::default();
        swarm.upsert(card("http://a:8080", &[], 10));
        swarm.upsert(card("http://b:8080", &[], 20));
        assert_eq!(swarm.ask(7).len(), 2);
    }
}
