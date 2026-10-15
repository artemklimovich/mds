use serde_json::Value;
use url::Url;

#[derive(Clone, Debug, Default)]
pub struct SpaBundleExtract {
    pub text: String,
    pub routes: Vec<String>,
}

pub fn json_state_text_and_routes(base_url: &str, raw_json: &str) -> SpaBundleExtract {
    let Ok(value) = serde_json::from_str::<Value>(raw_json.trim()) else {
        return SpaBundleExtract::default();
    };
    let mut text = String::new();
    let mut routes = Vec::new();
    let base = Url::parse(base_url).ok();
    walk_state(&value, base.as_ref(), &mut text, &mut routes);
    SpaBundleExtract {
        text: clip_chars(&text, 2400),
        routes,
    }
}

pub fn spa_script_urls(page_url: &str, html: &str) -> Vec<String> {
    let Ok(base) = Url::parse(page_url) else {
        return Vec::new();
    };
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(rel) = lower[cursor..].find("<script") {
        let start = cursor + rel;
        let Some(end_rel) = lower[start..].find('>') else {
            break;
        };
        let tag = &html[start + 1..start + end_rel];
        cursor = start + end_rel + 1;
        let Some(src) = find_attr(tag, "src") else {
            continue;
        };
        let src_lower = src.to_ascii_lowercase();
        if !src_lower.contains(".js") {
            continue;
        }
        if src_lower.contains("googletagmanager")
            || src_lower.contains("mc.yandex")
            || src_lower.contains("analytics")
            || src_lower.contains("metrika")
        {
            continue;
        }
        if let Ok(joined) = base.join(&src) {
            if joined.host_str() == base.host_str()
                && (joined.scheme() == "http" || joined.scheme() == "https")
                && !out.contains(&joined.to_string())
            {
                out.push(joined.to_string());
                if out.len() >= 2 {
                    break;
                }
            }
        }
    }
    out
}

pub fn parse_js_bundle(page_url: &str, js: &str) -> SpaBundleExtract {
    let base = Url::parse(page_url).ok();
    let mut text = String::new();
    let mut routes = Vec::new();

    for literal in string_literals(js) {
        let trimmed = literal.trim();
        if let Some(base) = base.as_ref() {
            if is_route_candidate(trimmed) {
                if let Ok(joined) = base.join(trimmed) {
                    let full = joined.to_string();
                    if routes.len() < 40 && !routes.contains(&full) {
                        routes.push(full);
                    }
                }
                continue;
            }
        }
        if looks_like_human_copy(trimmed) && text.chars().count() < 4000 && !text.contains(trimmed) {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(trimmed);
        }
    }

    SpaBundleExtract {
        text: clip_chars(&text, 4000),
        routes,
    }
}

fn walk_state(value: &Value, base: Option<&Url>, text: &mut String, routes: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let k = key.to_ascii_lowercase();
                if matches!(
                    k.as_str(),
                    "buildid" | "assetprefix" | "scriptloader" | "__n_ssg" | "__n_ssp" | "locale"
                ) {
                    continue;
                }
                walk_state(child, base, text, routes);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk_state(item, base, text, routes);
            }
        }
        Value::String(s) => {
            let trimmed = s.trim();
            if let Some(base) = base {
                if is_route_candidate(trimmed) {
                    if let Ok(joined) = base.join(trimmed) {
                        let full = joined.to_string();
                        if routes.len() < 40 && !routes.contains(&full) {
                            routes.push(full);
                        }
                    }
                    return;
                }
            }
            if (looks_like_human_copy(trimmed) || is_clean_latin_copy(trimmed))
                && text.chars().count() < 2400
                && !text.contains(trimmed)
            {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(trimmed);
            }
        }
        _ => {}
    }
}

fn is_route_candidate(s: &str) -> bool {
    if !s.starts_with('/') || s.starts_with("//") || s.len() < 2 || s.len() > 90 {
        return false;
    }
    if s.contains(char::is_whitespace)
        || s.contains(':')
        || s.contains('*')
        || s.contains('[')
        || s.contains('{')
        || s.contains('$')
    {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    for bad in [
        "/api/",
        "/assets/",
        "/static/",
        "/_next/",
        "/node_modules/",
        ".js",
        ".css",
        ".png",
        ".jpg",
        ".jpeg",
        ".svg",
        ".webp",
        ".ico",
        ".woff",
        ".ttf",
        ".map",
        ".json",
        ".mdx",
    ] {
        if lower.starts_with(bad) || lower.ends_with(bad) {
            return false;
        }
    }
    s[1..]
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_'))
}

fn looks_like_human_copy(s: &str) -> bool {
    let chars = s.chars().count();
    if !(6..=320).contains(&chars) {
        return false;
    }
    if s.contains("function(")
        || s.contains("=>")
        || s.contains("return ")
        || s.contains("Object.")
        || s.contains("window.")
        || s.contains("document.")
        || s.contains("http://")
        || s.contains("https://")
        || s.contains("</")
        || s.contains('{')
        || s.contains('}')
        || s.contains("Пароль")
        || s.contains("Должна быть")
        || s.contains("Минимум ")
        || s.contains("Ошибка")
        || s.contains("Нет данных")
        || s.contains("Таймаут")
        || s.contains("Попробуйте ещё раз")
        || s.contains("Проверьте настройки")
        || s.contains("Напишите сообщение")
        || s.contains("Я помощник")
        || s.contains("персональных данных")
        || s.contains("152-ФЗ")
        || s.contains("Политик")
        || s.contains("cookie")
        || s.contains("Уведомление об использовании")
        || s.contains("Все права защищены")
        || s.contains("Ой, тут ничего")
        || s.contains("Перенаправляем в админку")
        || s.contains("Введите email")
        || s.contains("Нужен email")
        || s.contains("личный кабинет")
        || s.contains("Личный кабинет")
        || s.contains("Войдите или зарегистрируйтесь")
        || s.contains("Ваш браузер не поддерживает")
        || s.contains("Сообщение отправлено")
        || s.contains("Мы свяжемся с вами")
    {
        return false;
    }
    let spaces = s.chars().filter(|c| *c == ' ').count();
    let cyrillic = s.chars().filter(|c| ('\u{0400}'..='\u{04FF}').contains(c)).count();
    cy_phrase(chars, spaces, cyrillic)
}

fn cy_phrase(chars: usize, spaces: usize, cyrillic: usize) -> bool {
    cyrillic >= 5 && spaces >= 1 && cyrillic * 4 >= chars * 3
}

fn is_clean_latin_copy(s: &str) -> bool {
    let chars = s.chars().count();
    let spaces = s.chars().filter(|c| *c == ' ').count();
    let letters = s.chars().filter(|c| c.is_alphabetic()).count();
    (16..=320).contains(&chars)
        && spaces >= 3
        && letters * 3 >= chars * 2
        && !s.contains("Minified")
        && !s.contains("react-")
        && !s.contains("Error")
        && !s.contains("exception")
}

fn string_literals(js: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = js.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '"' && ch != '\'' {
            continue;
        }
        let quote = ch;
        let mut buf = String::new();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(next) = chars.next() {
                    if next == 'n' || next == 't' {
                        buf.push(' ');
                    } else if next == 'u' {
                        let mut hex = String::with_capacity(4);
                        for _ in 0..4 {
                            if let Some(&h) = chars.peek() {
                                if h.is_ascii_hexdigit() {
                                    hex.push(h);
                                    chars.next();
                                }
                            }
                        }
                        if let Ok(code) = u32::from_str_radix(&hex, 16) {
                            if let Some(decoded) = char::from_u32(code) {
                                buf.push(decoded);
                            }
                        }
                    } else {
                        buf.push(next);
                    }
                }
                continue;
            }
            if c == quote {
                break;
            }
            if buf.len() < 600 {
                buf.push(c);
            }
        }
        if !buf.is_empty() {
            out.push(buf);
            if out.len() >= 25_000 {
                break;
            }
        }
    }
    out
}

fn find_attr(tag: &str, attr_name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let pat = format!("{attr_name}=");
    let pos = lower.find(&pat)?;
    let rest = &tag[pos + pat.len()..].trim_start();
    let first = rest.chars().next()?;
    if first == '"' || first == '\'' {
        let content = &rest[first.len_utf8()..];
        let end = content.find(first)?;
        Some(content[..end].to_string())
    } else {
        let end = rest
            .find(|c: char| c.is_ascii_whitespace() || c == '>')
            .unwrap_or(rest.len());
        Some(rest[..end].to_string())
    }
}

fn clip_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}
