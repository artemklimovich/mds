use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};
use mds_crawl::CrawlPolicy;
use mds_node::{index_fixture_dir, run_lite, serve, Engine, LiteConfig, ServeConfig};
use mds_update::{generate_keypair, public_from_secret, save_release, sign_release, PINNED_PUBLIC_HEX};

#[derive(Parser)]
#[command(name = "mds", about = "Децентрализованный поиск MDS")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Обойти страницы и открыть сайт со строкой поиска.
    Node {
        #[arg(long, default_value = "127.0.0.1:8080")]
        listen: String,
        #[arg(long, default_value = "data/mds.db")]
        db: PathBuf,
        #[arg(long, default_value = "data/models")]
        models: PathBuf,
        #[arg(long, default_value = "fixtures")]
        fixtures: PathBuf,
        /// Файл со стартовыми адресами реального интернета, по одному URL в строке.
        #[arg(long)]
        seeds: Option<PathBuf>,
        #[arg(long, default_value_t = 30)]
        max_pages: usize,
        #[arg(long, default_value_t = 1000)]
        delay_ms: u64,
        /// Ходить по ссылкам на другие сайты. Без флага нода остаётся на сайте из списка.
        #[arg(long)]
        follow_external: bool,
        /// Автоматически искать новые домены через Certificate Transparency (crt.sh), когда очередь пустеет.
        #[arg(long, default_value_t = true)]
        discover_ct: bool,
        #[arg(long)]
        recrawl: bool,
        /// Адрес соседней ноды, например http://10.0.0.8:8080. Можно указать несколько раз.
        #[arg(long)]
        peers: Vec<String>,
        /// Публичный улей, которому эта пчела сама отдаёт страницы. Для домашнего адреса без белого IP.
        #[arg(long)]
        hive: Option<String>,
        /// Общий секрет улья и домашних пчёл. Без него чужие страницы в индекс не принимаются.
        #[arg(long, default_value = "")]
        swarm_key: String,
        /// Лёгкая пчела на сервере сайта: без модели и без общего индекса.
        #[arg(long)]
        lite: bool,
        /// Адрес своего сайта, например https://tehnoveles.ru. Нужен вместе с --lite.
        #[arg(long)]
        site: Option<String>,
        #[arg(long, default_value = "data/releases")]
        releases: PathBuf,
    },
    /// Создать секретный ключ издателя. Публичная половина должна совпасть с ключом в этой сборке.
    Keygen {
        #[arg(long, default_value = "data/release.key")]
        out: PathBuf,
    },
    /// Подписать собранный бинарник. Соседи заберут его из каталога релизов этой ноды.
    Publish {
        #[arg(long, default_value = "data/release.key")]
        key: PathBuf,
        #[arg(long)]
        bin: PathBuf,
        #[arg(long)]
        version: String,
        #[arg(long, default_value = "data/releases")]
        out: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Node {
            listen,
            db,
            models,
            fixtures,
            seeds,
            max_pages,
            delay_ms,
            follow_external,
            discover_ct,
            recrawl,
            peers,
            hive,
            swarm_key,
            lite,
            site,
            releases,
        } => {
            if lite {
                let site = site.ok_or_else(|| anyhow::anyhow!("для --lite нужен --site"))?;
                let hive = hive.ok_or_else(|| anyhow::anyhow!("для --lite нужен --hive"))?;
                run_lite(LiteConfig {
                    site,
                    hive,
                    swarm_key,
                    label: bee_label(&std::path::PathBuf::from("data/mds.db")),
                    delay_ms,
                })
                .await?;
                return Ok(());
            }
            if recrawl {
                let _ = std::fs::remove_file(&db);
            }
            let mut engine = Engine::open(&db, &models)?;
            let policy = CrawlPolicy {
                delay: Duration::from_millis(delay_ms),
                max_bytes: 1_000_000,
                max_pages,
                follow_external,
            };
            let has_seeds = seeds.is_some();
            if let Some(seeds) = seeds {
                let urls = read_seeds(&seeds)?;
                engine.enqueue_urls(&urls)?;
                if engine.len()? == 0 || recrawl {
                    eprintln!(
                        "MDS: обход интернета, адресов {}, пауза {} мс, лимит {} страниц",
                        urls.len(),
                        delay_ms,
                        max_pages
                    );
                    let stored = engine.index_urls(&urls, &policy).await?;
                    eprintln!("MDS: по списку URL в индекс попало {stored}");
                } else {
                    let seed_policy = CrawlPolicy {
                        delay: Duration::from_millis(delay_ms),
                        max_bytes: 1_000_000,
                        max_pages: urls.len().min(12),
                        follow_external: false,
                    };
                    let _ = engine.refresh_urls(&urls, &seed_policy).await;
                    eprintln!(
                        "MDS: в индексе уже {} страниц; очередь продолжает обход в фоне",
                        engine.len()?
                    );
                }
            } else if engine.len()? == 0 && fixtures.is_dir() {
                let stored = index_fixture_dir(&mut engine, &fixtures, &policy).await?;
                eprintln!("MDS: из fixtures в индекс попало {stored}");
            } else {
                eprintln!(
                    "MDS: в индексе уже {} страниц. Новый обход: --recrawl",
                    engine.len()?
                );
            }
            if engine.len()? == 0 {
                anyhow::bail!("индекс пуст: положите HTML в fixtures или передайте --seeds");
            }
            serve(
                engine,
                ServeConfig {
                    listen,
                    peers,
                    releases,
                    background_policy: if has_seeds || follow_external {
                        Some(policy)
                    } else {
                        None
                    },
                    discover_ct: follow_external && discover_ct,
                    hive,
                    swarm_key,
                    label: bee_label(&db),
                },
            )
            .await?;
        }
        Command::Keygen { out } => {
            if out.exists() {
                anyhow::bail!("ключ уже есть: {}", out.display());
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let (secret, public) = generate_keypair();
            std::fs::write(&out, &secret)?;
            println!("секрет записан в {}", out.display());
            println!("публичный ключ этого файла: {public}");
            println!("публичный ключ сборки:       {PINNED_PUBLIC_HEX}");
            if public != PINNED_PUBLIC_HEX {
                println!("ключи разные: эта сборка не примет релизы, подписанные новым секретом");
            }
        }
        Command::Publish { key, bin, version, out } => {
            let secret = std::fs::read_to_string(&key)?;
            let public = public_from_secret(&secret)?;
            if public != PINNED_PUBLIC_HEX {
                anyhow::bail!("секрет не от ключа этой сборки");
            }
            let bytes = std::fs::read(&bin)?;
            let manifest = sign_release(&secret, &version, &bytes)?;
            save_release(&out, &manifest, &bytes)?;
            println!(
                "релиз {} записан в {} ({} байт)",
                version,
                out.display(),
                bytes.len()
            );
            println!("работающая нода подхватит файл и отдаст его соседям по --peers");
        }
    }
    Ok(())
}

fn bee_label(db: &PathBuf) -> String {
    let id_path = db.with_extension("bee");
    if let Ok(saved) = std::fs::read_to_string(&id_path) {
        let saved = saved.trim();
        if !saved.is_empty() && saved.len() <= 64 {
            return saved.to_string();
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(1);
    let id = format!("{nanos:x}");
    if let Some(parent) = id_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&id_path, &id);
    id
}

fn read_seeds(path: &PathBuf) -> anyhow::Result<Vec<String>> {
    let text = std::fs::read_to_string(path)?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect())
}
