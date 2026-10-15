use std::collections::HashSet;
use std::time::Duration;

use serde_json::Value;

use crate::USER_AGENT;

pub async fn discover_ct_domains(queries: &[&str], limit: usize) -> Vec<String> {
    let Ok(client) = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(12))
        .build()
    else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut urls = Vec::new();
    for query in queries {
        if urls.len() >= limit {
            break;
        }
        let endpoint = format!("https://crt.sh/?q={query}&output=json");
        let Ok(resp) = client.get(&endpoint).send().await else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(bytes) = resp.bytes().await else {
            continue;
        };
        for host in parse_crt_json(&bytes) {
            if seen.insert(host.clone()) {
                urls.push(format!("https://{host}/"));
                if urls.len() >= limit {
                    break;
                }
            }
        }
    }
    urls
}

pub fn parse_crt_json(bytes: &[u8]) -> Vec<String> {
    let Ok(Value::Array(entries)) = serde_json::from_slice::<Value>(bytes) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        let Some(names) = entry.get("name_value").and_then(|v| v.as_str()) else {
            continue;
        };
        for raw in names.lines() {
            let host = raw.trim().trim_start_matches("*.").to_ascii_lowercase();
            if is_clean_web_host(&host) && seen.insert(host.clone()) {
                out.push(host);
            }
        }
    }
    out
}

fn is_clean_web_host(host: &str) -> bool {
    if host.len() < 4 || host.len() > 70 || !host.contains('.') {
        return false;
    }
    if host.contains(char::is_whitespace) || host.contains('*') || host.contains('@') {
        return false;
    }
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() > 3 {
        return false;
    }
    for bad in [
        "mail.",
        "smtp.",
        "imap.",
        "pop.",
        "autodiscover.",
        "cpanel.",
        "webmail.",
        "vpn.",
        "api.",
        "cdn.",
        "ns1.",
        "ns2.",
        "test.",
        "dev.",
        "staging.",
    ] {
        if host.starts_with(bad) {
            return false;
        }
    }
    host.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}
