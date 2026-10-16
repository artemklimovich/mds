/// Заголовки из RSS 2.0. Лента в индекс не пишется: это разбор текста, который уже прочитан.
pub fn rss_headlines(xml: &str, limit: usize) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if limit == 0 {
        return out;
    }
    let lower = xml.to_ascii_lowercase();
    let mut from = 0;
    while out.len() < limit {
        let Some(rel) = lower[from..].find("<item") else {
            break;
        };
        let start = from + rel;
        let Some(end_rel) = lower[start..].find("</item>") else {
            break;
        };
        let block = &xml[start..start + end_rel];
        from = start + end_rel + "</item>".len();
        let title = tag_text(block, "title");
        let link = item_link(block);
        if title.is_empty() || !(link.starts_with("https://") || link.starts_with("http://")) {
            continue;
        }
        let title = title.chars().take(180).collect();
        let link = link.chars().take(500).collect();
        out.push((title, link));
    }
    out
}

fn item_link(block: &str) -> String {
    let text = tag_text(block, "link");
    if text.starts_with("http://") || text.starts_with("https://") {
        return text;
    }
    let lower = block.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower[from..].find("<link") {
        let start = from + rel;
        let after = &block[start..];
        let Some(end) = after.find('>') else {
            break;
        };
        let tag = &after[..end];
        from = start + end + 1;
        if let Some(href) = attr(tag, "href") {
            if href.starts_with("http://") || href.starts_with("https://") {
                return href;
            }
        }
    }
    String::new()
}

fn attr(tag: &str, name: &str) -> Option<String> {
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
    Some(decode_entities(&body[..end]))
}

fn tag_text(block: &str, tag: &str) -> String {
    let lower = block.to_ascii_lowercase();
    let open = format!("<{tag}");
    let rel = match lower.find(&open) {
        Some(rel) => rel,
        None => return String::new(),
    };
    let after = &block[rel + open.len()..];
    let Some(gt) = after.find('>') else {
        return String::new();
    };
    let rest = &after[gt + 1..];
    let close = format!("</{tag}>");
    let lower_rest = rest.to_ascii_lowercase();
    let Some(end) = lower_rest.find(&close) else {
        return String::new();
    };
    let raw = rest[..end].trim();
    let text = if let Some(inner) = raw.strip_prefix("<![CDATA[") {
        decode_entities(inner.strip_suffix("]]>").unwrap_or(inner))
    } else {
        decode_entities(&strip_tags(raw))
    };
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_tags(raw: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for ch in raw.chars() {
        match ch {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => out.push(ch),
            _ => {}
        }
    }
    out
}

fn decode_entities(input: &str) -> String {
    let mut out = String::new();
    let mut rest = input;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        let Some(end) = rest.find(';') else {
            out.push_str(rest);
            return out;
        };
        let token = &rest[1..end];
        let ch = match token {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{00a0}'),
            _ if token.starts_with('#') => decode_num(&token[1..]),
            _ => None,
        };
        if let Some(ch) = ch {
            out.push(ch);
        } else {
            out.push_str(&rest[..=end]);
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

fn decode_num(token: &str) -> Option<char> {
    let code = if let Some(hex) = token.strip_prefix('x').or_else(|| token.strip_prefix('X')) {
        u32::from_str_radix(hex, 16).ok()?
    } else {
        token.parse().ok()?
    };
    char::from_u32(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headlines_come_from_items_not_the_channel() {
        let xml = r#"<?xml version="1.0"?><rss><channel><title>Канал</title><link>https://example.ru/</link>
        <item><title><![CDATA[Мёд &amp; пасека]]></title><link>https://example.ru/news/1</link></item>
        <item><title>Вторая</title><link href="https://example.ru/news/2" /></item>
        </channel></rss>"#;
        let lines = rss_headlines(xml, 8);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].0, "Мёд & пасека");
        assert_eq!(lines[0].1, "https://example.ru/news/1");
        assert_eq!(lines[1].1, "https://example.ru/news/2");
    }
}
