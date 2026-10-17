use url::Url;

const FILE_EXT: &[&str] = &[
    "html", "htm", "php", "asp", "aspx", "css", "js", "json", "xml", "txt", "md", "pdf", "jpg",
    "jpeg", "png", "gif", "svg", "webp", "zip", "ico", "mp4", "mp3", "woff", "woff2",
];

const HOST_TLD: &[&str] = &[
    "ru", "su", "com", "org", "net", "io", "by", "kz", "ua", "de", "eu", "info", "biz", "pro",
    "online", "site", "shop", "store", "xyz", "app", "dev", "me", "tv", "cc", "co", "uk", "fr",
    "it", "pl", "cz", "xn--p1ai",
];

const NOT_FOUND: &[&str] = &[
    "страница не найдена",
    "страница не существует",
    "проект не найден",
    "документ не найден",
    "ничего не найдено",
    "такой страницы нет",
    "page not found",
    "404 not found",
    "error 404",
    "ошибка 404",
];

/// Путь уже содержит чужой адрес: `https://союз.ru/www.завод.ru` или `…/http://завод.ru`.
pub fn is_broken_url(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return true;
    };
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return true;
    }
    let path = parsed.path().to_ascii_lowercase();
    if path.contains("http://") || path.contains("https://") || path.contains("http:/") {
        return true;
    }
    path.split('/').any(segment_is_foreign_host)
}

pub fn is_not_found(title: &str, text: &str) -> bool {
    let title_l = title.to_lowercase();
    let title_short = title.chars().count() < 80;
    if title_short && NOT_FOUND.iter().any(|phrase| title_l.contains(phrase)) {
        return true;
    }
    if title_short && (title_l.trim() == "404" || title_l.starts_with("404 ") || title_l.starts_with("404-")) {
        return true;
    }
    let text_l = text.to_lowercase();
    text.chars().count() < 280 && NOT_FOUND.iter().any(|phrase| text_l.contains(phrase))
}

pub fn strip_boilerplate(text: &str) -> String {
    let text = cut_phrase(text, "наша форма -- наш webhook crm");
    let text = cut_phrase(&text, "наша форма — наш webhook crm");
    let text = cut_phrase(&text, "наш webhook crm");
    let text = cut_phrase(&text, "мы используем файлы cookie");
    let text = cut_phrase(&text, "мы используем cookie");
    let text = cut_phrase(&text, "этот сайт использует cookie");
    let mut out = String::new();
    for sentence in split_sentences(&text) {
        if is_boilerplate_sentence(&sentence) {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(sentence.trim());
    }
    out
}

pub fn strip_banner_nodes(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::new();
    let mut i = 0;
    while i < html.len() {
        if lower[i..].starts_with('<') {
            if let Some(rel_end) = lower[i..].find('>') {
                let open = &lower[i..i + rel_end];
                if let Some(tag) = banner_tag(open) {
                    i = end_of_element(&lower, i + rel_end + 1, tag);
                    continue;
                }
            }
        }
        let ch = html[i..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

pub fn resolve_href(base: &Url, raw: &str) -> Option<Url> {
    let raw = raw.trim();
    if raw.is_empty()
        || raw.starts_with('#')
        || raw.starts_with("mailto:")
        || raw.starts_with("javascript:")
        || raw.starts_with("tel:")
    {
        return None;
    }
    let joined = if let Some(promoted) = bare_host_url(raw) {
        Url::parse(&promoted).ok()?
    } else {
        base.join(raw).ok()?
    };
    if joined.scheme() != "http" && joined.scheme() != "https" {
        return None;
    }
    if is_broken_url(joined.as_str()) {
        return None;
    }
    Some(joined)
}

fn bare_host_url(raw: &str) -> Option<String> {
    if raw.contains("://") || raw.starts_with('/') || raw.starts_with('#') || raw.starts_with('?') {
        return None;
    }
    let host = raw.split(['/', '?', '#']).next().unwrap_or("");
    if segment_is_foreign_host(host) {
        Some(format!("https://{raw}"))
    } else {
        None
    }
}

fn segment_is_foreign_host(segment: &str) -> bool {
    let segment = segment.trim().trim_end_matches('.').to_ascii_lowercase();
    if segment.is_empty() || !segment.contains('.') {
        return false;
    }
    let Some((name, tld)) = segment.rsplit_once('.') else {
        return false;
    };
    if name.is_empty() || FILE_EXT.contains(&tld) || !HOST_TLD.contains(&tld) {
        return false;
    }
    name.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_')
}

fn cut_phrase(text: &str, phrase: &str) -> String {
    let lower = text.to_lowercase();
    let phrase = phrase.to_lowercase();
    let mut out = String::new();
    let mut cursor = 0;
    while let Some(rel) = lower[cursor..].find(&phrase) {
        let at = cursor + rel;
        out.push_str(&text[cursor..at]);
        cursor = at + phrase.len();
    }
    out.push_str(&text[cursor..]);
    out
}

fn is_boilerplate_sentence(sentence: &str) -> bool {
    let lower = sentence.to_lowercase();
    let short = sentence.chars().count() < 320;
    if !short {
        return false;
    }
    if lower.contains("webhook") {
        return true;
    }
    let cookie = lower.contains("cookie") || lower.contains("cookies") || lower.contains("куки");
    cookie
        && (lower.contains("использу")
            || lower.contains("принима")
            || lower.contains("соглас")
            || lower.contains("продолжа")
            || lower.contains("уведом"))
}

fn split_sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (idx, ch) in text.char_indices() {
        if ch == '.' || ch == '!' || ch == '?' || ch == '\n' {
            let end = idx + ch.len_utf8();
            let piece = text[start..end].trim();
            if !piece.is_empty() {
                out.push(piece);
            }
            start = end;
        }
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

fn banner_tag(open_lower: &str) -> Option<&'static str> {
    let rest = open_lower.trim_start_matches('<').trim_start();
    if rest.starts_with('/') || rest.starts_with('!') {
        return None;
    }
    let tag = rest
        .split(|ch: char| ch.is_whitespace() || ch == '/' || ch == '>')
        .next()
        .unwrap_or("");
    let known = match tag {
        "div" => "div",
        "section" => "section",
        "aside" => "aside",
        "dialog" => "dialog",
        "form" => "form",
        _ => return None,
    };
    if open_lower.contains("cookie") || open_lower.contains("gdpr") || open_lower.contains("consent") {
        Some(known)
    } else {
        None
    }
}

fn end_of_element(lower: &str, mut from: usize, tag: &str) -> usize {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut depth = 1_i32;
    while depth > 0 && from < lower.len() {
        let next_open = lower[from..].find(&open).map(|pos| from + pos);
        let next_close = lower[from..].find(&close).map(|pos| from + pos);
        match (next_open, next_close) {
            (Some(open_at), Some(close_at)) if open_at < close_at => {
                depth += 1;
                from = open_at + open.len();
            }
            (_, Some(close_at)) => {
                depth -= 1;
                from = close_at + close.len();
            }
            _ => return lower.len(),
        }
    }
    from
}

/// Убирает хвост после `#` и метки перехода (`utm_*`, клики рекламы). Остальной запрос остаётся.
pub fn tidy_url(url: &str) -> String {
    let trimmed = url.trim();
    let Ok(mut parsed) = Url::parse(trimmed) else {
        return trimmed.to_string();
    };
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return trimmed.to_string();
    }
    parsed.set_fragment(None);
    let kept: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(key, _)| !is_track_param(key))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    if kept.is_empty() {
        parsed.set_query(None);
    } else {
        parsed.query_pairs_mut().clear().extend_pairs(&kept);
    }
    parsed.to_string()
}

fn is_track_param(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.starts_with("utm_")
        || matches!(
            key.as_str(),
            "gclid" | "yclid" | "fbclid" | "ysclid" | "gclsrc" | "_openstat" | "mc_cid" | "mc_eid"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glued_host_and_scheme_in_path_are_broken() {
        assert!(is_broken_url("https://rusprodsoyuz.ru/www.thelivecoffee.ru"));
        assert!(is_broken_url("https://associaciasip.ru/ag-ecostroy.ru"));
        assert!(is_broken_url("https://rusprodsoyuz.ru/http://factory.example/"));
        assert!(!is_broken_url("https://shop.example/catalog/item.html"));
        assert!(!is_broken_url("https://shop.example/bitrix/js/main/core.min.js"));
    }

    #[test]
    fn bare_host_becomes_its_own_site() {
        let base = Url::parse("https://rusprodsoyuz.ru/participants/").unwrap();
        let joined = resolve_href(&base, "www.thelivecoffee.ru").unwrap();
        assert_eq!(joined.host_str(), Some("www.thelivecoffee.ru"));
        assert!(!joined.path().contains("thelivecoffee"));
        let ordinary = resolve_href(&base, "/members/").unwrap();
        assert_eq!(ordinary.host_str(), Some("rusprodsoyuz.ru"));
    }

    #[test]
    fn track_marks_and_fragment_leave_the_address() {
        let tidy = tidy_url("https://shop.example/jar.html?id=3&utm_source=ads&utm_medium=cpc#top");
        assert_eq!(tidy, "https://shop.example/jar.html?id=3");
        assert_eq!(
            tidy_url("https://shop.example/jar.html?utm_campaign=spring#x"),
            "https://shop.example/jar.html"
        );
        assert_eq!(tidy_url("https://shop.example/jar.html?id=3"), "https://shop.example/jar.html?id=3");
    }

    #[test]
    fn missing_page_and_banner_do_not_stay_in_the_text() {
        assert!(is_not_found("Проект не найден", "Проект не найден"));
        assert!(!is_not_found("Пасека", "Натуральный мёд с пасеки в Камышине. Доставка по краю."));
        let html = "<html><title>Мёд</title><div class=\"cookie-banner\">Мы используем cookie. Нажмите принять.</div><p>Настоящий мёд с пасеки.</p></html>";
        let stripped = strip_banner_nodes(html);
        assert!(!stripped.to_lowercase().contains("cookie"), "{stripped}");
        assert!(stripped.contains("мёд"), "{stripped}");
        let text = strip_boilerplate("Настоящий мёд с пасеки. Наша форма -- наш webhook CRM.");
        assert!(text.contains("мёд"), "{text}");
        assert!(!text.to_lowercase().contains("webhook"), "{text}");
        let glued = strip_boilerplate(
            "Жизнь в ЖК Катуар наша форма -- наш webhook CRM -- Перейти к содержимому. Квартиры у леса.",
        );
        assert!(!glued.to_lowercase().contains("webhook"), "{glued}");
        assert!(glued.contains("Квартиры"), "{glued}");
    }
}
