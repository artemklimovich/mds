use mds_types::{
    is_channel_host, is_document_url, is_marketplace_host, is_press_host, is_product_card, kind_from_stored,
    KIND_ADS, KIND_AI, KIND_AUTO, KIND_BUILD, KIND_CHANNEL, KIND_EDU, KIND_GOODS, KIND_HAUL, KIND_IMAGE, KIND_JOBS, KIND_MEDICINE, KIND_NEWS, KIND_REALTY, KIND_SERVICE, KIND_SHOP, KIND_VIDEO,
};
use url::Url;

use crate::hygiene::resolve_href;
use crate::{simhash, FetchedPage};

pub struct PageKind {
    pub kind: u32,
    pub media_url: Option<String>,
    pub feed_url: Option<String>,
    pub news_feed: Option<String>,
    pub price: Option<String>,
    pub docs: Vec<FetchedPage>,
}

pub fn classify_html(url: &str, host: &str, title: &str, text: &str, html: &str) -> PageKind {
    let mut kind = kind_from_stored(url, title, text);
    let lower = html.to_ascii_lowercase();
    let blocked = is_marketplace_host(host);
    if json_type(&lower, "newsarticle") {
        kind |= KIND_NEWS;
    }
    if !blocked && (json_type(&lower, "jobposting") || lower.contains("\"@type\":\"jobposting\"")) {
        kind |= KIND_JOBS;
    }
    if !blocked && (json_type(&lower, "product") || json_type(&lower, "offer")) {
        kind |= KIND_GOODS;
    }
    if !blocked && (is_product_card(url) || html_has_product_card(&lower) || json_type(&lower, "product") || json_type(&lower, "offer")) {
        kind |= KIND_SHOP;
    }
    if !blocked {
        let medical = json_type(&lower, "physician")
            || json_type(&lower, "dentist")
            || json_type(&lower, "medicalclinic")
            || json_type(&lower, "hospital");
        let equipment = text.to_lowercase().contains("оборудование") || title.to_lowercase().contains("оборудование");
        let pharmacy = title.to_lowercase().contains("аптека");
        if medical && !equipment && !pharmacy {
            kind |= KIND_MEDICINE;
        }
        if equipment {
            kind &= !KIND_MEDICINE;
            kind |= KIND_GOODS;
        }
        kind |= json_theme(&lower);
    }
    if blocked {
        kind &= !(KIND_JOBS | KIND_SERVICE | KIND_MEDICINE | KIND_GOODS | KIND_SHOP | KIND_AUTO | KIND_REALTY | KIND_BUILD | KIND_EDU | KIND_ADS | KIND_AI);
    }
    if !blocked && kind & (KIND_GOODS | KIND_JOBS) == 0 && json_type(&lower, "service") {
        kind |= KIND_SERVICE;
    }
    let feed_url = yml_href(url, html);
    let news_feed = news_href(url, html).or_else(|| mds_types::press_feed(host).map(str::to_string));
    let price = if kind & KIND_GOODS != 0 { price_of(html) } else { None };
    let (image, video) = media_flags(url, text, html);
    if image {
        kind |= KIND_IMAGE;
    }
    if video {
        kind |= KIND_VIDEO;
    }
    if is_channel_host(host) {
        kind |= KIND_VIDEO | KIND_CHANNEL;
        kind &= !(KIND_JOBS | KIND_SERVICE | KIND_MEDICINE | KIND_GOODS | KIND_NEWS | KIND_SHOP | KIND_HAUL | KIND_AUTO | KIND_REALTY | KIND_BUILD | KIND_EDU | KIND_ADS | KIND_AI);
    }
    if is_press_host(host) {
        kind = KIND_NEWS;
    }
    let media_url = if kind & KIND_IMAGE != 0 { first_image(url, html) } else { None };
    let docs = document_pages(url, host, html);
    PageKind {
        kind,
        media_url,
        feed_url,
        news_feed,
        price,
        docs,
    }
}

fn json_type(lower_html: &str, name: &str) -> bool {
    lower_html.contains(&format!("\"@type\":\"{name}\""))
        || lower_html.contains(&format!("\"@type\": \"{name}\""))
}

fn json_theme(lower_html: &str) -> u32 {
    let mut kind = 0;
    if json_type(lower_html, "autodealer")
        || json_type(lower_html, "autorepair")
        || json_type(lower_html, "autopartsstore")
        || json_type(lower_html, "motorcycledealer")
    {
        kind |= KIND_AUTO;
    }
    if json_type(lower_html, "realestateagent") {
        kind |= KIND_REALTY;
    }
    if json_type(lower_html, "generalcontractor") || json_type(lower_html, "homeandconstructionbusiness") {
        kind |= KIND_BUILD;
    }
    if json_type(lower_html, "educationalorganization")
        || json_type(lower_html, "school")
        || json_type(lower_html, "collegeoruniversity")
        || json_type(lower_html, "highschool")
        || json_type(lower_html, "preschool")
    {
        kind |= KIND_EDU;
    }
    kind
}

fn media_flags(url: &str, text: &str, html: &str) -> (bool, bool) {
    let lower = html.to_ascii_lowercase();
    let path = url.to_ascii_lowercase();
    let video = lower.contains("<video")
        || og_type(&lower, "video")
        || lower.contains("youtube.com/embed")
        || lower.contains("youtube.com/watch")
        || lower.contains("rutube.ru/")
        || lower.contains("player.vimeo.com")
        || lower.contains("vk.com/video")
        || lower.contains("vkvideo.ru");
    let short = text.chars().count() < 400;
    let image_path = path.contains("/photo") || path.contains("/gallery") || path.contains("/foto") || path.contains("/image");
    let image = (short || image_path || og_type(&lower, "image")) && lower.contains("<img");
    (image, video)
}

fn og_type(lower_html: &str, value: &str) -> bool {
    lower_html.contains("property=\"og:type\"") && lower_html.contains(&format!("content=\"{value}"))
}

fn news_href(base: &str, html: &str) -> Option<String> {
    let Ok(base) = Url::parse(base) else {
        return None;
    };
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower[from..].find("<link") {
        let start = from + rel;
        let tag_end = lower[start..].find('>').map(|pos| start + pos).unwrap_or(html.len());
        let tag = &html[start..tag_end.min(html.len())];
        from = (tag_end + 1).min(html.len());
        let tag_lower = tag.to_ascii_lowercase();
        let channel = tag_lower.contains("rss+xml") || tag_lower.contains("atom+xml") || tag_lower.contains("feed+json");
        if !tag_lower.contains("alternate") || !channel {
            continue;
        }
        let Some(href) = attr_quoted(tag, "href") else {
            continue;
        };
        let Some(joined) = resolve_href(&base, href) else {
            continue;
        };
        if let Some(feed) = mds_types::open_feed(&joined.to_string()) {
            return Some(feed);
        }
    }
    None
}

fn attr_quoted<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let key = format!("{name}=");
    let rel = lower.find(&key)?;
    let rest = &tag[rel + key.len()..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let body = &rest[quote.len_utf8()..];
    let end = body.find(quote)?;
    Some(&body[..end])
}

fn yml_href(base: &str, html: &str) -> Option<String> {
    let Ok(base) = Url::parse(base) else {
        return None;
    };
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower[from..].find("href=") {
        let start = from + rel + 5;
        let bytes = html[start..].as_bytes();
        if bytes.is_empty() {
            break;
        }
        let quote = bytes[0] as char;
        if quote != '"' && quote != '\'' {
            from = start;
            continue;
        }
        let rest = &html[start + 1..];
        let Some(end) = rest.find(quote) else {
            break;
        };
        let raw = &rest[..end];
        from = start + 1 + end;
        let raw_lower = raw.to_ascii_lowercase();
        if !raw_lower.contains("yml") && !raw_lower.ends_with(".xml") {
            continue;
        }
        if raw_lower.contains("yml") || raw_lower.contains("yandex") {
            if let Some(joined) = resolve_href(&base, raw) {
                return Some(joined.to_string());
            }
        }
    }
    None
}

fn price_of(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let key = "itemprop=\"price\"";
    let Some(rel) = lower.find(key) else {
        return None;
    };
    let window = &html[rel..cut_at_char(html, rel, 180)];
    let window_lower = window.to_ascii_lowercase();
    let content = window_lower.find("content=\"").map(|pos| pos + 9)?;
    let rest = &window[content..];
    let end = rest.find('"')?;
    let price = rest[..end].trim();
    if price.is_empty() || price.chars().count() > 24 {
        None
    } else {
        Some(price.to_string())
    }
}

fn cut_at_char(text: &str, start: usize, span: usize) -> usize {
    let mut end = text.len().min(start.saturating_add(span));
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

fn first_image(base: &str, html: &str) -> Option<String> {
    let Ok(base) = Url::parse(base) else {
        return None;
    };
    let lower = html.to_ascii_lowercase();
    let Some(rel) = lower.find("<img") else {
        return None;
    };
    let tag = &html[rel..cut_at_char(html, rel, 400)];
    let tag_lower = tag.to_ascii_lowercase();
    let src_at = tag_lower.find("src=")?;
    let bytes = tag[src_at + 4..].as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let quote = bytes[0] as char;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &tag[src_at + 5..];
    let end = rest.find(quote)?;
    let raw = rest[..end].trim();
    if raw.is_empty() || raw.starts_with("data:") {
        return None;
    }
    resolve_href(&base, raw).map(|url| url.to_string())
}

fn document_pages(base: &str, host: &str, html: &str) -> Vec<FetchedPage> {
    let Ok(base_url) = Url::parse(base) else {
        return Vec::new();
    };
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    let mut docs = Vec::new();
    while let Some(rel) = lower[from..].find("href=") {
        let start = from + rel + 5;
        let bytes = html[start..].as_bytes();
        if bytes.is_empty() {
            break;
        }
        let quote = bytes[0] as char;
        if quote != '"' && quote != '\'' {
            from = start;
            continue;
        }
        let rest = &html[start + 1..];
        let Some(end) = rest.find(quote) else {
            break;
        };
        let raw = &rest[..end];
        from = start + 1 + end;
        let Some(joined) = resolve_href(&base_url, raw) else {
            continue;
        };
        let url = joined.to_string();
        if !is_document_url(&url) || is_marketplace_host(host) {
            continue;
        }
        let anchor = anchor_after(html, from);
        let title = if anchor.is_empty() {
            joined
                .path_segments()
                .and_then(|mut parts| parts.next_back())
                .unwrap_or("документ")
                .to_string()
        } else {
            anchor
        };
        let text = title.clone();
        docs.push(FetchedPage {
            url,
            host: host.to_string(),
            title,
            text: text.clone(),
            simhash: simhash(&text),
            links: Vec::new(),
            anchors: Vec::new(),
            indexable: true,
            kind: mds_types::KIND_DOC,
            media_url: None,
            feed_url: None,
            news_feed: None,
            price: None,
            docs: Vec::new(),
            priority: Vec::new(),
        });
        if docs.len() >= 8 {
            break;
        }
    }
    docs
}

fn anchor_after(html: &str, from: usize) -> String {
    let rest = &html[from..];
    let Some(open) = rest.find('>') else {
        return String::new();
    };
    let body = &rest[open + 1..];
    let Some(close) = body.to_ascii_lowercase().find("</a>") else {
        return String::new();
    };
    let raw = &body[..close];
    let mut out = String::new();
    let mut skip = false;
    for ch in raw.chars() {
        if ch == '<' {
            skip = true;
            continue;
        }
        if ch == '>' {
            skip = false;
            continue;
        }
        if !skip {
            out.push(ch);
        }
    }
    let text = out.split_whitespace().collect::<Vec<_>>().join(" ");
    text.chars().take(120).collect()
}

pub fn drop_marketplace_cards(links: &mut Vec<String>) {
    links.retain(|url| {
        let host = mds_types::host_of_url(url).unwrap_or_default();
        !(is_marketplace_host(&host) && is_product_card(url))
    });
}

fn html_has_product_card(lower_html: &str) -> bool {
    lower_html.contains("/product")
        || lower_html.contains("/item/")
        || lower_html.contains("/goods/")
        || lower_html.contains("/tovar/")
        || lower_html.contains("/detail")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_tag_cut_does_not_split_a_letter() {
        let html = format!("<img src=\"a.jpg\">{}\">", "и".repeat(250));
        assert!(first_image("https://example.test/", &html).is_some());
    }

    #[test]
    fn article_picture_is_not_an_image_page() {
        let html = r#"<html><head><meta property="og:type" content="article"></head><body><img src="/hero.jpg"><p>Длинный текст статьи про завод и его цех, который никак не является галереей. Здесь много слов, чтобы страница осталась обычной веб-страницей, а не вкладкой картинок. Повтор той же мысли нужен только для длины текста и ни для чего больше, потому что короткая страница с картинкой как раз должна попасть во вкладку.</p></body></html>"#;
        let text = "Длинный текст статьи про завод и его цех, который никак не является галереей. ".repeat(8);
        let kind = classify_html("https://zavod.example/about", "zavod.example", "О заводе", &text, html);
        assert_eq!(kind.kind & KIND_IMAGE, 0);
    }

    #[test]
    fn shop_feed_is_remembered_and_offers_are_not_goods() {
        let html = r#"<html><body><p>Интернет-магазин крепежа</p><a href="/feed.yml">фид</a><a href="/product/bolt">болт</a></body></html>"#;
        let kind = classify_html(
            "https://shop.example/",
            "shop.example",
            "Интернет-магазин",
            "Интернет-магазин крепежа",
            html,
        );
        assert_eq!(kind.kind & KIND_SHOP, KIND_SHOP);
        assert!(kind.feed_url.unwrap().contains("feed.yml"));
        assert!(kind.news_feed.is_none());
        let mut links = vec![
            "https://www.ozon.ru/product/phone".into(),
            "https://www.ozon.ru/category/phones".into(),
            "https://shop.example/product/bolt".into(),
            "https://shop.example/about".into(),
        ];
        drop_marketplace_cards(&mut links);
        assert_eq!(
            links,
            vec![
                "https://www.ozon.ru/category/phones".to_string(),
                "https://shop.example/product/bolt".to_string(),
                "https://shop.example/about".to_string(),
            ]
        );
    }

    #[test]
    fn rss_link_is_the_news_channel() {
        let html = r#"<html><head><link rel="alternate" type="application/rss+xml" href="/rss/news.xml"></head><body><p>Новости завода</p></body></html>"#;
        let kind = classify_html("https://zavod.example/news", "zavod.example", "Новости", "Новости завода", html);
        assert_eq!(kind.news_feed.as_deref(), Some("https://zavod.example/rss/news.xml"));
    }

    #[test]
    fn known_paper_keeps_its_feed_without_a_link_tag() {
        let kind = classify_html("https://www.kommersant.ru/", "www.kommersant.ru", "Коммерсантъ", "новости", "<html></html>");
        assert!(kind.news_feed.unwrap().contains("kommersant.ru/rss"));
    }
}
