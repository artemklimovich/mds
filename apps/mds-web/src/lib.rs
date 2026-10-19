use std::collections::HashSet;

use mds_types::{host_of_url, open_feed, press_feed, region_names, tab_mask, theme_mask, Hit};
use serde::{Deserialize, Serialize};

pub const PAGE_SIZE: usize = 25;
pub const SOURCE_REPO: &str = "https://github.com/artemklimovich/mds";
pub const RELEASE_WINDOWS: &str =
    "https://github.com/artemklimovich/mds/releases/latest/download/pchela-windows-x64.zip";
pub const RELEASE_LINUX: &str =
    "https://github.com/artemklimovich/mds/releases/latest/download/pchela-linux-x64.tar.gz";
pub const RELEASE_ANDROID: &str =
    "https://github.com/artemklimovich/mds/releases/latest/download/pchela-android.apk";

const PUBLIC_PATHS: &[&str] = &[
    "/",
    "/manifesto",
    "/how",
    "/for-sites",
    "/indexing",
    "/add",
    "/llms",
    "/help",
    "/bees",
    "/android",
    "/hive",
    "/computer",
    "/download",
    "/llms.txt",
    "/llms.md",
];

pub fn hive_card() -> String {
    format!(
        r#"# Пчелопоиск

> Волонтёрский поиск реальных сайтов. В выдаче ссылка на страницу, без рекламы и без пересказа вместо ссылки. Ноду можно поставить самому. Одна нода называется пчела.

- [Поиск](/)
- [Манифест](/manifesto)
- [Как устроен поиск](/how)
- [Что сделать на сайте](/for-sites)
- [Индексация](/indexing)
- [Добавить сайт](/add)
- [Карточка llms](/llms)
- [Приложение для Android](/android)
- [Скачать ноду](/download)
- [Исходный код]({SOURCE_REPO})
"#
    )
}

pub fn robots_txt(origin: &str) -> String {
    format!(
        "User-agent: *\nAllow: /\nDisallow: /operator\nDisallow: /peer/\nDisallow: /search\nDisallow: /download/mds.exe\nDisallow: /download/pchela.zip\n\nSitemap: {origin}/sitemap.xml\n"
    )
}

pub fn sitemap_xml(origin: &str) -> String {
    let mut body = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for path in PUBLIC_PATHS {
        body.push_str(&format!("<url><loc>{origin}{path}</loc></url>\n"));
    }
    body.push_str("</urlset>\n");
    body
}

pub fn offer_region(page: String, label: &str, href: &str) -> String {
    let line = format!(
        "<p class=\"handoff\"><a href=\"{}\">Искать в {}</a></p>",
        escape(href),
        escape(label)
    );
    page.replacen("</main>", &format!("{line}</main>"), 1)
}

pub fn render(query: &str, hits: &[Hit], page: usize, error: Option<&str>) -> String {
    render_results(query, hits, &[], false, page, error, None, "", "", None, None, None, true)
}

pub fn render_results(
    query: &str,
    hits: &[Hit],
    adult: &[Hit],
    adult_open: bool,
    page: usize,
    error: Option<&str>,
    region: Option<&str>,
    tab: &str,
    theme: &str,
    lat: Option<f64>,
    lon: Option<f64>,
    radius: Option<f64>,
    show_bees: bool,
) -> String {
    let page = visible_page(page, hits.len());
    let mut body = if let Some(error) = error {
        format!("<p class=\"error\">{}</p>", escape(error))
    } else if query.is_empty() {
        "<p class=\"hint\">Поиск по страницам, которые обошла эта пчела. Слова и смысл, на русском и на английском, без фильтра по языку.</p>".into()
    } else if hits.is_empty() && adult.is_empty() {
        if tab_mask(tab) == 0 && theme_mask(theme) == 0 {
            "<p class=\"hint\">Ничего не нашлось.</p>".into()
        } else {
            "<p class=\"hint\">В этой вкладке пока нет страниц.</p>".into()
        }
    } else if hits.is_empty() {
        adult_section(query, adult, adult_open)
    } else {
        let start = (page - 1) * PAGE_SIZE;
        let end = (start + PAGE_SIZE).min(hits.len());
        let mut list = format!(
            "<p class=\"stats\">Найдено {}. Страница {page} из {}.</p>",
            hits.len(),
            page_count(hits.len())
        );
        let mut band = "";
        let mut open_list = false;
        let mut seen_hosts = HashSet::new();
        let mut seen_shops = HashSet::new();
        for hit in &hits[start..end] {
            if let Some(region) = region {
                let kind = if hit.region.as_deref() == Some(region) {
                    "home"
                } else {
                    "other"
                };
                if kind != band {
                    if open_list {
                        list.push_str("</ol>");
                        open_list = false;
                    }
                    if kind == "home" {
                        list.push_str(&format!(
                            "<h2 class=\"band\">В вашем регионе: {}</h2>",
                            escape(region)
                        ));
                    } else if band.is_empty() && start == 0 {
                        list.push_str(
                            "<p class=\"hint\">В этом регионе страниц не нашлось.</p><h2 class=\"band\">В других регионах</h2>",
                        );
                    } else {
                        list.push_str("<h2 class=\"band\">В других регионах</h2>");
                    }
                    band = kind;
                }
            }
            if !open_list {
                list.push_str("<ol class=\"results\">");
                open_list = true;
            }
            let badges = badges(
                hit.via_words,
                hit.via_meaning,
                hit.volunteer,
                hit.region.as_deref(),
                hit.place.as_deref(),
                hit.distance_km,
            );
            let shot = hit.media_url.as_ref().map(|src| {
                format!(
                    "<a class=\"shot\" href=\"{url}\"><img src=\"{src}\" alt=\"\" referrerpolicy=\"no-referrer\"></a>",
                    url = escape(&hit.url),
                    src = escape(src),
                )
            }).unwrap_or_default();
            let price = hit.price.as_ref().map(|price| format!("<span class=\"price\">{}</span>", escape(price))).unwrap_or_default();
            let rail = if tab == "news" {
                news_rail(hit, &mut seen_hosts)
            } else {
                shop_rail(hit, &mut seen_shops)
            };
            list.push_str(&format!(
                "<li>{shot}<a href=\"{url}\">{title}</a>{price}<div class=\"url\">{url}</div><p>{snippet}</p>{badges}{rail}</li>",
                url = escape(&hit.url),
                title = escape(&hit.title),
                snippet = escape(&hit.snippet),
            ));
        }
        if open_list {
            list.push_str("</ol>");
        }
        list.push_str(&pager(query, region, tab, theme, lat, lon, radius, page, hits.len()));
        list.push_str(&adult_section(query, adult, adult_open));
        list
    };
    if body.contains("class=\"wire") {
        body.push_str(WIRE_SCRIPT);
    }
    document(query, &body, true, query.is_empty() && error.is_none(), region, tab, theme, lat, lon, radius, show_bees)
}

fn news_rail(hit: &Hit, seen: &mut HashSet<String>) -> String {
    let marked = hit.news_feed.as_deref().and_then(open_feed);
    let feed = marked.or_else(|| host_of_url(&hit.url).and_then(|host| press_feed(&host).map(str::to_string)));
    let Some(feed) = feed else {
        return String::new();
    };
    let host = host_of_url(&hit.url).unwrap_or_default();
    if !seen.insert(host) {
        return String::new();
    }
    format!(
        r#"<section class="wire site" data-feed="{feed}"><p class="wire-line"><button type="button" class="wire-open">Раскрыть</button> Свежие заголовки</p><div class="rail" hidden><button type="button" class="nudge back" aria-label="Влево">‹</button><div class="wire-row"></div><button type="button" class="nudge forward" aria-label="Вправо">›</button></div></section>"#,
        feed = escape(&feed),
    )
}

fn shop_rail(hit: &Hit, seen: &mut HashSet<String>) -> String {
    let Some(feed) = hit.feed_url.as_deref().and_then(open_feed) else {
        return String::new();
    };
    let host = host_of_url(&hit.url).unwrap_or_default();
    if !seen.insert(host) {
        return String::new();
    }
    format!(
        r#"<section class="wire site" data-offers="{feed}"><p class="wire-line"><button type="button" class="wire-open">Раскрыть</button> Товары</p><div class="rail" hidden><button type="button" class="nudge back" aria-label="Влево">‹</button><div class="wire-row"></div><button type="button" class="nudge forward" aria-label="Вправо">›</button></div></section>"#,
        feed = escape(&feed),
    )
}

fn bee_svg(class_name: &str) -> String {
    format!(
        r##"<svg class="{class_name}" viewBox="0 0 512 512" aria-hidden="true"><g><path style="fill:#414B58;" d="M195.613,512c-5.301,0-10.592-2.086-14.538-6.227c-7.648-8.027-7.341-20.734,0.686-28.383 c21.12-20.124,21.12-44.704,21.12-62.651c0-11.087,8.988-20.075,20.075-20.075s20.075,8.988,20.075,20.075 c0,20.874,0,59.728-33.574,91.72C205.572,510.162,200.588,512,195.613,512z"/><path style="fill:#414B58;" d="M382.31,512c-5.301,0-10.592-2.086-14.538-6.227c-7.648-8.027-7.341-20.734,0.686-28.383 c21.12-20.124,21.12-44.704,21.12-62.651c0-11.087,8.988-20.075,20.075-20.075s20.076,8.988,20.076,20.075 c0,20.874,0,59.728-33.574,91.72C392.268,510.162,387.285,512,382.31,512z"/><path style="fill:#414B58;" d="M97.165,344.265L97.165,344.265c-30.469,0-59.833-11.415-82.301-31.995l-6.482-5.937 c-3.27-2.995-3.27-8.148,0-11.143l6.482-5.937c22.468-20.58,51.832-31.995,82.301-31.995l0,0V344.265z"/></g><path style="opacity:0.1;" d="M97.165,257.257c-17.054,0-33.761,3.579-49.107,10.337 c-2.254,10.743-3.441,21.841-3.441,33.167l0,0c0,11.326,1.187,22.424,3.441,33.167c15.346,6.758,32.053,10.337,49.107,10.337l0,0 L97.165,257.257L97.165,257.257z"/><g class="wing right"><path style="fill:#C9F4FF;" d="M278.466,172.689h61.742c47.687,0,86.344-38.658,86.344-86.344l0,0 C426.552,38.658,387.895,0,340.208,0l0,0c-47.687,0-86.344,38.658-86.344,86.344v61.742 C253.863,161.674,264.878,172.689,278.466,172.689z"/><path style="fill:#AEEAF9;" d="M282.341,22.267c-17.486,15.801-28.477,38.654-28.477,64.077v61.742 c0,13.587,11.015,24.602,24.602,24.602h61.742c18.536,0,35.692-5.86,49.76-15.801C293.8,156.554,283.193,63.008,282.341,22.267z"/></g><path style="fill:#FBDD56;" d="M381.149,457.852L381.149,457.852c-63.495,14.677-129.502,14.677-192.997,0l0,0 C115.023,440.948,63.23,375.819,63.23,300.761l0,0c0-75.057,51.792-140.187,124.922-157.091l0,0 c63.495-14.677,129.502-14.677,192.997,0l0,0c73.129,16.904,124.922,82.033,124.922,157.091l0,0 C506.071,375.819,454.278,440.948,381.149,457.852z"/><path style="fill:#596675;" d="M312.109,133.554c-38-2.44-76.243,0.188-113.713,7.889c-26.02,104.543-26.02,214.093,0,318.636 c37.471,7.701,75.714,10.329,113.713,7.889C277.549,359.294,277.549,242.228,312.109,133.554z"/><path style="opacity:0.1;" d="M141.449,162.53C93.94,191.068,63.23,242.969,63.23,300.761l0,0 c0,75.057,51.792,140.187,124.922,157.091l0,0c63.495,14.677,129.502,14.677,192.997,0l0,0c2.259-0.522,4.497-1.091,6.714-1.704 C179.894,463.353,134.688,271.517,141.449,162.53z"/><g class="wing left"><path style="fill:#C9F4FF;" d="M229.261,172.689h-61.742c-47.687,0-86.344-38.658-86.344-86.344l0,0 C81.175,38.658,119.832,0,167.519,0l0,0c47.687,0,86.344,38.658,86.344,86.344v61.742 C253.863,161.674,242.849,172.689,229.261,172.689z"/></g><g><ellipse style="fill:#FFA6BB;" cx="346.48" cy="304.81" rx="23.07" ry="18.372"/><ellipse style="fill:#FFA6BB;" cx="471.53" cy="304.81" rx="23.07" ry="18.372"/></g><g><path style="fill:#414B58;" d="M359.928,293.985c-4.143,0-7.5-3.358-7.5-7.5v-8.147c0-4.142,3.357-7.5,7.5-7.5s7.5,3.358,7.5,7.5 v8.147C367.428,290.627,364.07,293.985,359.928,293.985z"/><path style="fill:#414B58;" d="M458.087,293.985c-4.143,0-7.5-3.358-7.5-7.5v-8.147c0-4.142,3.357-7.5,7.5-7.5s7.5,3.358,7.5,7.5 v8.147C465.587,290.627,462.229,293.985,458.087,293.985z"/><path style="fill:#414B58;" d="M409.007,300.241c-7.127,0-13.903-3.086-18.594-8.467c-2.722-3.122-2.396-7.86,0.727-10.582 c3.122-2.721,7.859-2.397,10.582,0.727c1.84,2.112,4.495,3.323,7.285,3.323s5.445-1.211,7.284-3.322 c2.723-3.124,7.459-3.449,10.582-0.728s3.449,7.459,0.729,10.582C422.912,297.154,416.135,300.241,409.007,300.241z"/><path style="fill:#414B58;" d="M297.627,512c-5.301,0-10.592-2.086-14.538-6.227c-7.648-8.027-7.341-20.734,0.686-28.383 c21.12-20.124,21.12-44.704,21.12-62.651c0-11.087,8.988-20.075,20.075-20.075s20.076,8.988,20.076,20.075 c0,20.874,0,59.728-33.575,91.72C307.586,510.162,302.602,512,297.627,512z"/><path style="fill:#414B58;" d="M110.931,512c-5.301,0-10.592-2.086-14.538-6.227c-7.648-8.027-7.341-20.734,0.686-28.383 c21.12-20.124,21.12-44.704,21.12-62.651c0-11.087,8.988-20.075,20.075-20.075s20.075,8.988,20.075,20.075 c0,20.874,0,59.728-33.575,91.72C120.89,510.162,115.905,512,110.931,512z"/></g><g class="wing left"><path style="fill:#AEEAF9;" d="M137.795,5.253c-33.043,12.115-56.621,43.849-56.621,81.091l0,0 c0,47.687,38.658,86.344,86.344,86.344h54.07C135.028,162.9,122.447,44.153,137.795,5.253z"/></g></svg>"##,
        class_name = class_name,
    )
}

fn search_bee_svg() -> String {
    r##"<svg class="go" viewBox="0 54 512 405" aria-hidden="true"><g fill="#1c1612"><path d="m453.724 212.315c6.019-22.869 26.671-39.016 50.773-39.016v-15.006c-29.469 0-54.913 18.79-63.995 45.93-3.898-2.088-7.909-3.989-12.023-5.693-1.413-29.878 22.517-55.328 52.52-55.242v-15.006c-36.407 0-66.159 28.967-67.463 65.061-11.64-3.28-23.912-5.039-36.587-5.039h-37.27c17.546-15.59 29.109-35.945 28.845-59.852 0-19.995-7.787-38.794-21.925-52.932-22.852-22.851-57.807-28.337-86.4-14.032-10.158-5.078-21.598-7.894-33.625-7.894-41.276 0-74.857 33.581-74.857 74.858 0 25.975 13.305 48.895 33.451 62.324-50.362 9.76-90.828 47.586-104.405 96.492l-80.763 29.61v12.588l80.783 30.007c2.919 10.547 7.098 20.733 12.501 30.284l13.062-7.389c-10.128-17.904-15.481-38.31-15.481-59.013 0-66.193 53.852-120.045 120.045-120.045h.053v240.09h-.053c-28.442 0-56.026-10.124-77.67-28.509l-9.715 11.437c24.354 20.685 55.387 32.077 87.384 32.077h166.041c74.467 0 135.05-60.583 135.05-135.05 0-45.968-23.091-86.637-58.276-111.04zm-427.437 110.908 50.884-18.656c-.859 6.143-1.313 12.413-1.313 18.789 0 6.299.456 12.571 1.325 18.773zm289.725-67.393v187.57h-45.155v-240.09h45.155v37.514h15.006v-37.514h29.884v240.09h-29.884v-187.57zm-149.29-127.378c0-33.003 26.849-59.852 59.851-59.852 27.894 0 51.848 18.929 58.251 46.033l14.603-3.45c-3.774-15.975-12.422-29.677-24.16-39.705 21.019-6.775 44.624-1.444 60.719 14.652 11.305 11.305 17.531 26.335 17.531 42.322s-6.226 31.017-17.531 42.321l-17.53 17.53h-17.026v-59.852h-15.006v59.852h-59.852c-33.001.002-59.85-26.848-59.85-59.851zm59.245 74.858h29.884v240.09h-29.884zm149.94 240.09v-240.09c66.551-.595 121.121 53.483 121.087 120.045 0 66.193-54.767 120.045-121.087 120.045z"/><path d="m101.872 403.055c3.157 4.312 6.605 8.479 10.248 12.385l10.975-10.234c-3.24-3.474-6.307-7.181-9.115-11.016z"/><path d="m451.528 285.841h15.006v30.011h-15.006z"/><path d="m143.373 241.889h15.006v15.006h-15.006z"/><path d="m173.384 241.889h15.006v15.006h-15.006z"/><path d="m203.395 90.768h15.006v15.006h-15.006z"/></g></svg>"##.to_string()
}

fn wordmark() -> String {
    format!(
        r##"<a class="logo" href="/" aria-label="ПчелоПоиск">{}<span class="name">Поиск</span></a>"##,
        bee_svg("bee"),
    )
}

fn document(
    query: &str,
    body: &str,
    autofocus: bool,
    home: bool,
    region: Option<&str>,
    tab: &str,
    theme: &str,
    lat: Option<f64>,
    lon: Option<f64>,
    radius: Option<f64>,
    show_bees: bool,
) -> String {
    let q = escape(query);
    let focus = if autofocus { " autofocus" } else { "" };
    let clear_hidden = if query.trim().is_empty() { " hidden" } else { "" };
    let root = if home { " class=\"home\"" } else { "" };
    let mark = wordmark();
    let english = if home { "<p class=\"en\">Bee search</p>" } else { "" };
    format!(
        r#"<!DOCTYPE html>
<html lang="ru">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Пчелопоиск</title>
<link rel="icon" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'%3E%3Crect width='32' height='32' rx='6' fill='%23f6f1e7'/%3E%3Cpath d='M8 12c-4-5-9-4-10 1 4 2 8 2 10-1zM14 16c2-2 7-2 9 1 1 4-2 8-7 8s-8-3-7-7c0-1 2-2 5-2z' fill='%231c1612'/%3E%3Cpath d='M16 26c0 0-2 2-2 3a2 2 0 0 0 4 0c0-1-2-3-2-3z' fill='%23c9a227'/%3E%3C/svg%3E">
<style>
body {{ margin: 0; min-height: 100vh; display: flex; flex-direction: column; font-family: Arial, Helvetica, sans-serif; color: #202124; background: #fff; }}
main {{ flex: 1 0 auto; width: 100%; max-width: 680px; margin: 0 auto; padding: 28px 16px 32px; box-sizing: border-box; }}
h1 {{ margin: 0 0 8px; text-align: center; }}
.logo {{ display: inline-flex; align-items: center; gap: 2px; line-height: 1; text-decoration: none; color: #1c1612; }}
.logo .bee {{ width: 84px; height: 84px; flex: 0 0 auto; }}
.home .logo .bee {{ width: 109px; height: 109px; }}
.logo .name {{ font-family: Georgia, "Times New Roman", serif; font-size: 42px; font-weight: 700; letter-spacing: -0.03em; }}
.home .logo .name {{ font-size: 68px; }}
.home main {{ max-width: 640px; padding-top: 10vh; text-align: center; }}
.home .en {{ margin: 4px 0 28px; color: #6b645c; font-size: 13px; letter-spacing: 0.22em; text-transform: uppercase; }}
form {{ position: relative; margin: 12px 0 22px; }}
form:has(details[open]) {{ margin-bottom: 58px; }}
.ask {{ display: flex; align-items: center; gap: 8px; border: 1px solid #e7e1d6; border-radius: 32px; padding: 6px 6px 6px 20px; background: #fff; box-shadow: 0 6px 18px rgba(28,22,18,.08); }}
.ask:focus-within {{ border-color: #1c1612; box-shadow: 0 8px 22px rgba(28,22,18,.12); }}
h2.band {{ font-size: 16px; font-weight: 600; margin: 8px 0 12px; }}
textarea[name=q] {{ flex: 1; min-width: 0; box-sizing: border-box; min-height: 40px; max-height: 112px; border: 0; margin: 0; padding: 8px 2px; resize: none; overflow: hidden; font: inherit; font-size: 16px; line-height: 24px; background: transparent; }}
textarea[name=q]:focus {{ outline: none; }}
.ask button.clear {{ width: 32px; height: 32px; flex: 0 0 auto; border: 0; border-radius: 50%; background: transparent; color: #5f6368; font-size: 22px; line-height: 1; cursor: pointer; }}
.ask button.clear[hidden] {{ display: none; }}
.ask button.clear:hover {{ background: #f6f1e7; color: #1c1612; }}
.ask button.go {{ display: grid; place-items: center; width: 54px; height: 54px; flex: 0 0 auto; padding: 0; border: 0; background: transparent; cursor: pointer; }}
.ask button.go svg {{ width: 54px; height: 54px; display: block; }}
.ask button.go:hover svg, .ask button.go:focus-visible svg {{ transform: scale(1.06); }}
.ask button.go:focus-visible {{ outline: none; }}
form.admit button.go {{ height: 44px; padding: 0 18px; border: 0; border-radius: 22px; background: #1c1612; color: #fff; font: inherit; font-size: 15px; cursor: pointer; }}
details.tune {{ position: static; }}
summary.settings {{ display: grid; place-items: center; width: 40px; height: 40px; margin-right: 2px; border-radius: 50%; color: #5f6368; cursor: pointer; list-style: none; }}
summary.settings::-webkit-details-marker {{ display: none; }}
summary.settings:hover {{ background: #f6f1e7; }}
details.tune[open] > summary.settings {{ background: #1c1612; color: #fff; }}
.filters {{ position: absolute; left: 0; right: 0; top: calc(100% + 8px); z-index: 4; padding: 14px 14px 8px; border: 1px solid #e7e1d6; border-radius: 18px; background: #fff; box-shadow: 0 10px 28px rgba(28,22,18,.12); text-align: left; }}
.filter-label {{ margin: 0 0 8px; color: #6b645c; font-size: 12px; letter-spacing: .08em; text-transform: uppercase; }}
.themes {{ display: flex; flex-wrap: wrap; gap: 8px; margin: 0 0 14px; }}
button.theme {{ border: 1px solid #e7e1d6; border-radius: 16px; padding: 6px 12px; background: #fff; color: #1c1612; font: inherit; font-size: 14px; cursor: pointer; }}
button.theme.on, button.theme:hover {{ background: #fff8e1; border-color: #f0e2a8; }}
input.region-find {{ box-sizing: border-box; width: 100%; height: 40px; margin: 0 0 8px; padding: 0 14px; border: 1px solid #e7e1d6; border-radius: 14px; background: #fff; color: #1c1612; font: inherit; font-size: 15px; }}
input.region-find:focus {{ outline: none; border-color: #1c1612; }}
.region-menu {{ max-height: 220px; overflow: auto; margin: 0 0 6px; padding: 4px; border-radius: 12px; background: #fbfaf7; scrollbar-color: #e4d39a transparent; scrollbar-width: thin; }}
.region-menu::-webkit-scrollbar {{ width: 10px; }}
.region-menu::-webkit-scrollbar-thumb {{ background: #e4d39a; border-radius: 8px; border: 2px solid #fbfaf7; }}
button.region-opt {{ display: block; width: 100%; margin: 0; padding: 9px 12px; border: 0; border-radius: 10px; background: transparent; color: #1c1612; font: inherit; font-size: 14px; line-height: 1.3; text-align: left; cursor: pointer; }}
button.region-opt[hidden] {{ display: none; }}
button.region-opt:hover, button.region-opt:focus-visible {{ background: #fff8e1; outline: none; }}
button.region-opt.on {{ background: #f6e7b0; font-weight: 600; }}
p.region-now, p.subject-now {{ margin: 8px 0 0 18px; color: #5c3a12; font-size: 13px; text-align: left; }}
p.region-now + p.subject-now, p.subject-now + p.region-now {{ margin-top: 2px; }}
nav.kinds {{ display: flex; flex-wrap: wrap; gap: 8px 14px; margin: 4px 0 16px; }}
nav.kinds a {{ color: #5f6368; text-decoration: none; font-size: 14px; padding-bottom: 6px; }}
nav.kinds a.on {{ color: #202124; font-weight: 600; border-bottom: 2px solid #202124; }}
.bee-story .portrait {{ display: block; width: 220px; height: 220px; margin: 4px auto 8px; }}
.bee-story h3 {{ margin: 22px 0 8px; font-size: 20px; }}
form.bee-find {{ display: flex; gap: 8px; align-items: center; margin: 12px 0; }}
form.bee-find input[name=q] {{ flex: 1; min-width: 0; height: 44px; border: 1px solid #e7e1d6; border-radius: 22px; padding: 0 16px; font: inherit; font-size: 16px; }}
form.bee-find button {{ height: 44px; padding: 0 16px; border: 0; border-radius: 22px; background: #1c1612; color: #fff; font: inherit; cursor: pointer; }}
.bee-picks {{ display: flex; flex-wrap: wrap; gap: 8px; margin: 8px 0 16px; }}
.bee-picks a {{ padding: 6px 12px; border-radius: 16px; background: #fff8e1; border: 1px solid #f0e2a8; color: #1c1612; text-decoration: none; }}
a.shot img {{ display: block; max-width: 120px; max-height: 72px; margin: 0 0 6px; object-fit: cover; border-radius: 8px; }}
span.price {{ margin-left: 8px; color: #137333; font-size: 16px; }}
ul.offers {{ list-style: none; padding: 0; margin: 6px 0 0; }}
ul.offers li {{ margin: 0 0 4px; font-size: 14px; }}
section.wire {{ margin: 0 0 18px; }}
.wire-line {{ margin: 0 0 8px; color: #4d5156; font-size: 14px; }}
button.wire-open, button.nudge {{ height: 32px; border: 1px solid #dadce0; border-radius: 16px; background: #fff; color: #202124; cursor: pointer; }}
button.wire-open {{ padding: 0 14px; }}
button.nudge {{ width: 32px; padding: 0; flex: 0 0 auto; }}
.rail {{ display: flex; align-items: center; gap: 8px; }}
.rail[hidden] {{ display: none; }}
.wire-row {{ display: flex; gap: 10px; overflow-x: auto; scroll-behavior: smooth; flex: 1; min-width: 0; padding: 2px 0 8px; }}
.wire-row a.card {{ flex: 0 0 220px; display: block; padding: 8px 10px; border: 1px solid #dadce0; border-radius: 8px; text-decoration: none; }}
.wire-row a.card b {{ display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; color: #1a0dab; font-size: 14px; font-weight: 600; line-height: 1.35; }}
.wire-row a.card span {{ display: block; margin-top: 4px; color: #006621; font-size: 12px; }}
.url {{ color: #006621; font-size: 14px; }}
.stats {{ color: #70757a; font-size: 14px; }}
.results {{ list-style: none; padding: 0; margin: 0; }}
.results li {{ margin: 0 0 22px; }}
.results a {{ color: #1a0dab; font-size: 20px; text-decoration: none; }}
.results a:hover {{ text-decoration: underline; }}
.results p {{ margin: 4px 0; color: #4d5156; line-height: 1.4; }}
.badge {{ display: inline-block; margin-right: 6px; padding: 1px 8px; border-radius: 10px; background: #e8f0fe; color: #174ea6; font-size: 12px; }}
.badge.bee {{ background: #fff8e1; color: #5c3a12; }}
.pager {{ display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin: 8px 0 28px; }}
.pager a, .pager span {{ color: #1a0dab; text-decoration: none; padding: 6px 10px; border-radius: 4px; }}
.pager a:hover {{ background: #f1f3f4; }}
.pager .current {{ background: #1a73e8; color: #fff; }}
.hint, .error {{ color: #4d5156; }}
.error {{ color: #a50e0e; }}
.adult {{ margin-top: 8px; padding-top: 12px; border-top: 1px solid #dadce0; }}
.adult h2 {{ font-size: 18px; font-weight: 600; margin: 0 0 8px; }}
.blurred {{ filter: blur(8px); }}
a.confirm {{ display: inline-block; margin: 8px 0 16px; padding: 10px 16px; border-radius: 4px; background: #1a73e8; color: #fff; text-decoration: none; }}
table.log {{ width: 100%; border-collapse: collapse; font-size: 14px; }}
table.log th, table.log td {{ text-align: left; padding: 6px 8px; border-bottom: 1px solid #dadce0; }}
table.bees th, table.bees td {{ text-align: right; }}
table.bees th:first-child, table.bees td:first-child {{ text-align: left; }}
article h2 {{ font-size: 28px; font-weight: 500; margin: 8px 0 12px; }}
article h3 {{ font-size: 18px; font-weight: 600; margin: 26px 0 8px; }}
article p, article li {{ color: #3c4043; line-height: 1.5; }}
article ul {{ margin: 8px 0 16px; padding-left: 18px; }}
article pre {{ background: #f8f9fa; border: 1px solid #dadce0; border-radius: 8px; padding: 12px 14px; overflow-x: auto; font-size: 13px; line-height: 1.45; }}
article a.button {{ display: inline-block; margin: 8px 0 16px; padding: 10px 16px; border-radius: 4px; background: #1a73e8; color: #fff; text-decoration: none; }}
form.admit {{ display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }}
form.admit input[type=url] {{ flex: 1 1 220px; height: 44px; border: 1px solid #dfe1e5; border-radius: 22px; padding: 0 14px; font-size: 16px; }}
ul.check {{ list-style: none; padding: 0; margin: 8px 0 16px; }}
ul.check li {{ margin: 0 0 8px; }}
ul.check li.ok {{ color: #137333; }}
ul.check li.fix {{ color: #8a4b08; }}
footer {{ position: relative; flex-shrink: 0; margin-top: auto; border-top: 1px solid #dadce0; background: #f8f9fa; }}
.foot {{ max-width: 680px; margin: 0 auto; padding: 22px 16px 28px; color: #4d5156; font-size: 14px; line-height: 1.5; text-align: center; }}
.foot nav {{ display: flex; flex-wrap: wrap; gap: 14px; margin-bottom: 12px; justify-content: center; }}
.foot a {{ color: #1a0dab; }}
p.census {{ margin: 0 0 10px; color: #70757a; font-size: 13px; text-align: center; }}
p.credit {{ display: flex; align-items: center; flex-wrap: wrap; gap: 12px; margin: 0; justify-content: center; }}
.comb {{ position: static; display: block; width: 72px; height: 72px; margin: 12px auto 0; }}
a.help {{ flex: 0 0 auto; margin: 0; padding: 3px 10px; border-radius: 12px; background: #fff8e1; border: 1px solid #f0c14b; color: #5c3a12; font-size: 12px; font-weight: 600; line-height: 1.4; text-decoration: none; white-space: nowrap; }}
body.cabinet main {{ max-width: 1080px; }}
.cabinet {{ display: flex; gap: 28px; align-items: flex-start; }}
.side {{ position: sticky; top: 12px; flex: 0 0 168px; display: grid; gap: 2px; }}
.side a {{ display: block; padding: 8px 12px; border-radius: 8px; color: #3c4043; text-decoration: none; font-size: 14px; }}
.side a:hover {{ background: #f1f3f4; }}
.sheets {{ flex: 1; min-width: 0; }}
.sheets section {{ scroll-margin-top: 12px; margin: 0 0 28px; padding-bottom: 8px; }}
.bars {{ display: grid; gap: 8px; margin: 8px 0 16px; }}
.bar {{ display: grid; grid-template-columns: minmax(72px, 150px) 1fr auto; gap: 8px; align-items: center; font-size: 13px; }}
.bar i {{ display: block; height: 12px; background: #f1f3f4; border-radius: 6px; overflow: hidden; }}
.bar i b {{ display: block; height: 100%; border-radius: 6px; }}
.bar em {{ font-style: normal; font-weight: 600; }}
.legend {{ color: #70757a; font-size: 13px; }}
.bee-row {{ display: flex; justify-content: space-between; gap: 12px; align-items: baseline; }}
.bee-row .nums {{ color: #3c4043; font-size: 13px; text-align: right; }}
.quiet {{ color: #8a4b08; font-weight: 600; }}
svg.chart {{ width: min(100%, 720px); height: auto; margin: 4px 0 12px; }}
p.series {{ display: flex; align-items: center; gap: 8px; margin: 10px 0 2px; font-size: 14px; }}
p.series i {{ width: 12px; height: 12px; border-radius: 3px; flex: 0 0 auto; }}
p.series b {{ margin-left: auto; font-size: 20px; font-weight: 650; }}
h2 a.refresh {{ float: right; margin-top: 2px; padding: 6px 12px; border-radius: 8px; background: #1c1612; color: #fff; font-size: 14px; font-weight: 600; text-decoration: none; }}
nav.spans {{ display: flex; flex-wrap: wrap; gap: 8px; margin: 8px 0 14px; }}
nav.spans a {{ padding: 6px 12px; border-radius: 14px; background: #f1f3f4; color: #202124; text-decoration: none; font-size: 14px; }}
nav.spans a.on {{ background: #1c1612; color: #fff; }}
.map {{ display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin: 8px 0 16px; }}
.map b, .map span {{ padding: 6px 10px; border-radius: 14px; background: #f1f3f4; }}
.map b {{ background: #fff8e1; }}
form.inline {{ display: flex; flex-wrap: wrap; gap: 8px; align-items: end; margin: 8px 0 16px; }}
form.inline input {{ height: 36px; border: 1px solid #dadce0; border-radius: 8px; padding: 0 10px; }}
@media (max-width: 760px) {{
  .cabinet {{ display: block; }}
  .side {{ position: static; display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 16px; }}
  .comb {{ margin: 12px auto 0; }}
}}
</style>
</head>
<body{root}>
<main>
<h1>{mark}</h1>
{english}
<form action="/search" method="get">
<div class="ask">
<textarea name="q" rows="1"{focus} placeholder="Запрос">{q}</textarea>
<button class="clear" type="button" aria-label="Очистить запрос"{clear_hidden}>×</button>
<button class="go" type="submit" aria-label="Найти">{bee}</button>
{region_select}
</div>
{region_line}
{subject_line}
</form>
{tabs}
{body}
</main>
{footer}
{ask_script}
</body>
</html>"#,
        footer = footer(),
        region_select = search_settings(region, tab, theme, lat, lon, radius),
        region_line = region_line(region),
        subject_line = subject_line(theme),
        tabs = tab_row(query, region, tab, theme, lat, lon, radius, home, show_bees),
        bee = search_bee_svg(),
        ask_script = ASK_SCRIPT,
    )
}

fn tab_row(
    query: &str,
    region: Option<&str>,
    current: &str,
    theme: &str,
    lat: Option<f64>,
    lon: Option<f64>,
    radius: Option<f64>,
    home: bool,
    show_bees: bool,
) -> String {
    if !show_bees {
        if query.trim().is_empty() {
            return String::new();
        }
    } else if query.trim().is_empty() {
        if !home {
            return String::new();
        }
        return String::from("<nav class=\"kinds\" aria-label=\"Тип страницы\"><a href=\"/bees\">Всё о пчёлах</a></nav>");
    }
    if query.trim().is_empty() {
        return String::new();
    }
    const TABS: &[(&str, &str)] = &[
        ("", "Все"),
        ("news", "Новости"),
        ("gov", "Органы власти"),
        ("jobs", "Работа"),
        ("services", "Услуги"),
    ];
    let mut out = String::from("<nav class=\"kinds\" aria-label=\"Тип страницы\">");
    for (id, label) in TABS {
        let class = if *id == current { " class=\"on\"" } else { "" };
        out.push_str(&format!(
            "<a{class} href=\"{}\">{label}</a>",
            escape(&search_href(query, region, 1, id, theme, lat, lon, radius))
        ));
    }
    if show_bees {
        out.push_str("<a href=\"/bees\">Всё о пчёлах</a>");
    }
    out.push_str("</nav>");
    out
}

const ASK_SCRIPT: &str = r#"<script>
(function () {
  var box = document.querySelector("textarea[name=q]");
  var clear = document.querySelector("button.clear");
  if (box && clear) {
    function syncClear() { clear.hidden = !box.value.trim(); }
    syncClear();
    box.addEventListener("input", syncClear);
    clear.addEventListener("click", function () {
      box.value = "";
      syncClear();
      box.focus();
    });
  }
  var menu = document.querySelector(".region-menu");
  var hidden = document.querySelector("input[name=region]");
  var form = document.querySelector("form[action='/search']");
  function showRegion(name) {
    var line = document.querySelector("p.region-now");
    if (!name) {
      if (line) line.remove();
      return;
    }
    if (!line && form) {
      line = document.createElement("p");
      line.className = "region-now";
      var ask = form.querySelector(".ask");
      if (ask && ask.nextSibling) form.insertBefore(line, ask.nextSibling);
      else form.appendChild(line);
    }
    if (line) line.textContent = name;
  }
  if (menu && hidden) {
    menu.addEventListener("click", function (event) {
      var button = event.target.closest("button.region-opt");
      if (!button) return;
      hidden.value = button.getAttribute("data-region") || "";
      var items = menu.querySelectorAll("button.region-opt");
      for (var i = 0; i < items.length; i++) {
        if (items[i] === button) items[i].classList.add("on");
        else items[i].classList.remove("on");
      }
      showRegion(hidden.value ? button.textContent : "");
      var tune = menu.closest("details");
      if (tune) tune.open = false;
    });
  }
  var find = document.querySelector("input.region-find");
  var tune = document.querySelector("details.tune");
  if (find && menu) {
    find.addEventListener("keydown", function (event) {
      if (event.key === "Enter") event.preventDefault();
    });
    find.addEventListener("input", function () {
      var needle = find.value.trim().toLowerCase();
      var items = menu.querySelectorAll("button.region-opt");
      for (var i = 0; i < items.length; i++) {
        var button = items[i];
        var all = !button.getAttribute("data-region");
        var show = !needle || all || button.textContent.toLowerCase().indexOf(needle) !== -1;
        button.hidden = !show;
      }
    });
  }
  if (tune && find) {
    tune.addEventListener("toggle", function () {
      if (tune.open) return;
      find.value = "";
      find.dispatchEvent(new Event("input"));
    });
  }
  var themes = document.querySelector(".themes");
  var themeInput = document.querySelector("input[name=theme]");
  if (themes && themeInput) {
    themes.addEventListener("click", function (event) {
      var button = event.target.closest("button.theme[data-theme]");
      if (!button) return;
      themeInput.value = button.getAttribute("data-theme") || "";
      var items = themes.querySelectorAll("button.theme[data-theme]");
      for (var i = 0; i < items.length; i++) {
        if (items[i] === button) items[i].classList.add("on");
        else items[i].classList.remove("on");
      }
      var box = document.querySelector("textarea[name=q]");
      if (box && box.value.trim() && box.form) box.form.requestSubmit();
    });
  }
  var radii = document.querySelector(".radii");
  var radiusInput = document.querySelector("input[name=radius]");
  var latInput = document.querySelector("input[name=lat]");
  var lonInput = document.querySelector("input[name=lon]");
  if (radii && radiusInput && latInput && lonInput) {
    var params = new URLSearchParams(window.location.search);
    var curRadius = params.get("radius") || "";
    radiusInput.value = curRadius;
    latInput.value = params.get("lat") || "";
    lonInput.value = params.get("lon") || "";
    var rButtons = radii.querySelectorAll("button.radius-opt");
    for (var r = 0; r < rButtons.length; r++) {
      if ((rButtons[r].getAttribute("data-radius") || "") === curRadius) rButtons[r].classList.add("on");
      else rButtons[r].classList.remove("on");
    }
    radii.addEventListener("click", function (event) {
      var button = event.target.closest("button.radius-opt");
      if (!button) return;
      var rad = button.getAttribute("data-radius") || "";
      var items = radii.querySelectorAll("button.radius-opt");
      for (var i = 0; i < items.length; i++) {
        if (items[i] === button) items[i].classList.add("on");
        else items[i].classList.remove("on");
      }
      if (!rad) {
        radiusInput.value = "";
        latInput.value = "";
        lonInput.value = "";
        var box = document.querySelector("textarea[name=q]");
        if (box && box.value.trim() && box.form) box.form.requestSubmit();
        return;
      }
      radiusInput.value = rad;
      if (latInput.value && lonInput.value) {
        var boxReady = document.querySelector("textarea[name=q]");
        if (boxReady && boxReady.value.trim() && boxReady.form) boxReady.form.requestSubmit();
        return;
      }
      if (navigator.geolocation) {
        navigator.geolocation.getCurrentPosition(function (pos) {
          latInput.value = pos.coords.latitude.toFixed(3);
          lonInput.value = pos.coords.longitude.toFixed(3);
          var boxNow = document.querySelector("textarea[name=q]");
          if (boxNow && boxNow.value.trim() && boxNow.form) boxNow.form.requestSubmit();
        });
      }
    });
  }
  var box = document.querySelector("textarea[name=q]");
  if (!box) return;
  var max = 24 * 4 + 16;
  function grow() {
    box.style.height = "0px";
    var full = box.scrollHeight;
    var next = full > max ? max : full;
    box.style.height = next + "px";
    box.style.overflowY = full > max ? "auto" : "hidden";
  }
  box.addEventListener("input", grow);
  box.addEventListener("keydown", function (event) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      if (box.form) box.form.requestSubmit();
    }
  });
  grow();
})();
</script>
"#;

const WIRE_SCRIPT: &str = r#"<script>
document.querySelectorAll("section.wire").forEach(function (box) {
  var button = box.querySelector("button.wire-open");
  var rail = box.querySelector(".rail");
  var row = box.querySelector(".wire-row");
  var query = (box.getAttribute("data-q") || "").toLowerCase().split(/\s+/).filter(Boolean);
  var feed = box.getAttribute("data-feed") || "";
  var offers = box.getAttribute("data-offers") || "";
  var loaded = false;
  button.addEventListener("click", function () {
    if (!rail.hidden) {
      rail.hidden = true;
      button.textContent = "Раскрыть";
      return;
    }
    rail.hidden = false;
    button.textContent = "Скрыть";
    if (loaded) return;
    loaded = true;
    button.textContent = "Загрузка";
    var address = offers ? "/offers?feed=" + encodeURIComponent(offers) : (feed ? "/wire?feed=" + encodeURIComponent(feed) : "/wire");
    fetch(address).then(function (response) { return response.json(); }).then(function (items) {
      button.textContent = "Скрыть";
      var shown = 0;
      (items || []).forEach(function (item) {
        var titleText = item && (item.title || item.name);
        if (shown >= 40 || !titleText || !item.url) return;
        var hay = String(titleText).toLowerCase();
        if (query.length && !query.every(function (word) { return hay.indexOf(word) !== -1; })) return;
        var card = document.createElement("a");
        card.className = "card";
        card.href = item.url;
        var title = document.createElement("b");
        title.textContent = item.price ? titleText + " — " + item.price : titleText;
        var source = document.createElement("span");
        source.textContent = item.source || "";
        card.appendChild(title);
        card.appendChild(source);
        row.appendChild(card);
        shown++;
      });
      if (!shown) {
        var empty = document.createElement("p");
        empty.className = "hint";
        empty.textContent = offers ? "В фиде товаров сейчас нет." : "В свежих лентах этого запроса нет.";
        row.appendChild(empty);
      }
    }).catch(function () {
      button.textContent = "Скрыть";
      var empty = document.createElement("p");
      empty.className = "hint";
      empty.textContent = "Ленту сейчас не прочитать.";
      row.appendChild(empty);
    });
  });
  box.querySelector("button.back").addEventListener("click", function () {
    row.scrollBy({ left: -240, behavior: "smooth" });
  });
  box.querySelector("button.forward").addEventListener("click", function () {
    row.scrollBy({ left: 240, behavior: "smooth" });
  });
});
</script>"#;

fn subject_line(theme: &str) -> String {
    let Some(label) = subject_label(theme) else {
        return String::new();
    };
    format!("<p class=\"subject-now\">{label}</p>")
}

fn subject_label(tab: &str) -> Option<&'static str> {
    match tab {
        "medicine" => Some("Медицина"),
        "shop" => Some("Интернет-магазины"),
        "haul" => Some("Перевозки"),
        "auto" => Some("Авто"),
        "realty" => Some("Недвижимость"),
        "build" => Some("Стройка и ремонт"),
        "edu" => Some("Образование"),
        "ads" => Some("Реклама и маркетинг"),
        "ai" => Some("Искусственный интеллект"),
        _ => None,
    }
}

fn region_line(selected: Option<&str>) -> String {
    match selected {
        Some(name) if !name.is_empty() => format!("<p class=\"region-now\">{}</p>", escape(name)),
        _ => String::new(),
    }
}

fn search_settings(
    selected: Option<&str>,
    tab: &str,
    theme: &str,
    lat: Option<f64>,
    lon: Option<f64>,
    radius: Option<f64>,
) -> String {
    let current = selected.unwrap_or("");
    let mut options = String::new();
    let all_on = if current.is_empty() { " on" } else { "" };
    options.push_str(&format!(
        r#"<button type="button" class="region-opt{all_on}" data-region="" role="option">Все регионы</button>"#
    ));
    for name in region_names() {
        let on = if Some(*name) == selected { " on" } else { "" };
        options.push_str(&format!(
            "<button type=\"button\" class=\"region-opt{on}\" data-region=\"{}\" role=\"option\">{name}</button>",
            escape(name)
        ));
    }
    let mut themes = String::new();
    let clear_on = if subject_label(theme).is_none() { " on" } else { "" };
    themes.push_str(&format!(
        "<button type=\"button\" class=\"theme{clear_on}\" data-theme=\"\">Без тематики</button>"
    ));
    for (id, label) in [
        ("medicine", "Медицина"),
        ("shop", "Интернет-магазины"),
        ("haul", "Перевозки"),
        ("auto", "Авто"),
        ("realty", "Недвижимость"),
        ("build", "Стройка и ремонт"),
        ("edu", "Образование"),
        ("ads", "Реклама и маркетинг"),
        ("ai", "Искусственный интеллект"),
    ] {
        let on = if id == theme { " on" } else { "" };
        themes.push_str(&format!(
            "<button type=\"button\" class=\"theme{on}\" data-theme=\"{id}\">{label}</button>"
        ));
    }
    let mut radii = String::new();
    for (val, label) in [
        ("", "Выкл"),
        ("0.5", "500 м"),
        ("1", "1 км"),
        ("5", "5 км"),
        ("50", "50 км"),
        ("100", "100 км"),
    ] {
        let on = if val.is_empty() { " on" } else { "" };
        radii.push_str(&format!(
            "<button type=\"button\" class=\"theme radius-opt{on}\" data-radius=\"{val}\">{label}</button>"
        ));
    }
    format!(
        r#"<details class="tune">
<summary class="settings" aria-label="Настройки поиска"><svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" d="M4 7h16M4 12h16M4 17h16"/><circle cx="9" cy="7" r="1.7" fill="currentColor" stroke="none"/><circle cx="15" cy="12" r="1.7" fill="currentColor" stroke="none"/><circle cx="8" cy="17" r="1.7" fill="currentColor" stroke="none"/></svg></summary>
<div class="filters">
<p class="filter-label">Тематика</p>
<div class="themes">{themes}</div>
<p class="filter-label">Рядом со мной</p>
<div class="themes radii">{radii}</div>
<p class="filter-label">Регион</p>
<input class="region-find" type="text" placeholder="Найти регион" aria-label="Найти регион" autocomplete="off">
<div class="region-menu" role="listbox" aria-label="Регион">{options}</div>
<input type="hidden" name="region" value="{current}">
<input type="hidden" name="tab" value="{tab}">
<input type="hidden" name="theme" value="{theme}">
<input type="hidden" name="lat" value="{lat}">
<input type="hidden" name="lon" value="{lon}">
<input type="hidden" name="radius" value="{radius}">
</div>
</details>"#,
        current = escape(current),
        tab = escape(tab),
        theme = escape(theme),
        lat = lat.map(coord_text).unwrap_or_default(),
        lon = lon.map(coord_text).unwrap_or_default(),
        radius = radius.map(coord_text).unwrap_or_default(),
    )
}

fn visible_page(page: usize, total: usize) -> usize {
    let page = page.max(1);
    let pages = page_count(total);
    page.min(pages)
}

fn page_count(total: usize) -> usize {
    if total == 0 {
        1
    } else {
        total.div_ceil(PAGE_SIZE)
    }
}

fn pager(
    query: &str,
    region: Option<&str>,
    tab: &str,
    theme: &str,
    lat: Option<f64>,
    lon: Option<f64>,
    radius: Option<f64>,
    page: usize,
    total: usize,
) -> String {
    let pages = page_count(total);
    if pages <= 1 {
        return String::new();
    }
    let mut out = String::from("<nav class=\"pager\" aria-label=\"Страницы выдачи\">");
    if page > 1 {
        out.push_str(&format!(
            "<a href=\"{}\">Назад</a>",
            escape(&search_href(query, region, page - 1, tab, theme, lat, lon, radius))
        ));
    }
    for number in 1..=pages {
        if number == page {
            out.push_str(&format!("<span class=\"current\">{number}</span>"));
        } else {
            out.push_str(&format!(
                "<a href=\"{}\">{number}</a>",
                escape(&search_href(query, region, number, tab, theme, lat, lon, radius))
            ));
        }
    }
    if page < pages {
        out.push_str(&format!(
            "<a href=\"{}\">Дальше</a>",
            escape(&search_href(query, region, page + 1, tab, theme, lat, lon, radius))
        ));
    }
    out.push_str("</nav>");
    out
}

fn search_href(
    query: &str,
    region: Option<&str>,
    page: usize,
    tab: &str,
    theme: &str,
    lat: Option<f64>,
    lon: Option<f64>,
    radius: Option<f64>,
) -> String {
    let mut href = format!("/search?q={}&page={page}", encode_query(query));
    if let Some(region) = region {
        if !region.is_empty() {
            href.push_str("&region=");
            href.push_str(&encode_query(region));
        }
    }
    if !tab.is_empty() {
        href.push_str("&tab=");
        href.push_str(&encode_query(tab));
    }
    if !theme.is_empty() {
        href.push_str("&theme=");
        href.push_str(&encode_query(theme));
    }
    if let (Some(lat), Some(lon), Some(radius)) = (lat, lon, radius) {
        href.push_str("&lat=");
        href.push_str(&coord_text(lat));
        href.push_str("&lon=");
        href.push_str(&coord_text(lon));
        href.push_str("&radius=");
        href.push_str(&coord_text(radius));
    }
    href
}

fn coord_text(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    if (rounded - rounded.round()).abs() < 1e-9 {
        format!("{}", rounded.round() as i64)
    } else {
        let text = format!("{rounded:.3}");
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn adult_section(query: &str, adult: &[Hit], adult_open: bool) -> String {
    if adult.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "<section class=\"adult\"><h2>18+</h2><p>Эти страницы отдельно от обычной выдачи.</p>",
    );
    if adult_open {
        out.push_str("<ol class=\"results\">");
        for hit in adult.iter().take(10) {
            out.push_str(&format!(
                "<li><a href=\"{url}\">{title}</a><div class=\"url\">{url}</div><p>{snippet}</p></li>",
                url = escape(&hit.url),
                title = escape(&hit.title),
                snippet = escape(&hit.snippet),
            ));
        }
        out.push_str("</ol>");
    } else {
        out.push_str("<ol class=\"results blurred\">");
        for hit in adult.iter().take(10) {
            out.push_str(&format!(
                "<li><span>{title}</span><p>{snippet}</p></li>",
                title = escape(&hit.title),
                snippet = escape(&hit.snippet),
            ));
        }
        out.push_str("</ol>");
        out.push_str(&format!(
            "<a class=\"confirm\" href=\"{}\">Мне есть 18 лет</a>",
            escape(&format!("/search?q={}&adult=1", encode_query(query)))
        ));
    }
    out.push_str("</section>");
    out
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerLine {
    pub name: String,
    pub latency_ms: u64,
    pub alive: bool,
    pub buckets: usize,
    pub pages: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShellLine {
    pub day_apps: i64,
    pub day_pages: i64,
    pub week_apps: i64,
    pub week_pages: i64,
    pub month_apps: i64,
    pub month_pages: i64,
    pub all_apps: i64,
    pub all_pages: i64,
    pub batch: i64,
    pub gap_min: i64,
    pub window_max: i64,
    #[serde(default = "default_shell_max")]
    pub max_batch: i64,
    #[serde(default = "default_shell_gap")]
    pub min_gap: i64,
    #[serde(default = "default_grants")]
    pub grants: i64,
}

fn default_shell_max() -> i64 {
    8
}

fn default_shell_gap() -> i64 {
    1
}

fn default_grants() -> i64 {
    60
}

fn default_refresh_batch() -> i64 {
    1
}

fn default_refresh_gap() -> i64 {
    120
}

fn default_on() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BeeLine {
    pub host: String,
    pub day: i64,
    pub week: i64,
    pub month: i64,
    pub all: i64,
    pub quiet: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ActivityPoint {
    pub label: String,
    pub new_pages: i64,
    pub new_hosts: i64,
    pub walked: i64,
    pub shell_pages: i64,
    pub cpu: i64,
    pub ram: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ActivityBook {
    pub minutes: Vec<ActivityPoint>,
    pub hours: Vec<ActivityPoint>,
    pub days: Vec<ActivityPoint>,
    pub weeks: Vec<ActivityPoint>,
    pub months: Vec<ActivityPoint>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperatorReport {
    pub pages: u64,
    pub queue: u64,
    pub db_bytes: u64,
    pub searches_per_minute: u64,
    pub avg_ms: u64,
    pub peers: Vec<PeerLine>,
    pub shells: ShellLine,
    pub bees: Vec<BeeLine>,
    pub refused: Vec<String>,
    pub rows: Vec<(String, i64, i64, i64, i64)>,
    pub note: String,
    pub csrf: String,
    pub base: String,
    #[serde(default)]
    pub sites: u64,
    #[serde(default)]
    pub new_today: i64,
    #[serde(default)]
    pub new_yesterday: i64,
    #[serde(default)]
    pub new_week: i64,
    #[serde(default)]
    pub new_month: i64,
    #[serde(default)]
    pub changed_today: i64,
    #[serde(default)]
    pub same_today: i64,
    #[serde(default)]
    pub hosts_today: i64,
    #[serde(default)]
    pub crawl_batch: i64,
    #[serde(default)]
    pub crawl_gap: i64,
    #[serde(default)]
    pub bee_batch: i64,
    #[serde(default)]
    pub bee_gap: i64,
    #[serde(default)]
    pub intake_days: Vec<(String, i64)>,
    #[serde(default)]
    pub cpu_hours: Vec<(String, i64)>,
    #[serde(default)]
    pub ram_hours: Vec<(String, i64)>,
    #[serde(default)]
    pub leased: u64,
    #[serde(default)]
    pub refresh_due: i64,
    #[serde(default = "default_refresh_batch")]
    pub refresh_batch: i64,
    #[serde(default = "default_refresh_gap")]
    pub refresh_gap: i64,
    #[serde(default)]
    pub span: String,
    #[serde(default)]
    pub activity: ActivityBook,
    /// Раздел «Всё о пчёлах» на этой ноде. Нет поля в старом ответе — раздел включён.
    #[serde(default = "default_on")]
    pub bees_page: bool,
}

fn chosen_span(span: &str) -> &'static str {
    match span {
        "hour" => "hour",
        "day" => "day",
        "week" => "week",
        "month" => "month",
        _ => "minute",
    }
}

fn activity_rows<'a>(report: &'a OperatorReport, span: &str) -> &'a [ActivityPoint] {
    match span {
        "hour" => &report.activity.hours,
        "day" => &report.activity.days,
        "week" => &report.activity.weeks,
        "month" => &report.activity.months,
        _ => &report.activity.minutes,
    }
}

fn short_bucket(span: &str, bucket: &str) -> String {
    match span {
        "minute" => bucket.rsplit(' ').next().unwrap_or(bucket).to_string(),
        "hour" => {
            let (date, hour) = bucket.split_once(' ').unwrap_or((bucket, ""));
            let mut parts = date.split('-');
            let _year = parts.next();
            match (parts.next(), parts.next()) {
                (Some(month), Some(date)) if !month.is_empty() && !date.is_empty() => format!("{date}.{month} {hour}"),
                _ => bucket.to_string(),
            }
        }
        "week" => {
            let week = bucket.split('-').nth(1).unwrap_or(bucket);
            format!("нед {week}")
        }
        "month" => {
            let mut parts = bucket.split('-');
            let year = parts.next().unwrap_or("");
            let month = parts.next().unwrap_or("");
            if month.is_empty() { bucket.to_string() } else { format!("{month}.{year}") }
        }
        _ => short_label(bucket),
    }
}

fn series_chart(name: &str, unit: &str, points: &[ActivityPoint], span: &str, color: &str, pick: fn(&ActivityPoint) -> i64) -> String {
    let drawn = points
        .iter()
        .map(|point| (short_bucket(span, &point.label), pick(point)))
        .collect::<Vec<_>>();
    let last = drawn.last().map(|(_, value)| *value).unwrap_or(0);
    let unit = if unit.is_empty() { String::new() } else { format!(" {unit}") };
    format!(
        "<p class=\"series\"><i style=\"background:{color}\"></i><span>{}</span><b>{}{unit}</b></p>{}",
        escape(name),
        grouped(last),
        time_chart(&drawn, color)
    )
}

fn grouped(value: i64) -> String {
    let negative = value < 0;
    let digits = value.unsigned_abs().to_string();
    let mut out = String::new();
    for (index, ch) in digits.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            out.push(' ');
        }
        out.push(ch);
    }
    let mut text: String = out.chars().rev().collect();
    if negative {
        text.insert(0, '−');
    }
    text
}

fn span_nav(base: &str, span: &str) -> String {
    let mut nav = String::from("<nav class=\"spans\">");
    for (key, title) in [
        ("minute", "Минуты"),
        ("hour", "Часы"),
        ("day", "Дни"),
        ("week", "Недели"),
        ("month", "Месяцы"),
    ] {
        let class = if key == span { " class=\"on\"" } else { "" };
        nav.push_str(&format!(
            "<a href=\"{}/operator?sheet=node&span={key}\"{class}>{title}</a>",
            escape(base)
        ));
    }
    nav.push_str("</nav>");
    nav
}

pub fn time_chart(points: &[(String, i64)], color: &str) -> String {
    if points.is_empty() || points.iter().all(|(_, value)| *value == 0) {
        return "<p>За этот срок чисел ещё нет.</p>".into();
    }
    let max = points.iter().map(|(_, value)| *value).max().unwrap_or(1).max(1);
    let left = 54.0;
    let right = 688.0;
    let top = 18.0;
    let bottom = 150.0;
    let width = right - left;
    let height = bottom - top;
    let step = if points.len() < 2 { 0.0 } else { width / (points.len() - 1) as f64 };
    let x_of = |index: usize| if points.len() < 2 { left + width / 2.0 } else { left + index as f64 * step };
    let y_of = |value: i64| bottom - (value as f64 / max as f64) * height;
    let mut grid = String::new();
    let mut axis = String::new();
    let mut seen = Vec::new();
    for mark in [max, max / 2, 0] {
        if seen.contains(&mark) {
            continue;
        }
        seen.push(mark);
        let y = y_of(mark);
        grid.push_str(&format!(
            "<line x1=\"{left:.1}\" y1=\"{y:.1}\" x2=\"{right:.1}\" y2=\"{y:.1}\" stroke=\"#e7e1d6\" stroke-width=\"1\"/>"
        ));
        axis.push_str(&format!(
            "<text x=\"46\" y=\"{:.1}\" text-anchor=\"end\" fill=\"#70757a\" font-size=\"11\">{}</text>",
            y + 4.0,
            grouped(mark)
        ));
    }
    let mut line = String::new();
    let mut area = format!("{:.1},{bottom:.1}", x_of(0));
    let mut marks = String::new();
    let mut labels = String::new();
    let value_every = if points.len() <= 8 { 1 } else { (points.len() / 6).max(1) };
    let axis_every = if points.len() <= 6 { 1 } else { (points.len() / 5).max(1) };
    let peak = points.iter().enumerate().max_by_key(|(_, (_, value))| *value).map(|(index, _)| index);
    for (index, (label, value)) in points.iter().enumerate() {
        let x = x_of(index);
        let y = y_of(*value);
        if index > 0 {
            line.push(' ');
        }
        line.push_str(&format!("{x:.1},{y:.1}"));
        area.push_str(&format!(" {x:.1},{y:.1}"));
        marks.push_str(&format!(
            "<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"3.5\" fill=\"{color}\"><title>{}: {}</title></circle>",
            escape(label),
            grouped(*value)
        ));
        let show_value = index % value_every == 0 || index + 1 == points.len() || peak == Some(index);
        if show_value {
            let text_y = if y < 28.0 { y + 14.0 } else { y - 8.0 };
            labels.push_str(&format!(
                "<text x=\"{x:.1}\" y=\"{text_y:.1}\" text-anchor=\"middle\" fill=\"#202124\" font-size=\"11\" font-weight=\"600\">{}</text>",
                grouped(*value)
            ));
        }
        if index % axis_every == 0 || index + 1 == points.len() {
            labels.push_str(&format!(
                "<text x=\"{x:.1}\" y=\"170\" text-anchor=\"middle\" fill=\"#70757a\" font-size=\"11\">{}</text>",
                escape(label)
            ));
        }
    }
    area.push_str(&format!(" {:.1},{bottom:.1}", x_of(points.len() - 1)));
    format!(
        "<svg class=\"chart\" viewBox=\"0 0 700 184\" role=\"img\">{grid}{axis}<polygon points=\"{area}\" fill=\"{color}\" fill-opacity=\"0.14\"/><polyline fill=\"none\" stroke=\"{color}\" stroke-width=\"2.5\" points=\"{line}\"/>{marks}{labels}</svg>"
    )
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct VisitNumbers {
    pub visits: i64,
    pub views: i64,
    pub searches: i64,
    pub bounces: i64,
    pub dwell_seconds: i64,
    pub timed_visits: i64,
    pub empty_searches: i64,
    pub search_errors: i64,
    pub refused: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeChip {
    pub base: String,
    pub alive: bool,
    pub latency_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapLine {
    pub country: String,
    pub region: String,
    pub door: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CabinetSheets {
    pub version: String,
    pub region_label: String,
    pub model: String,
    pub days: Vec<(String, i64, i64)>,
    pub visits_day: VisitNumbers,
    pub visits_week: VisitNumbers,
    pub visits_month: VisitNumbers,
    pub queries_day: Vec<(String, i64)>,
    pub queries_week: Vec<(String, i64)>,
    pub queries_month: Vec<(String, i64)>,
    pub index_urls: Vec<String>,
    #[serde(default)]
    pub kind_counts: Vec<(String, String, String, i64)>,
    #[serde(default)]
    pub muted: Vec<String>,
    pub bee_hosts: Vec<String>,
    pub nodes: Vec<NodeChip>,
    pub map_version: Option<u64>,
    pub map_places: Vec<MapLine>,
    pub csrf: String,
    pub base: String,
}

pub fn cabinet_sheets(view: &CabinetSheets, sheet: &str) -> (String, String) {
    let mut stats = String::from(
        "<section id=\"visits\"><h2>Заходы</h2><p>Это браузеры, не имена людей. Адрес не пишется и к запросу не привязывается. Время — от первого запроса визита до последнего; одна открытая страница времени не даёт. Отказ — ушёл, не поискав и не открыв вторую страницу.</p>",
    );
    stats.push_str("<h3>По дням</h3>");
    stats.push_str(&time_chart(
        &view.days.iter().map(|(day, visits, _)| (short_label(day), *visits)).collect::<Vec<_>>(),
        "#1a73e8",
    ));
    stats.push_str("<p class=\"legend\">Точки — заходы по дням.</p>");
    for (title, report) in [
        ("Сегодня", view.visits_day),
        ("За неделю", view.visits_week),
        ("За месяц", view.visits_month),
    ] {
        stats.push_str(&format!("<h3>{title}</h3>{}", visit_block(report)));
    }
    if sheet == "visits" {
        stats.push_str("</section>");
        return (stats, String::new());
    }
    stats.clear();
    if sheet != "queries" {
        stats.push_str("");
    } else {
    stats.push_str("<section id=\"queries\"><h2>Запросы</h2><p>Одинаковый текст склеен. Число — сколько раз его спросили за этот срок. Буквы ё и е не склеиваются.</p>");
    stats.push_str(&time_chart(
        &view.days.iter().map(|(day, _, searches)| (short_label(day), *searches)).collect::<Vec<_>>(),
        "#e6a817",
    ));
    stats.push_str("<p class=\"legend\">Точки — поиски по дням.</p>");
    stats.push_str("<h3>Сегодня</h3><ul class=\"check\">");
    for (query, count) in view.queries_day.iter().take(8) {
        stats.push_str(&format!("<li>{} — {}</li>", escape(query), count));
    }
    stats.push_str("</ul>");
    for (title, rows) in [("За неделю", &view.queries_week), ("За месяц", &view.queries_month)] {
        stats.push_str(&format!("<h3>{title}</h3><ul class=\"check\">"));
        for (query, count) in rows.iter().take(8) {
            stats.push_str(&format!("<li>{} — {}</li>", escape(query), count));
        }
        stats.push_str("</ul>");
    }
    stats.push_str("</section>");
    }
    let csrf = escape(&view.csrf);
    let base = escape(&view.base);
    let mut rest = format!(
        "<section id=\"index\"><h2>Индекс</h2><ul class=\"load\"><li>Версия: {}</li><li>Край: {}</li><li>Поколение модели: {}</li></ul>
<p>Ниже первые страницы, с которых индекс начал расти. Это стартовые цветки. Пчела читает такой сайт, находит на нём ссылки и несёт текст в улей. Новая ссылка встаёт в очередь обхода. Следующий заход берёт очередь, снова находит ссылки, и индекс растёт. Весь индекс в этот список не входит: число сайтов и страниц стоит на странице «Нода».</p>
<p>Поле добавляет свой сайт в этот старт. Улей ставит адрес в очередь и обходит его по обычным правилам: robots.txt, noindex и потолок страниц.</p>
<form class=\"inline\" method=\"post\" action=\"{base}/operator?sheet=index\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\"><input name=\"url\" placeholder=\"https://сайт/страница\" required><button name=\"action\" value=\"enqueue\">Добавить в индекс</button></form>
<h3>Первые страницы</h3><ul class=\"check\">",
        escape(&view.version),
        escape(&view.region_label),
        escape(&view.model)
    );
    for url in &view.index_urls {
        rest.push_str(&format!("<li>{}</li>", escape(url)));
    }
    rest.push_str("</ul><h3>Метки индекса</h3><p>Публичные метки видны человеку в полосе типа страницы и в шестерёнке тематик. Скрытые метки (<code>docs</code>, <code>goods</code>, <code>images</code>, <code>video</code>, <code>channel</code>) человеку не показываются и нужны для поиска и отбора через MCP (параметр <code>kind</code> в <code>search</code> и <code>index_pages</code>, сводка <code>kind_counts</code>).</p><table class=\"log\"><tr><th>Метка</th><th>Код для MCP</th><th>Где живёт</th><th>Страниц</th></tr>");
    for (code, label, place, count) in &view.kind_counts {
        rest.push_str(&format!(
            "<tr><td>{}</td><td><code>{}</code></td><td>{}</td><td>{count}</td></tr>",
            escape(label),
            escape(code),
            escape(place),
        ));
    }
    rest.push_str("</table><h3>Скрыть сайт из выдачи</h3>");
    rest.push_str("<p>Страницы сайта остаются в базе этой ноды. В поиске этой ноды сайт не показывается, пока оператор его не вернёт. Другой улей этот список не получает и решает сам. Удаления страниц здесь нет: обход иначе принёс бы их снова. Остановить пчелу, которая приносит страницы, — отдельная кнопка на странице «Пчёлы».</p>");
    rest.push_str(&format!(
        "<form class=\"inline\" method=\"post\" action=\"{base}/operator?sheet=index\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\"><input name=\"host\" placeholder=\"сайт или https://сайт\" required><button name=\"action\" value=\"mute\">Скрыть из выдачи</button><button name=\"action\" value=\"unmute\">Вернуть в выдачу</button></form>"
    ));
    if view.muted.is_empty() {
        rest.push_str("<p>Скрытых сайтов нет.</p>");
    } else {
        rest.push_str("<ul class=\"check\">");
        for host in &view.muted {
            rest.push_str(&format!("<li>{}</li>", escape(host)));
        }
        rest.push_str("</ul>");
    }
    rest.push_str("</section><section id=\"door\"><h2>Дверь</h2><h3>Связи</h3><div class=\"map\"><b>Этот улей</b>");
    for host in &view.bee_hosts {
        rest.push_str(&format!("<span>пчела {}</span>", escape(host)));
    }
    for node in &view.nodes {
        let mark = if node.alive { "жива" } else { "не отвечает" };
        rest.push_str(&format!("<span>{} · {mark}</span>", escape(&node.base)));
    }
    rest.push_str("</div>");
    rest.push_str(&format!(
        "<form class=\"inline\" method=\"post\" action=\"{base}/operator\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\"><input name=\"base\" placeholder=\"https://хост:порт\" required><button name=\"action\" value=\"handshake\">Рукопожатие</button></form>"
    ));
    if !view.nodes.is_empty() {
        rest.push_str("<ul class=\"check\">");
        for node in &view.nodes {
            let state_text = if node.alive {
                format!("жива, {} мс", node.latency_ms)
            } else {
                "не отвечает".into()
            };
            rest.push_str(&format!(
                "<li>{} — {state_text} <form class=\"inline\" method=\"post\" action=\"{base}/operator\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\"><input type=\"hidden\" name=\"base\" value=\"{}\"><button name=\"action\" value=\"forget_node\">Убрать</button></form></li>",
                escape(&node.base),
                escape(&node.base)
            ));
        }
        rest.push_str("</ul>");
    }
    rest.push_str("<h3>Карта сети</h3>");
    if let Some(version) = view.map_version {
        rest.push_str(&format!("<p>Версия карты {version}.</p><ul class=\"check\">"));
        for place in &view.map_places {
            rest.push_str(&format!(
                "<li>{} / {} — {}</li>",
                escape(&place.country),
                escape(&place.region),
                escape(&place.door)
            ));
        }
        rest.push_str("</ul>");
    } else {
        rest.push_str("<p>Подписанной карты нет, поэтому кнопка другого края не показывается.</p>");
    }
    rest.push_str(&format!(
        "<h3>MCP</h3><p>Адрес MCP: <code>{base}/mcp</code>. Снаружи это https://admin.beesearch.ru/mcp. Ключ отдельно от пароля кабинета.</p>
<form method=\"post\" action=\"{base}/operator\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\"><button name=\"action\" value=\"issue_mcp\">Выдать ключ</button><button name=\"action\" value=\"revoke_mcp\">Отозвать</button></form></section>"
    ));
    let door_at = rest.find("<section id=\"door\">").unwrap_or(rest.len());
    let index_html = rest[..door_at].to_string();
    let door_html = rest[door_at..].to_string();
    let rest = match sheet {
        "index" => index_html,
        "door" => door_html,
        _ => String::new(),
    };
    (stats, rest)
}

fn short_label(day: &str) -> String {
    let mut parts = day.split('-');
    let _year = parts.next();
    match (parts.next(), parts.next()) {
        (Some(month), Some(date)) if !month.is_empty() && !date.is_empty() => format!("{date}.{month}"),
        _ => day.to_string(),
    }
}

fn visit_block(report: VisitNumbers) -> String {
    let time = if report.timed_visits > 0 {
        format!("{} с", report.dwell_seconds / report.timed_visits)
    } else {
        "ещё не измерено".into()
    };
    let bounces = if report.visits > 0 {
        format!("{} ({}%)", report.bounces, report.bounces * 100 / report.visits)
    } else {
        "0".into()
    };
    format!(
        "<ul class=\"load\"><li>Заходы: {}</li><li>Просмотры: {}</li><li>Среднее время: {time}</li><li>Отказы: {bounces}</li><li>Поиски: {}</li><li>Пустая выдача: {}</li><li>Ошибки поиска: {}</li><li>Запросы, которые нода не ищет: {}</li></ul>",
        report.visits, report.views, report.searches, report.empty_searches, report.search_errors, report.refused
    )
}

pub fn page_operator(report: &OperatorReport, stats: &str, rest: &str, sheet: &str) -> String {
    let sheet = match sheet {
        "visits" | "queries" | "journal" | "bees" | "index" | "door" => sheet,
        _ => "node",
    };
    let pace = if report.searches_per_minute == 0 {
        "Поисков за последнюю минуту не было.".to_string()
    } else if report.avg_ms == 0 {
        format!(
            "Поисков за последнюю минуту: {}. Время ответа в этом запуске ещё не измерено.",
            report.searches_per_minute
        )
    } else {
        format!(
            "Поисков за последнюю минуту: {}. Среднее время ответа: {} мс.",
            report.searches_per_minute, report.avg_ms
        )
    };
    let span = chosen_span(&report.span);
    let base_nav = escape(&report.base);
    let spans = span_nav(&report.base, span);
    let pages_chart = series_chart("Новые страницы", "", activity_rows(report, span), span, "#1a73e8", |point| point.new_pages);
    let hosts_chart = series_chart("Новые сайты", "", activity_rows(report, span), span, "#e37400", |point| point.new_hosts);
    let walk_chart = series_chart("Обход", "", activity_rows(report, span), span, "#188038", |point| point.walked);
    let phone_chart = series_chart("Телефоны", "", activity_rows(report, span), span, "#c9a227", |point| point.shell_pages);
    let cpu_chart = series_chart("Процессор", "%", activity_rows(report, span), span, "#188038", |point| point.cpu);
    let ram_chart = series_chart("Память", "%", activity_rows(report, span), span, "#a142f4", |point| point.ram);
    let mut body = format!(
        "<div class=\"cabinet\"><nav class=\"side\"><a href=\"{base_nav}/operator?sheet=node&span={span}\">Нода</a><a href=\"{base_nav}/operator?sheet=visits\">Заходы</a><a href=\"{base_nav}/operator?sheet=queries\">Запросы</a><a href=\"{base_nav}/operator?sheet=journal\">Журнал</a><a href=\"{base_nav}/operator?sheet=bees\">Пчёлы</a><a href=\"{base_nav}/operator?sheet=index\">Индекс</a><a href=\"{base_nav}/operator?sheet=door\">Дверь</a></nav><div class=\"sheets\">"
    );
    if sheet == "node" {
    body.push_str(&format!(
        "<section id=\"node\"><h2>Нода <a class=\"refresh\" href=\"{base}/operator?sheet=node&span={span}\">Обновить</a></h2>
<p>Адреса людей сюда не пишутся. Новая страница — адрес, которого в индексе ещё не было. Повтор без изменений в это число не входит.</p>
<p>График читается один раз, когда страница открылась. Кнопка «Обновить» спрашивает улей ещё раз. Пока страница открыта, улей сам ничего не шлёт, поэтому от графика машина не грузится. Минуты копятся три часа, часы — две недели. Дни, недели и месяцы складываются из уже записанных суток.</p>
{spans}
<ul class=\"load\">
<li>Сайтов в поиске: {sites}</li>
<li>Страниц в индексе: {pages}</li>
<li>Новых страниц сегодня: {new_today}, вчера: {new_yesterday}</li>
<li>Новых за неделю: {new_week}, за месяц: {new_month}</li>
<li>Новых сайтов сегодня: {hosts_today}</li>
<li>Сегодня изменённых: {changed_today}, без изменений: {same_today}</li>
<li>Очередь обхода: {queue}</li>
<li>Сейчас закреплено адресов: {leased}</li>
<li>Переобход уже пора: {refresh_due}</li>
<li>База: {db} МБ</li>
<li>{pace}</li>
</ul>
<h3>Новые страницы</h3>
{pages_chart}
<p class=\"legend\">Адреса, которых в индексе ещё не было.</p>
<h3>Новые сайты</h3>
{hosts_chart}
<p class=\"legend\">Хосты, которые появились впервые.</p>
<h3>Обход</h3>
{walk_chart}
<p class=\"legend\">Новые, изменённые и страницы без изменений. Линия идёт, даже когда новых адресов мало.</p>
<h3>Процессор</h3>
{cpu_chart}
<p class=\"legend\">Доля процессора машины, проценты. Если линия упирается в верх, скорость лучше снизить.</p>
<h3>Память</h3>
{ram_chart}
<p class=\"legend\">Доля памяти машины, проценты.</p>
<form class=\"inline\" method=\"post\" action=\"{base}/operator?sheet=node\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">
<label>Страниц за заход улья <input name=\"batch\" type=\"number\" min=\"1\" max=\"24\" value=\"{crawl_batch}\"></label>
<label>Пауза улья, секунд <input name=\"gap\" type=\"number\" min=\"2\" max=\"300\" value=\"{crawl_gap}\"></label>
<button name=\"action\" value=\"crawl_pace\">Скорость улья</button></form>
<p>Сейчас улей берёт {crawl_batch} адресов и ждёт {crawl_gap} секунд. Это и есть скорость добавления. Поднять числа — больше страниц в сутки, пока график нагрузки не упрётся вверх. Снизить — если машина занята.</p>
<h3>Раздел «Всё о пчёлах»</h3>
<p>{bees_line}</p>
<form class=\"inline\" method=\"post\" action=\"{base}/operator?sheet=node\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\"><input type=\"hidden\" name=\"on\" value=\"{bees_next}\"><button name=\"action\" value=\"bees_page\">{bees_button}</button></form>
<form class=\"inline\" method=\"post\" action=\"{base}/operator?sheet=node\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">
<label>Страниц переобхода за заход <input name=\"batch\" type=\"number\" min=\"1\" max=\"4\" value=\"{refresh_batch}\"></label>
<label>Пауза переобхода, секунд <input name=\"gap\" type=\"number\" min=\"60\" max=\"3600\" value=\"{refresh_gap}\"></label>
<button name=\"action\" value=\"refresh_pace\">Скорость переобхода</button></form>
<p>Переобход не встаёт в очередь новых адресов. Сначала улей спрашивает метку страницы. Если она не менялась, тело не качается. Обычный сайт без своей пчелы проверяется этим темпом. Сайт, на котором стоит пчела, сюда не входит.</p>
<h3>Телефоны</h3>
<p>Разные приложения и страницы, которые улей записал как новые или изменённые. Повтор той же страницы не считается. Идентификатор приложения к запросу не привязывается. Каждый телефон получает свой темп, а не одну общую очередь на всех.</p>
{phone_chart}
<p class=\"legend\">Страницы, которые телефоны принесли за выбранный срок.</p>
<ul class=\"load\">
<li>Сегодня: {day_apps} приложений, {day_pages} страниц</li>
<li>За неделю: {week_apps} приложений, {week_pages} страниц</li>
<li>За месяц: {month_apps} приложений, {month_pages} страниц</li>
<li>За всё время: {all_apps} приложений, {all_pages} страниц</li>
</ul>
<form class=\"inline\" method=\"post\" action=\"{base}/operator?sheet=node\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">
<label>Страниц за заход <input name=\"batch\" type=\"number\" min=\"2\" max=\"8\" value=\"{batch}\"></label>
<label>Пауза, минут <input name=\"gap\" type=\"number\" min=\"2\" max=\"10\" value=\"{gap}\"></label>
<label>Заходов телефона за 10 минут <input name=\"window\" type=\"number\" min=\"4\" max=\"16\" value=\"{window}\"></label>
<label>Потолок страниц <input name=\"max_batch\" type=\"number\" min=\"2\" max=\"30\" value=\"{max_batch}\"></label>
<label>Минимальная пауза, минут <input name=\"min_gap\" type=\"number\" min=\"1\" max=\"60\" value=\"{min_gap}\"></label>
<label>Выдач всем помощникам за 10 минут <input name=\"grants\" type=\"number\" min=\"4\" max=\"600\" value=\"{grants}\"></label>
<button name=\"action\" value=\"shell_pace\">Сохранить</button></form>
<p>Обычный режим телефона берёт первые три числа: две страницы и десять минут. Приложение 0.1.9 слушает паузу улья от одной минуты. Общий потолок выдач держит эту машину: смысл считает она одна, и писатель базы тоже один. Поднимать его стоит, пока график процессора не упирается вверх.</p>",
        sites = report.sites,
        pages = report.pages,
        new_today = report.new_today,
        new_yesterday = report.new_yesterday,
        new_week = report.new_week,
        new_month = report.new_month,
        hosts_today = report.hosts_today,
        changed_today = report.changed_today,
        same_today = report.same_today,
        queue = report.queue,
        db = report.db_bytes / 1_048_576,
        span = span,
        spans = spans,
        pages_chart = pages_chart,
        hosts_chart = hosts_chart,
        walk_chart = walk_chart,
        phone_chart = phone_chart,
        cpu_chart = cpu_chart,
        ram_chart = ram_chart,
        crawl_batch = report.crawl_batch,
        crawl_gap = report.crawl_gap,
        leased = report.leased,
        refresh_due = report.refresh_due,
        refresh_batch = report.refresh_batch,
        refresh_gap = report.refresh_gap,
        day_apps = report.shells.day_apps,
        day_pages = report.shells.day_pages,
        week_apps = report.shells.week_apps,
        week_pages = report.shells.week_pages,
        month_apps = report.shells.month_apps,
        month_pages = report.shells.month_pages,
        all_apps = report.shells.all_apps,
        all_pages = report.shells.all_pages,
        batch = report.shells.batch,
        gap = report.shells.gap_min,
        window = report.shells.window_max,
        max_batch = report.shells.max_batch,
        min_gap = report.shells.min_gap,
        grants = report.shells.grants,
        bees_line = if report.bees_page {
            "Раздел включён: на поиске есть ссылка «Всё о пчёлах», адрес /bees открывает справку. Пока человек не нажал «Найти», эта страница модель не будит. Другая нода может выключить раздел у себя."
        } else {
            "Раздел выключен. Ссылки нет, адрес /bees отвечает «не найдено». Включить можно здесь, это только флаг ноды."
        },
        bees_next = if report.bees_page { "0" } else { "1" },
        bees_button = if report.bees_page { "Выключить раздел" } else { "Включить раздел" },
        base = escape(&report.base),
        csrf = escape(&report.csrf),
    ));
    if !report.note.is_empty() {
        body.push_str(&format!("<p class=\"hint\">{}</p>", escape(&report.note)));
    }
    if report.peers.is_empty() {
        body.push_str("<p>Соседей в этом запуске нет. Нода отвечает своим индексом.</p>");
    } else {
        body.push_str("<table class=\"log\"><tr><th>Пчела</th><th>Ответ, мс</th><th>Жива</th><th>Бакеты</th><th>Страницы</th></tr>");
        for peer in &report.peers {
            let alive = if peer.alive { "да" } else { "нет" };
            let silent = peer.latency_ms == 0 && peer.buckets == 0 && peer.pages == 0;
            let latency = if silent { "—".to_string() } else { peer.latency_ms.to_string() };
            let buckets = if peer.buckets >= 65_536 {
                "весь индекс".to_string()
            } else if silent {
                "—".to_string()
            } else {
                peer.buckets.to_string()
            };
            let pages = if silent { "—".to_string() } else { peer.pages.to_string() };
            body.push_str(&format!(
                "<tr><td>{}</td><td>{latency}</td><td>{alive}</td><td>{buckets}</td><td>{pages}</td></tr>",
                escape(&peer.name)
            ));
        }
        body.push_str("</table>");
        body.push_str("<p class=\"legend\">«Жива» значит, что сосед ответил в этом запуске. Прочерк в ответе, бакетах и страницах значит, что отчёта об индексе ещё не было. Это не пустой поиск и не остановка обхода.</p>");
    }
    body.push_str("</section>");
    }
    body.push_str(stats);
    if sheet == "journal" {
    body.push_str("<section id=\"journal\"><h2>Журнал</h2><p>Каждая строка — один поиск, не сводка за день. «Найдено» — сколько обычных ссылок открылось. «По словам» и «По смыслу» — сколько из них нашлось точным текстом и близким смыслом. «18+» — сколько ссылок ушло в закрытый блок. Ноль значит, что таких ссылок в том поиске не было. Сколько раз спросили одно и то же, видно в разделе «Запросы».</p>");
    if report.rows.is_empty() {
        body.push_str("<p>Запросов пока нет.</p>");
    } else {
        body.push_str("<table class=\"log\"><tr><th>Запрос</th><th>Найдено</th><th>По словам</th><th>По смыслу</th><th>18+</th></tr>");
        for (text, results, words, meaning, adult) in &report.rows {
            body.push_str(&format!(
                "<tr><td>{}</td><td>{results}</td><td>{words}</td><td>{meaning}</td><td>{adult}</td></tr>",
                escape(text)
            ));
        }
        body.push_str("</table>");
    }
    body.push_str("</section>");
    }
    if sheet == "bees" {
    body.push_str("<section id=\"bees\"><h2>Пчёлы</h2>");
    body.push_str("<p>Столбцы — страницы, которые улей записал как новые или изменённые. Та же страница без изменений больше не считается. Свой сайт идёт первым: новые и изменённые страницы забираются сразу. Когда своих изменений нет, пчела берёт чужие адреса из очереди улья.</p>");
    body.push_str("<p>Компьютер без своего сайта запускается так: <code>mds node --lite --hive https://адрес-улья</code>. Модель и база ему не нужны. Он разбирает чужую очередь темпом ниже. На сайте пчела по-прежнему доказывает себя файлом <code>/.well-known/pchela.txt</code>.</p>");
    body.push_str(&format!(
        "<form class=\"inline\" method=\"post\" action=\"{}/operator?sheet=bees\"><input type=\"hidden\" name=\"csrf\" value=\"{}\"><label>Чужих адресов за заход <input name=\"batch\" type=\"number\" min=\"1\" max=\"24\" value=\"{}\"></label><label>Пауза, секунд <input name=\"gap\" type=\"number\" min=\"60\" max=\"3600\" value=\"{}\"></label><button name=\"action\" value=\"bee_pace\">Скорость пчёл</button></form>",
        escape(&report.base),
        escape(&report.csrf),
        if report.bee_batch == 0 { 4 } else { report.bee_batch },
        if report.bee_gap == 0 { 600 } else { report.bee_gap },
    ));
    if report.bees.is_empty() {
        body.push_str("<p>Подтверждённых сайтов нет.</p>");
    } else {
        body.push_str("<table class=\"log bees\"><tr><th>Сайт</th><th>День</th><th>Неделя</th><th>Месяц</th><th>Всего</th></tr>");
        for bee in &report.bees {
            let mark = if bee.quiet { " <span class=\"quiet\">молчит</span>" } else { "" };
            body.push_str(&format!(
                "<tr><td>{}{mark}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape(&bee.host), bee.day, bee.week, bee.month, bee.all
            ));
        }
        body.push_str("</table>");
    }
    if !report.refused.is_empty() {
        body.push_str("<p>Остановлены:</p><ul class=\"check\">");
        for host in &report.refused {
            body.push_str(&format!("<li class=\"fix\">{}</li>", escape(host)));
        }
        body.push_str("</ul>");
    }
    body.push_str(
        &format!(
            "<form class=\"admit\" method=\"post\" action=\"{}/operator\"><input type=\"hidden\" name=\"csrf\" value=\"{}\"><input name=\"host\" placeholder=\"сайт пчелы\" required><button name=\"action\" value=\"refuse\">Остановить</button><button name=\"action\" value=\"restore\">Вернуть</button></form>",
            escape(&report.base),
            escape(&report.csrf)
        ),
    );
    body.push_str("</section>");
    }
    if sheet != "node" && !report.note.is_empty() {
        body.push_str(&format!("<p class=\"hint\">{}</p>", escape(&report.note)));
    }
    body.push_str(rest);
    body.push_str("</div></div>");
    document("", &body, false, false, None, "", "", None, None, None, false).replacen("<body>", "<body class=\"cabinet\">", 1)
}

pub fn page_how() -> String {
    article(
        "Как устроен поиск",
        r#"<p>Человек открывает страницу ноды, вводит запрос и получает ссылки. Страница сама ничего не хранит: текст уходит в ноду, которая стоит за этим адресом.</p>
<h3>Откуда берутся страницы</h3>
<p>Нода обходит публичные адреса. Она читает robots.txt, строки Sitemap и Crawl-delay, карточку /llms.txt, а если её нет — /llms.md, и обычные страницы .md. Со страницы забирается видимый текст, JSON-LD, микроразметка, Open Graph, Twitter Card, Dublin Core, description, keywords и canonical.</p>
<p>Если на странице написан край или город, нода помечает регион. Кнопка настроек у строки поиска открывает плашку с текущим краем. Сначала показываются страницы этого края, ниже — остальные. Адрес человека не сохраняется.</p>
<p>Страница с noindex или заголовком X-Robots-Tag: noindex в индекс не попадает. Ссылка с nofollow в очередь не ставится. Файлы .mdx нода не читает.</p>
<h3>Как выбирается выдача</h3>
<p>Один и тот же запрос ищется двумя способами. Точные слова ищутся по заголовку и тексту. Близкий смысл ищется по вектору страницы. Списки склеиваются, и в выдаче остаётся ссылка, короткий отрывок и отметка «слова», «смысл» или обе сразу.</p>
<p>Буквы «е» и «ё» считаются разными: «мёд» и «мед» — разные запросы. Если во фразе есть намерение купить, известный хост маркетплейса получает штраф, прямой сайт остаётся выше.</p>
<h3>Пчела, рой, улей, пасека</h3>
<p>Одна нода — это пчела. Несколько нод, соединённых в одну сеть, — рой. Улей — региональный вход, домен, с которого люди ищут. Этот улей открыт на <a href="https://beesearch.ru/">https://beesearch.ru/</a>, кабинет оператора — на <a href="https://admin.beesearch.ru/login">https://admin.beesearch.ru/login</a>. Пасека — страна, сеть таких ульев. Робот при обходе по-прежнему называется MDS-Bot.</p>
<h3>Несколько нод</h3>
<p>Сервер, компьютер и телефон говорят одним протоколом. Главной ноды, которой подчиняются остальные, нет. Сервер и компьютер держат крупный кусок индекса и отдают сайт со строкой поиска. Телефон открывает поиск улья и раз в десять минут читает несколько страниц из очереди, в том числе когда приложение свёрнуто. Чужой трафик через него не идёт.</p>
<p>Запрос идёт к тем пчёлам, которые держат нужный шард, и не больше чем к трём копиям. Если одна копия отвечает заметно медленнее остальных, улей в этот раз её не ждёт. Пока сосед ещё не сообщил свои бакеты, улей спрашивает его как запасной.</p>
<p>Пчела за домашним роутером не открывает порт наружу. Лёгкая пчела сама звонит улью: <code>mds node --lite --site https://ваш-сайт --hive https://beesearch.ru</code>. На сайте лежит файл <code>/.well-known/pchela.txt</code>, в нём одна строка — имя, которое пчела печатает при запуске. Улей принимает новые и изменённые страницы этого сайта. Когда своих изменений нет, пчела берёт чужие адреса из очереди. Компьютер без своего сайта запускается без <code>--site</code>, команда на странице <a href="/computer">Компьютер</a>. Общий пароль для входа не нужен. Улей открыт на https://beesearch.ru. Домашней пчеле хватает обычного интернета.</p>
<p>Страницы с нарушением закона в выдачу не попадают. Материалы 18+ стоят отдельным блоком и замкнуты, пока человек не подтвердит возраст. Журнал запросов хранит только текст и числа на этой ноде, без адреса человека, и открывается только с компьютера, где нода запущена.</p>
<h3>Сколько страниц берётся с одного сайта</h3>
<p>Обычный сайт обходится до 100 страниц: этого хватает, чтобы его найти, и один каталог не заполняет поиск. Если страниц больше, владелец перечисляет до ста своих ссылок в <a href="/llms">/llms.txt</a>. Индексируются они. Сайт, на сервере которого стоит лёгкая пчела, отдаёт все свои страницы. Подробности и ответы, зачем ставить ноду, — на странице <a href="/indexing">Индексация</a>.</p>
<p><a href="/download">Скачать ноду и подключить её</a></p>"#,
    )
}

pub struct AddLine {
    pub ok: bool,
    pub text: String,
}

pub fn page_add(value: &str, lines: Option<&[AddLine]>, queued: bool) -> String {
    let verdict = if lines.is_none() {
        String::new()
    } else if queued {
        "<p>Сайт поставлен в очередь. Обход возьмёт его одним из ближайших.</p>".to_string()
    } else {
        "<p class=\"error\">В очередь сайт пока не поставлен. Исправьте пункты ниже и отправьте адрес ещё раз.</p>".to_string()
    };
    let report = match lines {
        Some(lines) if !lines.is_empty() => {
            let mut list = String::from("<ul class=\"check\">");
            for line in lines {
                let class = if line.ok { "ok" } else { "fix" };
                list.push_str(&format!("<li class=\"{class}\">{}</li>", escape(&line.text)));
            }
            list.push_str("</ul>");
            list
        }
        _ => String::new(),
    };
    article(
        "Добавить сайт",
        &format!(
            r#"{verdict}{report}<p>Напишите адрес. Нода проверит, что страницу можно читать, и поставит подходящий сайт в очередь обхода.</p>
<form class="admit" method="post" action="/add">
<input type="url" name="url" required placeholder="https://сайт.ru" value="{value}">
<button class="go" type="submit">Проверить и добавить</button>
</form>
<h3>Что делает индекс качественнее</h3>
<ul>
<li>Нужные страницы открыты без пароля и без noindex.</li>
<li>В корне лежит robots.txt: он разрешает MDS-Bot и содержит строку Sitemap.</li>
<li>Карточка <a href="/llms">/llms.txt</a> коротко говорит, как сайт называется и какие страницы важные. Пишите её словами, которыми сайт ищут.</li>
<li>У страницы свой текст. Баннер cookie и служебные пометки в выдачу не попадают, но и не заменяют описание.</li>
<li>Если сайт про край или город, назовите это место на странице: тогда она встанет в полосу этого региона.</li>
<li>Своя лёгкая пчела снимает потолок в сто страниц, даёт значок «держит пчелу» и небольшой подъём среди близких ссылок. Как поставить — на странице <a href="/indexing">Индексация</a>.</li>
</ul>"#,
            verdict = verdict,
            report = report,
            value = escape(value),
        ),
    )
}

pub fn page_for_sites() -> String {
    article(
        "Что сделать на сайте",
        r#"<p>Нода видит то, что сервер отдаёт без пароля. Страница, закрытая от робота или помеченная noindex, в выдаче не появится.</p>
<ul>
<li>Оставьте нужные страницы публичными и не ставьте на них noindex.</li>
<li>В robots.txt разрешите обход MDS-Bot и укажите Sitemap.</li>
<li>Положите в корень карточку /llms.txt. Если этот адрес отвечает ошибкой, нода запросит /llms.md. Правила и образец — на странице <a href="/llms">llms.md</a>.</li>
<li>Пишите карточку на том языке, на котором люди ищут. Карточка со словом honey не отвечает на запрос «мёд».</li>
<li>Важную страницу можно продублировать файлом .md рядом с обычным адресом. Такие файлы тоже индексируются.</li>
</ul>
<p>Сайт на React или Vite, который без JavaScript показывает только заставку, находится по текстам из страницы и из своего скрипта. Готовый HTML с заголовком и описанием нода читает сразу.</p>
<h3>Своя пчела на сервере сайта</h3>
<p>Полный индекс рядом с сайтом не ставится: модель занимает около 1 ГБ, а общий индекс — десятки гигабайт. Серверу на 2 ядра, 3–4 ГБ памяти и 20 ГБ диска хватает лёгкой пчелы. Она не открывает поиск для чужих людей и не считает векторы. Человек остаётся на улье.</p>
<p>Сначала пчела отдаёт свои новые и изменённые страницы, сразу и по обычным правилам обхода. Страница, в которой ничего не изменилось, по кругу не гоняется. Когда своих изменений нет, пчела берёт чужие адреса из очереди улья и помогает сети. Сколько чужих адресов за заход и какая пауза — решает оператор улья в кабинете. Пауза между загрузками страниц своего сайта не меньше двух секунд.</p>
<pre>mds node --lite --site https://ваш-сайт --hive https://beesearch.ru</pre>
<p>Имя из журнала пчелы кладётся одной строкой в <code>https://ваш-сайт/.well-known/pchela.txt</code>.</p>
<h3>Что это даёт сайту</h3>
<ul>
<li>Свои новые и изменённые страницы попадают в индекс сразу. Потолок в сто страниц для такой пчелы снимается: улей принимает страницы, которые она отдаёт.</li>
<li>В выдаче появляется значок «держит пчелу» и небольшой подъём среди близких результатов. Страница с более точным совпадением слов остаётся выше.</li>
<li>Когда на сайте тихо, пчела разбирает чужую очередь и ускоряет общий индекс.</li>
</ul>
<p>Новую сборку кладёт администратор этого сайта и перезапускает службу. Лёгкая пчела файл сама не меняет, и кабинет улья на чужой сервер обновление не присылает. Чем пчела отличается от полной ноды — на странице <a href="/indexing">Индексация</a>.</p>"#,
    )
}

pub fn page_llms() -> String {
    article(
        "Карточка llms.txt и llms.md",
        r#"<p>Карточка — короткий Markdown в корне сайта. По ней нода понимает, как сайт называется, о чём он и какие страницы обойти первыми. Текст карточки добавляется к остальным страницам этого же сайта, а сам файл тоже попадает в индекс.</p>
<h3>Куда положить файл</h3>
<ul>
<li>Основной адрес — <code>https://ваш-сайт/llms.txt</code>.</li>
<li>Если сервер отвечает на него ошибкой, нода запрашивает <code>https://ваш-сайт/llms.md</code>.</li>
<li>Достаточно одного файла. Оба читаются одинаково.</li>
<li>Это обычный Markdown. HTML-страница и файл .mdx карточкой не считаются.</li>
</ul>
<h3>Как писать</h3>
<ul>
<li>Первая строка <code># Название</code> становится названием.</li>
<li>Строки, которые начинаются с <code>&gt;</code>, становятся коротким описанием. Название и описание, вместе до 400 символов, дописываются к страницам сайта.</li>
<li>Важные адреса записывайте ссылками Markdown: <code>[Магазин](https://ваш-сайт/shop)</code>. Если страниц больше ста, нода индексирует эти ссылки своего сайта и не берёт больше ста. Остальные адреса с сайта в очередь не ставятся. Своя пчела на сервере по-прежнему отдаёт все страницы.</li>
<li>Ссылки на .mdx пропускаются. Текст внутри блока кода в индекс не берётся.</li>
<li>Описание пишите словами, которыми сайт ищут. Английское honey и русское «мёд» — разные слова.</li>
</ul>
<p>Карточка этой ноды лежит по адресам <a href="/llms.txt">/llms.txt</a> и <a href="/llms.md">/llms.md</a>. Текст у них один.</p>
<h3>Образец</h3>
<pre># КФХ ТехноВелес

&gt; Фермерское хозяйство в Лотошино: натуральный мёд с пасеки, агротуризм и отдых на ферме.

- [Главная](https://tehnoveles.ru/)
- [Магазин](https://tehnoveles.ru/shop)</pre>
<p>Рядом с важной HTML-страницей можно положить отдельный файл .md с тем же рассказом. Нода прочитает и его. Это не замена карточки в корне.</p>"#,
    )
}

pub fn page_indexing() -> String {
    article(
        "Индексация",
        r#"<p>Пчелопоиск находит сайт по его страницам. Сколько страниц берётся и зачем рядом с сайтом стоит своя нода — здесь.</p>
<h3>Сколько страниц с одного сайта</h3>
<p>Обычный сайт обходится до ста страниц. Этого хватает, чтобы сайт находился, и длинный каталог не занимает весь поиск. Страницы, которые уже лежат в индексе, из-за этого потолка не удаляются.</p>
<p>Если страниц больше ста, владелец сам выбирает, какие важнее. В корне сайта, в <a href="/llms">/llms.txt</a> или <a href="/llms">/llms.md</a>, он пишет ссылки своего сайта, не больше ста. Индексируются они. Ссылки на чужие сайты из этой карточки в список не входят. Остальные адреса этого сайта в очередь не ставятся.</p>
<p>Сайт завода и мастерской остаётся со своими изделиями: у производства короткий свой список. Интернет-магазин в индекс целиком не копируется. В выдаче виден сам магазин, а несколько подходящих предложений браузер может подгрузить с сайта магазина в момент запроса, если магазин это разрешает. Сто магазинов с одним и тем же товаром поиск не собирает.</p>
<h3>Зачем нода рядом с сайтом</h3>
<p>Лёгкая пчела стоит на том же сервере, что и сайт. Она не открывает чужой поиск и не держит модель: модели нужен примерно 1 ГБ, а общий индекс — десятки гигабайт. Сначала она отдаёт свои новые и изменённые страницы. Страница без изменений по кругу не гоняется. Когда своих изменений нет, она помогает обойти чужую очередь улья. Темп этой очереди задаёт кабинет улья. Человек ищет на улье.</p>
<p>Польза для сайта: потолок в сто страниц снимается, улей принимает страницы, которые отдаёт пчела. В выдаче появляется значок «держит пчелу» и небольшой подъём среди близких ссылок. Более точное совпадение слов остаётся выше. Как поставить:</p>
<pre>mds node --lite --site https://ваш-сайт --hive https://beesearch.ru</pre>
<p>Имя из журнала пчелы кладётся одной строкой в <code>/.well-known/pchela.txt</code> на этом сайте. Новую сборку лёгкой пчелы на сервер кладут вручную и перезапускают службу: сама она файл не меняет.</p>
<h3>Зачем полная нода</h3>
<p>Полная нода — свой улей. Она держит кусок индекса, считает смысл запроса и открывает страницу поиска для людей своего края или своей компании. Журнал запросов остаётся на этой ноде и не уезжает в общий центр. Нужны память под модель и диск под страницы. Без соседней ноды она отвечает только своим индексом. С соседями запрос идёт к тем, кто держит нужный шард, и не больше чем к трём копиям.</p>
<p>Её ставят, когда нужен свой адрес поиска в той же сети. Лёгкая пчела для этого не подходит: она поиск для людей не открывает. Файл и подключение — на странице <a href="/download">Скачать ноду</a>.</p>
<h3>Зачем приложение на телефоне</h3>
<p>Приложение открывает поиск улья. Сайт из выдачи открывается в том же окне, кнопка «К поиску» возвращает к строке. Раз в десять минут, и по мобильной сети, и когда приложение свёрнуто, оно читает до двух страниц из очереди и отдаёт текст улью. Это не шард и не кабинет. Подробно — на странице <a href="/android">Приложение для Android</a>.</p>
<h3>Если сломается Яндекс или Гугл</h3>
<p>Их выдача стоит на их дата-центрах. Сбой там гасит поиск сразу для всех, кто им пользуется. У Пчелопоиска нет одной компании и одного центра. Запрос принимает любая живая нода. Кусок индекса лежит в трёх копиях. Пока нод мало, каждая держит весь индекс, который успела собрать. Падение одного улья не гасит остальные. Свой адрес может открыть регион или компания, и он остаётся в той же сети.</p>
<p><a href="/for-sites">Что сделать на сайте</a></p>"#,
    )
}

pub fn page_manifesto() -> String {
    article(
        "Манифест",
        r#"<p>Пчелопоиск — волонтёрский поиск. Это не коммерческий проект: результаты не продаются, рекламодателя в выдаче нет, и за участие в сети плату не берут. Одна нода в этой сети называется пчела.</p>
<h3>Что происходит с обычным поиском</h3>
<p>Поисковый ответ, который собирает ИИ, всё чаще остаётся на странице поиска. Человек получает пересказ и не переходит на сайт. У небольшой компании, фермы или мастерской от такого индекса мало пользы: страницу прочитали, а визита нет.</p>
<p>Маркетплейс закрывает ту же выдачу миллионами карточек. Прямой сайт тонет под чужими товарами.</p>
<ul>
<li>Крупный поиск стоит на чужих дата-центрах. Сбой там гасит выдачу сразу для всех.</li>
<li>Ответ языковой модели берёт примерно первые десять страниц чужой выдачи. Всё, что ниже этого круга, в ответ не входит.</li>
<li>В индексе много мошеннических и спам-сайтов. Сайт действующей компании теряется среди них.</li>
<li>Инфосайты перепродают переход и завалены сгенерированным текстом. Человек ищет компанию, а открывает пересказ.</li>
</ul>
<h3>Что делает Пчелопоиск</h3>
<p>Пчелопоиск задуман как чистый поиск реальных сайтов компаний: без рекламы и без навязанного мусора. В выдаче остаётся ссылка на страницу, а не чужая карточка и не сгенерированный пересказ. Если в запросе есть намерение купить, известный маркетплейс получает штраф, прямой сайт остаётся выше.</p>
<p>Это попытка собрать другой поиск. Ноду ставит тот, кто хочет. Несколько таких нод держат общий индекс, и компании-владельца у сети нет. Регион может открыть свой адрес и остаться в той же сети.</p>
<h3>Если крупный поиск встанет</h3>
<p>Яндекс и Гугл отвечают со своих дата-центров. Сбой там гасит выдачу сразу для всех, кто ими пользуется. Здесь индекс разобран по отдельным нодам. Если один улей встанет, остальные продолжают отвечать своим куском. Зачем ставить ноду рядом с сайтом, полную ноду или приложение на телефоне — на странице <a href="/indexing">Индексация</a>.</p>
<p><a href="/download">Поставить свою ноду</a></p>"#,
    )
}

pub fn page_download(release_version: Option<&str>) -> String {
    let file = match release_version {
        Some(version) => format!(
            "<p><a class=\"button\" href=\"/download/pchela.zip\">Скачать Пчелопоиск для Windows, версия {version}</a></p>
<p><a href=\"/download/mds.exe\">Только программа, без библиотеки модели</a></p>
<p>Архив — подписанная программа и urls.txt. Если на этой ноде лежит onnxruntime.dll, библиотека модели тоже попадает в архив.</p>",
        ),
        None => "<p>Подписанный файл для Windows на этой ноде ещё не выложен. Когда оператор опубликует выпуск, на этой странице появится кнопка «Скачать ноду для Windows».</p>"
            .to_string(),
    };
    article(
        "Скачать ноду",
        &format!(
            r#"{file}
<h3>Скачать готовую программу</h3>
<p>Одна программа. Мини-нода рядом с сайтом, полная нода и дополнительная нода на свободный компьютер — это один и тот же файл, отличаются команды. В Windows файл называется <code>mds.exe</code>, в Linux — <code>mds</code>. Полная нода забирает модель с улья при первом запуске, около 1 ГБ. Ссылки на файл весов на этой странице нет. Мини-нода модель не качает.</p>
<ul>
<li><a href="/download/pchela-windows-x64.zip">Windows, 64-бит, с этого улья</a></li>
<li><a href="/download/pchela-linux-x64.tar.gz">Linux x86_64 с этого улья: Ubuntu 24.04 и другие Linux с glibc 2.39</a></li>
<li><a href="{RELEASE_WINDOWS}">Windows, 64-бит, копия на GitHub</a></li>
<li><a href="{RELEASE_LINUX}">Linux x86_64, копия на GitHub</a></li>
<li><a href="/android">Приложение для Android</a></li>
</ul>
<p>Сайт этого улья — <a href="https://beesearch.ru/">https://beesearch.ru/</a>. Кабинет оператора — <a href="https://admin.beesearch.ru/login">https://admin.beesearch.ru/login</a>. В выложенную программу вписан адрес <a href="https://45.158.46.52/">https://45.158.46.52/</a>: он открывает ту же машину. Если адрес соседа не указан, нода подключается к ней сама. Улей запускают с <code>--alone</code>, чтобы он не звонил сам себе.</p>
<p>Полная нода подхватывает новую версию сама, если в сети лежит подписанный файл новее того, что у неё запущен. На странице поиска появляется строка «Нужно обновиться». Архив со страницы загрузки сам программу не подменяет. Лёгкая пчела свой файл не меняет: новую сборку кладут на сервер сайта вручную и перезапускают службу. Телефон публичным входом не является. Приложение и то, как оно помогает индексу, — на странице <a href="/android">Приложение для Android</a>.</p>
<p>Исходный код: <a href="{SOURCE_REPO}">{SOURCE_REPO}</a>.</p>
<h3>Как подключить её к поиску</h3>
<p>После запуска нода открывает свой сайт со строкой поиска. Чтобы она вошла в общую сеть, при старте укажите хотя бы один адрес соседней ноды. Без соседа нода отвечает только своим индексом.</p>
<pre>mds node --listen 0.0.0.0:8080 --seeds urls.txt --follow-external --peers https://beesearch.ru</pre>
<ul>
<li>В urls.txt — стартовые адреса сайтов, по одному в строке.</li>
<li>Этот улей для людей открыт на https://beesearch.ru.</li>
<li>Пчела дома или на старом компьютере сидит за динамическим IP. Ей белый адрес не нужен: она сама соединяется с ульем.</li>
</ul>
<pre>mds node --lite --site https://ваш-сайт --hive https://beesearch.ru</pre>
<p>Лёгкая пчела не открывает свою страницу поиска. Она подтверждает сайт файлом <code>/.well-known/pchela.txt</code>: в файле одна строка, имя пчелы из её журнала. Сначала улей берёт новые и изменённые страницы этого сайта. Когда своих изменений нет, пчела разбирает чужую очередь. Подробно — на страницах <a href="/for-sites">Сайту</a>, <a href="/hive">Улей</a> и <a href="/computer">Компьютер</a>.</p>
<ul>
<li>Своя страница поиска будет на том адресе, который указан в --listen. Для людей снаружи на этот адрес ставят домен.</li>
<li>Телефон к команде полной ноды не относится и публичным входом не является. Приложение ставится со страницы <a href="/android">Приложение для Android</a>.</li>
<li>Обезличенный журнал запросов открывается только с компьютера, где запущена нода, по адресу /operator.</li>
</ul>
<h3>Какую ноду ставить</h3>
<p>Рядом с сайтом достаточно лёгкой пчелы: она не открывает поиск для чужих и отдаёт улью страницы своего сайта. Полную ноду ставят, когда нужен свой вход в поиск и свой кусок индекса. Компьютер без сайта разбирает чужую очередь и свою страницу поиска не открывает. Приложение на телефоне — поиск улья и короткий обход очереди, в том числе в фоне. Зачем каждая роль — на страницах <a href="/hive">Сервер-улей</a>, <a href="/computer">Компьютер без сайта</a>, <a href="/indexing">Индексация</a> и <a href="/android">Приложение для Android</a>.</p>
<p><a href="/how">Как устроен поиск</a></p>"#
        ),
    )
}

pub fn page_hive() -> String {
    article(
        "Сервер-улей",
        r#"<p>Улей — вход, с которого люди ищут. На нём лежат индекс, модель смысла и кабинет оператора. Лёгкая пчела, компьютер без сайта и телефон приносят ему текст. Пишет индекс улей.</p>
<h3>Что ставить</h3>
<p>Та же программа, что на странице <a href="/download">Скачать ноду</a>. В Windows это <code>mds.exe</code>, в Linux — <code>mds</code>. При первом запуске полная нода забирает модель с улья, около 1 ГБ. Ссылки на файл весов на странице загрузки нет.</p>
<pre>mds node --listen 0.0.0.0:8080 --seeds urls.txt --follow-external --peers https://beesearch.ru</pre>
<ul>
<li>В urls.txt — стартовые адреса сайтов, по одному в строке. С них обход находит ссылки и ставит новые адреса в очередь.</li>
<li>Публичный улей, на который смотрит домен, запускают с <code>--alone</code>, чтобы он не звонил сам себе.</li>
<li>Второй улей, который входит в ту же сеть, указывает соседа в <code>--peers</code>.</li>
<li>Для людей этот улей открыт на https://beesearch.ru.</li>
</ul>
<h3>Сколько нужно машине</h3>
<p>Один публичный улей — 4 ядра, 8 ГБ памяти и SSD около 100 ГБ. Модель занимает около 1 ГБ из этой памяти. База растёт вместе со страницами, её пишет один процесс. Узкий канал не подходит: с этой машины идут и ответы людям, и обход.</p>
<h3>Как это работает с шардами</h3>
<p>Шард — сохранённый кусок индекса. Пока улей один, он отвечает всем индексом, который собрал сам. Когда полных нод несколько, запрос идёт к тем, кто держит нужный шард, и не больше чем к трём копиям. Если одна копия отвечает заметно медленнее, улей в этот раз её не ждёт.</p>
<p>Лёгкая пчела, компьютер без сайта и телефон шард не держат и на чужой поиск не отвечают. Они приносят текст, индекс остаётся на улье. Журнал запросов хранится на той ноде, которая ответила, без адреса человека.</p>
<h3>Обновление</h3>
<p>Полная нода подхватывает новую версию сама, если в сети лежит подписанный файл новее того, что у неё запущен. На странице поиска появляется строка «Нужно обновиться». Пока такого выпуска нет, новую сборку кладут вручную и перезапускают службу. Архив со страницы загрузки сам программу не подменяет.</p>
<p><a href="/computer">Компьютер без сайта</a></p>"#,
    )
}

pub fn page_computer() -> String {
    article(
        "Компьютер без сайта",
        r#"<p>Это старый или свободный компьютер с интернетом. Белый адрес и открытый порт ему не нужны: он сам соединяется с ульем. Свою страницу поиска он не открывает. Люди по-прежнему ищут на улье.</p>
<h3>Что ставить</h3>
<p>Та же программа, что на странице <a href="/download">Скачать ноду</a>. Модель и свою базу он не качает. Памяти хватает обычной: гигабайт под модель ему не нужен.</p>
<pre>mds node --lite --hive https://beesearch.ru</pre>
<p>В Windows та же команда у <code>mds.exe</code>. Компьютер можно оставить включённым на сутки: он разбирает чужую очередь, пока работает.</p>
<h3>Как он работает</h3>
<p>Улей выдаёт адреса из очереди. Компьютер открывает страницы, забирает текст и отдаёт его улью. Улей записывает текст в индекс и сам считает смысл. Страница без изменений в индекс повторно не встраивается. Своего сайта у этой пчелы нет, поэтому файл <code>/.well-known/pchela.txt</code> не нужен.</p>
<p>Темп берётся из кабинета улья, из потолка на странице «Нода»: сколько страниц за заход и какая минимальная пауза. Это тот же потолок, что у телефона в режиме «как сервер». Пчела рядом с сайтом ходит другим темпом, его задают на странице «Пчёлы»: сначала свои новые и изменённые страницы, потом чужая очередь.</p>
<h3>Обновление</h3>
<p>Файл на этом компьютере сам не меняется. Новую сборку скачивают со страницы загрузки, кладут на место старой и перезапускают программу. Кабинет улья обновление на чужой компьютер не отправляет: это было бы управление чужой машиной.</p>
<p>Если у компьютера есть свой сайт, команда другая: <code>mds node --lite --site https://ваш-сайт --hive https://beesearch.ru</code>. Как её подтвердить — на странице <a href="/for-sites">Сайту</a>.</p>
<p><a href="/hive">Сервер-улей</a></p>"#,
    )
}

pub fn page_android() -> String {
    article(
        "Приложение для Android",
        r#"<p>Это поиск Пчелопоиска на телефоне. Это не кабинет оператора и не админская программа. Кабинет открывается только на компьютере, где запущена нода.</p>
<p><a class="button" href="/download/pchela-android.apk">Скачать приложение для Android</a></p>
<h3>Что видно на экране</h3>
<p>Приложение открывает поиск этого улья. Ссылка из выдачи открывается в том же окне: это браузер внутри приложения, отдельное приложение браузера не запускается. Кнопка «К поиску» возвращает на строку поиска. Сайт из выдачи отдаёт сам сайт. Улей в этот момент страницу сайта не качает и этим не нагружается: он уже отдал только страницу поиска.</p>
<h3>Как телефон помогает сети</h3>
<p>Примерно раз в десять минут телефон берёт у улья до двух адресов из очереди. Эти адреса улей и так собирался прочитать. Телефон сам открывает их, забирает текст и отдаёт улью. Улей записывает текст в индекс и сам считает смысл страницы. Эти две страницы улей уже не качает, поэтому очередь обхода идёт быстрее. Телефон индекс у себя не хранит и не разбирает: он приносит текст, а индекс пишет улей.</p>
<p>Так происходит и по Wi-Fi, и по мобильной сети, и когда приложение свёрнуто. На экране телефона остаётся короткое уведомление «Пчелопоиск». Кнопка «Остановить» в уведомлении помощь прекращает. У каждого телефона свой темп. Обычный режим — две страницы и пауза десять минут. В приложении 0.1.9 кнопка «Темп» может взять больше, до потолка, который оператор задал в кабинете, и пауза слушается от одной минуты. Старый телефон у розетки в режиме «как сервер» разбирает очередь круглосуточно в этих пределах.</p>
<p>Чужой поиск через телефон не идёт: люди по-прежнему ищут на улье.</p>
<h3>Что такое шард</h3>
<p>Шард — это сохранённый кусок индекса. Нода, которая его держит, отвечает по этому куску на чужие запросы. Телефон такой кусок не хранит и на чужие запросы не отвечает. Поэтому приложение шардом не называется: оно не запасная копия поиска и не вход в сеть для других людей.</p>
<h3>Безопасно ли это</h3>
<p>Телефон читает только те адреса, которые только что назвал улей, и не больше двух за раз. Свои запросы человека в этот обход не входят. Улей не записывает адрес телефона и не связывает обход с человеком. Порт наружу телефон не открывает, чужой трафик через него не проходит.</p>
<h3>Обновление</h3>
<p>Если улей объявил версию новее той, что стоит на телефоне, поиск не открывается, пока человек не поставит файл. Android один раз просит подтвердить установку. Если приложение уже стояло из старой сборки с другим ключом, его сначала удаляют: Android не кладёт новую сборку поверх.</p>
<p><a href="/download">Скачать ноду</a></p>"#,
    )
}

pub fn page_help() -> String {
    article(
        "Помочь проекту",
        r#"<p>Пчелопоиск — волонтёрский поиск. Выдачу не продают. Деньги нужны на улей: сервер, диск и нормальный канал, чтобы страницы открывались и обход не вставал.</p>
<p>Один публичный улей — это 4 ядра, 8 ГБ памяти и SSD около 100 ГБ. Такой сервер стоит порядка двух тысяч рублей в месяц. Узкий канал за эти деньги не подходит.</p>
<p>Перевод держит инфраструктуру роя. Рекламы в выдаче от этого не появится.</p>
<p>Пожертвования идут на развитие проекта и в фонд на сохранение медоносных пчёл.</p>
<p>Фонд сохранения медоносных пчёл, Московская область. ОГРН 1265000072116, присвоен 15.10.2026. ИНН 5003176058, КПП 500301001. Президент Климович Артём Геннадьевич.</p>"#,
    )
}

pub fn page_bees() -> String {
    let picks = [
        ("пчеловодство", "Пчеловодство"),
        ("пасека", "Пасека"),
        ("медоносная пчела", "Медоносная пчела"),
        ("мёд", "Мёд"),
        ("медоносные растения", "Медоносы"),
    ]
    .into_iter()
    .map(|(query, label)| format!("<a href=\"/search?q={}&amp;region=\">{label}</a>", encode_query(query)))
    .collect::<String>();
    article(
        "Всё о пчёлах",
        &format!(
            r#"<div class="bee-story">{bee}
<p>Короткая справка о медоносной пчеле и пчеловодстве. Это не пересказ чужой статьи и не ответ вместо ссылки. Ниже можно искать живые страницы пасек, хозяйств и книг по всей стране: регион этой ноды к запросу не прикрепляется.</p>
<h3>Медоносная пчела</h3>
<p>Медоносная пчела живёт семьёй, а не поодиночке. Семья держит одну матку, летом десятки тысяч рабочих пчёл и трутней. Рабочие приносят нектар и пыльцу, строят соты, кормят расплод и стерегут леток.</p>
<h3>Мёд и опыление</h3>
<p>Мёд семья делает из нектара и кормится им зимой. По дороге с цветка на цветок пчёлы переносят пыльцу. Без этого хуже урожаи садов, полей гречихи, подсолнечника и лугового разнотравья.</p>
<h3>Пчеловодство</h3>
<p>Пчеловод держит семьи в ульях на пасеке. В сезон смотрит расплод, силу семьи и взяток, откачивает зрелый мёд и готовит зимовку. Сроки медосбора и породы разные: на юге, в средней полосе и в Сибири пасека живёт не в один календарь.</p>
<h3>Найти по всей стране</h3>
<p>Запрос уходит в тот же поиск, что и обычная строка. Пока кнопку не нажали, улей страницы не считает и модель не будит. Карточки маркетплейсов и видеоплощадки в эту выдачу не подмешиваются: их отсекает общий поиск.</p>
<form class="bee-find" action="/search" method="get">
<input type="hidden" name="region" value="">
<input name="q" placeholder="Порода, пасека, мёд, растение" aria-label="Запрос про пчёл">
<button type="submit">Найти</button>
</form>
<div class="bee-picks">{picks}</div>
</div>"#,
            bee = bee_svg("portrait"),
        ),
    )
}

pub fn page_lost() -> String {
    page_fault(404)
}

pub fn page_fault(code: u16) -> String {
    let (title, why) = match code {
        500 => ("На улье сбой", "Страница не собралась. Откройте поиск ещё раз."),
        502 => ("Улей не отвечает", "Пчела долетела, но улей сейчас молчит. Откройте поиск через минуту."),
        503 => ("Улей занят", "Слишком много работы сразу. Зайдите через минуту."),
        _ => ("Страница не найдена", "Пчела облетела улей и не нашла этот адрес."),
    };
    let shown = if matches!(code, 404 | 500 | 502 | 503) { code } else { 404 };
    format!(
        r##"<!DOCTYPE html>
<html lang="ru">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex">
<title>{shown} — Пчелопоиск</title>
<style>
body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; font-family: Arial, Helvetica, sans-serif; color: #1c1612; background: #fffaf2; }}
main {{ width: min(100% - 32px, 520px); padding: 28px 8px 40px; text-align: center; }}
.code {{ margin: 0; font-family: Georgia, "Times New Roman", serif; font-size: 84px; line-height: 1; letter-spacing: -0.04em; color: #5c3a12; }}
.sky {{ position: relative; height: 168px; margin: 8px 0 4px; }}
.sky .bee {{ position: absolute; left: 50%; width: 148px; height: 148px; margin-left: -74px; animation: float 2.6s ease-in-out infinite; }}
.sky .wing {{ transform-box: fill-box; transform-origin: 50% 100%; animation: flap .08s ease-in-out infinite alternate; }}
.sky .wing.right {{ animation-duration: .06s; }}
.shade {{ position: absolute; left: 50%; bottom: 8px; width: 78px; height: 14px; margin-left: -39px; border-radius: 50%; background: rgba(28,22,18,.14); animation: shade 2.6s ease-in-out infinite; }}
h1 {{ margin: 0 0 10px; font-family: Georgia, "Times New Roman", serif; font-size: 28px; font-weight: 500; }}
p.why {{ margin: 0 0 22px; color: #4d453c; line-height: 1.5; }}
a.back {{ display: inline-block; padding: 12px 22px; border-radius: 22px; background: #1c1612; color: #fff; text-decoration: none; }}
a.back:hover {{ background: #5c3a12; }}
@keyframes float {{
  0%, 100% {{ transform: translateY(8px) rotate(-2deg); }}
  50% {{ transform: translateY(-14px) rotate(2deg); }}
}}
@keyframes flap {{
  from {{ transform: rotate(-20deg); }}
  to {{ transform: rotate(18deg); }}
}}
@keyframes shade {{
  0%, 100% {{ transform: scaleX(1.05); opacity: .45; }}
  50% {{ transform: scaleX(.78); opacity: .22; }}
}}
@media (prefers-reduced-motion: reduce) {{
  .sky .bee, .sky .wing, .shade {{ animation: none; }}
}}
</style>
</head>
<body>
<main>
<p class="code">{shown}</p>
<div class="sky" aria-hidden="true">{bee}<i class="shade"></i></div>
<h1>{title}</h1>
<p class="why">{why}</p>
<a class="back" href="/">К поиску</a>
</main>
</body>
</html>"##,
        bee = bee_svg("bee"),
    )
}

fn article(title: &str, body: &str) -> String {
    document("", &format!("<article><h2>{}</h2>{body}</article>", escape(title)), false, false, None, "", "", None, None, None, false)
}

fn footer() -> String {
    r##"<footer>
<div class="foot">
<nav>
<a href="/">Поиск</a>
<a href="/manifesto">Манифест</a>
<a href="/how">Как устроен</a>
<a href="/for-sites">Сайту</a>
<a href="/indexing">Индексация</a>
<a href="/llms">llms.md</a>
<a href="/android">Приложение</a>
<a href="/hive">Улей</a>
<a href="/computer">Компьютер</a>
<a href="/download">Скачать ноду</a>
<a href="/add">Добавить сайт</a>
</nav>
<p class="census">В поиске %%SITES%% и %%PAGES%%.</p>
<p class="credit"><span>Разработка креативной лаборатории <a href="https://dartalab.ru/">DARTA</a>.</span><a class="help" href="/help">Помочь проекту</a></p>
<svg class="comb" viewBox="0 0 128 128" aria-hidden="true"><g fill="#f9cb0b"><path d="m59.86475 67.82129a1.48481 1.48481 0 0 1 -.74463-.19922l-14.39258-8.25293-5.50635 3.08447a1.49985 1.49985 0 0 1 -1.46582-2.61718l6.248-3.5a1.49964 1.49964 0 0 1 1.479.00732l15.12993 8.67676a1.49973 1.49973 0 0 1 -.74755 2.80078z"/><path d="m54.354 107.72754a1.50024 1.50024 0 0 1 -.75244-2.79883l13.09668-7.57422-.0166-25.32812-.94922-.54688a1.50018 1.50018 0 1 1 1.498-2.59961l1.69922.9795a1.49827 1.49827 0 0 1 .751 1.29882l.01856 27.06055a1.50127 1.50127 0 0 1 -.749 1.2998l-13.84668 8.00782a1.49211 1.49211 0 0 1 -.74952.20117z"/><path d="m44.77344 113.2666a1.49676 1.49676 0 0 1 -.749-.20019l-23.44436-13.51465a1.50034 1.50034 0 0 1 -.751-1.29883l-.00342-4.55371a1.5004 1.5004 0 0 1 1.499-1.501h.001a1.49986 1.49986 0 0 1 1.5 1.499l.00293 3.6875 21.94385 12.64941 2.63428-1.52344a1.50031 1.50031 0 0 1 1.502 2.59766l-3.38428 1.957a1.5049 1.5049 0 0 1 -.751.20125z"/><path d="m76.32324 23.25293a1.49178 1.49178 0 0 1 -.748-.20117l-6.37797-3.67676a1.49975 1.49975 0 1 1 1.498-2.59863l6.37793 3.67676a1.50014 1.50014 0 0 1 -.75 2.7998z"/><path d="m68.18555 72.66016a1.50009 1.50009 0 0 1 -.752-2.79883l22.67583-13.11328-.001-2.24366a1.49976 1.49976 0 0 1 1.499-1.501h.001a1.50018 1.50018 0 0 1 1.5 1.499l.00195 3.10986a1.49988 1.49988 0 0 1 -.749 1.29932l-23.42578 13.54743a1.49387 1.49387 0 0 1 -.75.20116z"/><path d="m44.73975 59.146a1.49986 1.49986 0 0 1 -1.5-1.499l-.01856-27.061a1.49947 1.49947 0 0 1 .749-1.29932l23.42629-13.54645a1.49707 1.49707 0 0 1 1.5-.001l1.79883 1.03711a1.49975 1.49975 0 1 1 -1.498 2.59863l-1.04883-.60449-21.9268 12.67972.01807 26.1948a1.49977 1.49977 0 0 1 -1.499 1.501z"/><path d="m107.92871 103.81836a1.50008 1.50008 0 0 1 -.75195-2.79883l6.39355-3.69726-.0166-25.32813-21.94433-12.64941-21.92579 12.67968.01661 25.32813 6.32617 3.64648a1.50018 1.50018 0 0 1 -1.49805 2.59961l-7.07617-4.0791a1.49827 1.49827 0 0 1 -.751-1.29883l-.01855-27.06054a1.50127 1.50127 0 0 1 .749-1.29981l23.42578-13.5459a1.49825 1.49825 0 0 1 1.5-.001l23.44434 13.51367a1.49828 1.49828 0 0 1 .751 1.29883l.01856 27.06054a1.50129 1.50129 0 0 1 -.749 1.29981l-7.14356 4.13086a1.49386 1.49386 0 0 1 -.75001.2012z"/></g><path d="m39.56354 76.5401c.32348-.35632.64062-.72253.94281-1.10846a16.814 16.814 0 0 0 3.54785-7.876c.38721-2.8916-.37354-5.24658-2.14209-6.63134-1.76856-1.38575-4.23779-1.55762-6.95117-.48828a16.13117 16.13117 0 0 0 -5.81757 4.18774c-4.15484-4.55859-9.57715-6.1994-12.77325-3.69946-3.46 2.70947-2.84229 9.08154 1.40576 14.50732.247.31531.50018.6308.75622.94226a9.20241 9.20241 0 0 0 2.10315 18.16223h16.18409a9.20737 9.20737 0 0 0 2.7442-17.996zm-12.95026 14.996a35 35 0 0 0 .54541-6.207 35.46647 35.46647 0 0 0 -.36389-5.03 5.72943 5.72943 0 0 0 3.569 1.15985 8.512 8.512 0 0 0 2.317-.3493 32.74556 32.74556 0 0 1 .26941 4.21942 31.76 31.76 0 0 1 -.60749 6.207zm9.44824-28.30908a6.16994 6.16994 0 0 1 2.229-.47559 2.82389 2.82389 0 0 1 1.772.53516c.88379.6919 1.25488 2.103 1.01807 3.87061a13.78553 13.78553 0 0 1 -2.93659 6.4248 13.791 13.791 0 0 1 -5.5332 4.39258c-1.65967.65332-3.11719.63281-4.00146-.05957-1.80127-1.41016-1.45606-5.98535 1.91845-10.2959a13.78338 13.78338 0 0 1 5.53373-4.39206zm-17.84179.05957c1.70953-1.33856 5.74981-.09717 9.04364 3.74207a14.06909 14.06909 0 0 0 -2.6695 9.09338h-2.20849a20.31311 20.31311 0 0 1 -2.2472-2.54c-3.375-4.30958-3.71972-8.88526-1.91845-10.29542zm-3.791 22.04248a6.21359 6.21359 0 0 1 6.20654-6.207h2.91651a31.78523 31.78523 0 0 1 .60693 6.207 31.70458 31.70458 0 0 1 -.60821 6.207h-2.91525a6.21359 6.21359 0 0 1 -6.20654-6.20697zm22.39063 6.207h-1.42178a35.00225 35.00225 0 0 0 .55262-6.207 35.3936 35.3936 0 0 0 -.42414-5.45233c.38855-.22748.7713-.48444 1.15058-.7547h.1427a6.207 6.207 0 0 1 0 12.41406z" fill="#153d45"/><circle cx="18.544" cy="84.209" fill="#153d45" r="1.821"/><path d="m104.79517 34.35016q.71245-.75742 1.38256-1.60993a19.48713 19.48713 0 0 0 4.11622-9.12988c.44238-3.30127-.417-5.98291-2.41895-7.55029-3.70154-2.89795-10.03082-.89856-14.848 4.52472-4.8121-5.41321-11.148-7.41535-14.84341-4.52472-2.00195 1.56738-2.86132 4.249-2.41894 7.55029a19.48711 19.48711 0 0 0 4.11621 9.12988c.37726.48188.767.962 1.16095 1.43024a10.554 10.554 0 0 0 1.99139 20.91889h19.01953a10.5502 10.5502 0 0 0 2.74244-20.7392zm-18.12256 17.7392h-3.63941a7.55665 7.55665 0 0 1 0-15.11329h3.645a37.89224 37.89224 0 0 1 -.00555 15.11329zm1.19751-19.22071a9.76145 9.76145 0 0 0 .11029 1.10742h-2.99414a23.47223 23.47223 0 0 1 -2.74408-3.08544 16.471 16.471 0 0 1 -3.50391-7.67823c-.1709-1.27685-.23047-3.59619 1.29492-4.79 2.42542-1.90149 7.41877.198 11.11347 4.58185a21.07219 21.07219 0 0 0 -2.09491 3.89175 1.49991 1.49991 0 1 0 2.77929 1.1289 18.68337 18.68337 0 0 1 2.62012-4.46631c3.73731-4.77148 9.03906-7.12255 11.57422-5.13623 1.52539 1.19385 1.46582 3.51319 1.29492 4.79a16.47084 16.47084 0 0 1 -3.5039 7.67823c-3.73633 4.77148-9.03418 7.12255-11.57422 5.13671a4.28061 4.28061 0 0 1 -1.374-3.27 1.50005 1.50005 0 1 0 -2.998.11133zm9.13714 19.22071h-7.27185a40.6984 40.6984 0 0 0 .70111-7.55665 41.20461 41.20461 0 0 0 -.51751-6.56347 6.04962 6.04962 0 0 0 .47357.41992 6.49577 6.49577 0 0 0 4.084 1.32813 9.97228 9.97228 0 0 0 2.92749-.47107 38.50108 38.50108 0 0 1 .3645 5.28649 37.46367 37.46367 0 0 1 -.76131 7.55665zm5.04547 0h-1.9898a40.729 40.729 0 0 0 .70562-7.55665 41.27586 41.27586 0 0 0 -.52215-6.54229 17.61976 17.61976 0 0 0 1.57342-1.01435h.23291a7.55665 7.55665 0 0 1 0 15.11329z" fill="#153d45"/><path d="m80.57617 41.33887a1.87744 1.87744 0 1 0 1.87793 1.87744 1.87924 1.87924 0 0 0 -1.87793-1.87744z" fill="#153d45"/><path d="m102.56335 94.54c.32343-.35633.64045-.72241.94251-1.10834a16.80326 16.80326 0 0 0 3.54785-7.877c.3877-2.89063-.373-5.2461-2.1416-6.62989-3.204-2.50964-8.61322-.8645-12.76959 3.69751-4.15515-4.55694-9.5774-6.19781-12.7724-3.69848-1.76856 1.38476-2.5293 3.74023-2.1416 6.63086a16.80335 16.80335 0 0 0 3.54687 7.876c.24689.31586.50012.63166.75629.94342a9.20236 9.20236 0 0 0 2.10406 18.162h16.1836a9.20732 9.20732 0 0 0 2.744-17.99615zm-12.94989 14.99613a35.00845 35.00845 0 0 0 .54572-6.207 35.41766 35.41766 0 0 0 -.36414-5.02978 5.72948 5.72948 0 0 0 3.56824 1.15965 8.51228 8.51228 0 0 0 2.31677-.34912 32.68884 32.68884 0 0 1 .27015 4.21924 31.71952 31.71952 0 0 1 -.60834 6.207zm11.69416-28.77343a2.79691 2.79691 0 0 1 1.75488.52441c.88379.69141 1.25488 2.10156 1.01855 3.86914a13.794 13.794 0 0 1 -2.93652 6.42578h-.001a13.78539 13.78539 0 0 1 -5.53222 4.39258c-1.6582.6543-3.11817.63379-4.00195-.06055-.88379-.6914-1.25489-2.10156-1.01856-3.86914a13.80279 13.80279 0 0 1 2.93652-6.42578c2.54006-3.24314 5.63577-4.85644 7.7803-4.85644zm-21.10645 4.39355c-.23633-1.76758.13477-3.17773 1.01856-3.87012 1.70648-1.33453 5.74768-.09784 9.04431 3.74451a16.10514 16.10514 0 0 0 -2.64588 6.61584 9.82692 9.82692 0 0 0 -.02545 2.47559h-2.20782a20.29205 20.29205 0 0 1 -2.24719-2.54 13.79411 13.79411 0 0 1 -2.93653-6.42582zm-2.77246 18.17285a6.2137 6.2137 0 0 1 6.207-6.207h2.91663a32.00952 32.00952 0 0 1 -.00153 12.41406h-2.9151a6.2137 6.2137 0 0 1 -6.207-6.20706zm22.39063 6.207h-1.42182a35.00923 35.00923 0 0 0 .55268-6.207 35.38662 35.38662 0 0 0 -.42414-5.45233c.38855-.22748.7713-.48444 1.15058-.7547h.1427a6.207 6.207 0 0 1 0 12.41406z" fill="#153d45"/><path d="m81.54492 100.3877a1.82129 1.82129 0 1 0 1.82031 1.82128 1.82381 1.82381 0 0 0 -1.82031-1.82128z" fill="#153d45"/></svg>
</div>
</footer>"##
        .to_string()
}

pub fn notice_bar(version: &str) -> String {
    format!(
        r#"<p class="notice" style="margin:0;padding:10px 16px;background:#fff4c8;color:#5c3a12;text-align:center;">Вышла версия {}. Нужно обновиться. <a href="/download">Скачать</a></p>"#,
        escape(version)
    )
}

pub fn stamp_notice(html: String, version: Option<&str>) -> String {
    let Some(version) = version else {
        return html;
    };
    let Some(start) = html.find("<body") else {
        return html;
    };
    let Some(close) = html[start..].find('>') else {
        return html;
    };
    let at = start + close + 1;
    let mut out = String::with_capacity(html.len() + 160);
    out.push_str(&html[..at]);
    out.push_str(&notice_bar(version));
    out.push_str(&html[at..]);
    out
}

pub fn stamp_census(html: String, sites: usize, pages: usize) -> String {
    html.replace("%%SITES%%", &ru_count(sites, "сайт", "сайта", "сайтов"))
        .replace("%%PAGES%%", &ru_count(pages, "страница", "страницы", "страниц"))
}

fn ru_count(n: usize, one: &str, few: &str, many: &str) -> String {
    let n10 = n % 10;
    let n100 = n % 100;
    let word = if (11..15).contains(&n100) {
        many
    } else if n10 == 1 {
        one
    } else if (2..5).contains(&n10) {
        few
    } else {
        many
    };
    format!("{n} {word}")
}

fn badges(
    words: bool,
    meaning: bool,
    volunteer: bool,
    region: Option<&str>,
    place: Option<&str>,
    distance_km: Option<f32>,
) -> String {
    let mut out = String::new();
    if let Some(km) = distance_km {
        let label = if km < 1.0 {
            format!("{} м", ((km * 1000.0).round() as i64).max(10))
        } else if km < 10.0 {
            format!("{km:.1} км").replace('.', ",")
        } else {
            format!("{} км", km.round() as i64)
        };
        out.push_str(&format!("<span class=\"badge\">{label}</span>"));
    }
    if let Some(region) = region {
        out.push_str(&format!("<span class=\"badge\">{region}</span>", region = escape(region)));
    }
    if let Some(place) = place {
        if Some(place) != region {
            out.push_str(&format!("<span class=\"badge\">{place}</span>", place = escape(place)));
        }
    }
    if words {
        out.push_str("<span class=\"badge\">слова</span>");
    }
    if meaning {
        out.push_str("<span class=\"badge\">смысл</span>");
    }
    if volunteer {
        out.push_str("<span class=\"badge bee\">держит пчелу</span>");
    }
    out
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

fn encode_query(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(n: usize) -> Hit {
        Hit {
            url: format!("https://example.com/{n}"),
            title: format!("Страница {n}"),
            snippet: "текст".into(),
            score: 1.0,
            via_words: true,
            via_meaning: false,
            adult: false,
            volunteer: false,
            region: None,
            place: None,
            distance_km: None,
            lat: None,
            lon: None,
            kind: 0,
            media_url: None,
            feed_url: None,
            news_feed: None,
            price: None,
        }
    }

    #[test]
    fn lost_page_points_back_to_search() {
        let page = page_lost();
        assert!(page.contains("Страница не найдена"));
        assert!(page.contains("href=\"/\""));
        assert!(page.contains("class=\"wing left\""));
        assert!(page.contains("class=\"wing right\""));
        assert!(page.contains("#FBDD56"));
        assert!(page.contains("noindex"));
        assert!(page_fault(502).contains("Улей не отвечает"));
        if let Ok(dir) = std::env::var("MDS_FAULT_DIR") {
            for code in [404, 500, 502, 503] {
                std::fs::write(format!("{dir}/{code}.html"), page_fault(code)).unwrap();
            }
        }
    }

    #[test]
    fn query_is_escaped() {
        let html = render("<script>", &[], 1, None);
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("<textarea name=\"q\" rows=\"1\" autofocus placeholder=\"Запрос\">&lt;script&gt;</textarea>"));
        assert_eq!(html.matches("<script>").count(), 1);
        assert!(html.contains("https://dartalab.ru/"));
        assert!(html.contains("<title>Пчелопоиск</title>"));
        let home = render("", &[], 1, None);
        assert!(home.contains("class=\"home\""));
        assert!(home.contains("Bee search"));
        assert!(home.contains("Пчелопоиск"));
        assert!(home.contains("без фильтра по языку"));
        assert!(home.contains("ПчелоПоиск"));
        assert!(home.contains("class=\"name\">Поиск"));
        assert!(home.contains("class=\"bee\""));
        assert!(home.contains("<textarea name=\"q\""));
        assert!(home.contains("aria-label=\"Найти\""));
        assert!(home.contains("aria-label=\"Очистить запрос\" hidden"));
        assert!(home.contains("href=\"/bees\">Всё о пчёлах"));
        assert!(!home.contains("tab=jobs"));
        assert!(!home.contains("class=\"eye\""));
        assert!(home.contains("<svg"));
        assert!(home.contains("Помочь проекту"));
        assert!(home.contains("class=\"credit\""));
        assert!(home.contains("h1 { margin: 0 0 8px; text-align: center; }"));
        let credit = home.find("Разработка креативной лаборатории").unwrap();
        let help = home.find("class=\"help\"").unwrap();
        assert!(credit < help);
        let stamped = stamp_census("%%SITES%% и %%PAGES%%".into(), 21, 5);
        assert_eq!(stamped, "21 сайт и 5 страниц");
        assert_eq!(stamp_census("%%SITES%% %%PAGES%%".into(), 11, 22), "11 сайтов 22 страницы");
        assert_eq!(stamp_census("%%SITES%% %%PAGES%%".into(), 2, 1), "2 сайта 1 страница");
        assert!(home.contains("class=\"comb\""));
        let comb = home.find("class=\"comb\"").unwrap();
        assert!(help < comb);
        let mut bee = sample(1);
        bee.volunteer = true;
        let marked = render("мёд", &[bee], 1, None);
        assert!(marked.contains("держит пчелу"));
        assert!(page_for_sites().contains("--lite"));
        assert!(html.contains("href=\"/manifesto\""));
        assert!(html.contains("href=\"/how\""));
        assert!(html.contains("href=\"/llms\""));
        assert!(html.contains("href=\"/download\""));
        assert!(html.contains("href=\"/indexing\">Индексация"));
        assert!(html.contains("href=\"/add\">Добавить сайт"));
        assert!(html.contains("nav class=\"kinds\""));
        assert!(html.contains(">Работа</a>"));
        assert!(html.contains("tab=jobs"));
        assert!(html.contains("href=\"/bees\">Всё о пчёлах"));
        assert!(html.contains("aria-label=\"Очистить запрос\""));
        assert!(!html.contains("Очистить запрос\" hidden"));
        let kept = render_results(
            "мёд",
            &[sample(1)],
            &[],
            false,
            1,
            None,
            None,
            "jobs",
            "auto",
            Some(55.755),
            Some(37.617),
            Some(1.0),
            true,
        );
        assert!(kept.contains("tab=jobs"));
        assert!(kept.contains("theme=auto"));
        assert!(kept.contains("lat=55.755"));
        assert!(kept.contains("lon=37.617"));
        assert!(kept.contains("radius=1"));
        assert!(kept.contains("name=\"theme\" value=\"auto\""));
        assert!(kept.contains("name=\"tab\" value=\"jobs\""));
        let help = page_help();
        assert!(help.contains("1265000072116"));
        assert!(help.contains("Фонд сохранения медоносных пчёл"));
        assert!(help.contains("на развитие проекта и в фонд"));
        assert!(help.contains("5003176058"));
        assert!(!help.contains("Когда появятся реквизиты"));
        assert!(help.contains("https://dartalab.ru/"));
        let story = page_bees();
        assert!(story.contains("Медоносная пчела"));
        assert!(story.contains("одну матку"));
        assert!(story.contains("по всей стране"));
        assert!(story.contains("class=\"portrait\""));
        assert!(story.contains("name=\"region\" value=\"\""));
        assert!(story.contains("Пчеловодство"));
        let hidden = render_results("", &[], &[], false, 1, None, None, "", "", None, None, None, false);
        assert!(!hidden.contains("href=\"/bees\""));
        assert!(hidden.contains("class=\"home\""));
        assert_eq!(html.matches("https://dartalab.ru/").count(), 1);
        assert!(!html.contains("id=\"how\""));
    }

    #[test]
    fn info_pages_explain_llms_and_the_node_file() {
        let llms = page_llms();
        assert!(llms.contains("/llms.txt"));
        assert!(llms.contains("/llms.md"));
        assert!(llms.contains("400 символов"));
        assert!(llms.contains("не берёт больше ста"));
        assert!(llms.contains("натуральный мёд"));
        assert!(llms.contains("href=\"/llms.txt\""));
        assert!(llms.contains("href=\"/llms.md\""));
        let card = hive_card();
        assert!(card.contains("# Пчелопоиск"));
        assert!(card.contains("/indexing"));
        assert!(card.contains("Волонтёрский поиск"));
        assert!(card.contains(SOURCE_REPO));
        let robots = robots_txt("https://example.test");
        assert!(robots.contains("Disallow: /operator"));
        assert!(robots.contains("Sitemap: https://example.test/sitemap.xml"));
        let map = sitemap_xml("https://example.test");
        assert!(map.contains("<loc>https://example.test/manifesto</loc>"));
        assert!(map.contains("<loc>https://example.test/llms.md</loc>"));
        assert!(map.contains("<loc>https://example.test/add</loc>"));
        assert!(map.contains("<loc>https://example.test/indexing</loc>"));
        let indexing = page_indexing();
        assert!(indexing.contains("Зачем нода рядом с сайтом"));
        assert!(indexing.contains("Зачем полная нода"));
        assert!(indexing.contains("Зачем приложение на телефоне"));
        assert!(indexing.contains("href=\"/android\""));
        let phone = page_android();
        assert!(phone.contains("не кабинет"));
        assert!(phone.contains("Что такое шард"));
        assert!(phone.contains("браузер внутри приложения"));
        assert!(phone.contains("когда приложение свёрнуто"));
        assert!(phone.contains("индекс пишет улей"));
        assert!(phone.contains("К поиску"));
        assert!(phone.contains("/download/pchela-android.apk"));
        assert!(indexing.contains("Если сломается Яндекс или Гугл"));
        assert!(indexing.contains("до ста страниц"));
        assert!(page_manifesto().contains("Если крупный поиск встанет"));
        assert!(page_how().contains("href=\"/indexing\""));
        assert!(page_for_sites().contains("чужие адреса"));
        assert!(page_hive().contains("4 ядра"));
        assert!(page_hive().contains("--alone"));
        assert!(page_computer().contains("--lite --hive"));
        assert!(page_computer().contains("сам не меняется"));
        assert!(page_download(None).contains("href=\"/hive\""));
        assert!(page_download(None).contains("href=\"/computer\""));
        assert!(page_download(None).contains("Какую ноду ставить"));
        assert!(page_download(None).contains("Лёгкая пчела свой файл не меняет"));
        let add = page_add("https://пасека.рф", None, false);
        assert!(add.contains("Проверить и добавить"));
        assert!(add.contains("https://пасека.рф"));
        assert!(add.contains("value=\"https://пасека.рф\""));
        assert!(!map.contains("/operator"));
        let missing = page_download(None);
        assert!(missing.contains("ещё не выложен"));
        assert!(missing.contains(RELEASE_WINDOWS));
        assert!(missing.contains(RELEASE_LINUX));
        assert!(missing.contains("href=\"/android\""));
        assert!(missing.contains("/download/pchela-linux-x64.tar.gz"));
        assert!(missing.contains("/download/pchela-windows-x64.zip"));
        assert!(missing.contains("забирает модель с улья"));
        assert!(!missing.contains("/peer/model"));
        assert!(missing.contains("https://admin.beesearch.ru/login"));
        assert!(page_how().contains("https://admin.beesearch.ru/login"));
        assert!(missing.contains("Нужно обновиться"));
        let noted = stamp_notice("<body class=\"home\">страница</body>".into(), Some("0.2.0"));
        assert!(noted.contains("Вышла версия 0.2.0"));
        assert!(noted.starts_with("<body class=\"home\">"));
        assert!(missing.contains(SOURCE_REPO));
        assert!(!missing.contains("href=\"/download/mds.exe\""));
        let ready = page_download(Some("0.2.0"));
        assert!(ready.contains("href=\"/download/mds.exe\""));
        assert!(ready.contains("href=\"/download/pchela.zip\""));
        assert!(ready.contains("0.2.0"));
        assert!(ready.contains("--peers"));
        assert!(ready.contains("--hive"));
        let manifesto = page_manifesto();
        assert!(manifesto.contains("не коммерческий проект"));
        assert!(manifesto.contains("волонтёрский"));
        assert!(manifesto.contains("маркетплейс"));
        assert!(manifesto.contains("дата-центрах"));
        assert!(manifesto.contains("спам-сайтов"));
        assert!(manifesto.contains("чистый поиск"));
        let adult = Hit {
            url: "https://studio.example/v".into(),
            title: "Порно видео".into(),
            snippet: "ролик".into(),
            score: 1.0,
            via_words: true,
            via_meaning: false,
            adult: true,
            volunteer: false,
            region: None,
            place: None,
            distance_km: None,
            lat: None,
            lon: None,
            kind: 0,
            media_url: None,
            feed_url: None,
            news_feed: None,
            price: None,
        };
        let closed = render_results("мёд", &[], &[adult.clone()], false, 1, None, None, "", "", None, None, None, true);
        assert!(closed.contains("18+"));
        assert!(closed.contains("class=\"results blurred\""));
        assert!(!closed.contains("https://studio.example/v"));
        let open = render_results("мёд", &[], &[adult], true, 1, None, None, "", "", None, None, None, true);
        assert!(open.contains("https://studio.example/v"));
        assert!(!open.contains("class=\"results blurred\""));
    }

    #[test]
    fn results_split_into_pages_of_25() {
        let hits: Vec<Hit> = (1..=26).map(sample).collect();
        let first = render("крепеж", &hits, 1, None);
        let second = render("крепеж", &hits, 2, None);
        assert_eq!(first.matches("class=\"url\"").count(), 25);
        assert!(first.contains("Найдено 26. Страница 1 из 2."));
        assert!(first.contains("/search?q=%D0%BA%D1%80%D0%B5%D0%BF%D0%B5%D0%B6&amp;page=2"));
        assert!(!first.contains("https://example.com/26"));
        assert_eq!(second.matches("class=\"url\"").count(), 1);
        assert!(second.contains("https://example.com/26"));
        assert!(second.contains("Страница 2 из 2."));
    }

    #[test]
    fn chosen_region_is_a_separate_band() {
        let mut near = sample(1);
        near.region = Some("Волгоградская область".into());
        near.title = "Банки Камышин".into();
        let mut far = sample(2);
        far.region = Some("Орловская область".into());
        far.title = "Мёд Орёл".into();
        let html = render_results(
            "банки",
            &[near, far],
            &[],
            false,
            1,
            None,
            Some("Волгоградская область"),
            "",
            "",
            None,
            None,
            None,
            true,
        );
        let home = html.find("В вашем регионе").unwrap();
        let other = html.find("В других регионах").unwrap();
        assert!(home < other);
        assert!(html.contains("Банки Камышин"));
        assert!(html.contains("data-region=\"Волгоградская область\""));
        assert!(html.contains("class=\"region-opt on\""));
        assert!(html.contains("name=\"region\" value=\"Волгоградская область\""));
        assert!(html.contains("aria-label=\"Настройки поиска\""));
        let news = render_results("мёд", &[], &[], false, 1, None, None, "news", "", None, None, None, true);
        assert!(!news.contains("Свежие заголовки изданий"));
        let mut paper = sample(1);
        paper.url = "https://www.kommersant.ru/".into();
        let with_paper = render_results("мёд", &[paper], &[], false, 1, None, None, "news", "", None, None, None, true);
        assert!(with_paper.contains("data-feed="));
        assert!(with_paper.contains("kommersant.ru/rss"));
        assert!(with_paper.contains("/wire?feed="));
        assert!(!with_paper.contains("Свежие заголовки изданий"));
        let mut shop = sample(2);
        shop.url = "https://shop.example/".into();
        shop.feed_url = Some("https://shop.example/feed.yml".into());
        let goods = render_results("болт", &[shop], &[], false, 1, None, None, "goods", "", None, None, None, true);
        assert!(goods.contains("data-offers="));
        assert!(goods.contains("/offers?feed="));
        assert!(goods.contains("Товары"));
        assert!(!html.contains("class=\"wire\""));
        assert!(html.contains("<p class=\"region-now\">Волгоградская область</p>"));
        assert!(html.contains("<details class=\"tune\">"));
        assert!(!html.contains("<details class=\"tune\" open>"));
    }

    #[test]
    fn region_button_keeps_this_pages_answer() {
        let page = render("завод в казани", &[], 1, None);
        let with = offer_region(page, "Татарстан", "https://kazan.example/search?q=завод");
        assert!(with.contains("Искать в Татарстан"));
        assert!(with.contains("https://kazan.example/search?q=завод"));
        assert!(with.contains("завод в казани"));
    }
}
