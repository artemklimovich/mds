use mds_types::{region_names, tab_mask, Hit};

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
    render_results(query, hits, &[], false, page, error, None, "")
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
) -> String {
    let page = visible_page(page, hits.len());
    let body = if let Some(error) = error {
        format!("<p class=\"error\">{}</p>", escape(error))
    } else if query.is_empty() {
        "<p class=\"hint\">Поиск по страницам, которые обошла эта пчела. Слова и смысл, на русском и на английском, без фильтра по языку.</p>".into()
    } else if hits.is_empty() && adult.is_empty() {
        if tab_mask(tab) == 0 {
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
            let badges = badges(hit.via_words, hit.via_meaning, hit.volunteer, hit.region.as_deref());
            let shot = hit.media_url.as_ref().map(|src| {
                format!(
                    "<a class=\"shot\" href=\"{url}\"><img src=\"{src}\" alt=\"\" referrerpolicy=\"no-referrer\"></a>",
                    url = escape(&hit.url),
                    src = escape(src),
                )
            }).unwrap_or_default();
            let price = hit.price.as_ref().map(|price| format!("<span class=\"price\">{}</span>", escape(price))).unwrap_or_default();
            let feed = hit.feed_url.as_ref().map(|feed| {
                format!(
                    "<ul class=\"offers\" data-feed=\"{}\" data-q=\"{}\"></ul>",
                    escape(feed),
                    escape(query),
                )
            }).unwrap_or_default();
            list.push_str(&format!(
                "<li>{shot}<a href=\"{url}\">{title}</a>{price}<div class=\"url\">{url}</div><p>{snippet}</p>{badges}{feed}</li>",
                url = escape(&hit.url),
                title = escape(&hit.title),
                snippet = escape(&hit.snippet),
            ));
        }
        if open_list {
            list.push_str("</ol>");
        }
        list.push_str(&pager(query, region, tab, page, hits.len()));
        if hits.iter().any(|hit| hit.feed_url.is_some()) {
            list.push_str(FEED_SCRIPT);
        }
        list.push_str(&adult_section(query, adult, adult_open));
        list
    };
    let body = if tab == "news" && !query.trim().is_empty() {
        format!("{}{body}", wire_strip(query))
    } else {
        body
    };
    document(query, &body, true, query.is_empty() && error.is_none(), region, tab)
}

fn wire_strip(query: &str) -> String {
    format!(
        r#"<section class="wire" data-q="{q}"><p class="wire-line"><button type="button" class="wire-open">Раскрыть</button> Свежие заголовки изданий</p><div class="rail" hidden><button type="button" class="nudge back" aria-label="Влево">‹</button><div class="wire-row"></div><button type="button" class="nudge forward" aria-label="Вправо">›</button></div></section>{script}"#,
        q = escape(query),
        script = WIRE_SCRIPT,
    )
}

fn wordmark() -> &'static str {
    r##"<a class="logo" href="/" aria-label="Пчелопоиск"><svg viewBox="0 0 640 230" role="img">
<defs>
<g id="bloom">
<ellipse cy="-12" rx="6.2" ry="10"/>
<ellipse cy="-12" rx="6.2" ry="10" transform="rotate(72)"/>
<ellipse cy="-12" rx="6.2" ry="10" transform="rotate(144)"/>
<ellipse cy="-12" rx="6.2" ry="10" transform="rotate(216)"/>
<ellipse cy="-12" rx="6.2" ry="10" transform="rotate(288)"/>
<circle r="4" fill="#f6c445"/>
</g>
</defs>
<use class="flower" href="#bloom" transform="translate(72,78)" fill="#f4a7b9"/>
<use class="flower" href="#bloom" transform="translate(58,128)" fill="#fff4c8"/>
<use class="flower" href="#bloom" transform="translate(78,176)" fill="#e7c6ff"/>
<use class="flower" href="#bloom" transform="translate(568,78)" fill="#ffd6e0"/>
<use class="flower" href="#bloom" transform="translate(582,128)" fill="#f4a7b9"/>
<use class="flower" href="#bloom" transform="translate(562,176)" fill="#fff4c8"/>
<text x="320" y="138" text-anchor="middle" font-family="Georgia, 'Times New Roman', serif" font-size="46"><tspan class="c1" fill="#e6a817">П</tspan><tspan fill="#1f1a17">ч</tspan><tspan fill="#f0c14b">е</tspan><tspan fill="#c47d12">л</tspan><tspan fill="#5c3a12">о</tspan><tspan fill="#e6a817">п</tspan><tspan fill="#1f1a17">о</tspan><tspan fill="#f0c14b">и</tspan><tspan fill="#c47d12">с</tspan><tspan fill="#5c3a12">к</tspan></text>
<g class="flyer a"><ellipse class="w l" cx="-10" cy="-6" rx="11" ry="5" fill="#fffdf6" stroke="#ead9a0" stroke-width="0.6"/><ellipse class="w r" cx="10" cy="-6" rx="11" ry="5" fill="#fffdf6" stroke="#ead9a0" stroke-width="0.6"/><ellipse cy="2" rx="6.5" ry="8.5" fill="#f6c445"/><path d="M-6 0h12M-6.2 3h12.4M-5.2 6h10.4" stroke="#1f1a17" stroke-width="1.4" stroke-linecap="round"/><circle cy="-7.5" r="3.6" fill="#1f1a17"/></g>
<g class="flyer b"><ellipse class="w l" cx="-8" cy="-5" rx="9" ry="4" fill="#fffdf6" stroke="#ead9a0" stroke-width="0.5"/><ellipse class="w r" cx="8" cy="-5" rx="9" ry="4" fill="#fffdf6" stroke="#ead9a0" stroke-width="0.5"/><ellipse cy="1.5" rx="5.2" ry="7" fill="#f6c445"/><path d="M-4.8 -0.2h9.6M-5 2.4h10M-4.2 5h8.4" stroke="#1f1a17" stroke-width="1.2" stroke-linecap="round"/><circle cy="-6" r="3" fill="#1f1a17"/></g>
<g class="flyer c"><ellipse class="w l" cx="-7" cy="-4" rx="7.5" ry="3.4" fill="#fffdf6" stroke="#ead9a0" stroke-width="0.5"/><ellipse class="w r" cx="7" cy="-4" rx="7.5" ry="3.4" fill="#fffdf6" stroke="#ead9a0" stroke-width="0.5"/><ellipse cy="1" rx="4.4" ry="6" fill="#f6c445"/><path d="M-4 -0.2h8M-4.2 2h8.4M-3.5 4.2h7" stroke="#1f1a17" stroke-width="1.1" stroke-linecap="round"/><circle cy="-5" r="2.6" fill="#1f1a17"/></g>
</svg></a>"##
}

fn document(query: &str, body: &str, autofocus: bool, home: bool, region: Option<&str>, tab: &str) -> String {
    let q = escape(query);
    let focus = if autofocus { " autofocus" } else { "" };
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
<style>
body {{ margin: 0; min-height: 100vh; display: flex; flex-direction: column; font-family: Arial, Helvetica, sans-serif; color: #202124; background: #fff; }}
main {{ flex: 1 0 auto; width: 100%; max-width: 680px; margin: 0 auto; padding: 28px 16px 32px; box-sizing: border-box; }}
h1 {{ margin: 0 0 8px; text-align: center; }}
.logo {{ display: inline-block; line-height: 0; text-decoration: none; }}
.logo svg {{ width: 240px; height: auto; overflow: visible; }}
.home .logo svg {{ width: min(460px, 94vw); }}
.w {{ transform-box: fill-box; transform-origin: center; }}
.w.l {{ animation: flap-l 0.22s ease-in-out infinite alternate; }}
.w.r {{ animation: flap-r 0.22s ease-in-out infinite alternate; }}
@keyframes flap-l {{ from {{ transform: rotate(-18deg); }} to {{ transform: rotate(-42deg); }} }}
@keyframes flap-r {{ from {{ transform: rotate(18deg); }} to {{ transform: rotate(42deg); }} }}
.flyer.a {{ animation: fly-a 6s ease-in-out infinite; }}
.flyer.b {{ animation: fly-b 8s ease-in-out infinite; }}
.flyer.c {{ animation: fly-c 7s ease-in-out infinite; }}
@keyframes fly-a {{
  0% {{ transform: translate(108px, 52px) rotate(-8deg); }}
  50% {{ transform: translate(520px, 46px) rotate(8deg); }}
  100% {{ transform: translate(108px, 52px) rotate(-8deg); }}
}}
@keyframes fly-b {{
  0% {{ transform: translate(530px, 168px) rotate(170deg); }}
  50% {{ transform: translate(100px, 160px) rotate(190deg); }}
  100% {{ transform: translate(530px, 168px) rotate(170deg); }}
}}
@keyframes fly-c {{
  0% {{ transform: translate(500px, 78px) rotate(172deg); }}
  50% {{ transform: translate(120px, 70px) rotate(188deg); }}
  100% {{ transform: translate(500px, 78px) rotate(172deg); }}
}}
.home main {{ max-width: 640px; padding-top: 10vh; text-align: center; }}
.home .en {{ margin: 8px 0 28px; color: #70757a; font-size: 13px; letter-spacing: 0.22em; text-transform: uppercase; }}
form {{ position: relative; margin: 12px 0 22px; }}
form:has(details[open]) {{ margin-bottom: 58px; }}
.ask {{ display: flex; align-items: center; gap: 4px; border: 1px solid #dfe1e5; border-radius: 24px; padding: 0 6px 0 8px; background: #fff; }}
.home .ask {{ box-shadow: 0 1px 6px rgba(32,33,36,.16); }}
.ask:focus-within {{ border-color: #4285f4; box-shadow: 0 1px 6px rgba(32,33,36,.28); }}
h2.band {{ font-size: 16px; font-weight: 600; margin: 8px 0 12px; }}
input[type=search] {{ flex: 1; min-width: 0; height: 44px; border: 0; border-radius: 24px; padding: 0 12px; font-size: 16px; background: transparent; }}
input[type=search]:focus {{ outline: none; }}
button.go {{ height: 36px; border: 0; border-radius: 18px; padding: 0 14px; background: #1a73e8; color: #fff; font-size: 14px; cursor: pointer; }}
details.tune {{ position: static; }}
summary.settings {{ display: grid; place-items: center; width: 36px; height: 36px; border-radius: 50%; color: #5f6368; cursor: pointer; list-style: none; }}
summary.settings::-webkit-details-marker {{ display: none; }}
details.tune[open] > summary.settings {{ background: #202124; color: #fff; }}
.filters {{ position: absolute; left: 0; top: calc(100% + 10px); }}
label.chip {{ position: relative; display: inline-flex; align-items: center; }}
select.region {{ appearance: none; height: 36px; max-width: min(100%, 320px); border: 0; border-radius: 18px; padding: 0 36px 0 14px; font-size: 14px; background: #f1f3f4; color: #202124; cursor: pointer; }}
label.chip svg {{ position: absolute; right: 12px; color: #5f6368; pointer-events: none; }}
nav.kinds {{ display: flex; flex-wrap: wrap; gap: 8px 14px; margin: 4px 0 16px; }}
nav.kinds a {{ color: #5f6368; text-decoration: none; font-size: 14px; padding-bottom: 6px; }}
nav.kinds a.on {{ color: #202124; font-weight: 600; border-bottom: 2px solid #202124; }}
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
footer {{ flex-shrink: 0; margin-top: auto; border-top: 1px solid #dadce0; background: #f8f9fa; }}
.foot {{ max-width: 680px; margin: 0 auto; padding: 22px 16px 28px; color: #4d5156; font-size: 14px; line-height: 1.5; }}
.foot nav {{ display: flex; flex-wrap: wrap; gap: 14px; margin-bottom: 12px; }}
.foot a {{ color: #1a0dab; }}
p.census {{ margin: 0 0 10px; color: #70757a; font-size: 13px; }}
p.credit {{ display: flex; align-items: center; justify-content: space-between; gap: 12px; margin: 0; }}
a.help {{ flex: 0 0 auto; margin: 0; padding: 3px 10px; border-radius: 12px; background: #fff8e1; border: 1px solid #f0c14b; color: #5c3a12; font-size: 12px; font-weight: 600; line-height: 1.4; text-decoration: none; white-space: nowrap; }}
</style>
</head>
<body{root}>
<main>
<h1>{mark}</h1>
{english}
<form action="/search" method="get">
<div class="ask">
<input type="search" name="q" value="{q}"{focus} placeholder="Запрос">
<button class="go" type="submit">Найти</button>
{region_select}
</div>
</form>
{tabs}
{body}
</main>
{footer}
</body>
</html>"#,
        footer = footer(),
        region_select = region_select(region),
        tabs = tab_row(query, region, tab),
    )
}

fn tab_row(query: &str, region: Option<&str>, current: &str) -> String {
    if query.trim().is_empty() {
        return String::new();
    }
    const TABS: &[(&str, &str)] = &[
        ("", "Все"),
        ("news", "Новости"),
        ("gov", "Органы власти"),
        ("jobs", "Работа"),
        ("services", "Услуги"),
        ("medicine", "Медицина"),
        ("goods", "Товары"),
        ("docs", "Документы"),
        ("images", "Картинки"),
        ("video", "Видео"),
    ];
    let mut out = String::from("<nav class=\"kinds\" aria-label=\"Тип страницы\">");
    for (id, label) in TABS {
        let class = if *id == current { " class=\"on\"" } else { "" };
        out.push_str(&format!(
            "<a{class} href=\"{}\">{label}</a>",
            escape(&search_href(query, region, 1, id))
        ));
    }
    out.push_str("</nav>");
    out
}

const FEED_SCRIPT: &str = r#"<script>
document.querySelectorAll("ul.offers").forEach(function (list) {
  var feed = list.getAttribute("data-feed");
  var query = (list.getAttribute("data-q") || "").toLowerCase().split(/\s+/).filter(Boolean);
  if (!feed) return;
  fetch(feed).then(function (response) { return response.text(); }).then(function (xml) {
    var offers = xml.match(/<offer\b[\s\S]*?<\/offer>/gi) || [];
    var shown = 0;
    for (var i = 0; i < offers.length && shown < 5; i++) {
      var name = tagText(offers[i], "name");
      var href = tagText(offers[i], "url");
      var price = tagText(offers[i], "price");
      if (!name || !href) continue;
      var hay = name.toLowerCase();
      if (query.length && !query.every(function (word) { return hay.indexOf(word) !== -1; })) continue;
      var item = document.createElement("li");
      var link = document.createElement("a");
      link.href = href;
      link.textContent = price ? name + " — " + price : name;
      item.appendChild(link);
      list.appendChild(item);
      shown++;
    }
  }).catch(function () {});
});
function tagText(block, tag) {
  var match = block.match(new RegExp("<" + tag + "[^>]*>([\\s\\S]*?)</" + tag + ">", "i"));
  if (!match) return "";
  return match[1].replace(/<!\[CDATA\[|\]\]>/g, "").replace(/<[^>]+>/g, "").trim();
}
</script>"#;

const WIRE_SCRIPT: &str = r#"<script>
document.querySelectorAll("section.wire").forEach(function (box) {
  var button = box.querySelector("button.wire-open");
  var rail = box.querySelector(".rail");
  var row = box.querySelector(".wire-row");
  var query = (box.getAttribute("data-q") || "").toLowerCase().split(/\s+/).filter(Boolean);
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
    fetch("/wire").then(function (response) { return response.json(); }).then(function (items) {
      button.textContent = "Скрыть";
      var shown = 0;
      (items || []).forEach(function (item) {
        if (shown >= 40 || !item || !item.title || !item.url) return;
        var hay = String(item.title).toLowerCase();
        if (query.length && !query.every(function (word) { return hay.indexOf(word) !== -1; })) return;
        var card = document.createElement("a");
        card.className = "card";
        card.href = item.url;
        var title = document.createElement("b");
        title.textContent = item.title;
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
        empty.textContent = "В свежих лентах этого запроса нет.";
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

fn region_select(selected: Option<&str>) -> String {
    let open = if selected.is_some() { " open" } else { "" };
    let mut options = String::from("<option value=\"\">Все регионы</option>");
    for name in region_names() {
        if Some(*name) == selected {
            options.push_str(&format!("<option value=\"{}\" selected>{name}</option>", escape(name)));
        } else {
            options.push_str(&format!("<option value=\"{}\">{name}</option>", escape(name)));
        }
    }
    format!(
        r#"<details class="tune"{open}>
<summary class="settings" aria-label="Настройки поиска"><svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" d="M4 7h16M4 12h16M4 17h16"/><circle cx="9" cy="7" r="1.7" fill="currentColor" stroke="none"/><circle cx="15" cy="12" r="1.7" fill="currentColor" stroke="none"/><circle cx="8" cy="17" r="1.7" fill="currentColor" stroke="none"/></svg></summary>
<div class="filters"><label class="chip"><select class="region" name="region" aria-label="Регион">{options}</select><svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true"><path fill="currentColor" d="M3 17.2V21h3.8L17.8 9.9l-3.8-3.8L3 17.2zm15.1-9.3 1.6-1.6a1 1 0 0 0 0-1.4l-2.3-2.3a1 1 0 0 0-1.4 0l-1.6 1.6 3.7 3.7z"/></svg></label></div>
</details>"#
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

fn pager(query: &str, region: Option<&str>, tab: &str, page: usize, total: usize) -> String {
    let pages = page_count(total);
    if pages <= 1 {
        return String::new();
    }
    let mut out = String::from("<nav class=\"pager\" aria-label=\"Страницы выдачи\">");
    if page > 1 {
        out.push_str(&format!(
            "<a href=\"{}\">Назад</a>",
            escape(&search_href(query, region, page - 1, tab))
        ));
    }
    for number in 1..=pages {
        if number == page {
            out.push_str(&format!("<span class=\"current\">{number}</span>"));
        } else {
            out.push_str(&format!(
                "<a href=\"{}\">{number}</a>",
                escape(&search_href(query, region, number, tab))
            ));
        }
    }
    if page < pages {
        out.push_str(&format!(
            "<a href=\"{}\">Дальше</a>",
            escape(&search_href(query, region, page + 1, tab))
        ));
    }
    out.push_str("</nav>");
    out
}

fn search_href(query: &str, region: Option<&str>, page: usize, tab: &str) -> String {
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
    href
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

pub struct PeerLine {
    pub name: String,
    pub latency_ms: u64,
    pub alive: bool,
    pub buckets: usize,
    pub pages: u32,
}

pub struct OperatorReport {
    pub pages: u64,
    pub queue: u64,
    pub db_bytes: u64,
    pub searches_per_minute: u64,
    pub avg_ms: u64,
    pub peers: Vec<PeerLine>,
    pub volunteers: Vec<String>,
    pub refused: Vec<String>,
    pub rows: Vec<(String, i64, i64, i64, i64)>,
    pub csrf: String,
}

pub fn page_operator(report: &OperatorReport) -> String {
    let mut body = format!(
        "<p>Обезличенный журнал этой ноды. Адреса людей сюда не пишутся.</p>
<ul class=\"load\">
<li>Страниц в индексе: {}</li>
<li>Очередь обхода: {}</li>
<li>База: {} МБ</li>
<li>Поисков за минуту: {}</li>
<li>Среднее время ответа: {} мс</li>
</ul>",
        report.pages,
        report.queue,
        report.db_bytes / 1_048_576,
        report.searches_per_minute,
        report.avg_ms
    );
    if report.peers.is_empty() {
        body.push_str("<p>Соседей в этом запуске нет. Нода отвечает своим индексом.</p>");
    } else {
        body.push_str("<table class=\"log\"><tr><th>Пчела</th><th>Ответ, мс</th><th>Жива</th><th>Бакеты</th><th>Страницы</th></tr>");
        for peer in &report.peers {
            let alive = if peer.alive { "да" } else { "нет" };
            let buckets = if peer.buckets >= 65_536 {
                "весь индекс".to_string()
            } else {
                peer.buckets.to_string()
            };
            body.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{alive}</td><td>{buckets}</td><td>{}</td></tr>",
                escape(&peer.name),
                peer.latency_ms,
                peer.pages
            ));
        }
        body.push_str("</table>");
    }
    body.push_str("<h3>Лёгкие пчёлы</h3>");
    if report.volunteers.is_empty() {
        body.push_str("<p>Подтверждённых сайтов нет.</p>");
    } else {
        body.push_str("<ul class=\"check\">");
        for host in &report.volunteers {
            body.push_str(&format!("<li class=\"ok\">{}</li>", escape(host)));
        }
        body.push_str("</ul>");
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
            "<form class=\"admit\" method=\"post\" action=\"/operator\"><input type=\"hidden\" name=\"csrf\" value=\"{}\"><input name=\"host\" placeholder=\"сайт пчелы\" required><button name=\"action\" value=\"refuse\">Остановить</button><button name=\"action\" value=\"restore\">Вернуть</button></form>",
            escape(&report.csrf)
        ),
    );
    if report.rows.is_empty() {
        body.push_str("<p>Запросов пока нет.</p>");
    } else {
        body.push_str("<table class=\"log\"><tr><th>Запрос</th><th>Обычные</th><th>Слова</th><th>Смысл</th><th>18+</th></tr>");
        for (text, results, words, meaning, adult) in &report.rows {
            body.push_str(&format!(
                "<tr><td>{}</td><td>{results}</td><td>{words}</td><td>{meaning}</td><td>{adult}</td></tr>",
                escape(text)
            ));
        }
        body.push_str("</table>");
    }
    document("", &format!("<article><h2>Журнал запросов</h2>{body}</article>"), false, false, None, "")
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
<p>Одна нода — это пчела. Несколько нод, соединённых в одну сеть, — рой. Улей — региональный вход, домен, с которого люди ищут. Пасека — страна, сеть таких ульев. Робот при обходе по-прежнему называется MDS-Bot.</p>
<h3>Несколько нод</h3>
<p>Сервер, компьютер и телефон говорят одним протоколом. Главной ноды, которой подчиняются остальные, нет. Сервер и компьютер держат крупный кусок индекса и отдают сайт со строкой поиска. Телефон хранит маленький шард и отвечает по тому же ключу. Экран на телефоне — для владельца, чужой трафик через него не идёт.</p>
<p>Запрос идёт к тем пчёлам, которые держат нужный шард, и не больше чем к трём копиям. Если одна копия отвечает заметно медленнее остальных, улей в этот раз её не ждёт. Пока сосед ещё не сообщил свои бакеты, улей спрашивает его как запасной.</p>
<p>Пчела за домашним роутером не открывает порт наружу. Лёгкая пчела сама звонит улью: <code>mds node --lite --site https://ваш-сайт --hive https://45.158.46.52</code>. На сайте лежит файл <code>/.well-known/pchela.txt</code>, в нём одна строка — имя, которое пчела печатает при запуске. Улей читает этот файл и принимает страницы этого сайта и несколько чужих адресов из своей очереди. Общий пароль для входа не нужен. Белый статический IP нужен улью, на который смотрит домен. Домашней пчеле хватает обычного интернета.</p>
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
<p>Полный индекс рядом с сайтом не ставится: модель занимает около 1 ГБ, а общий индекс — десятки гигабайт. Серверу на 2 ядра, 3–4 ГБ памяти и 20 ГБ диска хватает лёгкой пчелы. Она не открывает поиск для чужих людей и не считает векторы. Человека никуда не перекидывает: человек остаётся на улье. Раз в десять минут пчела отдаёт улью несколько своих страниц и до четырёх чужих адресов из очереди улья. Пауза между загрузками не меньше двух секунд, чтобы не мешать сайту.</p>
<pre>mds node --lite --site https://ваш-сайт --hive https://45.158.46.52</pre>
<p>Имя из журнала пчелы кладётся одной строкой в <code>https://ваш-сайт/.well-known/pchela.txt</code>.</p>
<p>В выдаче у такого сайта появляется значок «держит пчелу» и небольшой подъём среди близких результатов. Страница с более точным совпадением слов остаётся выше. Зачем это сайту и чем лёгкая пчела отличается от полной ноды — на странице <a href="/indexing">Индексация</a>.</p>"#,
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
<p>Лёгкая пчела стоит на том же сервере, что и сайт. Она не открывает чужой поиск и не держит модель: модели нужен примерно 1 ГБ, а общий индекс — десятки гигабайт. Раз в десять минут она отдаёт улью свои страницы и помогает обойти несколько чужих адресов из очереди. Человек по-прежнему ищет на улье, сайт не становится чужим входом.</p>
<p>Польза для сайта: потолок в сто страниц снимается, улей принимает страницы, которые отдаёт пчела. В выдаче появляется значок «держит пчелу» и небольшой подъём среди близких ссылок. Более точное совпадение слов остаётся выше. Как поставить:</p>
<pre>mds node --lite --site https://ваш-сайт --hive https://45.158.46.52</pre>
<p>Имя из журнала пчелы кладётся одной строкой в <code>/.well-known/pchela.txt</code> на этом сайте. Новую сборку лёгкой пчелы на сервер кладут вручную и перезапускают службу: сама она файл не меняет.</p>
<h3>Зачем полная нода</h3>
<p>Полная нода — свой улей. Она держит кусок индекса, считает смысл запроса и открывает страницу поиска для людей своего края или своей компании. Журнал запросов остаётся на этой ноде и не уезжает в общий центр. Нужны память под модель и диск под страницы. Без соседней ноды она отвечает только своим индексом. С соседями запрос идёт к тем, кто держит нужный шард, и не больше чем к трём копиям.</p>
<p>Её ставят, когда нужен свой адрес поиска в той же сети. Лёгкая пчела для этого не подходит: она поиск для людей не открывает. Файл и подключение — на странице <a href="/download">Скачать ноду</a>.</p>
<h3>Зачем шард на телефоне</h3>
<p>Телефон хранит маленькую копию куска индекса. Это запасная копия, а не вход для чужих людей и не прокси: чужой трафик через телефон не идёт, экран нужен владельцу. Обход идёт по Wi-Fi и с зарядки. Если серверная нода встанет, телефон всё ещё может ответить по своему шарду. Сборки для телефона на странице загрузки нет.</p>
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
<p>Яндекс и Гугл отвечают со своих дата-центров. Сбой там гасит выдачу сразу для всех, кто ими пользуется. Здесь индекс разобран по отдельным нодам. Если один улей встанет, остальные продолжают отвечать своим куском. Зачем ставить ноду рядом с сайтом, полную ноду или шард на телефоне — на странице <a href="/indexing">Индексация</a>.</p>
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
<li><a href="{RELEASE_ANDROID}">Android, оболочка владельца</a></li>
</ul>
<p>В программу вписана доверенная нода <a href="https://45.158.46.52/">https://45.158.46.52/</a>. Если адрес соседа не указан, нода подключается к ней сама. Улей запускают с <code>--alone</code>, чтобы он не звонил сам себе. Новый адрес появится в следующей версии.</p>
<p>Полная нода подхватывает новую версию сама, если в сети лежит подписанный файл новее того, что у неё запущен. На странице поиска появляется строка «Нужно обновиться». Архив со страницы загрузки сам программу не подменяет. Лёгкая пчела свой файл не меняет: новую сборку кладут на сервер сайта вручную и перезапускают службу. Телефон публичным входом не является: приложение открывает поиск доверенной ноды и показывает то же сообщение.</p>
<p>Исходный код: <a href="{SOURCE_REPO}">{SOURCE_REPO}</a>.</p>
<h3>Как подключить её к поиску</h3>
<p>После запуска нода открывает свой сайт со строкой поиска. Чтобы она вошла в общую сеть, при старте укажите хотя бы один адрес соседней ноды. Без соседа нода отвечает только своим индексом.</p>
<pre>mds node --listen 0.0.0.0:8080 --seeds urls.txt --follow-external --peers https://45.158.46.52</pre>
<ul>
<li>В urls.txt — стартовые адреса сайтов, по одному в строке.</li>
<li>Публичный улей должен быть доступен из интернета по статическому IP или домену.</li>
<li>Пчела дома или на старом компьютере сидит за динамическим IP. Ей белый адрес не нужен: она сама соединяется с ульем.</li>
</ul>
<pre>mds node --lite --site https://ваш-сайт --hive https://45.158.46.52</pre>
<p>Лёгкая пчела не открывает свою страницу поиска. Она подтверждает сайт файлом <code>/.well-known/pchela.txt</code>: в файле одна строка, имя пчелы из её журнала. Дальше улей берёт страницы этого сайта и даёт пчеле до четырёх чужих адресов из очереди.</p>
<ul>
<li>Своя страница поиска будет на том адресе, который указан в --listen. Для людей снаружи на этот адрес ставят домен.</li>
<li>Телефон к этой кнопке не относится: сборка для телефона здесь не раздаётся, и телефон не является публичным входом.</li>
<li>Обезличенный журнал запросов открывается только с компьютера, где запущена нода, по адресу /operator.</li>
</ul>
<h3>Какую ноду ставить</h3>
<p>Рядом с сайтом достаточно лёгкой пчелы: она не открывает поиск для чужих и отдаёт улью все страницы своего сайта. Полную ноду ставят, когда нужен свой вход в поиск и свой кусок индекса. Телефон хранит маленький шард для владельца и не является публичным входом. Зачем каждая из них — на странице <a href="/indexing">Индексация</a>.</p>
<p><a href="/how">Как устроен поиск</a></p>"#
        ),
    )
}

pub fn page_help() -> String {
    article(
        "Помочь проекту",
        r#"<p>Пчелопоиск — волонтёрский поиск. Выдачу не продают. Деньги нужны на улей: сервер, диск и нормальный канал, чтобы страницы открывались и обход не вставал.</p>
<p>Один публичный улей — это 4 ядра, 8 ГБ памяти и SSD около 100 ГБ. Такой сервер стоит порядка двух тысяч рублей в месяц. Узкий канал за эти деньги не подходит.</p>
<p>Перевод держит инфраструктуру роя. Рекламы в выдаче от этого не появится.</p>
<p>Прямой кнопки оплаты на этой ноде ещё нет. Пока помощь можно передать через лабораторию <a href="https://dartalab.ru/">DARTA</a>. Когда появятся реквизиты, они будут на этой же странице.</p>"#,
    )
}

fn article(title: &str, body: &str) -> String {
    document("", &format!("<article><h2>{}</h2>{body}</article>", escape(title)), false, false, None, "")
}

fn footer() -> String {
    r#"<footer>
<div class="foot">
<nav>
<a href="/">Поиск</a>
<a href="/manifesto">Манифест</a>
<a href="/how">Как устроен</a>
<a href="/for-sites">Сайту</a>
<a href="/indexing">Индексация</a>
<a href="/llms">llms.md</a>
<a href="/download">Скачать ноду</a>
<a href="/add">Добавить сайт</a>
</nav>
<p class="census">В поиске %%SITES%% и %%PAGES%%.</p>
<p class="credit"><span>Разработка креативной лаборатории <a href="https://dartalab.ru/">DARTA</a>.</span><a class="help" href="/help">Помочь проекту</a></p>
</div>
</footer>"#
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

fn badges(words: bool, meaning: bool, volunteer: bool, region: Option<&str>) -> String {
    let mut out = String::new();
    if let Some(region) = region {
        out.push_str(&format!("<span class=\"badge\">{region}</span>", region = escape(region)));
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
            kind: 0,
            media_url: None,
            feed_url: None,
            price: None,
        }
    }

    #[test]
    fn query_is_escaped() {
        let html = render("<script>", &[], 1, None);
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("https://dartalab.ru/"));
        assert!(html.contains("<title>Пчелопоиск</title>"));
        let home = render("", &[], 1, None);
        assert!(home.contains("class=\"home\""));
        assert!(home.contains("Bee search"));
        assert!(home.contains("Пчелопоиск"));
        assert!(home.contains("без фильтра по языку"));
        assert!(home.contains("class=\"c1\""));
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
        assert!(home.contains("class=\"flyer"));
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
        assert!(indexing.contains("Зачем шард на телефоне"));
        assert!(indexing.contains("Если сломается Яндекс или Гугл"));
        assert!(indexing.contains("до ста страниц"));
        assert!(page_manifesto().contains("Если крупный поиск встанет"));
        assert!(page_how().contains("href=\"/indexing\""));
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
        assert!(missing.contains(RELEASE_ANDROID));
        assert!(missing.contains("/download/pchela-linux-x64.tar.gz"));
        assert!(missing.contains("/download/pchela-windows-x64.zip"));
        assert!(missing.contains("забирает модель с улья"));
        assert!(!missing.contains("/peer/model"));
        assert!(missing.contains("https://45.158.46.52/"));
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
            kind: 0,
            media_url: None,
            feed_url: None,
            price: None,
        };
        let closed = render_results("мёд", &[], &[adult.clone()], false, 1, None, None, "");
        assert!(closed.contains("18+"));
        assert!(closed.contains("class=\"results blurred\""));
        assert!(!closed.contains("https://studio.example/v"));
        let open = render_results("мёд", &[], &[adult], true, 1, None, None, "");
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
        );
        let home = html.find("В вашем регионе").unwrap();
        let other = html.find("В других регионах").unwrap();
        assert!(home < other);
        assert!(html.contains("Банки Камышин"));
        assert!(html.contains(">Волгоградская область</option>"));
        assert!(html.contains("aria-label=\"Настройки поиска\""));
        let news = render_results("мёд", &[], &[], false, 1, None, None, "news");
        assert!(news.contains("class=\"wire\""));
        assert!(news.contains("Раскрыть"));
        assert!(news.contains("fetch(\"/wire\")"));
        assert!(!html.contains("class=\"wire\""));
        assert!(html.contains("<details class=\"tune\" open>"));
        let closed = render("банки", &[], 1, None);
        assert!(closed.contains("<details class=\"tune\">"));
        assert!(!closed.contains("<details class=\"tune\" open>"));
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
