use url::Url;

use crate::hygiene::{is_broken_url, is_not_found, resolve_href, strip_boilerplate};
use crate::{simhash, FetchedPage};

pub fn is_llms_card(url: &Url) -> bool {
    let path = url.path().to_ascii_lowercase();
    path == "/llms.txt" || path.ends_with("/llms.txt") || path == "/llms.md" || path.ends_with("/llms.md")
}

pub fn is_markdown_document(url: &Url, content_type: &str, body: &str) -> bool {
    let path = url.path().to_ascii_lowercase();
    if path.ends_with(".mdx") {
        return false;
    }
    let head = body.chars().take(320).collect::<String>().to_ascii_lowercase();
    if head.contains("<!doctype") || head.contains("<html") || head.contains("<head") || head.contains("<body") {
        return false;
    }
    if is_llms_card(url) || path.ends_with(".md") {
        return true;
    }
    let content_type = content_type.to_ascii_lowercase();
    content_type.contains("markdown") && !content_type.contains("html")
}

pub fn page_from_markdown(url: &str, host: &str, markdown: &str) -> FetchedPage {
    let text = strip_boilerplate(&markdown_plain(markdown));
    let title = first_heading(markdown);
    let title = if title.is_empty() {
        text.chars().take(80).collect()
    } else {
        title
    };
    let kind = mds_types::kind_from_stored(url, &title, &text);
    FetchedPage {
        url: url.to_string(),
        host: host.to_string(),
        simhash: simhash(&text),
        indexable: !is_not_found(&title, &text) && !is_broken_url(url),
        title,
        text,
        links: markdown_links(url, markdown),
        kind,
        media_url: None,
        feed_url: None,
        price: None,
        docs: Vec::new(),
        priority: if Url::parse(url).ok().as_ref().is_some_and(is_llms_card) {
            priority_pages(url, host, markdown)
        } else {
            Vec::new()
        },
    }
}

pub fn priority_pages(base: &str, host: &str, markdown: &str) -> Vec<String> {
    let bare = mds_types::bare_host(host);
    let mut out = Vec::new();
    for link in markdown_links(base, markdown) {
        push_priority(&mut out, &bare, &link);
    }
    for line in markdown.lines() {
        let line = line.trim().trim_start_matches(['-', '*']).trim();
        if !(line.starts_with("http://") || line.starts_with("https://")) {
            continue;
        }
        let url = line.split_whitespace().next().unwrap_or(line);
        push_priority(&mut out, &bare, url);
    }
    out
}

fn push_priority(out: &mut Vec<String>, bare: &str, url: &str) {
    if out.len() >= 100 || is_broken_url(url) {
        return;
    }
    let Some(host) = mds_types::host_of_url(url) else {
        return;
    };
    if mds_types::bare_host(&host) != bare {
        return;
    }
    let lower = url.to_ascii_lowercase();
    if lower.contains("/llms.txt") || lower.contains("/llms.md") {
        return;
    }
    if out.iter().any(|known| known == url) {
        return;
    }
    out.push(url.to_string());
}

pub fn llms_card_blurb(markdown: &str) -> String {
    let mut title = String::new();
    let mut quote = String::new();
    for line in markdown.lines() {
        let trimmed = line.trim();
        if title.is_empty() {
            if let Some(rest) = trimmed.strip_prefix("# ") {
                title = rest.trim().to_string();
            }
        }
        if let Some(rest) = trimmed.strip_prefix('>') {
            let rest = rest.trim();
            if rest.is_empty() {
                continue;
            }
            if !quote.is_empty() {
                quote.push(' ');
            }
            quote.push_str(rest);
        }
    }
    let mut out = String::new();
    if !title.is_empty() {
        out.push_str(&title);
    }
    if !quote.is_empty() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&quote);
    }
    clip(&out, 400)
}

fn markdown_links(base: &str, markdown: &str) -> Vec<String> {
    let Ok(base) = Url::parse(base) else {
        return Vec::new();
    };
    let mut links = Vec::new();
    let mut from = 0;
    while let Some(rel) = markdown[from..].find("](") {
        let start = from + rel + 2;
        let Some(end_rel) = markdown[start..].find(')') else {
            break;
        };
        let inside = markdown[start..start + end_rel].trim();
        from = start + end_rel + 1;
        let Some(dest) = inside.split_whitespace().next() else {
            continue;
        };
        let dest = dest.trim_matches(|ch| ch == '<' || ch == '>');
        if dest.is_empty() || dest.starts_with('#') || is_mdx(dest) {
            continue;
        }
        if let Some(joined) = resolve_href(&base, dest) {
            if !is_mdx(joined.path()) {
                links.push(joined.to_string());
            }
        }
    }
    links
}

fn markdown_plain(markdown: &str) -> String {
    let stripped = strip_fences(markdown);
    let mut without_urls = String::new();
    let mut rest = stripped.as_str();
    while let Some(rel) = rest.find("](") {
        let end = rest[rel..].find(')').map(|pos| rel + pos + 1).unwrap_or(rest.len());
        without_urls.push_str(&rest[..rel]);
        rest = &rest[end..];
    }
    without_urls.push_str(rest);
    let mut text = String::new();
    for line in without_urls.lines() {
        let mut line = line.trim();
        if line.starts_with('#') {
            line = line.trim_start_matches('#').trim();
        }
        if let Some(rest) = line.strip_prefix('>') {
            line = rest.trim();
        }
        if line.is_empty() {
            continue;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(line);
    }
    text.replace(['[', ']', '*', '_'], "")
}

fn strip_fences(markdown: &str) -> String {
    let mut out = String::new();
    let mut fence = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn first_heading(markdown: &str) -> String {
    for line in markdown.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            return rest.trim().to_string();
        }
    }
    String::new()
}

fn is_mdx(path: &str) -> bool {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    path.to_ascii_lowercase().ends_with(".mdx")
}

#[cfg(test)]
mod tests {
    use super::priority_pages;

    #[test]
    fn card_keeps_one_hundred_links_of_the_same_site() {
        let mut body = String::from("# Сайт\n\n> Описание хозяйства.\n\n");
        for number in 0..120 {
            body.push_str(&format!("- [Страница](https://zavod.example/p{number})\n"));
        }
        body.push_str("- [Чужой](https://other.example/x)\n");
        body.push_str("https://zavod.example/plain\n");
        let picks = priority_pages("https://zavod.example/llms.txt", "zavod.example", &body);
        assert_eq!(picks.len(), 100);
        assert!(picks.iter().all(|url| url.contains("zavod.example")));
        assert!(!picks.iter().any(|url| url.contains("other.example")));
        assert!(!picks.iter().any(|url| url.ends_with("/plain")));
    }
}

fn clip(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}
