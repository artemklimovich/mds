use std::path::PathBuf;
use std::time::Duration;

mod author;

use clap::{Parser, Subcommand};
use mds_crawl::CrawlPolicy;
use mds_node::{
    hash_admin_password, index_fixture_dir, run_lite, serve, serve_face, sync_model, Engine, FaceConfig, LiteConfig,
    ServeConfig,
};
use mds_update::{
    generate_keypair, public_from_secret, save_release, sign_model, sign_release, PINNED_PUBLIC_HEX,
    TRUSTED_NODES,
};

#[derive(Parser)]
#[command(
    name = "mds",
    version,
    about = "Децентрализованный поиск MDS. Автор: Артём Климович, +79060331235, artem.klimovich@gmail.com."
)]
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
        /// Если не указан, программа сама берёт вписанные доверенные ноды.
        #[arg(long)]
        peers: Vec<String>,
        /// Не подключаться к вписанным доверенным нодам. Так запускают сам улей.
        #[arg(long)]
        alone: bool,
        /// Публичный адрес этого улья. Вместе с --peers отдаёт ему свои домены, соседу — остальные.
        /// Без флага улей держит все домены, даже если соседи указаны.
        #[arg(long)]
        self_base: Option<String>,
        /// Публичный улей, которому эта пчела сама отдаёт страницы. Для домашнего адреса без белого IP.
        #[arg(long)]
        hive: Option<String>,
        /// Раньше общий пароль улья. Лёгкая пчела входит карточкой на своём сайте и этот флаг не использует.
        #[arg(long, default_value = "")]
        swarm_key: String,
        /// Регион этого улья, например «Москва» или «Волгоградская область». Человек может сменить его в строке поиска.
        #[arg(long)]
        region: Option<String>,
        /// Лёгкая пчела на сервере сайта: без модели и без общего индекса.
        #[arg(long)]
        lite: bool,
        /// Адрес своего сайта, например https://tehnoveles.ru. Нужен вместе с --lite.
        #[arg(long)]
        site: Option<String>,
        #[arg(long, default_value = "data/releases")]
        releases: PathBuf,
        /// Каталог, откуда улей отдаёт архивы программы.
        #[arg(long, default_value = "data/downloads")]
        downloads: PathBuf,
        /// Подписанная карта сети. Без верной подписи кнопка другого края не появляется.
        #[arg(long, default_value = "data/network.json")]
        map: PathBuf,
        /// Кабинет и MCP. Открывается только если заданы имя и хеш пароля.
        #[arg(long, default_value = "127.0.0.1:8081")]
        admin_listen: String,
        /// Не открывать базу, пока не появится этот файл. Модель к тому времени уже в памяти.
        #[arg(long)]
        go_file: Option<PathBuf>,
        /// Сюда пишется отметка, когда модель уже в памяти и процесс ждёт --go-file.
        #[arg(long)]
        warm_file: Option<PathBuf>,
    },
    /// Страницы поиска и кабинета без модели и без базы. Данные берёт у улья.
    Face {
        #[arg(long, default_value = "127.0.0.1:8090")]
        listen: String,
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        hive: String,
        #[arg(long, default_value = "http://127.0.0.1:8081")]
        admin: String,
        #[arg(long, default_value = "data/face.token")]
        token_file: PathBuf,
    },
    /// Спросить пароль без эха и напечатать Argon2id-хеш для MDS_ADMIN_PASSWORD_HASH.
    AdminPassword,
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
    /// Подписать каталог модели роя. Секрет остаётся у оператора, улей только отдаёт файлы.
    PublishModel {
        #[arg(long, default_value = "data/release.key")]
        key: PathBuf,
        #[arg(long, default_value = "data/models/swarm")]
        dir: PathBuf,
        #[arg(long, default_value = "multilingual-e5-small")]
        name: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    std::hint::black_box(author::NOTICE);
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
            alone,
            self_base,
            hive,
            swarm_key,
            region,
            lite,
            site,
            releases,
            downloads,
            map,
            admin_listen,
            go_file,
            warm_file,
        } => {
            if lite {
                let hive = hive.ok_or_else(|| anyhow::anyhow!("для --lite нужен --hive"))?;
                let site = site.unwrap_or_default();
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
            let model_hive = match hive.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
                Some(url) => Some(url.to_string()),
                None if !alone => Some(TRUSTED_NODES[0].to_string()),
                None => None,
            };
            sync_model(&models, model_hive.as_deref()).await?;
            let mut engine = Engine::open_when(&db, &models, go_file.as_deref(), warm_file.as_deref())?;
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
                    eprintln!(
                        "MDS: в индексе уже {} страниц; страница поиска откроется сразу, очередь продолжит обход",
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
            let peers = if peers.is_empty() && !alone {
                TRUSTED_NODES.iter().map(|node| (*node).to_string()).collect()
            } else {
                peers
            };
            let region = match region {
                Some(value) => Some(
                    mds_types::canonical_region(&value)
                        .ok_or_else(|| anyhow::anyhow!("неизвестный регион: {value}"))?
                        .to_string(),
                ),
                None => None,
            };
            if engine.len()? == 0 {
                anyhow::bail!("индекс пуст: положите HTML в fixtures или передайте --seeds");
            }
            serve(
                engine,
                ServeConfig {
                    listen,
                    peers,
                    releases,
                    models: models.clone(),
                    downloads,
                    background_policy: if has_seeds || follow_external {
                        Some(policy)
                    } else {
                        None
                    },
                    discover_ct: follow_external && discover_ct,
                    hive,
                    swarm_key,
                    label: bee_label(&db),
                    region,
                    map,
                    admin_listen,
                    alone,
                    self_base,
                    mcp_path: db
                        .parent()
                        .unwrap_or(std::path::Path::new("data"))
                        .join("mcp.hash"),
                },
            )
            .await?;
        }
        Command::Face {
            listen,
            hive,
            admin,
            token_file,
        } => {
            serve_face(FaceConfig {
                listen,
                hive,
                admin,
                token_file,
            })
            .await?;
        }
        Command::AdminPassword => {
            let password = rpassword::prompt_password("Пароль кабинета: ")?;
            let hash = hash_admin_password(&password).map_err(|err| anyhow::anyhow!(err))?;
            println!("{hash}");
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
        Command::PublishModel { key, dir, name } => {
            let secret = std::fs::read_to_string(&key)?;
            let public = public_from_secret(&secret)?;
            if public != PINNED_PUBLIC_HEX {
                anyhow::bail!("секрет не от ключа этой сборки");
            }
            let manifest = sign_model(&secret, &dir, &name)?;
            println!(
                "модель {} подписана в {} (файлов {})",
                manifest.name,
                dir.display(),
                manifest.files.len()
            );
            println!("улей отдаёт её по /peer/model. На странице загрузки ссылки на веса нет");
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
