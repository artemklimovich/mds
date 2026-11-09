/// Обычный сайт обходится не дальше этого числа страниц.
pub const HOST_PAGE_CAP: i64 = 100;

pub const KIND_NEWS: u32 = 1;
pub const KIND_DOC: u32 = 2;
pub const KIND_GOV: u32 = 4;
pub const KIND_IMAGE: u32 = 8;
pub const KIND_VIDEO: u32 = 16;
pub const KIND_JOBS: u32 = 32;
pub const KIND_SERVICE: u32 = 64;
pub const KIND_MEDICINE: u32 = 128;
pub const KIND_GOODS: u32 = 256;
/// Публичная карточка канала, группы или бота. Во вкладке «Все» её нет.
pub const KIND_CHANNEL: u32 = 512;
/// У страницы есть магазин с карточками товара. Саму страницу из индекса это не убирает.
pub const KIND_SHOP: u32 = 1024;
/// Логистический сайт: перевозка грузов, а не строка «доставка» у магазина.
pub const KIND_HAUL: u32 = 2048;
/// Автосалон, автосервис, запчасти. Слово «автомобиль» само по себе метку не ставит.
pub const KIND_AUTO: u32 = 4096;
/// Агентство, застройщик, продажа и аренда жилья.
pub const KIND_REALTY: u32 = 8192;
/// Строительная компания или ремонт жилья.
pub const KIND_BUILD: u32 = 16384;
/// Школа, вуз, сад, учебный центр.
pub const KIND_EDU: u32 = 32768;
/// Рекламное или маркетинговое агентство. Слово «реклама» само по себе метку не ставит.
pub const KIND_ADS: u32 = 65536;
/// Сайт про искусственный интеллект, нейросети или машинное обучение.
pub const KIND_AI: u32 = 131072;
/// Тематики шестерёнки. У издания из ленты этих битов нет.
pub const THEME_BITS: u32 = KIND_MEDICINE
    | KIND_SHOP
    | KIND_HAUL
    | KIND_AUTO
    | KIND_REALTY
    | KIND_BUILD
    | KIND_EDU
    | KIND_ADS
    | KIND_AI;

/// У органа власти это не клиника, не магазин и не автосалон. Слово в подвале метку не ставит.
const GOV_OFF: u32 = KIND_JOBS
    | KIND_SERVICE
    | KIND_MEDICINE
    | KIND_GOODS
    | KIND_SHOP
    | KIND_HAUL
    | KIND_AUTO
    | KIND_REALTY
    | KIND_BUILD
    | KIND_EDU
    | KIND_ADS
    | KIND_AI;

#[derive(Clone, Copy, Debug)]
pub struct KindSpec {
    pub code: &'static str,
    pub label: &'static str,
    pub mask: u32,
    /// Полоса типа страницы, шестерёнка тематик или внутренняя метка для MCP.
    pub place: &'static str,
}

const KINDS: &[KindSpec] = &[
    KindSpec { code: "news", label: "Новости", mask: KIND_NEWS, place: "тип страницы" },
    KindSpec { code: "gov", label: "Органы власти", mask: KIND_GOV, place: "тип страницы" },
    KindSpec { code: "jobs", label: "Работа", mask: KIND_JOBS, place: "тип страницы" },
    KindSpec { code: "services", label: "Услуги", mask: KIND_SERVICE, place: "тип страницы" },
    KindSpec { code: "medicine", label: "Медицина", mask: KIND_MEDICINE, place: "тематика" },
    KindSpec { code: "shop", label: "Интернет-магазины", mask: KIND_SHOP, place: "тематика" },
    KindSpec { code: "haul", label: "Перевозки", mask: KIND_HAUL, place: "тематика" },
    KindSpec { code: "auto", label: "Авто", mask: KIND_AUTO, place: "тематика" },
    KindSpec { code: "realty", label: "Недвижимость", mask: KIND_REALTY, place: "тематика" },
    KindSpec { code: "build", label: "Стройка и ремонт", mask: KIND_BUILD, place: "тематика" },
    KindSpec { code: "edu", label: "Образование", mask: KIND_EDU, place: "тематика" },
    KindSpec { code: "ads", label: "Реклама и маркетинг", mask: KIND_ADS, place: "тематика" },
    KindSpec { code: "ai", label: "Искусственный интеллект", mask: KIND_AI, place: "тематика" },
    KindSpec { code: "docs", label: "Документы", mask: KIND_DOC, place: "скрытая, MCP" },
    KindSpec { code: "goods", label: "Товары и оборудование", mask: KIND_GOODS, place: "скрытая, MCP" },
    KindSpec { code: "images", label: "Картинки на странице", mask: KIND_IMAGE, place: "скрытая, MCP" },
    KindSpec { code: "video", label: "Видео на странице", mask: KIND_VIDEO, place: "скрытая, MCP" },
    KindSpec { code: "channel", label: "Каналы и соцсети", mask: KIND_CHANNEL, place: "тип страницы" },
];

pub fn all_kinds() -> &'static [KindSpec] {
    KINDS
}

pub fn kind_codes(kind: u32) -> Vec<&'static str> {
    KINDS
        .iter()
        .filter(|item| kind & item.mask != 0)
        .map(|item| item.code)
        .collect()
}

/// Издание с открытым RSS. Статьи в индекс не пишутся, заголовки читаются в момент раскрытия ленты.
pub struct HostFeed {
    pub host: &'static str,
    pub name: &'static str,
    pub rss: &'static str,
}

const PRESS: &[HostFeed] = &[
    HostFeed { host: "lenta.ru", name: "Лента", rss: "https://lenta.ru/rss/news" },
    HostFeed { host: "rbc.ru", name: "РБК", rss: "https://rssexport.rbc.ru/rbcnews/news/30/full.rss" },
    HostFeed { host: "kommersant.ru", name: "Коммерсантъ", rss: "https://www.kommersant.ru/rss/news.xml" },
    HostFeed { host: "tass.ru", name: "ТАСС", rss: "https://tass.ru/rss/v2.xml" },
    HostFeed { host: "ria.ru", name: "РИА Новости", rss: "https://ria.ru/export/rss2/archive/index.xml" },
    HostFeed { host: "interfax.ru", name: "Интерфакс", rss: "https://www.interfax.ru/rss.asp" },
    HostFeed { host: "vedomosti.ru", name: "Ведомости", rss: "https://www.vedomosti.ru/rss/news" },
    HostFeed { host: "iz.ru", name: "Известия", rss: "https://iz.ru/xml/rss/all.xml" },
    HostFeed { host: "mk.ru", name: "Московский комсомолец", rss: "https://www.mk.ru/rss/index.xml" },
    HostFeed { host: "aif.ru", name: "Аргументы и факты", rss: "https://aif.ru/rss/news.php" },
];

const CHANNELS: &[&str] = &[
    "rutube.ru",
    "youtube.com",
    "youtu.be",
    "vk.com",
    "vk.ru",
    "vkvideo.ru",
    "max.ru",
    "ok.ru",
    "dzen.ru",
    "t.me",
    "telegram.me",
    "tiktok.com",
    "instagram.com",
    "facebook.com",
    "fb.com",
    "twitter.com",
    "x.com",
    "pikabu.ru",
    "livejournal.com",
    "boosty.to",
    "twitch.tv",
    "vimeo.com",
    "dailymotion.com",
    "coub.com",
    "nuum.ru",
];

/// Площадки, на чью публичную карточку пчела ходит. Instagram и loginwall — только контакт на сайте.
const CHANNEL_VISIT: &[&str] = &[
    "rutube.ru",
    "youtube.com",
    "vk.com",
    "vk.ru",
    "vkvideo.ru",
    "max.ru",
    "ok.ru",
    "dzen.ru",
    "t.me",
    "telegram.me",
];

const VIDEO_CHANNELS: &[&str] = &["rutube.ru", "youtube.com", "youtu.be", "vkvideo.ru"];

const MAX_RESERVED: &[&str] = &[
    "my",
    "feeds",
    "categories",
    "for_creators",
    "login",
    "download",
    "legal",
    "info",
    "app",
    "support",
    "business",
    "authors",
    "privacy",
    "terms",
    "about",
    "help",
    "docs",
    "api",
    "blog",
    "brand",
    "smarttv",
    "shorts",
];

const MARKETPLACES: &[&str] = &[
    "ozon.ru",
    "wildberries.ru",
    "wb.ru",
    "market.yandex.ru",
    "avito.ru",
    "megamarket.ru",
];

const JOB_BOARDS: &[&str] = &[
    "hh.ru",
    "superjob.ru",
    "zarplata.ru",
    "rabota.ru",
    "gorodrabot.ru",
    "trudvsem.ru",
];

/// Органы без суффикса .gov.ru. Остальные узнаются по суффиксу.
const GOVERNMENT: &[&str] = &[
    "gosuslugi.ru",
    "kremlin.ru",
    "government.ru",
    "gov.ru",
    "mos.ru",
    "mosreg.ru",
    "adm44.ru",
    "admin-smolensk.ru",
    "admoblkaluga.ru",
    "avo.ru",
    "belregion.ru",
    "glava.kbr.ru",
    "khural.rtyva.ru",
    "kursk.ru",
    "orel-region.ru",
    "tularegion.ru",
    "ryazangov.ru",
    "yarregion.ru",
    "govvrn.ru",
    "adm-nao.ru",
    "glava.region08.ru",
    "huralrk.ru",
    "kurskduma.ru",
    "nsrd.ru",
    "pgu.egov08.ru",
    "egov08.ru",
    "president.e-dag.ru",
    "e-dag.ru",
    "adm.astrobl.ru",
    "adygheya.ru",
    "astrasocial.ru",
    "bryanskobl.ru",
    "dumabryansk.ru",
    "ingushetia.ru",
    "ivanovoobl.ru",
    "kalmregion.ru",
    "kbr.ru",
    "lenobl.ru",
    "parlament09.ru",
    "parlamentri.ru",
    "sdnao.ru",
    "smoloblduma.ru",
    "suradmin.ru",
    "xn--80aaccp4ajwpkgbl4lpb.xn--p1ai",
    "xn--80aacoonefzg3am8b1fsb.xn--p1ai",
    "minfin.ru",
    "minjust.ru",
    "mid.ru",
    "mil.ru",
    "cbr.ru",
    "nalog.ru",
    "roskazna.ru",
    "consultant.ru",
    "garant.ru",
    "pravo.ru",
    "tatarstan.ru",
    "bashkortostan.ru",
    "krasnodar.ru",
    "spb.ru",
    "gov.spb.ru",
    "novosibirsk.ru",
    "admgor.spb.ru",
    "sochi.ru",
    "kazan.ru",
    "ekaterinburg.ru",
    "nnov.ru",
    "samregion.ru",
    "donland.ru",
    "krasnoyarsk.ru",
    "permkrai.ru",
    "tyumen.ru",
    "vologda-oblast.ru",
    "astrobl.ru",
];

pub struct SiteAsk {
    pub host: String,
    pub words: String,
}

/// `site:example.ru` и `сайт:example.ru` — страницы этого сайта из индекса.
/// Слова после домена оставляют только страницы, где они есть.
pub fn site_ask(raw: &str) -> Option<SiteAsk> {
    let raw = raw.trim();
    let lower = raw.to_lowercase();
    let rest = if lower.starts_with("site:") {
        &raw["site:".len()..]
    } else if lower.starts_with("сайт:") {
        &raw["сайт:".len()..]
    } else {
        return None;
    };
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    let (host_raw, words) = rest
        .split_once(char::is_whitespace)
        .map(|(host, words)| (host, words.trim()))
        .unwrap_or((rest, ""));
    let host_raw = host_raw
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host_raw = host_raw.split(['/', '?', '#']).next().unwrap_or(host_raw);
    let host = bare_host(host_raw);
    if host.len() < 3 || !host.contains('.') || host.contains('@') || host.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    Some(SiteAsk {
        host,
        words: words.to_string(),
    })
}

pub fn theme_mask(theme: &str) -> u32 {
    let mask = tab_mask(theme);
    if mask != 0 && mask & THEME_BITS == mask {
        mask
    } else {
        0
    }
}

/// Страница проходит полосу и тематику. Пустая полоса — обычные сайты, без новостей и площадок.
/// На «Новостях» издание без тематического бита остаётся, даже если тематика выбрана.
pub fn on_shelf(kind: u32, tab: u32, theme: u32) -> bool {
    let type_ok = if tab == 0 {
        in_general(kind)
    } else {
        kind & tab != 0
    };
    if !type_ok {
        return false;
    }
    if theme == 0 || kind & theme != 0 {
        return true;
    }
    tab == KIND_NEWS && kind & THEME_BITS == 0
}

pub fn tab_mask(tab: &str) -> u32 {
    let tab = tab.trim();
    match tab {
        "document" => return KIND_DOC,
        "image" => return KIND_IMAGE,
        "service" => return KIND_SERVICE,
        "job" => return KIND_JOBS,
        _ => {}
    }
    KINDS
        .iter()
        .find(|item| item.code == tab)
        .map(|item| item.mask)
        .unwrap_or(0)
}

pub fn host_of_url(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?.split('@').next_back()?;
    let host = host.split(':').next()?.trim().to_ascii_lowercase();
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

pub fn bare_host(host: &str) -> String {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    host.strip_prefix("www.").unwrap_or(&host).to_string()
}

const CIS_ZONES: &[&str] = &[
    "ru",
    "su",
    "by",
    "kz",
    "am",
    "az",
    "kg",
    "uz",
    "tj",
    "md",
    "tm",
    "xn--p1ai",
    "xn--90ais",
    "xn--80ao21a",
    "рф",
    "бел",
    "қаз",
];

/// Сайт компании, которая сама обучает модель или отдаёт свой сервис этой модели.
/// Облако, git и чужая документация сюда не входят.
const AI_SUPPLIERS: &[&str] = &[
    "openai.com",
    "chatgpt.com",
    "anthropic.com",
    "claude.ai",
    "mistral.ai",
    "deepseek.com",
    "cohere.com",
    "cohere.ai",
    "ai21.com",
    "stability.ai",
    "blackforestlabs.ai",
    "bfl.ai",
    "x.ai",
    "groq.com",
    "cerebras.ai",
    "sambanova.ai",
    "huggingface.co",
    "hf.co",
    "deepmind.com",
    "deepmind.google",
    "gemini.google.com",
    "aistudio.google.com",
    "ai.google",
    "ai.meta.com",
    "llama.meta.com",
    "meta.ai",
    "midjourney.com",
    "runwayml.com",
    "elevenlabs.io",
    "perplexity.ai",
    "character.ai",
    "inflection.ai",
    "suno.com",
    "udio.com",
    "deepgram.com",
    "assemblyai.com",
    "fireworks.ai",
    "together.ai",
    "replicate.com",
    "inworld.ai",
    "qwen.ai",
    "qwenlm.ai",
    "moonshot.cn",
    "kimi.ai",
    "kimi.com",
    "minimax.io",
    "stepfun.com",
    "01.ai",
    "zhipuai.cn",
    "bigmodel.cn",
    "chatglm.cn",
    "baichuan-ai.com",
    "tongyi.aliyun.com",
    "siliconflow.cn",
    "siliconflow.com",
    "hunyuan.tencent.com",
    "spark.xfyun.cn",
    "aleph-alpha.com",
    "lighton.ai",
    "nomic.ai",
    "copilot.microsoft.com",
    "grok.com",
    "giga.chat",
    "mws.ai",
    "cursor.com",
];

/// Инструменты, которые остаются в обходе рядом с поставщиками моделей.
const TOOL_HOSTS: &[&str] = &[
    "github.com",
    "aws.amazon.com",
    "code.visualstudio.com",
    "vscode.dev",
    "openclaw.ai",
];

/// Документация моделей, SDK и агентной инфраструктуры. Главы целиком, не чат и не витрина облака.
const TOOL_DOCS: &[&str] = &[
    "docs.openclaw.ai",
    "docs.github.com",
    "docs.aws.amazon.com",
    "code.visualstudio.com",
    "developers.openai.com",
    "docs.anthropic.com",
    "platform.claude.com",
    "code.claude.com",
    "ai.google.dev",
    "docs.x.ai",
    "docs.cohere.com",
    "docs.mistral.ai",
    "api-docs.deepseek.com",
    "docs.together.ai",
    "docs.fireworks.ai",
    "docs.groq.com",
    "docs.qwen.ai",
    "docs.qwencloud.com",
    "platform.moonshot.cn",
    "platform.kimi.ai",
    "platform.kimi.com",
    "platform.minimax.io",
    "docs.bigmodel.cn",
    "open.bigmodel.cn",
    "developers.sber.ru",
    "aistudio.yandex.ru",
    "docs.langchain.com",
    "python.langchain.com",
    "docs.llamaindex.ai",
    "docs.crewai.com",
    "modelcontextprotocol.io",
    "docs.litellm.ai",
    "docs.vllm.ai",
    "docs.ollama.com",
    "docs.dify.ai",
    "docs.continue.dev",
    "docs.aider.chat",
    "docs.agno.com",
    "docs.n8n.io",
    "openrouter.ai",
    "vercel.ai",
    "mastra.ai",
    "docs.coze.com",
    "microsoft.github.io",
];

pub fn tool_docs_hosts() -> &'static [&'static str] {
    TOOL_DOCS
}

pub fn model_maker_hosts() -> &'static [&'static str] {
    AI_SUPPLIERS
}

const REFERENCE_HOSTS: &[&str] = &["wikipedia.org", "ruwiki.ru"];

/// Википедия и Рувики. В общий индекс не входят: ответ показывается отдельным блоком.
pub fn is_reference_host(host: &str) -> bool {
    listed(host, REFERENCE_HOSTS)
}

/// Поставщик моделей. В индекс нужна главная, не весь чат.
pub fn is_model_maker_host(host: &str) -> bool {
    listed(&bare_host(host), AI_SUPPLIERS)
}

pub fn is_model_front(url: &str) -> bool {
    let Some(host) = host_of_url(url) else {
        return false;
    };
    if url.contains('?') || url.contains('#') {
        return false;
    }
    is_model_maker_host(&host) && path_of(url) == "/"
}

/// Внутренности чата и кнопки «спросить главу». Документация сюда не входит.
pub fn is_model_inner(url: &str) -> bool {
    let Some(host) = host_of_url(url) else {
        return false;
    };
    is_model_maker_host(&host) && !is_tool_docs_host(&host) && !is_model_front(url)
}

/// Справка моделей, SDK агентов, GitHub, AWS, VS Code и OpenClaw. Не весь github.com и не витрина AWS.
pub fn is_tool_docs_host(host: &str) -> bool {
    let host = bare_host(host);
    if listed(&host, TOOL_DOCS) {
        return true;
    }
    if !listed(&host, TOOL_HOSTS) {
        return false;
    }
    host.starts_with("docs.") || host == "openclaw.ai" || host == "code.visualstudio.com"
}

/// Россия, СНГ, поставщики моделей, их документация и агентные SDK. Остальной интернет в очередь не входит.
pub fn host_in_focus(host: &str) -> bool {
    let host = bare_host(host);
    if host.is_empty() || host.parse::<std::net::Ipv4Addr>().is_ok() || is_reference_host(&host) {
        return false;
    }
    let zone = host.rsplit('.').next().unwrap_or("");
    if CIS_ZONES.iter().any(|item| zone == *item) {
        return true;
    }
    listed(&host, AI_SUPPLIERS) || listed(&host, TOOL_HOSTS) || listed(&host, TOOL_DOCS)
}

pub fn is_marketplace_host(host: &str) -> bool {
    listed(host, MARKETPLACES)
}

pub fn is_job_board(host: &str) -> bool {
    listed(host, JOB_BOARDS)
}

pub fn is_channel_host(host: &str) -> bool {
    exact_listed(host, CHANNELS)
}

/// Видеохостинг: карточке канала можно оставить метку video.
pub fn is_video_channel_host(host: &str) -> bool {
    exact_listed(host, VIDEO_CHANNELS)
}

/// Площадка, на чью карточку пчела ходит. Не Instagram и не loginwall.
pub fn is_channel_visit_host(host: &str) -> bool {
    exact_listed(host, CHANNEL_VISIT)
}

pub fn press_feeds() -> &'static [HostFeed] {
    PRESS
}

pub fn press_feed(host: &str) -> Option<&'static str> {
    let host = bare_host(host);
    PRESS
        .iter()
        .find(|item| host == item.host || host.strip_suffix(item.host).is_some_and(|prefix| prefix.ends_with('.')))
        .map(|item| item.rss)
}

pub fn press_rss(url: &str) -> bool {
    PRESS.iter().any(|item| item.rss == url.trim())
}

/// Адрес ленты, который нода может сама запросить. Без входа, порта и внутреннего имени.
pub fn open_feed(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.len() > 500 {
        return None;
    }
    let rest = raw.strip_prefix("https://").or_else(|| raw.strip_prefix("http://"))?;
    if rest.is_empty() || rest.contains('@') || rest.contains('\\') || rest.starts_with('[') {
        return None;
    }
    let host = rest.split(['/', '?', '#']).next()?.trim().to_ascii_lowercase();
    if host.is_empty() || host.contains(':') || host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") || host.ends_with(".internal") || host.ends_with(".lan") {
        return None;
    }
    if host.parse::<std::net::Ipv4Addr>().is_ok() {
        return None;
    }
    Some(raw.to_string())
}

pub fn is_press_host(host: &str) -> bool {
    let host = bare_host(host);
    PRESS.iter().any(|item| host == item.host || host.strip_suffix(item.host).is_some_and(|prefix| prefix.ends_with('.')))
}

/// Главная издания. Остальные адреса этого хоста в индекс не кладутся.
pub fn is_press_front(url: &str) -> bool {
    let Some(host) = host_of_url(url) else {
        return false;
    };
    is_press_host(&host) && path_of(url) == "/"
}

/// Вкладка «Все» не показывает новости и страницы площадок.
pub fn in_general(kind: u32) -> bool {
    kind & (KIND_NEWS | KIND_CHANNEL) == 0
}

pub fn is_government_host(host: &str) -> bool {
    let host = bare_host(host);
    host == "gov.ru"
        || host.ends_with(".gov.ru")
        || host.ends_with(".mil.ru")
        || host.ends_with(".cbr.ru")
        || listed(&host, GOVERNMENT)
}

/// Публичное имя этого улья. В выдаче значок «улей», потолок страниц не режет.
pub fn is_hive_host(host: &str) -> bool {
    let host = bare_host(host);
    host == "beesearch.ru"
}

/// Органы власти, законы, улей и справка важных систем: сто страниц мало.
pub fn host_keeps_pages(host: &str) -> bool {
    is_government_host(host) || is_hive_host(host) || is_tool_docs_host(host)
}

pub fn is_webshop(title: &str, text: &str) -> bool {
    let blob = format!("{title}\n{text}").to_lowercase();
    blob.contains("интернет-магазин") || blob.contains("интернет магазин")
}

pub fn is_document_url(url: &str) -> bool {
    let path = path_of(url);
    [".pdf", ".doc", ".docx", ".xls", ".xlsx", ".rtf"]
        .iter()
        .any(|ext| path.ends_with(ext))
}

pub fn is_product_url(url: &str) -> bool {
    let path = path_of(url);
    path.contains("/tovar")
        || path.contains("/goods/")
        || path.contains("/item/")
        || path.contains("/katalog/")
        || path.contains("/catalog/")
        || path.contains("/product/")
        || path.contains("/products/")
}

/// Карточка одного товара. Каталог, раздел и страница «наши продукты» без слага сюда не входят.
pub fn is_product_card(url: &str) -> bool {
    let path = path_of(url);
    if path.contains("/catalog/") && path.contains("/detail") {
        return true;
    }
    for needle in ["/product/", "/products/", "/item/", "/goods/", "/tovar/"] {
        if let Some(rest) = path.split_once(needle).map(|(_, rest)| rest) {
            let slug = rest.split(['/', '?', '#']).next().unwrap_or("");
            if !slug.is_empty() {
                return true;
            }
        }
    }
    false
}

/// Галерея или страница фото, не статья с картинкой в шапке.
pub fn is_image_page(url: &str) -> bool {
    let path = path_of(url);
    path.contains("/photo")
        || path.contains("/gallery")
        || path.contains("/galere")
        || path.contains("/foto")
        || path.contains("/image")
}

/// Страница ролика. Виджет YouTube в подвале сюда не входит.
pub fn is_video_page(url: &str) -> bool {
    let path = path_of(url);
    path.contains("/video") || path.contains("/watch") || path.contains("/clip") || path.contains("/shorts")
}

/// Канонический адрес публичной карточки канала, группы или бота. Пост, ролик и личный чат — None.
pub fn channel_entity_url(raw: &str) -> Option<String> {
    let trimmed = raw
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | ')' | '>' | ',' | ';'));
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    let without_scheme = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .or_else(|| lower.strip_prefix("//"))
        .unwrap_or(&lower);
    let without_www = without_scheme.strip_prefix("www.").unwrap_or(without_scheme);
    let (host_raw, rest) = match without_www.split_once('/') {
        Some(pair) => pair,
        None => (without_www.split(['?', '#']).next().unwrap_or(""), ""),
    };
    let host = host_raw.split(':').next().unwrap_or("").trim();
    if !exact_listed(host, CHANNEL_VISIT) {
        return None;
    }
    let path = rest.split(['?', '#']).next().unwrap_or("").trim_matches('/');
    if path.is_empty() {
        return None;
    }
    let orig_after = trimmed
        .split("://")
        .nth(1)
        .unwrap_or(trimmed.trim_start_matches('/'));
    let orig_after = orig_after
        .strip_prefix("www.")
        .or_else(|| orig_after.strip_prefix("WWW."))
        .unwrap_or(orig_after);
    let orig_path = orig_after
        .split_once('/')
        .map(|(_, r)| r.split(['?', '#']).next().unwrap_or("").trim_matches('/'))
        .unwrap_or(path);
    match host {
        "t.me" | "telegram.me" => telegram_entity(orig_path),
        "max.ru" => max_entity(orig_path),
        "rutube.ru" => rutube_entity(path),
        "vk.com" | "vk.ru" => vk_entity(orig_path, path),
        "ok.ru" => ok_entity(path),
        "dzen.ru" => dzen_entity(orig_path, path),
        "youtube.com" => youtube_entity(orig_path),
        "vkvideo.ru" => vkvideo_entity(orig_path),
        _ => None,
    }
}

/// Пост, ролик, главная площадки, служебный путь. Не карточка сущности.
pub fn is_channel_item(url: &str) -> bool {
    let Some(host) = host_of_url(url) else {
        return false;
    };
    is_channel_host(&host) && channel_entity_url(url).is_none()
}

/// Служебные адреса площадки и любой не-сущностный URL на канальном хосте.
pub fn is_channel_noise(url: &str) -> bool {
    is_channel_item(url)
}

/// Россия/СНГ или публичная карточка канала на t.me / vk.com / youtube.com.
pub fn url_in_focus(url: &str) -> bool {
    let Some(host) = host_of_url(url) else {
        return false;
    };
    host_in_focus(&host) || channel_entity_url(url).is_some()
}

fn telegram_entity(orig_path: &str) -> Option<String> {
    let segs: Vec<&str> = orig_path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return None;
    }
    let first = segs[0];
    let low = first.to_ascii_lowercase();
    if matches!(
        low.as_str(),
        "share" | "iv" | "addstickers" | "proxy" | "socks" | "joinchat" | "c" | "addlist"
    ) {
        return None;
    }
    if first.starts_with('+') {
        return None;
    }
    if low == "s" {
        if segs.len() != 2 || !entity_handle(segs[1]) {
            return None;
        }
        return Some(format!("https://t.me/{}", segs[1]));
    }
    if segs.len() != 1 || !entity_handle(first) {
        return None;
    }
    Some(format!("https://t.me/{first}"))
}

fn max_entity(orig_path: &str) -> Option<String> {
    let segs: Vec<&str> = orig_path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.len() != 1 {
        return None;
    }
    let handle = segs[0];
    let low = handle.to_ascii_lowercase();
    if MAX_RESERVED.contains(&low.as_str()) {
        return None;
    }
    if !entity_handle(handle) {
        return None;
    }
    Some(format!("https://max.ru/{handle}"))
}

fn rutube_entity(path: &str) -> Option<String> {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.len() < 2 {
        return None;
    }
    match segs[0] {
        "u" | "channel" => {
            if segs[1].is_empty() {
                return None;
            }
            Some(format!("https://rutube.ru/{}/{}", segs[0], segs[1]))
        }
        _ => None,
    }
}

fn vk_entity(orig_path: &str, path: &str) -> Option<String> {
    let segs: Vec<&str> = orig_path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.len() != 1 {
        return None;
    }
    let seg = segs[0];
    let low = path.split('/').next().unwrap_or("").to_ascii_lowercase();
    if matches!(
        low.as_str(),
        "share.php"
            | "feed"
            | "login"
            | "im"
            | "friends"
            | "settings"
            | "video"
            | "wall"
            | "music"
            | "clips"
            | "write"
            | "mail"
            | "app"
            | "widget_community.php"
    ) || low.starts_with("wall")
        || low.starts_with("video")
        || low.starts_with("photo")
    {
        return None;
    }
    if entity_handle(seg) {
        return Some(format!("https://vk.com/{seg}"));
    }
    None
}

fn ok_entity(path: &str) -> Option<String> {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return None;
    }
    if segs[0] == "group" && segs.len() >= 2 && segs[1].chars().all(|c| c.is_ascii_digit()) {
        return Some(format!("https://ok.ru/group/{}", segs[1]));
    }
    if segs.len() == 1 {
        let low = segs[0];
        if matches!(low, "dk" | "video" | "profile" | "feed" | "messages" | "games" | "music" | "live")
            || low.starts_with("dk")
        {
            return None;
        }
        if entity_handle(segs[0]) {
            return Some(format!("https://ok.ru/{}", segs[0]));
        }
    }
    None
}

fn dzen_entity(orig_path: &str, path: &str) -> Option<String> {
    let segs: Vec<&str> = orig_path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.len() != 1 {
        return None;
    }
    let low = path.split('/').next().unwrap_or("");
    if matches!(low, "news" | "a" | "media" | "video" | "topic" | "search" | "login") {
        return None;
    }
    if entity_handle(segs[0]) {
        return Some(format!("https://dzen.ru/{}", segs[0]));
    }
    None
}

fn youtube_entity(orig_path: &str) -> Option<String> {
    let segs: Vec<&str> = orig_path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return None;
    }
    let first = segs[0];
    let low = first.to_ascii_lowercase();
    if first.starts_with('@') && segs.len() == 1 && first.len() >= 3 {
        return Some(format!("https://youtube.com/{first}"));
    }
    if matches!(low.as_str(), "channel" | "c" | "user") && segs.len() >= 2 {
        return Some(format!("https://youtube.com/{}/{}", first, segs[1]));
    }
    None
}

fn vkvideo_entity(orig_path: &str) -> Option<String> {
    let segs: Vec<&str> = orig_path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return None;
    }
    if segs[0].starts_with('@') && segs.len() == 1 {
        return Some(format!("https://vkvideo.ru/{}", segs[0]));
    }
    if segs[0].eq_ignore_ascii_case("channel") && segs.len() >= 2 {
        return Some(format!("https://vkvideo.ru/channel/{}", segs[1]));
    }
    None
}

fn entity_handle(s: &str) -> bool {
    s.len() >= 2
        && s.len() <= 64
        && s.chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

/// Сырой PDF попал в заголовок или текст. Такой документ в индекс не кладём.
pub fn is_raw_office_text(title: &str, text: &str) -> bool {
    looks_like_pdf(title) || looks_like_pdf(text)
}

fn looks_like_pdf(value: &str) -> bool {
    let value = value.trim_start_matches('\u{FEFF}').trim();
    value.starts_with("%PDF") || value.contains("%PDF-")
}

fn theme_kind(title: &str, text: &str) -> u32 {
    let blob = format!("{title}\n{text}").to_lowercase();
    let mut kind = 0;
    if has_any(
        &blob,
        &[
            "автосервис",
            "автосалон",
            "автозапчаст",
            "шиномонтаж",
            "кузовной ремонт",
            "автотехцентр",
            "автодилер",
            "станция технического обслуживания",
            "продажа автомобилей",
            "автошкол",
        ],
    ) {
        kind |= KIND_AUTO;
    }
    if has_any(
        &blob,
        &[
            "агентство недвижимости",
            "агентства недвижимости",
            "риэлтор",
            "риелтор",
            "застройщик",
            "новостройк",
            "жилой комплекс",
            "продажа квартир",
            "аренда квартир",
        ],
    ) {
        kind |= KIND_REALTY;
    }
    if has_any(
        &blob,
        &[
            "строительная компания",
            "строительной компании",
            "строительство домов",
            "строительство коттедж",
            "ремонт квартир",
            "ремонт домов",
            "отделочные работы",
            "генподряд",
            "строительно-монтаж",
        ],
    ) {
        kind |= KIND_BUILD;
    }
    if edu_title(&title.to_lowercase())
        || has_any(
            &blob,
            &[
                "детский сад",
                "колледж",
                "лицей",
                "гимназия",
                "учебный центр",
                "общеобразовательн",
                "учебные курсы",
                "онлайн-курсы",
            ],
        )
    {
        kind |= KIND_EDU;
    }
    if has_any(
        &blob,
        &[
            "рекламное агентство",
            "рекламного агентства",
            "маркетинговое агентство",
            "маркетингового агентства",
            "агентство интернет-рекламы",
            "digital-агентство",
            "диджитал-агентство",
            "контекстная реклама",
            "контекстную рекламу",
            "таргетированная реклама",
            "smm-агентство",
            "агентство smm",
            "performance-маркетинг",
        ],
    ) {
        kind |= KIND_ADS;
    }
    if has_any(
        &blob,
        &[
            "искусственный интеллект",
            "искусственного интеллекта",
            "искусственным интеллектом",
            "нейросет",
            "машинное обучение",
            "машинного обучения",
            "языковая модель",
            "языковые модели",
        ],
    ) {
        kind |= KIND_AI;
    }
    kind
}

fn has_any(blob: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| blob.contains(needle))
}

fn edu_title(title: &str) -> bool {
    has_any(
        title,
        &[
            "университет",
            "школа",
            "детский сад",
            "колледж",
            "лицей",
            "гимназия",
            "учебный центр",
        ],
    )
}

pub fn is_haul_text(title: &str, text: &str) -> bool {
    let blob = format!("{title}\n{text}").to_lowercase();
    blob.contains("грузоперевоз")
        || blob.contains("транспортная компания")
        || blob.contains("транспортной компании")
        || blob.contains("логистическая компания")
        || blob.contains("логистической компании")
        || blob.contains("логистических услуг")
        || blob.contains("экспедитор")
        || blob.contains("перевозка груз")
        || blob.contains("перевозки груз")
        || blob.contains("грузовые перевоз")
}

/// Тип по адресу и уже сохранённому тексту. Картинка и видео без HTML не видны.
pub fn kind_from_stored(url: &str, title: &str, text: &str) -> u32 {
    let host = host_of_url(url).unwrap_or_default();
    let bare = bare_host(&host);
    let path = path_of(url);
    let blob = format!("{title}\n{text}").to_lowercase();
    let mut kind = 0_u32;
    if is_press_host(&bare) {
        return KIND_NEWS;
    }
    if is_government_host(&bare) {
        kind |= KIND_GOV;
    }
    if path_has(&path, &["/news/", "/novosti/", "/press/"]) {
        kind |= KIND_NEWS;
    }
    if is_document_url(url) {
        kind |= KIND_DOC;
    }
    if is_channel_host(&bare) {
        let mut kind = KIND_CHANNEL | theme_kind(title, text);
        if is_video_channel_host(&bare) {
            kind |= KIND_VIDEO;
        }
        return kind;
    }
    let blocked = is_marketplace_host(&bare) || is_job_board(&bare);
    let on_product = is_product_url(url) || is_product_card(url);
    if !blocked && path_has(&path, &["/vacancy", "/vacancies", "/vakans", "/karera", "/career", "/jobs"]) {
        kind |= KIND_JOBS;
    }
    let equipment = on_product
        && (blob.contains("оборудование")
            || blob.contains("медтехник")
            || blob.contains("расходные материал")
            || blob.contains("купить аппарат"));
    let pharmacy = bare.contains("аптек") || title.to_lowercase().contains("аптека");
    let clinic = blob.contains("запись на приём")
        || blob.contains("запись на прием")
        || blob.contains("запись к врачу")
        || path_has(&path, &["/vrachi", "/doctor", "/clinic", "/klinika"]);
    if !blocked && !is_government_host(&bare) && !equipment && !pharmacy && clinic {
        kind |= KIND_MEDICINE;
    }
    if !blocked && (equipment || on_product) {
        kind |= KIND_GOODS;
    }
    if !blocked && is_webshop(title, text) {
        kind |= KIND_SHOP;
    }
    if !blocked && is_haul_text(title, text) && kind & KIND_SHOP == 0 && !is_webshop(title, text) {
        kind |= KIND_HAUL;
    }
    if !blocked {
        kind |= theme_kind(title, text);
    }
    if !blocked && kind & (KIND_GOODS | KIND_JOBS) == 0 && path_has(&path, &["/uslugi", "/services"]) {
        kind |= KIND_SERVICE;
    }
    if is_government_host(&bare) {
        kind &= !GOV_OFF;
        kind |= KIND_GOV;
    }
    kind
}

fn listed(host: &str, items: &[&str]) -> bool {
    let host = bare_host(host);
    items.iter().any(|item| host == *item || host.ends_with(&format!(".{item}")))
}

fn exact_listed(host: &str, items: &[&str]) -> bool {
    let host = bare_host(host);
    items.iter().any(|item| host == *item)
}

fn path_of(url: &str) -> String {
    let Some((_, rest)) = url.split_once("://") else {
        return url.to_ascii_lowercase();
    };
    let path = rest.split(['?', '#']).next().unwrap_or(rest);
    let path = path.split_once('/').map(|(_, path)| path).unwrap_or("");
    format!("/{path}").to_ascii_lowercase()
}

fn path_has(path: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| path.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_stays_out_of_the_general_list() {
        let kind = kind_from_stored(
            "https://rutube.ru/u/stident/",
            "S.T.I. Dent",
            "канал стоматологической практики",
        );
        assert_eq!(kind & KIND_CHANNEL, KIND_CHANNEL);
        assert_eq!(kind & KIND_VIDEO, KIND_VIDEO);
        assert!(!in_general(kind));
        assert!(is_channel_host("www.vk.com"));
        assert!(!is_channel_host("example.ru"));
        assert!(!is_channel_host("business.max.ru"));
        let site = site_ask("site:www.Example.com крепёж").unwrap();
        assert_eq!(site.host, "example.com");
        assert_eq!(site.words, "крепёж");
        assert_eq!(site_ask("сайт:associaciasip.ru").unwrap().host, "associaciasip.ru");
        assert!(site_ask("новости россия").is_none());
        assert!(site_ask("site:127.0.0.1").is_none());
        let news = kind_from_stored("https://example.ru/news/1", "заметка", "текст");
        assert_eq!(news & KIND_NEWS, KIND_NEWS);
        assert!(!in_general(news));
        assert!(on_shelf(KIND_NEWS, KIND_NEWS, KIND_MEDICINE));
        assert!(on_shelf(KIND_NEWS | KIND_MEDICINE, KIND_NEWS, KIND_MEDICINE));
        assert!(!on_shelf(KIND_NEWS | KIND_AUTO, KIND_NEWS, KIND_MEDICINE));
        assert!(on_shelf(KIND_JOBS | KIND_MEDICINE, KIND_JOBS, KIND_MEDICINE));
        assert!(!on_shelf(KIND_JOBS, KIND_JOBS, KIND_MEDICINE));
        assert!(on_shelf(KIND_MEDICINE, 0, KIND_MEDICINE));
        assert!(!on_shelf(KIND_NEWS | KIND_MEDICINE, 0, KIND_MEDICINE));
        let article = kind_from_stored("https://www.kommersant.ru/doc/1", "заголовок", "текст");
        assert_eq!(article, KIND_NEWS);
        assert!(is_press_front("https://lenta.ru/"));
        assert!(!is_press_front("https://lenta.ru/news/1"));
        assert!(is_model_front("https://chatgpt.com/"));
        assert!(!is_model_front("https://chatgpt.com/c/abc"));
        assert!(!is_model_front(
            "https://chatgpt.com/?hints=search&q=Read%20from%20https://docs.openclaw.ai/index.md"
        ));
        assert!(is_model_maker_host("www.claude.ai"));
        assert!(!is_press_host("zavod.example"));
        assert_eq!(kind_from_stored("https://zavod.example/about", "новости компании", "текст") & KIND_NEWS, 0);
    }

    #[test]
    fn government_is_the_host_not_the_word() {
        assert!(is_government_host("rkn.gov.ru"));
        assert!(is_government_host("www.mos.ru"));
        assert!(is_government_host("consultant.ru"));
        assert!(is_government_host("www.gosuslugi.ru"));
        assert!(host_keeps_pages("fas.gov.ru"));
        assert!(host_keeps_pages("beesearch.ru"));
        assert!(host_keeps_pages("docs.openclaw.ai"));
        assert!(host_keeps_pages("docs.github.com"));
        assert!(host_keeps_pages("docs.aws.amazon.com"));
        assert!(host_keeps_pages("code.visualstudio.com"));
        assert!(is_tool_docs_host("www.openclaw.ai"));
        assert!(is_tool_docs_host("docs.anthropic.com"));
        assert!(is_tool_docs_host("developers.openai.com"));
        assert!(is_tool_docs_host("developers.sber.ru"));
        assert!(is_tool_docs_host("aistudio.yandex.ru"));
        assert!(is_tool_docs_host("docs.langchain.com"));
        assert!(is_tool_docs_host("modelcontextprotocol.io"));
        assert!(!is_tool_docs_host("chatgpt.com"));
        assert!(!is_model_inner("https://docs.anthropic.com/en/api/getting-started"));
        assert!(is_model_inner("https://chatgpt.com/c/abc"));
        assert!(!host_keeps_pages("github.com"));
        assert!(!host_keeps_pages("aws.amazon.com"));
        assert!(!host_keeps_pages("vscode.dev"));
        assert!(is_hive_host("www.beesearch.ru"));
        assert!(!host_keeps_pages("plant.ru"));
        assert_eq!(kind_from_stored("https://blog.example/post", "министерство", "слово"), 0);
    }

    #[test]
    fn job_board_and_factory_vacancy() {
        assert_eq!(kind_from_stored("https://hh.ru/vacancy/1", "токарь", "вакансия"), 0);
        assert_eq!(
            kind_from_stored("https://zavod.example/vacancy/tokar", "токарь", "смена") & KIND_JOBS,
            KIND_JOBS
        );
    }

    #[test]
    fn clinic_is_medicine_and_equipment_is_goods() {
        let clinic = kind_from_stored("https://klinika.example/vrachi", "Терапевт", "запись на приём");
        assert_eq!(clinic & KIND_MEDICINE, KIND_MEDICINE);
        assert_eq!(clinic & KIND_GOODS, 0);
        let device = kind_from_stored(
            "https://medshop.example/catalog/apparat",
            "Купить аппарат",
            "стоматологическое оборудование",
        );
        assert_eq!(device & KIND_MEDICINE, 0);
        assert_eq!(device & KIND_GOODS, KIND_GOODS);
    }

    #[test]
    fn ozon_is_not_goods() {
        let kind = kind_from_stored("https://www.ozon.ru/product/phone", "телефон", "цена 1000");
        assert_eq!(kind & KIND_GOODS, 0);
        assert_eq!(kind & KIND_SHOP, 0);
    }

    #[test]
    fn factory_catalog_stays_and_a_shop_card_is_marked() {
        let factory = kind_from_stored(
            "https://zavod.example/catalog/bolt",
            "Завод крепежа",
            "опт для хозяйств",
        );
        assert_eq!(factory & KIND_GOODS, KIND_GOODS);
        assert_eq!(factory & KIND_SHOP, 0);
        let card = kind_from_stored("https://shop.example/product/bolt", "Болт", "карточка");
        assert_eq!(card & KIND_SHOP, 0);
        assert_eq!(card & KIND_GOODS, KIND_GOODS);
        let shop_card = kind_from_stored(
            "https://shop.example/product/bolt",
            "Болт",
            "купить в интернет-магазине",
        );
        assert_eq!(shop_card & KIND_SHOP, KIND_SHOP);
        let haul = kind_from_stored("https://trans.example/", "Транспортная компания", "грузоперевозки по России");
        assert_eq!(haul & KIND_HAUL, KIND_HAUL);
        let delivery = kind_from_stored("https://zavod.example/about", "Завод", "доставка по России");
        assert_eq!(delivery & KIND_HAUL, 0);
        assert!(is_product_card("https://www.wildberries.ru/catalog/1/detail.aspx"));
        assert!(!is_product_card("https://www.ozon.ru/category/bolty"));
    }

    #[test]
    fn popular_themes_need_the_business_not_a_passing_word() {
        let auto = kind_from_stored("https://sto.example/", "Автосервис", "шиномонтаж и кузовной ремонт");
        assert_eq!(auto & KIND_AUTO, KIND_AUTO);
        let car = kind_from_stored("https://zavod.example/about", "Завод", "доставка автомобилей клиентам");
        assert_eq!(car & KIND_AUTO, 0);
        let flat = kind_from_stored("https://agency.example/", "Агентство недвижимости", "продажа квартир");
        assert_eq!(flat & KIND_REALTY, KIND_REALTY);
        let room = kind_from_stored("https://zavod.example/about", "Завод", "квартира для мастера");
        assert_eq!(room & KIND_REALTY, 0);
        let build = kind_from_stored("https://stroy.example/", "Строительная компания", "строительство домов");
        assert_eq!(build & KIND_BUILD, KIND_BUILD);
        let fix = kind_from_stored("https://zavod.example/about", "Завод", "ремонт оборудования");
        assert_eq!(fix & KIND_BUILD, 0);
        let school = kind_from_stored("https://school.example/", "Школа", "общеобразовательная");
        assert_eq!(school & KIND_EDU, KIND_EDU);
        let beauty = kind_from_stored("https://salon.example/", "Институт красоты", "запись");
        assert_eq!(beauty & KIND_EDU, 0);
        let ads = kind_from_stored("https://promo.example/", "Рекламное агентство", "контекстная реклама");
        assert_eq!(ads & KIND_ADS, KIND_ADS);
        let banner = kind_from_stored("https://zavod.example/about", "Завод", "реклама на сайте");
        assert_eq!(banner & KIND_ADS, 0);
        let ai = kind_from_stored("https://lab.example/", "Лаборатория", "искусственный интеллект и нейросети");
        assert_eq!(ai & KIND_AI, KIND_AI);
        let firm = kind_from_stored("https://zavod.example/about", "О компании", "наши компании");
        assert_eq!(firm & KIND_AI, 0);
        let market = kind_from_stored("https://www.ozon.ru/category/auto", "Автосервис", "автозапчасти");
        assert_eq!(market & (KIND_AUTO | KIND_ADS | KIND_AI), 0);
        let zavod = kind_from_stored(
            "https://zavod.example/catalog/bolt",
            "Завод и интернет-магазин",
            "купить оптом",
        );
        assert_eq!(zavod & KIND_GOODS, KIND_GOODS);
        assert_eq!(zavod & KIND_SHOP, KIND_SHOP);
        let saas = kind_from_stored(
            "https://lab.example/products/mag-master/",
            "MAG Master",
            "A-CRM для маркетинга",
        );
        assert_eq!(saas & KIND_SHOP, 0);
        let nalog = kind_from_stored(
            "https://www.nalog.gov.ru/rn77/",
            "ФНС",
            "запись на приём в инспекцию",
        );
        assert_eq!(nalog & KIND_GOV, KIND_GOV);
        assert_eq!(nalog & KIND_MEDICINE, 0);
        let yard = kind_from_stored(
            "https://jk.example/about/",
            "Описание",
            "во дворе школа и магазин",
        );
        assert_eq!(yard & KIND_EDU, 0);
        let ship = kind_from_stored(
            "https://shop.example/help/delivery/",
            "Доставка",
            "транспортная компания СДЭК, интернет-магазин",
        );
        assert_eq!(ship & KIND_HAUL, 0);
        assert_eq!(ship & KIND_SHOP, KIND_SHOP);
        assert!(is_channel_host("max.ru"));
        assert!(is_channel_noise("https://rutube.ru/my/history/"));
        assert!(!is_channel_noise("https://rutube.ru/u/stident/"));
        assert_eq!(
            channel_entity_url("https://t.me/s/clinic_msk"),
            Some("https://t.me/clinic_msk".into())
        );
        assert!(channel_entity_url("https://t.me/c/123").is_none());
        assert!(channel_entity_url("https://t.me/+invite").is_none());
        assert_eq!(
            channel_entity_url("https://max.ru/rkn"),
            Some("https://max.ru/rkn".into())
        );
        assert_eq!(
            channel_entity_url("https://max.ru/id7313004278_biz"),
            Some("https://max.ru/id7313004278_biz".into())
        );
        assert!(channel_entity_url("https://max.ru/").is_none());
        assert!(channel_entity_url("https://business.max.ru/").is_none());
        assert_eq!(
            channel_entity_url("https://rutube.ru/u/stident/"),
            Some("https://rutube.ru/u/stident".into())
        );
        assert!(channel_entity_url("https://rutube.ru/video/abc/").is_none());
        assert!(channel_entity_url("https://rutube.ru/plst/559288/").is_none());
        assert_eq!(
            channel_entity_url("https://vk.com/club123"),
            Some("https://vk.com/club123".into())
        );
        assert!(channel_entity_url("https://vk.com/wall-1_2").is_none());
        assert_eq!(
            channel_entity_url("https://ok.ru/group/53925560123526"),
            Some("https://ok.ru/group/53925560123526".into())
        );
        assert_eq!(
            channel_entity_url("https://www.youtube.com/@stident"),
            Some("https://youtube.com/@stident".into())
        );
        assert!(channel_entity_url("https://youtu.be/abc").is_none());
        assert!(is_channel_item("https://rutube.ru/video/abc/"));
        assert!(!is_channel_item("https://rutube.ru/u/stident/"));
        assert!(url_in_focus("https://t.me/clinic_msk"));
        assert!(!url_in_focus("https://t.me/c/1"));
        assert!(!url_in_focus("https://instagram.com/shop"));
        let max_ch = kind_from_stored(
            "https://max.ru/channel_jkkatuar",
            "ЖК Катуар",
            "жилой комплекс в мессенджере MAX",
        );
        assert_eq!(max_ch & KIND_CHANNEL, KIND_CHANNEL);
        assert_eq!(max_ch & KIND_VIDEO, 0);
        assert_eq!(max_ch & KIND_REALTY, KIND_REALTY);
        assert!(!in_general(max_ch));
        assert!(on_shelf(KIND_CHANNEL | KIND_MEDICINE, KIND_CHANNEL, KIND_MEDICINE));
        assert!(!on_shelf(KIND_CHANNEL, KIND_CHANNEL, KIND_MEDICINE));
        assert!(on_shelf(KIND_CHANNEL, KIND_CHANNEL, 0));
        assert!(is_raw_office_text("%PDF-1.5", "stream"));
        assert!(!is_raw_office_text("Политика", "обработка персональных данных"));
        assert_eq!(tab_mask("docs"), KIND_DOC);
        assert_eq!(tab_mask("goods"), KIND_GOODS);
        assert_eq!(tab_mask("images"), KIND_IMAGE);
        assert_eq!(tab_mask("video"), KIND_VIDEO);
        assert_eq!(tab_mask("channel"), KIND_CHANNEL);
        assert_eq!(kind_codes(KIND_DOC | KIND_AI), vec!["ai", "docs"]);
        assert_eq!(all_kinds().len(), 18);
    }

    #[test]
    fn focus_keeps_cis_and_model_makers() {
        assert!(host_in_focus("tehnoveles.ru"));
        assert!(host_in_focus("www.doctor.by"));
        assert!(host_in_focus("пасека.рф"));
        assert!(host_in_focus("api.openai.com"));
        assert!(host_in_focus("mistral.ai"));
        assert!(host_in_focus("gemini.google.com"));
        assert!(host_in_focus("docs.github.com"));
        assert!(host_in_focus("docs.openclaw.ai"));
        assert!(host_in_focus("docs.aws.amazon.com"));
        assert!(host_in_focus("code.visualstudio.com"));
        assert!(host_in_focus("docs.anthropic.com"));
        assert!(host_in_focus("developers.sber.ru"));
        assert!(host_in_focus("docs.langchain.com"));
        assert!(host_in_focus("giga.chat"));
        assert!(!host_in_focus("google.com"));
        assert!(!host_in_focus("amazon.com"));
        assert!(!host_in_focus("moex.com"));
        assert!(!host_in_focus("en.wikipedia.org"));
        assert!(!host_in_focus("ru.wikipedia.org"));
        assert!(!host_in_focus("ru.ruwiki.ru"));
        assert!(!host_in_focus("45.158.46.52"));
    }
}
