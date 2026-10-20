use std::path::Path;
use std::time::Duration;

use mds_update::{
    host_os, load_newest, place_verified, running_hash, save_notice, save_release, should_replace, stage_and_relaunch,
    verify_pinned, Placed, ReleaseManifest, VERSION,
};

use crate::NodeError;

pub async fn watch(peers: Vec<String>, releases: std::path::PathBuf) {
    loop {
        if let Err(err) = check_once(&peers, &releases).await {
            eprintln!("MDS: обновление не установлено: {err}");
        }
        tokio::time::sleep(Duration::from_secs(6 * 60 * 60)).await;
    }
}

async fn check_once(peers: &[String], releases: &Path) -> Result<(), NodeError> {
    if let Some((manifest, bytes)) = load_newest(releases)? {
        if should_replace(VERSION, &running_hash(), &manifest) {
            let _ = save_notice(releases, &manifest.version);
            announce(peers, &manifest.version).await;
        }
        try_apply(&manifest, &bytes)?;
    }
    if peers.is_empty() {
        return Ok(());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(mds_crawl::USER_AGENT)
        .build()
        .map_err(|err| NodeError::Message(err.to_string()))?;
    for peer in peers {
        let Some(base) = peer_base(peer) else {
            eprintln!("MDS: адрес ноды пропущен, нужен http: {peer}");
            continue;
        };
        let response = match client.get(format!("{base}/peer/update")).send().await {
            Ok(response) if response.status().is_success() => response,
            _ => continue,
        };
        let body = response
            .bytes()
            .await
            .map_err(|err| NodeError::Message(err.to_string()))?;
        let Ok(manifest) = serde_json::from_slice::<ReleaseManifest>(&body) else {
            continue;
        };
        if !should_replace(VERSION, &running_hash(), &manifest) {
            continue;
        }
        let _ = save_notice(releases, &manifest.version);
        announce(peers, &manifest.version).await;
        let blob = match client.get(format!("{base}/peer/update.bin")).send().await {
            Ok(response) if response.status().is_success() => response,
            _ => continue,
        };
        let bytes = blob
            .bytes()
            .await
            .map_err(|err| NodeError::Message(err.to_string()))?;
        if verify_pinned(&manifest, &bytes).is_err() {
            eprintln!("MDS: нода {base} прислала файл с неверной подписью");
            continue;
        }
        save_release(releases, &manifest, &bytes)?;
        try_apply(&manifest, &bytes)?;
    }
    Ok(())
}

pub async fn announce(peers: &[String], version: &str) {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(mds_crawl::USER_AGENT)
        .build()
    else {
        return;
    };
    for peer in peers {
        let Some(base) = peer_base(peer) else {
            continue;
        };
        let _ = client
            .post(format!("{base}/peer/notice"))
            .header("content-type", "application/json")
            .body(format!("\"{version}\""))
            .send()
            .await;
    }
}

fn try_apply(manifest: &ReleaseManifest, bytes: &[u8]) -> Result<(), NodeError> {
    if !should_replace(VERSION, &running_hash(), manifest) {
        return Ok(());
    }
    verify_pinned(manifest, bytes)?;
    eprintln!(
        "MDS: версия {} подписана верно, нода перезапускается",
        manifest.version
    );
    stage_and_relaunch(bytes)?;
    std::process::exit(0);
}

pub enum Pull {
    Current(String),
    Installed { version: String, placed: Placed },
}

pub async fn pull_release(hive: &str) -> Result<Pull, NodeError> {
    let hive = hive.trim().trim_end_matches('/');
    if !hive.starts_with("http://") && !hive.starts_with("https://") {
        return Err(NodeError::Message("улью нужен адрес http".into()));
    }
    let os = host_os().ok_or_else(|| NodeError::Message("для этой системы пакета нет".into()))?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent(mds_crawl::USER_AGENT)
        .build()
        .map_err(|err| NodeError::Message(err.to_string()))?;
    let response = client
        .get(format!("{hive}/peer/update"))
        .query(&[("os", os)])
        .send()
        .await
        .map_err(|err| NodeError::Message(err.to_string()))?;
    if !response.status().is_success() {
        return Err(NodeError::Message("улей не отдал описание версии".into()));
    }
    let body = response
        .bytes()
        .await
        .map_err(|err| NodeError::Message(err.to_string()))?;
    let manifest: ReleaseManifest = serde_json::from_slice(&body).map_err(|err| NodeError::Message(err.to_string()))?;
    if !should_replace(VERSION, &running_hash(), &manifest) {
        return Ok(Pull::Current(VERSION.to_string()));
    }
    let blob = client
        .get(format!("{hive}/peer/update.bin"))
        .query(&[("os", os)])
        .send()
        .await
        .map_err(|err| NodeError::Message(err.to_string()))?;
    if !blob.status().is_success() {
        return Err(NodeError::Message("улей не отдал файл версии".into()));
    }
    let bytes = blob
        .bytes()
        .await
        .map_err(|err| NodeError::Message(err.to_string()))?;
    verify_pinned(&manifest, &bytes)?;
    let placed = place_verified(&bytes)?;
    Ok(Pull::Installed {
        version: manifest.version,
        placed,
    })
}

fn peer_base(peer: &str) -> Option<String> {
    let peer = peer.trim().trim_end_matches('/');
    if peer.starts_with("http://") || peer.starts_with("https://") {
        Some(peer.to_string())
    } else {
        None
    }
}
