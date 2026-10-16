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

pub fn tab_mask(tab: &str) -> u32 {
    match tab {
        "news" => KIND_NEWS,
        "docs" => KIND_DOC,
        "gov" => KIND_GOV,
        "images" => KIND_IMAGE,
        "video" => KIND_VIDEO,
        "jobs" => KIND_JOBS,
        "services" => KIND_SERVICE,
        "medicine" => KIND_MEDICINE,
        "goods" => KIND_GOODS,
        _ => 0,
    }
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
    let shop = is_webshop(title, text);
    if !blocked && !shop && (equipment || is_product_url(url)) {
        kind |= KIND_GOODS;
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
        let news = kind_from_stored("https://example.ru/news/1", "заметка", "текст");
        assert_eq!(news & KIND_NEWS, KIND_NEWS);
        assert!(!in_general(news));
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
    }
}
