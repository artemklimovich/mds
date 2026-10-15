use serde_json::Value;
use url::Url;

#[derive(Clone, Debug)]
pub struct PageRead {
    pub title_hint: String,
    pub extra_text: String,
    pub canonical: Option<String>,
    pub indexable: bool,
    pub follow: bool,
    pub spa_routes: Vec<String>,
}

pub fn read_page(html: &str) -> PageRead {
    let mut description = String::new();
    let mut title_hint = String::new();
    let mut keywords = String::new();
    let mut canonical = None;
    let mut indexable = true;
    let mut follow = true;
    let mut spa_routes = Vec::new();

    let lower = html.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(rel) = lower[cursor..].find('<') {
        let start = cursor + rel;
        let Some(end_rel) = lower[start..].find('>') else {
            break;
        };
        let tag_end = start + end_rel;
        let raw_tag = &html[start + 1..tag_end];
        let tag_lower = &lower[start + 1..tag_end];
        let name = tag_name(tag_lower);
        if name == "meta" {
            let attrs = attrs(raw_tag);
            let key = attr(&attrs, "name")
                .or_else(|| attr(&attrs, "property"))
                .or_else(|| attr(&attrs, "itemprop"))
                .unwrap_or_default()
                .to_ascii_lowercase();
            let content = attr(&attrs, "content").unwrap_or_default();
            match key.as_str() {
                "description" | "og:description" | "twitter:description" | "dc.description" => {
                    push_unique(&mut description, &decode_basic(&content));
                }
                "og:title" | "twitter:title" | "dc.title" => {
                    if title_hint.is_empty() {
                        title_hint = decode_basic(&content);
                    }
                }
                "keywords" => keywords = decode_basic(&content),
                "robots" | "googlebot" | "yandex" => {
                    let flags = content.to_ascii_lowercase();
                    if flags.split([',', ' ']).any(|part| part == "noindex" || part == "none") {
                        indexable = false;
                    }
                    if flags.split([',', ' ']).any(|part| part == "nofollow" || part == "none") {
                        follow = false;
                    }
                }
                _ => {}
            }
        } else if name == "link" {
            let attrs = attrs(raw_tag);
            let rel = attr(&attrs, "rel").unwrap_or_default().to_ascii_lowercase();
            let href = attr(&attrs, "href").unwrap_or_default();
            if rel.split_whitespace().any(|part| part == "canonical") && !href.is_empty() {
                canonical = Some(decode_basic(&href));
            }
        } else if name == "h1" && title_hint.is_empty() {
            if let Some(text) = element_text(html, tag_end + 1, "h1") {
                title_hint = text;
            }
        } else if name == "script" {
            let attrs = attrs(raw_tag);
            let kind = attr(&attrs, "type").unwrap_or_default().to_ascii_lowercase();
            let id = attr(&attrs, "id").unwrap_or_default().to_ascii_lowercase();
            if kind.contains("ld+json") {
                if let Some(body) = element_raw(html, tag_end + 1, "script") {
                    push_unique(&mut description, &json_ld_text(&body));
                }
            } else if kind.contains("application/json") || id == "__next_data__" || id == "__nuxt_data__" {
                if let Some(body) = element_raw(html, tag_end + 1, "script") {
                    let parsed = crate::spa::json_state_text_and_routes("https://spa.local/", &body);
                    push_unique(&mut description, &parsed.text);
                    for route in parsed.routes {
                        if let Ok(u) = Url::parse(&route) {
                            let p = u.path().to_string();
                            if !spa_routes.contains(&p) {
                                spa_routes.push(p);
                            }
                        }
                    }
                }
            }
        } else if !name.is_empty() {
            let attrs = attrs(raw_tag);
            let prop = attr(&attrs, "itemprop").unwrap_or_default().to_ascii_lowercase();
            if matches!(
                prop.as_str(),
                "name" | "description" | "headline" | "alternatename" | "abstract"
            ) {
                if let Some(text) = element_text(html, tag_end + 1, name) {
                    push_unique(&mut description, &text);
                }
            }
        }
        cursor = tag_end + 1;
    }

    let mut extra = String::new();
    push_unique(&mut extra, &description);
    push_unique(&mut extra, &keywords);
    PageRead {
        title_hint,
        extra_text: clip(&extra, 2000),
        canonical,
        indexable,
        follow,
        spa_routes,
    }
}

pub fn same_host_canonical(page_url: &str, host: &str, canonical: Option<&str>) -> String {
    let Some(canonical) = canonical else {
        return page_url.to_string();
    };
    let Ok(base) = Url::parse(page_url) else {
        return page_url.to_string();
    };
    let Ok(joined) = base.join(canonical) else {
        return page_url.to_string();
    };
    if joined.host_str().unwrap_or("").eq_ignore_ascii_case(host)
        && (joined.scheme() == "http" || joined.scheme() == "https")
    {
        joined.to_string()
    } else {
        page_url.to_string()
    }
}

pub fn x_robots_tag(value: &str) -> (bool, bool) {
    let flags = value.to_ascii_lowercase();
    let blocked = |name: &str| flags.split([',', ' ', ';']).any(|part| part == name || part == "none");
    (blocked("noindex"), blocked("nofollow"))
}

pub fn robots_sitemaps(robots: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in robots.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("sitemap") {
            let url = value.trim();
            if url.starts_with("http://") || url.starts_with("https://") {
                out.push(url.to_string());
            }
        }
    }
    out
}

pub fn robots_crawl_delay(robots: &str, user_agent: &str) -> Option<u64> {
    let mut current_matches = false;
    let mut specific = None;
    let mut wildcard = None;
    let mut specific_group = false;
    for line in robots.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key.eq_ignore_ascii_case("user-agent") {
            current_matches = value == "*"
                || user_agent
                    .to_ascii_lowercase()
                    .starts_with(&value.to_ascii_lowercase());
            specific_group = current_matches && value != "*";
            continue;
        }
        if current_matches && key.eq_ignore_ascii_case("crawl-delay") {
            if let Some(seconds) = value.parse::<u64>().ok() {
                let seconds = seconds.min(60);
                if specific_group {
                    specific = Some(seconds);
                } else if specific.is_none() {
                    wildcard = Some(seconds);
                }
            }
        }
    }
    specific.or(wildcard)
}

pub fn sitemap_locs(xml: &str) -> Vec<String> {
    let lower = xml.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = lower[from..].find("<loc>") {
        let start = from + rel + 5;
        let Some(end_rel) = lower[start..].find("</loc>") else {
            break;
        };
        let loc = decode_basic(xml[start..start + end_rel].trim());
        if loc.starts_with("http://") || loc.starts_with("https://") {
            out.push(loc);
        }
        from = start + end_rel + 6;
    }
    out
}

pub fn is_sitemap_document(content_type: &str, url: &str, body: &str) -> bool {
    let content_type = content_type.to_ascii_lowercase();
    if content_type.contains("html") {
        return false;
    }
    let head: String = body.chars().take(400).collect();
    let head = head.to_ascii_lowercase();
    let looks_xml = head.contains("<urlset") || head.contains("<sitemapindex") || head.contains("<loc>");
    if content_type.contains("xml") || content_type.contains("sitemap") {
        return looks_xml;
    }
    let url = url.to_ascii_lowercase();
    (url.contains(".xml") || url.contains("sitemap")) && looks_xml && !head.contains("<html")
}

pub fn anchor_nofollow(html: &str, href_at: usize) -> bool {
    if !is_anchor_href(html, href_at) {
        return false;
    }
    let prefix = &html[..href_at.min(html.len())];
    let Some(open) = prefix.rfind('<') else {
        return false;
    };
    let Some(end) = html[href_at.min(html.len())..].find('>') else {
        return false;
    };
    let tag = &html[open..href_at.min(html.len()) + end];
    tag.to_ascii_lowercase().contains("nofollow")
}

pub fn is_anchor_href(html: &str, href_at: usize) -> bool {
    let prefix = &html[..href_at.min(html.len())];
    let Some(open) = prefix.rfind('<') else {
        return false;
    };
    let between = &html[open..href_at.min(html.len())];
    if between.contains('>') {
        return false;
    }
    tag_name(&between[1..].to_ascii_lowercase()) == "a"
}

fn json_ld_text(raw: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(raw.trim()) else {
        return String::new();
    };
    let mut out = String::new();
    walk_ld(&value, &mut out);
    collapse_ws(&out)
}

fn walk_ld(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if matches!(
                    key.as_str(),
                    "name"
                        | "headline"
                        | "description"
                        | "articleBody"
                        | "alternateName"
                        | "keywords"
                        | "abstract"
                        | "slogan"
                        | "price"
                        | "priceCurrency"
                ) {
                    push_scalar(child, out);
                } else {
                    walk_ld(child, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                walk_ld(item, out);
            }
        }
        _ => {}
    }
}

fn push_scalar(value: &Value, out: &mut String) {
    match value {
        Value::String(text) => push_unique(out, text),
        Value::Number(number) => push_unique(out, &number.to_string()),
        Value::Array(items) => {
            for item in items {
                push_scalar(item, out);
            }
        }
        _ => {}
    }
}

fn element_text(html: &str, from: usize, tag: &str) -> Option<String> {
    let raw = element_raw(html, from, tag)?;
    let mut out = String::new();
    let mut in_tag = false;
    for ch in raw.chars() {
        if ch == '<' {
            in_tag = true;
            continue;
        }
        if ch == '>' {
            in_tag = false;
            continue;
        }
        if !in_tag {
            out.push(ch);
        }
    }
    let text = collapse_ws(&decode_basic(&out));
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn element_raw(html: &str, from: usize, tag: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let close = format!("</{tag}>");
    let end = lower[from..].find(&close)?;
    Some(html[from..from + end].to_string())
}

fn tag_name(tag_lower: &str) -> &str {
    let trimmed = tag_lower.trim_start_matches(['/', '!', '?']);
    let end = trimmed
        .find(|ch: char| ch.is_ascii_whitespace() || ch == '>' || ch == '/')
        .unwrap_or(trimmed.len());
    &trimmed[..end]
}

fn attrs(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = tag.as_bytes();
    let mut i = 0;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
        i += 1;
    }
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b'>' {
            break;
        }
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
            i += 1;
        }
        if key_start == i {
            break;
        }
        let key = tag[key_start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            i += 1;
            let value_start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            out.push((key, tag[value_start..i].to_string()));
            if i < bytes.len() {
                i += 1;
            }
        } else {
            let value_start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                i += 1;
            }
            out.push((key, tag[value_start..i].to_string()));
        }
    }
    out
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn push_unique(out: &mut String, text: &str) {
    let text = text.trim();
    if text.is_empty() || out.contains(text) {
        return;
    }
    if !out.is_empty() {
        out.push(' ');
    }
    out.push_str(text);
}

fn clip(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}

fn collapse_ws(text: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(ch);
    }
    out
}

fn decode_basic(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#39;", "'")
}
