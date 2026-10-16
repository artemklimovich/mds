use std::net::IpAddr;
use std::time::Duration;

use url::Url;

use crate::signals::{read_page, x_robots_tag};
use crate::{is_broken_url, is_not_found, page_from_html, path_allowed, USER_AGENT};

const MARKETS: &[&str] = &[
    "ozon.ru",
    "wildberries.ru",
    "wb.ru",
    "market.yandex.ru",
    "avito.ru",
    "megamarket.ru",
];

#[derive(Clone, Debug)]
pub struct CheckLine {
    pub ok: bool,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct SiteCheck {
    pub home: String,
    pub queue: Vec<String>,
    pub lines: Vec<CheckLine>,
}

pub async fn check_site(raw: &str) -> SiteCheck {
    check_site_inner(raw, false).await
}

#[cfg(test)]
pub async fn check_site_local(raw: &str) -> SiteCheck {
    check_site_inner(raw, true).await
}

async fn check_site_inner(raw: &str, allow_local: bool) -> SiteCheck {
    let mut lines = Vec::new();
    let Some(url) = normalize_site(raw, allow_local, &mut lines) else {
        return SiteCheck {
            home: String::new(),
            queue: Vec::new(),
            lines,
        };
    };
    if is_market(url.host_str().unwrap_or("")) {
        lines.push(line(false, "Это маркетплейс. В очередь ставим прямой сайт компании, не витрину."));
        return SiteCheck {
            home: String::new(),
            queue: Vec::new(),
            lines,
        };
    }

    let client = match reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::limited(4))
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            lines.push(line(false, "Проверка сейчас не запускается. Попробуйте ещё раз."));
            return empty(lines);
        }
    };

    let response = match client.get(url.clone()).send().await {
        Ok(response) => response,
        Err(_) => {
            lines.push(line(
                false,
                "Сайт не открылся. Пока страница не отвечает, в очередь её не ставим.",
            ));
            return empty(lines);
        }
    };
    let final_url = response.url().clone();
    if !host_ok(final_url.host_str().unwrap_or(""), allow_local) || is_broken_url(final_url.as_str()) {
        lines.push(line(false, "Сайт увёл на закрытый адрес. В очередь его не ставим."));
        return empty(lines);
    }
    if !response.status().is_success() {
        lines.push(line(
            false,
            format!(
                "Главная ответила {}. В индекс попадает страница, которая открывается.",
                response.status().as_u16()
            ),
        ));
        return empty(lines);
    }
    let (header_noindex, _) = response
        .headers()
        .get("x-robots-tag")
        .and_then(|value| value.to_str().ok())
        .map(x_robots_tag)
        .unwrap_or((false, false));
    let bytes = response.bytes().await.unwrap_or_default();
    if bytes.len() > 1_000_000 {
        lines.push(line(false, "Страница тяжелее мегабайта. Такую нода не индексирует."));
        return empty(lines);
    }
    let body = String::from_utf8_lossy(&bytes);
    let host = final_url.host_str().unwrap_or("").to_ascii_lowercase();
    let page = page_from_html(final_url.as_str(), &host, &body);
    if header_noindex || !read_page(&body).indexable {
        lines.push(line(
            false,
            "На странице стоит noindex. Снимите его с тех страниц, которые нужно находить.",
        ));
        return SiteCheck {
            home: origin_of(&final_url),
            queue: Vec::new(),
            lines,
        };
    }
    if is_not_found(&page.title, &page.text) {
        lines.push(line(
            false,
            "Страница говорит, что ничего не найдено. В индекс такую не кладём.",
        ));
        return empty(lines);
    }
    lines.push(line(true, "Страница открывается и её можно индексировать."));

    let origin = origin_of(&final_url);
    let robots_url = format!("{origin}/robots.txt");
    let robots = match client.get(&robots_url).send().await {
        Ok(response) if response.status().is_success() => response.text().await.unwrap_or_default(),
        _ => String::new(),
    };
    if robots.is_empty() {
        lines.push(line(
            false,
            "Нет robots.txt. Положите файл в корень и укажите в нём Sitemap: так обход найдёт страницы сам.",
        ));
    } else if !path_allowed(&robots, USER_AGENT, "/") {
        lines.push(line(
            false,
            "robots.txt запрещает MDS-Bot. Разрешите обход, иначе сайт в индекс не попадёт.",
        ));
        return SiteCheck {
            home: origin,
            queue: Vec::new(),
            lines,
        };
    } else if robots.to_ascii_lowercase().contains("sitemap:") {
        lines.push(line(true, "robots.txt пускает MDS-Bot и указывает Sitemap."));
    } else {
        lines.push(line(
            true,
            "robots.txt пускает MDS-Bot. Добавьте строку Sitemap, чтобы обход не собирал страницы только по ссылкам.",
        ));
    }

    let card = match client.get(format!("{origin}/llms.txt")).send().await {
        Ok(response) if response.status().is_success() => true,
        _ => match client.get(format!("{origin}/llms.md")).send().await {
            Ok(response) if response.status().is_success() => true,
            _ => false,
        },
    };
    if card {
        lines.push(line(true, "Карточка /llms.txt или /llms.md на месте. Её текст попадёт в индекс."));
    } else {
        lines.push(line(
            false,
            "Нет карточки /llms.txt. Коротко напишите, как сайт называется и какие страницы важные. Образец на странице llms.md этого поиска.",
        ));
    }

    if page.text.chars().count() < 180 {
        lines.push(line(
            false,
            "На странице мало своего текста. Запрос находит слова со страницы, а не название домена.",
        ));
    } else {
        lines.push(line(true, "На странице достаточно своего текста для поиска по словам."));
    }
    lines.push(line(
        true,
        "Если сайт про конкретный край или город, назовите это место в тексте: тогда он встанет в полосу этого региона.",
    ));
    lines.push(line(
        true,
        "Лёгкая пчела на сервере сайта поднимает его среди близких результатов. Как поставить — на странице «Сайту».",
    ));

    let mut queue = vec![format!("{origin}/")];
    if final_url.path() != "/" && !is_broken_url(final_url.as_str()) {
        queue.push(final_url.to_string());
    }
    SiteCheck {
        home: origin,
        queue,
        lines,
    }
}

fn empty(lines: Vec<CheckLine>) -> SiteCheck {
    SiteCheck {
        home: String::new(),
        queue: Vec::new(),
        lines,
    }
}

fn line(ok: bool, text: impl Into<String>) -> CheckLine {
    CheckLine { ok, text: text.into() }
}

fn normalize_site(raw: &str, allow_local: bool, lines: &mut Vec<CheckLine>) -> Option<Url> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        lines.push(line(false, "Напишите адрес сайта."));
        return None;
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let Ok(url) = Url::parse(&with_scheme) else {
        lines.push(line(false, "Адрес не разбирается. Нужен вид https://сайт.ru"));
        return None;
    };
    if url.scheme() != "http" && url.scheme() != "https" {
        lines.push(line(false, "Нужен обычный адрес http или https."));
        return None;
    }
    if !url.username().is_empty() || url.password().is_some() {
        lines.push(line(false, "Адрес с паролем в очередь не ставим."));
        return None;
    }
    let host = url.host_str().unwrap_or("");
    if host.is_empty() || is_broken_url(url.as_str()) || !host_ok(host, allow_local) {
        lines.push(line(false, "Такой адрес в обход не берём."));
        return None;
    }
    Some(url)
}

fn host_ok(host: &str, allow_local: bool) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    if allow_local && (host == "localhost" || host == "127.0.0.1" || host == "::1") {
        return true;
    }
    if host == "localhost" || host.ends_with(".local") || host.ends_with(".internal") {
        return false;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return ip_is_public(ip);
    }
    true
}

fn ip_is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && !ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            let octets = ip.octets();
            let link_local = octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80;
            let unique_local = (octets[0] & 0xfe) == 0xfc;
            !ip.is_loopback() && !ip.is_unspecified() && !ip.is_multicast() && !link_local && !unique_local
        }
    }
}

fn is_market(host: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    MARKETS.iter().any(|market| host == *market || host.ends_with(&format!(".{market}")))
}

fn origin_of(url: &Url) -> String {
    format!("{}://{}", url.scheme(), url.host_str().unwrap_or(""))
        + &url.port().map(|port| format!(":{port}")).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn serve(pages: &[(&'static str, &'static str, &'static str)]) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let pages: Vec<(String, String, String)> = pages
            .iter()
            .map(|(path, kind, body)| ((*path).to_string(), (*kind).to_string(), (*body).to_string()))
            .collect();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = [0_u8; 2048];
                let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;
                let request = String::from_utf8_lossy(&buf);
                let mut status = "404 Not Found";
                let mut kind = "text/plain";
                let mut body = String::new();
                for (path, page_kind, page_body) in &pages {
                    if request.contains(&format!("GET {path} ")) {
                        status = "200 OK";
                        kind = page_kind;
                        body = page_body.clone();
                    }
                }
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {kind}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tokio::io::AsyncWriteExt::write_all(&mut socket, response.as_bytes()).await;
            }
        });
        format!("http://{addr}/")
    }

    #[tokio::test]
    async fn open_site_is_queued_and_missing_card_is_named() {
        let home = serve(&[
            (
                "/",
                "text/html",
                "<html><title>Пасека</title><p>Натуральный мёд с пасеки в Камышине, стеклянные банки и отдых на ферме для гостей из города.</p></html>",
            ),
            ("/robots.txt", "text/plain", "User-agent: *\nAllow: /\nSitemap: http://example.test/sitemap.xml\n"),
        ])
        .await;
        let check = check_site_local(&home).await;
        assert!(!check.queue.is_empty(), "{:?}", check.lines);
        assert!(check.lines.iter().any(|line| line.ok && line.text.contains("Sitemap")));
        assert!(check.lines.iter().any(|line| !line.ok && line.text.contains("llms.txt")));
    }

    #[tokio::test]
    async fn disallowed_site_stays_out_of_the_queue() {
        let home = serve(&[
            (
                "/",
                "text/html",
                "<html><title>Закрыто</title><p>Длинный текст страницы, которую робот читать не должен, хотя слов здесь достаточно.</p></html>",
            ),
            ("/robots.txt", "text/plain", "User-agent: *\nDisallow: /\n"),
        ])
        .await;
        let check = check_site_local(&home).await;
        assert!(check.queue.is_empty(), "{:?}", check.lines);
        assert!(check.lines.iter().any(|line| line.text.contains("запрещает")));
    }

    #[test]
    fn marketplace_and_private_host_are_refused() {
        let market = tokio::runtime::Runtime::new().unwrap().block_on(check_site("https://www.ozon.ru/product"));
        assert!(market.queue.is_empty());
        assert!(market.lines.iter().any(|line| line.text.contains("маркетплейс")));
        let local = tokio::runtime::Runtime::new().unwrap().block_on(check_site("http://127.0.0.1/secret"));
        assert!(local.queue.is_empty());
        assert!(local.lines.iter().any(|line| line.text.contains("не берём")));
    }
}
