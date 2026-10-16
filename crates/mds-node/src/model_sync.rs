use std::io::Write;
use std::path::Path;
use std::time::Duration;

use mds_update::{hash_path, model_file_ready, verify_model, ModelFile, ModelManifest};

use crate::NodeError;

/// Полная нода забирает канонические веса с улья. Лёгкая пчела эту функцию не вызывает.
/// Несовпадение сумм не пересчитывает индекс: в журнал пишется, что старые векторы больше не годятся.
pub async fn sync_model(models: &Path, hive: Option<&str>) -> Result<(), NodeError> {
    let Some(hive) = hive.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let base = hive.trim_end_matches('/');
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .connect_timeout(Duration::from_secs(20))
        .user_agent(mds_crawl::USER_AGENT)
        .build()
        .map_err(|err| NodeError::Message(err.to_string()))?;
    let response = match client.get(format!("{base}/peer/model")).send().await {
        Ok(response) => response,
        Err(err) => {
            eprintln!("MDS: улей модель не отдал ({err}). Берём локальный каталог");
            return Ok(());
        }
    };
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        eprintln!("MDS: улей модель ещё не выложил. Берём локальный каталог");
        return Ok(());
    }
    if !response.status().is_success() {
        eprintln!(
            "MDS: улей ответил {} на запрос модели. Берём локальный каталог",
            response.status()
        );
        return Ok(());
    }
    let body = response
        .bytes()
        .await
        .map_err(|err| NodeError::Message(err.to_string()))?;
    let manifest: ModelManifest =
        serde_json::from_slice(&body).map_err(|err| NodeError::Message(err.to_string()))?;
    if let Err(err) = verify_model(&manifest) {
        eprintln!("MDS: подпись модели с улья не сошлась ({err}). Файл не записан");
        return Ok(());
    }
    let swarm = models.join("swarm");
    std::fs::create_dir_all(&swarm)?;
    let stamp_path = swarm.join("stamp");
    let old_stamp = std::fs::read_to_string(&stamp_path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    let mut pending = Vec::new();
    for file in &manifest.files {
        let dest = swarm.join(&file.name);
        let ready = tokio::task::spawn_blocking({
            let swarm_dir = swarm.clone();
            let file = file.clone();
            move || model_file_ready(&swarm_dir, &file)
        })
        .await
        .map_err(|err| NodeError::Message(err.to_string()))?;
        if ready && dest.is_file() {
            continue;
        }
        if let Err(err) = download_part(&client, base, &swarm, file).await {
            eprintln!("MDS: {err}");
            let _ = std::fs::remove_file(swarm.join(format!("{}.part", file.name)));
            for name in &pending {
                let _ = std::fs::remove_file(swarm.join(format!("{name}.part")));
            }
            return Ok(());
        }
        pending.push(file.name.clone());
    }
    for name in &pending {
        let part = swarm.join(format!("{name}.part"));
        std::fs::rename(&part, swarm.join(name))?;
    }
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|err| NodeError::Message(err.to_string()))?;
    std::fs::write(swarm.join("manifest.json"), manifest_bytes)?;
    if !pending.is_empty()
        && old_stamp
            .as_deref()
            .is_some_and(|stamp| !stamp.eq_ignore_ascii_case(&manifest.signature))
    {
        eprintln!(
            "MDS: веса модели на улье другие. Старые векторы в индексе больше не годятся. Пересчёт страниц этим запуском не делается"
        );
    }
    std::fs::write(&stamp_path, &manifest.signature)?;
    if pending.is_empty() {
        eprintln!("MDS: локальная модель совпадает с ульем");
    } else {
        eprintln!("MDS: модель роя записана в {}", swarm.display());
    }
    Ok(())
}

async fn download_part(
    client: &reqwest::Client,
    base: &str,
    swarm: &Path,
    file: &ModelFile,
) -> Result<(), NodeError> {
    let response = client
        .get(format!("{base}/peer/model/{}", file.name))
        .send()
        .await
        .map_err(|err| NodeError::Message(err.to_string()))?;
    if !response.status().is_success() {
        return Err(NodeError::Message(format!("улей не отдал {}", file.name)));
    }
    let part = swarm.join(format!("{}.part", file.name));
    let mut out = std::fs::File::create(&part)?;
    let mut response = response;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| NodeError::Message(err.to_string()))?
    {
        out.write_all(&chunk)?;
    }
    out.sync_all()?;
    drop(out);
    let part_for_hash = part.clone();
    let expected_sha = file.sha256.clone();
    let expected_size = file.size;
    let matched = tokio::task::spawn_blocking(move || match hash_path(&part_for_hash) {
        Ok((sha, size)) => size == expected_size && sha.eq_ignore_ascii_case(&expected_sha),
        Err(_) => false,
    })
    .await
    .map_err(|err| NodeError::Message(err.to_string()))?;
    if !matched {
        let _ = std::fs::remove_file(&part);
        return Err(NodeError::Message(format!(
            "контрольная сумма {} не сошлась, файл не записан",
            file.name
        )));
    }
    Ok(())
}
