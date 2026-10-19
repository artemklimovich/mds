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
/// Страница живёт на видеоплощадке или в соцсети. В общей выдаче её нет.
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
    KindSpec { code: "channel", label: "Видеоплощадки и соцсети", mask: KIND_CHANNEL, place: "скрытая, вне выдачи" },
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

pub fn is_marketplace_host(host: &str) -> bool {
    listed(host, MARKETPLACES)
}

pub fn is_job_board(host: &str) -> bool {
    listed(host, JOB_BOARDS)
}

pub fn is_channel_host(host: &str) -> bool {
    listed(host, CHANNELS)
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
    host == "gov.ru" || host.ends_with(".gov.ru") || listed(&host, GOVERNMENT)
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
    path.contains("/product")
        || path.contains("/tovar")
        || path.contains("/goods/")
        || path.contains("/item/")
        || path.contains("/katalog/")
        || path.contains("/catalog/")
}

/// Карточка одного товара. Каталог и раздел сюда не входят.
pub fn is_product_card(url: &str) -> bool {
    let path = path_of(url);
    path.contains("/detail")
        || path.contains("/product")
        || path.contains("/item/")
        || path.contains("/goods/")
        || path.contains("/tovar/")
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
    if has_any(
        &blob,
        &[
            "университет",
            "детский сад",
            "колледж",
            "лицей",
            "гимназия",
            "учебный центр",
            "общеобразовательн",
            "учебные курсы",
            "онлайн-курсы",
            "школа",
        ],
    ) {
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
        return KIND_VIDEO | KIND_CHANNEL;
    }
    let blocked = is_marketplace_host(&bare) || is_job_board(&bare);
    if !blocked && path_has(&path, &["/vacancy", "/vacancies", "/vakans", "/karera", "/career", "/jobs"]) {
        kind |= KIND_JOBS;
    }
    let equipment = blob.contains("оборудование")
        || blob.contains("медтехник")
        || blob.contains("расходные материал")
        || blob.contains("купить аппарат");
    let pharmacy = bare.contains("аптек") || title.to_lowercase().contains("аптека");
    let clinic = blob.contains("запись на приём")
        || blob.contains("запись на прием")
        || blob.contains("запись к врачу")
        || path_has(&path, &["/vrachi", "/doctor", "/clinic", "/klinika"]);
    if !blocked && !equipment && !pharmacy && clinic {
        kind |= KIND_MEDICINE;
    }
    if !blocked && (equipment || is_product_url(url)) {
        kind |= KIND_GOODS;
    }
    if !blocked && is_product_card(url) {
        kind |= KIND_SHOP;
    }
    if !blocked && is_haul_text(title, text) {
        kind |= KIND_HAUL;
    }
    if !blocked {
        kind |= theme_kind(title, text);
    }
    if !blocked && kind & (KIND_GOODS | KIND_JOBS) == 0 && path_has(&path, &["/uslugi", "/services"]) {
        kind |= KIND_SERVICE;
    }
    kind
}

fn listed(host: &str, items: &[&str]) -> bool {
    let host = bare_host(host);
    items.iter().any(|item| host == *item || host.ends_with(&format!(".{item}")))
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
            "https://rutube.ru/video/abc/",
            "Трамп уберёт Зеленского",
            "смотреть видео",
        );
        assert_eq!(kind, KIND_VIDEO | KIND_CHANNEL);
        assert!(!in_general(kind));
        assert!(is_channel_host("www.vk.com"));
        assert!(!is_channel_host("example.ru"));
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
        assert!(!is_press_host("zavod.example"));
        assert_eq!(kind_from_stored("https://zavod.example/about", "новости компании", "текст") & KIND_NEWS, 0);
    }

    #[test]
    fn government_is_the_host_not_the_word() {
        assert!(is_government_host("rkn.gov.ru"));
        assert!(is_government_host("www.mos.ru"));
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
        assert_eq!(card & KIND_SHOP, KIND_SHOP);
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
        assert_eq!(tab_mask("docs"), KIND_DOC);
        assert_eq!(tab_mask("goods"), KIND_GOODS);
        assert_eq!(tab_mask("images"), KIND_IMAGE);
        assert_eq!(tab_mask("video"), KIND_VIDEO);
        assert_eq!(tab_mask("channel"), KIND_CHANNEL);
        assert_eq!(kind_codes(KIND_DOC | KIND_AI), vec!["ai", "docs"]);
        assert_eq!(all_kinds().len(), 18);
    }
}
