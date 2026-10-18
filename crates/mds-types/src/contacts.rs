use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contacts {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phones: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emails: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messengers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub socials: Vec<String>,
}

impl Contacts {
    pub fn is_empty(&self) -> bool {
        self.phones.is_empty()
            && self.emails.is_empty()
            && self.messengers.is_empty()
            && self.socials.is_empty()
    }

    pub fn merge(&mut self, other: &Contacts) {
        for phone in &other.phones {
            push_cap(&mut self.phones, phone, 6);
        }
        for email in &other.emails {
            push_cap(&mut self.emails, email, 6);
        }
        for msg in &other.messengers {
            push_cap(&mut self.messengers, msg, 8);
        }
        for soc in &other.socials {
            push_cap(&mut self.socials, soc, 8);
        }
    }

    pub fn to_marker_lines(&self) -> String {
        let mut lines = Vec::new();
        if !self.phones.is_empty() {
            lines.push(format!("Tel: {}", self.phones.join(", ")));
        }
        if !self.emails.is_empty() {
            lines.push(format!("Email: {}", self.emails.join(", ")));
        }
        if !self.messengers.is_empty() {
            lines.push(format!("Msg: {}", self.messengers.join(", ")));
        }
        if !self.socials.is_empty() {
            lines.push(format!("Soc: {}", self.socials.join(", ")));
        }
        lines.join("\n")
    }
}

fn push_cap(list: &mut Vec<String>, item: &str, max: usize) {
    let trimmed = item.trim();
    if trimmed.is_empty() || list.len() >= max {
        return;
    }
    if !list.iter().any(|existing| existing.eq_ignore_ascii_case(trimmed)) {
        list.push(trimmed.to_string());
    }
}

/// Нормализует российский телефон (+7XXXXXXXXXX) из произвольной строки или ссылки `tel:`.
pub fn clean_phone(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let raw = raw
        .strip_prefix("tel:")
        .or_else(|| raw.strip_prefix("TEL:"))
        .unwrap_or(raw)
        .trim();
    let digits: Vec<char> = raw.chars().filter(|ch| ch.is_ascii_digit()).collect();
    if digits.len() == 11 && (digits[0] == '7' || digits[0] == '8') {
        let tail: String = digits[1..].iter().collect();
        if tail.chars().all(|c| c == '0') {
            return None;
        }
        return Some(format!("+7{tail}"));
    }
    if digits.len() == 10 && digits[0] == '9' {
        let tail: String = digits.iter().collect();
        return Some(format!("+7{tail}"));
    }
    None
}

/// Проверяет и очищает открытый адрес электронной почты.
pub fn clean_email(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let raw = raw
        .strip_prefix("mailto:")
        .or_else(|| raw.strip_prefix("MAILTO:"))
        .unwrap_or(raw);
    let raw = raw.split('?').next().unwrap_or(raw).trim();
    let raw = raw.trim_matches(|ch: char| {
        matches!(
            ch,
            '<' | '>' | '(' | ')' | '[' | ']' | '"' | '\'' | ',' | ';' | ':' | '.'
        )
    });
    let lower = raw.to_ascii_lowercase();
    let (local, domain) = lower.split_once('@')?;
    if local.is_empty()
        || local.len() > 64
        || domain.len() < 4
        || domain.len() > 120
        || !domain.contains('.')
        || local.contains("..")
        || domain.contains("..")
    {
        return None;
    }
    if !local
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '+'))
    {
        return None;
    }
    if !domain
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-'))
    {
        return None;
    }
    let ext = domain.rsplit('.').next().unwrap_or("");
    if ext.len() < 2
        || ext.chars().all(|c| c.is_ascii_digit())
        || matches!(
            ext,
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "css" | "js" | "woff" | "woff2"
        )
    {
        return None;
    }
    if matches!(
        domain,
        "example.com" | "example.ru" | "domain.com" | "domain.ru" | "yoursite.ru" | "site.ru" | "test.ru"
    ) || lower.contains("sentry")
        || lower.ends_with("@2x.png")
    {
        return None;
    }
    Some(lower)
}

/// Распознаёт ссылку на мессенджер (Telegram, WhatsApp, Viber) или соцсеть (VK, OK, Rutube, Дзен, YouTube).
pub fn classify_contact_url(raw: &str) -> Option<(bool, String)> {
    let trimmed = raw
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | ')' | '>' | ',' | ';'));
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("tg://resolve?domain=") {
        let handle = trimmed[20..].split('&').next().unwrap_or("").trim();
        if valid_handle(handle) {
            return Some((true, format!("https://t.me/{handle}")));
        }
        return None;
    }
    if lower.starts_with("whatsapp://send") {
        if let Some(pos) = lower.find("phone=") {
            let digits: String = trimmed[pos + 6..]
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '+')
                .filter(|c| c.is_ascii_digit())
                .collect();
            if digits.len() >= 10 {
                return Some((true, format!("https://wa.me/{digits}")));
            }
        }
        return None;
    }
    if lower.starts_with("viber://") {
        return Some((true, trimmed.to_string()));
    }
    let without_scheme = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .or_else(|| lower.strip_prefix("//"))?;
    let without_www = without_scheme.strip_prefix("www.").unwrap_or(without_scheme);
    let (host, rest) = without_www.split_once('/')?;
    let path = rest.split(['?', '#']).next().unwrap_or("").trim_matches('/');
    if path.is_empty()
        || matches!(
            path,
            "share" | "sharer" | "sharer.php" | "share.php" | "intent" | "widgets"
        )
    {
        return None;
    }
    // Keep original case for path from trimmed URL
    let orig_after_scheme = trimmed
        .split("://")
        .nth(1)
        .unwrap_or_else(|| trimmed.trim_start_matches('/'));
    let orig_path = orig_after_scheme
        .split_once('/')
        .map(|(_, r)| r.split(['?', '#']).next().unwrap_or("").trim_matches('/'))
        .unwrap_or(path);

    match host {
        "t.me" | "telegram.me" => {
            let seg = orig_path.split('/').next().unwrap_or("");
            if matches!(seg.to_ascii_lowercase().as_str(), "share" | "iv" | "addstickers" | "proxy") {
                return None;
            }
            if valid_handle(seg) || seg.starts_with('+') {
                return Some((true, format!("https://t.me/{seg}")));
            }
            None
        }
        "wa.me" => {
            let seg = orig_path.split('/').next().unwrap_or("");
            let digits: String = seg.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.len() >= 10 {
                return Some((true, format!("https://wa.me/{digits}")));
            }
            None
        }
        "api.whatsapp.com" => {
            if let Some(pos) = lower.find("phone=") {
                let digits: String = trimmed[pos + 6..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '+')
                    .filter(|c| c.is_ascii_digit())
                    .collect();
                if digits.len() >= 10 {
                    return Some((true, format!("https://wa.me/{digits}")));
                }
            }
            None
        }
        "chat.whatsapp.com" => {
            let seg = orig_path.split('/').next().unwrap_or("");
            if valid_handle(seg) {
                return Some((true, format!("https://chat.whatsapp.com/{seg}")));
            }
            None
        }
        "vk.com" | "vk.ru" => {
            let seg = orig_path.split('/').next().unwrap_or("");
            let seg_low = seg.to_ascii_lowercase();
            if matches!(
                seg_low.as_str(),
                "share.php" | "feed" | "login" | "im" | "friends" | "settings" | "video" | "wall" | "widget_community.php"
            ) {
                return None;
            }
            if valid_handle(seg) {
                return Some((false, format!("https://vk.com/{seg}")));
            }
            None
        }
        "ok.ru" | "odnoklassniki.ru" => {
            let clean = orig_path
                .split('/')
                .take(2)
                .collect::<Vec<_>>()
                .join("/");
            if !clean.is_empty() && !clean.starts_with("dk") {
                return Some((false, format!("https://ok.ru/{clean}")));
            }
            None
        }
        "rutube.ru" => {
            let clean = orig_path
                .split('/')
                .take(2)
                .collect::<Vec<_>>()
                .join("/");
            if clean.starts_with("channel/") || clean.starts_with("u/") {
                return Some((false, format!("https://rutube.ru/{clean}")));
            }
            None
        }
        "dzen.ru" | "zen.yandex.ru" => {
            let seg = orig_path.split('/').next().unwrap_or("");
            if !seg.is_empty() && !matches!(seg, "news" | "a" | "media" | " suite") {
                return Some((false, format!("https://dzen.ru/{seg}")));
            }
            None
        }
        "youtube.com" => {
            let clean = orig_path
                .split('/')
                .take(2)
                .collect::<Vec<_>>()
                .join("/");
            if clean.starts_with('@')
                || clean.starts_with("c/")
                || clean.starts_with("channel/")
                || clean.starts_with("user/")
            {
                return Some((false, format!("https://youtube.com/{clean}")));
            }
            None
        }
        _ => None,
    }
}

fn valid_handle(s: &str) -> bool {
    s.len() >= 2
        && s.len() <= 64
        && s.chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

/// Извлекает открытые контакты компании (телефоны, email, мессенджеры, соцсети) из текста страницы и маркеров `Tel:`, `Email:`, `Msg:`, `Soc:`.
pub fn contacts_in_text(text: &str) -> Contacts {
    let mut out = Contacts::default();

    // Extract Tel: +7... tokens when push_unique joins them with spaces
    for (idx, _) in text.match_indices("Tel:") {
        let rest = text.get(idx + 4..).unwrap_or("");
        let end = rest.char_indices().nth(120).map(|(at, _)| at).unwrap_or(rest.len());
        let slice = &rest[..end];
        let stop = slice
            .find("Email:")
            .or_else(|| slice.find("Msg:"))
            .or_else(|| slice.find("Soc:"))
            .or_else(|| slice.find("Geo:"))
            .or_else(|| slice.find('\n'))
            .unwrap_or(slice.len());
        for part in slice[..stop].split(',') {
            if let Some(p) = clean_phone(part) {
                push_cap(&mut out.phones, &p, 6);
            }
        }
    }

    // Extract emails and social/messenger links from tokens
    for token in text.split_whitespace() {
        let token_clean = token.trim_matches(|c: char| matches!(c, ',' | ';' | ')' | '(' | '"' | '\''));
        if let Some(tel) = token_clean.strip_prefix("tel:").or_else(|| token_clean.strip_prefix("TEL:")) {
            if let Some(p) = clean_phone(tel) {
                push_cap(&mut out.phones, &p, 6);
            }
        } else if token_clean.starts_with("+7") && token_clean.len() == 12 {
            if let Some(p) = clean_phone(token_clean) {
                push_cap(&mut out.phones, &p, 6);
            }
        } else if token_clean.contains('@') && !token_clean.starts_with("http") {
            if let Some(email) = clean_email(token_clean) {
                push_cap(&mut out.emails, &email, 6);
            }
        } else if token_clean.starts_with("http://")
            || token_clean.starts_with("https://")
            || token_clean.starts_with("tg://")
            || token_clean.starts_with("viber://")
        {
            if let Some((is_msg, url)) = classify_contact_url(token_clean) {
                if is_msg {
                    push_cap(&mut out.messengers, &url, 8);
                } else {
                    push_cap(&mut out.socials, &url, 8);
                }
            }
        }
    }

    // Extract formatted Russian phone numbers in text like "+7 (495) 123-45-67" or "8 (800) 555-35-35"
    extract_formatted_phones(text, &mut out.phones);
    out
}

fn extract_formatted_phones(text: &str, phones: &mut Vec<String>) {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if phones.len() >= 6 {
            break;
        }
        let starts_plus7 = bytes[i] == b'+' && i + 1 < bytes.len() && bytes[i + 1] == b'7';
        let starts_8_sep = bytes[i] == b'8'
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
            && i + 2 < bytes.len()
            && matches!(bytes[i + 1], b' ' | b'(' | b'-')
            && (bytes[i + 2] == b'(' || bytes[i + 2] == b'9' || bytes[i + 2] == b'8' || bytes[i + 2] == b'4' || bytes[i + 2] == b'3');
        if starts_plus7 || starts_8_sep {
            let mut j = if starts_plus7 { i + 2 } else { i + 1 };
            let mut digits = String::with_capacity(11);
            digits.push('7');
            let mut has_sep = false;
            while j < bytes.len() && digits.len() < 11 {
                let b = bytes[j];
                if b.is_ascii_digit() {
                    digits.push(b as char);
                    j += 1;
                } else if matches!(b, b' ' | b'-' | b'(' | b')') {
                    has_sep = true;
                    j += 1;
                } else {
                    break;
                }
            }
            let next_is_digit = j < bytes.len() && bytes[j].is_ascii_digit();
            if digits.len() == 11 && has_sep && !next_is_digit {
                if let Some(clean) = clean_phone(&digits) {
                    push_cap(phones, &clean, 6);
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_phones_emails_socials_and_messengers() {
        let sample = "Контакты завода:\nTel: +7 (495) 123-45-67, 8 (800) 555-35-35\nПочта: sales@zavod-stroy.ru\nПишите в https://t.me/zavod_stroy и https://wa.me/79991234567\nМы в ВК: https://vk.com/zavod_stroy";
        let c = contacts_in_text(sample);
        assert_eq!(c.phones, vec!["+74951234567", "+78005553535"]);
        assert_eq!(c.emails, vec!["sales@zavod-stroy.ru"]);
        assert_eq!(
            c.messengers,
            vec!["https://t.me/zavod_stroy", "https://wa.me/79991234567"]
        );
        assert_eq!(c.socials, vec!["https://vk.com/zavod_stroy"]);
    }

    #[test]
    fn tel_marker_before_cyrillic_does_not_cut_a_letter() {
        let sample = format!("Адрес Tel: {}", "я".repeat(80));
        let c = contacts_in_text(&sample);
        assert!(c.phones.is_empty());
        let with_phone = contacts_in_text("Tel: +74951234567, дальше текст");
        assert_eq!(with_phone.phones, vec!["+74951234567"]);
    }
}
