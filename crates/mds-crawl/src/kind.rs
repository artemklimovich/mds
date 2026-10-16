use mds_types::{
    is_channel_host, is_document_url, is_marketplace_host, is_press_host, is_product_url, is_webshop, kind_from_stored,
    KIND_CHANNEL, KIND_GOODS, KIND_IMAGE, KIND_JOBS, KIND_MEDICINE, KIND_NEWS, KIND_SERVICE, KIND_VIDEO,
};
use url::Url;

use crate::hygiene::resolve_href;
use crate::{simhash, FetchedPage};

pub struct PageKind {
    pub kind: u32,
    pub media_url: Option<String>,
    pub feed_url: Option<String>,
    pub price: Option<String>,
    pub docs: Vec<FetchedPage>,
}

pub fn classify_html(url: &str, host: &str, title: &str, text: &str, html: &str) -> PageKind {
    let mut kind = kind_from_stored(url, title, text);
    let lower = html.to_ascii_lowercase();
    let shop = is_webshop(title, text) || lower.contains("интернет-магазин");
    let blocked = is_marketplace_host(host);
    if json_type(&lower, "newsarticle") {
        kind |= KIND_NEWS;
    }
    if !blocked && (json_type(&lower, "jobposting") || lower.contains("\"@type\":\"jobposting\"")) {
        kind |= KIND_JOBS;
    }
    if !blocked && !shop && (json_type(&lower, "product") || json_type(&lower, "offer")) {
        kind |= KIND_GOODS;
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
            if !shop {
                kind |= KIND_GOODS;
            }
        }
    }
    if blocked {
        kind &= !(KIND_JOBS | KIND_SERVICE | KIND_MEDICINE | KIND_GOODS);
    }
    if shop {
        kind &= !KIND_GOODS;
    }
    if !blocked && kind & (KIND_GOODS | KIND_JOBS) == 0 && json_type(&lower, "service") {
        kind |= KIND_SERVICE;
    }
    let feed_url = yml_href(url, html);
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
        kind &= !(KIND_JOBS | KIND_SERVICE | KIND_MEDICINE | KIND_GOODS | KIND_NEWS);
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
        price,
        docs,
    }
}

fn json_type(lower_html: &str, name: &str) -> bool {
    lower_html.contains(&format!("\"@type\":\"{name}\""))
        || lower_html.contains(&format!("\"@type\": \"{name}\""))
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
    let window = &html[rel..html.len().min(rel + 180)];
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

fn first_image(base: &str, html: &str) -> Option<String> {
    let Ok(base) = Url::parse(base) else {
        return None;
    };
    let lower = html.to_ascii_lowercase();
    let Some(rel) = lower.find("<img") else {
        return None;
    };
    let tag = &html[rel..html.len().min(rel + 400)];
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
            indexable: true,
            kind: mds_types::KIND_DOC,
            media_url: None,
            feed_url: None,
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

pub fn drop_shop_products(links: &mut Vec<String>, shop: bool) {
    if !shop {
        return;
    }
    links.retain(|url| !is_product_url(url) && !is_document_url(url));
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(kind.kind & KIND_GOODS, 0);
        assert!(kind.feed_url.unwrap().contains("feed.yml"));
        let mut links = vec!["https://shop.example/product/bolt".into(), "https://shop.example/about".into()];
        drop_shop_products(&mut links, true);
        assert_eq!(links, vec!["https://shop.example/about".to_string()]);
    }
}
