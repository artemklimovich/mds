use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

const REMEMBER: Duration = Duration::from_secs(600);
const MAX_BYTES: usize = 1_048_576;
const MAX_OFFERS: usize = 24;

#[derive(Clone, Debug, Serialize)]
pub struct ShopLine {
    pub name: String,
    pub url: String,
    pub price: String,
}

pub async fn read(url: &str) -> Vec<ShopLine> {
    if let Some(lines) = fresh(url) {
        return lines;
    }
    let lines = fetch(url).await;
    if lines.is_empty() {
        return stale(url);
    }
    if let Ok(mut guard) = CACHE.lock() {
        guard.retain(|(saved, _, _)| saved != url);
        guard.push((url.to_string(), Instant::now(), lines.clone()));
        if guard.len() > 64 {
            guard.remove(0);
        }
    }
    lines
}

fn fresh(url: &str) -> Option<Vec<ShopLine>> {
    let guard = CACHE.lock().ok()?;
    guard
        .iter()
        .find(|(saved, at, _)| saved == url && at.elapsed() < REMEMBER)
        .map(|(_, _, lines)| lines.clone())
}

fn stale(url: &str) -> Vec<ShopLine> {
    CACHE
        .lock()
        .ok()
        .and_then(|guard| guard.iter().find(|(saved, _, _)| saved == url).map(|(_, _, lines)| lines.clone()))
        .unwrap_or_default()
}

static CACHE: Mutex<Vec<(String, Instant, Vec<ShopLine>)>> = Mutex::new(Vec::new());

async fn fetch(url: &str) -> Vec<ShopLine> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent("MDS-Bot")
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let Ok(mut response) = client.get(url).send().await else {
        return Vec::new();
    };
    if !response.status().is_success() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    while buf.len() < MAX_BYTES {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                let room = MAX_BYTES - buf.len();
                if chunk.len() > room {
                    buf.extend_from_slice(&chunk[..room]);
                    break;
                }
                buf.extend_from_slice(&chunk);
            }
            _ => break,
        }
    }
    let xml = String::from_utf8_lossy(&buf);
    parse_offers(&xml)
}

fn parse_offers(xml: &str) -> Vec<ShopLine> {
    let lower = xml.to_ascii_lowercase();
    let mut from = 0;
    let mut out = Vec::new();
    while out.len() < MAX_OFFERS {
        let Some(rel) = lower[from..].find("<offer") else {
            break;
        };
        let start = from + rel;
        let Some(rel_end) = lower[start..].find("</offer>") else {
            break;
        };
        let block = &xml[start..start + rel_end];
        from = start + rel_end + "</offer>".len();
        let name = tag_text(block, "name");
        let url = tag_text(block, "url");
        if name.is_empty() || url.is_empty() {
            continue;
        }
        out.push(ShopLine {
            name,
            url,
            price: tag_text(block, "price"),
        });
    }
    out
}

fn tag_text(block: &str, tag: &str) -> String {
    let lower = block.to_ascii_lowercase();
    let open = format!("<{tag}");
    let Some(rel) = lower.find(&open) else {
        return String::new();
    };
    let after = &block[rel + open.len()..];
    let Some(close) = after.find('>') else {
        return String::new();
    };
    let body = &after[close + 1..];
    let end_tag = format!("</{tag}>");
    let lower_body = body.to_ascii_lowercase();
    let Some(end) = lower_body.find(&end_tag) else {
        return String::new();
    };
    body[..end]
        .replace("<![CDATA[", "")
        .replace("]]>", "")
        .split('<')
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yml_offer_keeps_name_price_and_url() {
        let xml = r#"<yml_catalog><shop><offers>
            <offer id="1"><name><![CDATA[Болт М8]]></name><price>12</price><url>https://shop.example/bolt</url></offer>
            <offer id="2"><name></name><url>https://shop.example/empty</url></offer>
        </offers></shop></yml_catalog>"#;
        let lines = parse_offers(xml);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].name, "Болт М8");
        assert_eq!(lines[0].price, "12");
        assert_eq!(lines[0].url, "https://shop.example/bolt");
    }
}
