use mds_types::Hit;

pub const PAGE_SIZE: usize = 25;

pub fn render(query: &str, hits: &[Hit], page: usize, error: Option<&str>) -> String {
    render_results(query, hits, &[], false, page, error)
}

pub fn render_results(
    query: &str,
    hits: &[Hit],
    adult: &[Hit],
    adult_open: bool,
    page: usize,
    error: Option<&str>,
) -> String {
    let page = visible_page(page, hits.len());
    let body = if let Some(error) = error {
        format!("<p class=\"error\">{}</p>", escape(error))
    } else if query.is_empty() {
        "<p class=\"hint\">Поиск по страницам, которые обошла эта пчела. Слова и смысл, на русском и на английском, без фильтра по языку.</p>".into()
    } else if hits.is_empty() && adult.is_empty() {
        "<p class=\"hint\">Ничего не нашлось.</p>".into()
    } else if hits.is_empty() {
        adult_section(query, adult, adult_open)
    } else {
        let start = (page - 1) * PAGE_SIZE;
        let end = (start + PAGE_SIZE).min(hits.len());
        let mut list = format!(
            "<p class=\"stats\">Найдено {}. Страница {page} из {}.</p><ol class=\"results\">",
            hits.len(),
            page_count(hits.len())
        );
        for hit in &hits[start..end] {
            let badges = badges(hit.via_words, hit.via_meaning, hit.volunteer);
            list.push_str(&format!(
                "<li><a href=\"{url}\">{title}</a><div class=\"url\">{url}</div><p>{snippet}</p>{badges}</li>",
                url = escape(&hit.url),
                title = escape(&hit.title),
                snippet = escape(&hit.snippet),
            ));
        }
        list.push_str("</ol>");
        list.push_str(&pager(query, page, hits.len()));
        list.push_str(&adult_section(query, adult, adult_open));
        list
    };
    document(query, &body, true, query.is_empty() && error.is_none())
}

fn wordmark() -> &'static str {
    r##"<a class="logo" href="/" aria-label="Пчела"><svg viewBox="0 0 640 230" role="img">
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
<use class="flower" href="#bloom" transform="translate(168,30)" fill="#f4a7b9"/>
<use class="flower" href="#bloom" transform="translate(248,22)" fill="#fff4c8"/>
<use class="flower" href="#bloom" transform="translate(328,18)" fill="#e7c6ff"/>
<use class="flower" href="#bloom" transform="translate(408,22)" fill="#ffd6e0"/>
<use class="flower" href="#bloom" transform="translate(488,30)" fill="#f4a7b9"/>
<use class="flower" href="#bloom" transform="translate(128,86)" fill="#fff4c8"/>
<use class="flower" href="#bloom" transform="translate(118,142)" fill="#e7c6ff"/>
<use class="flower" href="#bloom" transform="translate(512,86)" fill="#ffd6e0"/>
<use class="flower" href="#bloom" transform="translate(522,142)" fill="#f4a7b9"/>
<use class="flower" href="#bloom" transform="translate(188,198)" fill="#fff4c8"/>
<use class="flower" href="#bloom" transform="translate(278,208)" fill="#e7c6ff"/>
<use class="flower" href="#bloom" transform="translate(368,208)" fill="#ffd6e0"/>
<use class="flower" href="#bloom" transform="translate(458,198)" fill="#f4a7b9"/>
<text x="320" y="142" text-anchor="middle" font-family="Georgia, 'Times New Roman', serif" font-size="92"><tspan class="c1" fill="#e6a817">П</tspan><tspan fill="#1f1a17">ч</tspan><tspan fill="#f0c14b">е</tspan><tspan fill="#c47d12">л</tspan><tspan fill="#5c3a12">а</tspan></text>
<g class="flyer a"><ellipse class="w l" cx="-10" cy="-6" rx="11" ry="5" fill="#fffdf6" stroke="#ead9a0" stroke-width="0.6"/><ellipse class="w r" cx="10" cy="-6" rx="11" ry="5" fill="#fffdf6" stroke="#ead9a0" stroke-width="0.6"/><ellipse cy="2" rx="6.5" ry="8.5" fill="#f6c445"/><path d="M-6 0h12M-6.2 3h12.4M-5.2 6h10.4" stroke="#1f1a17" stroke-width="1.4" stroke-linecap="round"/><circle cy="-7.5" r="3.6" fill="#1f1a17"/></g>
<g class="flyer b"><ellipse class="w l" cx="-8" cy="-5" rx="9" ry="4" fill="#fffdf6" stroke="#ead9a0" stroke-width="0.5"/><ellipse class="w r" cx="8" cy="-5" rx="9" ry="4" fill="#fffdf6" stroke="#ead9a0" stroke-width="0.5"/><ellipse cy="1.5" rx="5.2" ry="7" fill="#f6c445"/><path d="M-4.8 -0.2h9.6M-5 2.4h10M-4.2 5h8.4" stroke="#1f1a17" stroke-width="1.2" stroke-linecap="round"/><circle cy="-6" r="3" fill="#1f1a17"/></g>
<g class="flyer c"><ellipse class="w l" cx="-7" cy="-4" rx="7.5" ry="3.4" fill="#fffdf6" stroke="#ead9a0" stroke-width="0.5"/><ellipse class="w r" cx="7" cy="-4" rx="7.5" ry="3.4" fill="#fffdf6" stroke="#ead9a0" stroke-width="0.5"/><ellipse cy="1" rx="4.4" ry="6" fill="#f6c445"/><path d="M-4 -0.2h8M-4.2 2h8.4M-3.5 4.2h7" stroke="#1f1a17" stroke-width="1.1" stroke-linecap="round"/><circle cy="-5" r="2.6" fill="#1f1a17"/></g>
</svg></a>"##
}

fn document(query: &str, body: &str, autofocus: bool, home: bool) -> String {
    let q = escape(query);
    let focus = if autofocus { " autofocus" } else { "" };
    let root = if home { " class=\"home\"" } else { "" };
    let mark = wordmark();
    let english = if home { "<p class=\"en\">Bee</p>" } else { "" };
    format!(
        r#"<!DOCTYPE html>
<html lang="ru">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Пчела</title>
<style>
body {{ margin: 0; min-height: 100vh; display: flex; flex-direction: column; font-family: Arial, Helvetica, sans-serif; color: #202124; background: #fff; }}
main {{ flex: 1 0 auto; width: 100%; max-width: 680px; margin: 0 auto; padding: 28px 16px 32px; box-sizing: border-box; }}
.logo {{ display: inline-block; line-height: 0; text-decoration: none; }}
.logo svg {{ width: 170px; height: auto; overflow: visible; }}
.home .logo svg {{ width: min(310px, 84vw); }}
.w {{ transform-box: fill-box; transform-origin: center; }}
.w.l {{ animation: flap-l 0.22s ease-in-out infinite alternate; }}
.w.r {{ animation: flap-r 0.22s ease-in-out infinite alternate; }}
@keyframes flap-l {{ from {{ transform: rotate(-18deg); }} to {{ transform: rotate(-42deg); }} }}
@keyframes flap-r {{ from {{ transform: rotate(18deg); }} to {{ transform: rotate(42deg); }} }}
.flyer.a {{ animation: fly-a 6s ease-in-out infinite; }}
.flyer.b {{ animation: fly-b 8s ease-in-out infinite; }}
.flyer.c {{ animation: fly-c 7s ease-in-out infinite; }}
@keyframes fly-a {{
  0% {{ transform: translate(150px, 48px) rotate(-18deg); }}
  25% {{ transform: translate(330px, 16px) rotate(8deg); }}
  50% {{ transform: translate(500px, 70px) rotate(20deg); }}
  75% {{ transform: translate(300px, 188px) rotate(-8deg); }}
  100% {{ transform: translate(150px, 48px) rotate(-18deg); }}
}}
@keyframes fly-b {{
  0% {{ transform: translate(470px, 170px) rotate(14deg); }}
  33% {{ transform: translate(210px, 196px) rotate(-10deg); }}
  66% {{ transform: translate(90px, 90px) rotate(-20deg); }}
  100% {{ transform: translate(470px, 170px) rotate(14deg); }}
}}
@keyframes fly-c {{
  0% {{ transform: translate(200px, 28px) rotate(-8deg); }}
  50% {{ transform: translate(430px, 176px) rotate(16deg); }}
  100% {{ transform: translate(200px, 28px) rotate(-8deg); }}
}}
.home main {{ max-width: 640px; padding-top: 10vh; text-align: center; }}
.home .en {{ margin: 8px 0 28px; color: #70757a; font-size: 13px; letter-spacing: 0.22em; text-transform: uppercase; }}
.home form {{ border: 1px solid #dfe1e5; border-radius: 24px; box-shadow: 0 1px 6px rgba(32,33,36,.16); padding: 0 8px 0 8px; }}
.home input[type=search] {{ border: 0; box-shadow: none; }}
.home input[type=search]:focus {{ border: 0; box-shadow: none; }}
.home button {{ border-radius: 18px; }}
form {{ display: flex; gap: 8px; margin: 12px 0 22px; }}
input[type=search] {{ flex: 1; height: 44px; border: 1px solid #dfe1e5; border-radius: 24px; padding: 0 18px; font-size: 16px; }}
input[type=search]:focus {{ outline: none; border-color: #4285f4; box-shadow: 0 1px 6px rgba(32,33,36,.28); }}
button {{ height: 44px; border: 0; border-radius: 4px; padding: 0 16px; background: #1a73e8; color: #fff; font-size: 14px; cursor: pointer; }}
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
footer {{ flex-shrink: 0; margin-top: auto; border-top: 1px solid #dadce0; background: #f8f9fa; }}
.foot {{ max-width: 680px; margin: 0 auto; padding: 22px 16px 36px; color: #4d5156; font-size: 14px; line-height: 1.5; }}
.foot nav {{ display: flex; flex-wrap: wrap; gap: 14px; margin-bottom: 16px; }}
.foot a {{ color: #1a0dab; }}
a.help {{ display: flex; flex-wrap: wrap; align-items: baseline; justify-content: space-between; gap: 8px 16px; margin: 0 0 16px; padding: 12px 16px; border-radius: 12px; background: #fff8e1; border: 1px solid #f0c14b; color: #5c3a12; font-weight: 600; text-decoration: none; }}
a.help span {{ font-weight: 400; font-size: 13px; color: #7a5a28; }}
</style>
</head>
<body{root}>
<main>
<h1>{mark}</h1>
{english}
<form action="/search" method="get">
<input type="search" name="q" value="{q}"{focus} placeholder="Запрос">
<button type="submit">Найти</button>
</form>
{body}
</main>
{footer}
</body>
</html>"#,
        footer = footer()
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

fn pager(query: &str, page: usize, total: usize) -> String {
    let pages = page_count(total);
    if pages <= 1 {
        return String::new();
    }
    let mut out = String::from("<nav class=\"pager\" aria-label=\"Страницы выдачи\">");
    if page > 1 {
        out.push_str(&format!(
            "<a href=\"{}\">Назад</a>",
            escape(&search_href(query, page - 1))
        ));
    }
    for number in 1..=pages {
        if number == page {
            out.push_str(&format!("<span class=\"current\">{number}</span>"));
        } else {
            out.push_str(&format!(
                "<a href=\"{}\">{number}</a>",
                escape(&search_href(query, number))
            ));
        }
    }
    if page < pages {
        out.push_str(&format!(
            "<a href=\"{}\">Дальше</a>",
            escape(&search_href(query, page + 1))
        ));
    }
    out.push_str("</nav>");
    out
}

fn search_href(query: &str, page: usize) -> String {
    format!("/search?q={}&page={page}", encode_query(query))
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
    pub rows: Vec<(String, i64, i64, i64, i64)>,
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
    document("", &format!("<article><h2>Журнал запросов</h2>{body}</article>"), false, false)
}

pub fn page_how() -> String {
    article(
        "Как устроен поиск",
        r#"<p>Человек открывает страницу ноды, вводит запрос и получает ссылки. Страница сама ничего не хранит: текст уходит в ноду, которая стоит за этим адресом.</p>
<h3>Откуда берутся страницы</h3>
<p>Нода обходит публичные адреса. Она читает robots.txt, строки Sitemap и Crawl-delay, карточку /llms.txt, а если её нет — /llms.md, и обычные страницы .md. Со страницы забирается видимый текст, JSON-LD, микроразметка, Open Graph, Twitter Card, Dublin Core, description, keywords и canonical.</p>
<p>Страница с noindex или заголовком X-Robots-Tag: noindex в индекс не попадает. Ссылка с nofollow в очередь не ставится. Файлы .mdx нода не читает.</p>
<h3>Как выбирается выдача</h3>
<p>Один и тот же запрос ищется двумя способами. Точные слова ищутся по заголовку и тексту. Близкий смысл ищется по вектору страницы. Списки склеиваются, и в выдаче остаётся ссылка, короткий отрывок и отметка «слова», «смысл» или обе сразу.</p>
<p>Буквы «е» и «ё» считаются разными: «мёд» и «мед» — разные запросы. Если во фразе есть намерение купить, известный хост маркетплейса получает штраф, прямой сайт остаётся выше.</p>
<h3>Пчела, рой, улей, пасека</h3>
<p>Одна нода — это пчела. Несколько нод, соединённых в одну сеть, — рой. Улей — региональный вход, домен, с которого люди ищут. Пасека — страна, сеть таких ульев. Робот при обходе по-прежнему называется MDS-Bot.</p>
<h3>Несколько нод</h3>
<p>Сервер, компьютер и телефон говорят одним протоколом. Главной ноды, которой подчиняются остальные, нет. Сервер и компьютер держат крупный кусок индекса и отдают сайт со строкой поиска. Телефон хранит маленький шард и отвечает по тому же ключу. Экран на телефоне — для владельца, чужой трафик через него не идёт.</p>
<p>Запрос идёт к тем пчёлам, которые держат нужный шард, и не больше чем к трём копиям. Если одна копия отвечает заметно медленнее остальных, улей в этот раз её не ждёт. Пока сосед ещё не сообщил свои бакеты, улей спрашивает его как запасной.</p>
<p>Пчела за домашним роутером не открывает порт наружу. Она сама звонит улью: <code>mds node --hive http://адрес-улья:8080 --swarm-key секрет</code>. Улей принимает страницы только с тем же секретом. Белый статический IP нужен улью, на который смотрит домен. Домашней пчеле хватает обычного интернета.</p>
<p>Страницы с нарушением закона в выдачу не попадают. Материалы 18+ стоят отдельным блоком и замкнуты, пока человек не подтвердит возраст. Журнал запросов хранит только текст и числа на этой ноде, без адреса человека, и открывается только с компьютера, где нода запущена.</p>
<p><a href="/download">Скачать ноду и подключить её</a></p>"#,
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
<pre>mds node --lite --site https://ваш-сайт --hive http://адрес-улья:8080 --swarm-key секрет</pre>
<p>В выдаче у такого сайта появляется значок «держит пчелу» и небольшой подъём среди близких результатов. Страница с более точным совпадением слов остаётся выше.</p>"#,
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
<li>Важные адреса записывайте ссылками Markdown: <code>[Магазин](https://ваш-сайт/shop)</code>. Нода поставит их в очередь обхода.</li>
<li>Ссылки на .mdx пропускаются. Текст внутри блока кода в индекс не берётся.</li>
<li>Описание пишите словами, которыми сайт ищут. Английское honey и русское «мёд» — разные слова.</li>
</ul>
<h3>Образец</h3>
<pre># КФХ ТехноВелес

&gt; Фермерское хозяйство в Лотошино: натуральный мёд с пасеки, агротуризм и отдых на ферме.

- [Главная](https://tehnoveles.ru/)
- [Магазин](https://tehnoveles.ru/shop)</pre>
<p>Рядом с важной HTML-страницей можно положить отдельный файл .md с тем же рассказом. Нода прочитает и его. Это не замена карточки в корне.</p>"#,
    )
}

pub fn page_manifesto() -> String {
    article(
        "Манифест",
        r#"<p>Пчела — волонтёрский поиск. Это не коммерческий проект: результаты не продаются, рекламодателя в выдаче нет, и за участие в сети плату не берут.</p>
<h3>Что происходит с обычным поиском</h3>
<p>Поисковый ответ, который собирает ИИ, всё чаще остаётся на странице поиска. Человек получает пересказ и не переходит на сайт. У небольшой компании, фермы или мастерской от такого индекса мало пользы: страницу прочитали, а визита нет.</p>
<p>Маркетплейс закрывает ту же выдачу миллионами карточек. Прямой сайт тонет под чужими товарами.</p>
<ul>
<li>Крупный поиск стоит на чужих дата-центрах. Сбой там гасит выдачу сразу для всех.</li>
<li>Ответ языковой модели берёт примерно первые десять страниц чужой выдачи. Всё, что ниже этого круга, в ответ не входит.</li>
<li>В индексе много мошеннических и спам-сайтов. Сайт действующей компании теряется среди них.</li>
<li>Инфосайты перепродают переход и завалены сгенерированным текстом. Человек ищет компанию, а открывает пересказ.</li>
</ul>
<h3>Что делает Пчела</h3>
<p>Пчела задумана как чистый поиск реальных сайтов компаний: без рекламы и без навязанного мусора. В выдаче остаётся ссылка на страницу, а не чужая карточка и не сгенерированный пересказ. Если в запросе есть намерение купить, известный маркетплейс получает штраф, прямой сайт остаётся выше.</p>
<p>Это попытка собрать другой поиск. Ноду ставит тот, кто хочет. Несколько таких нод держат общий индекс, и компании-владельца у сети нет. Регион может открыть свой адрес и остаться в той же сети.</p>
<p><a href="/download">Поставить свою ноду</a></p>"#,
    )
}

pub fn page_download(release_version: Option<&str>) -> String {
    let file = match release_version {
        Some(version) => format!(
            "<p><a class=\"button\" href=\"/download/pchela.zip\">Скачать Пчелу для Windows, версия {version}</a></p>
<p><a href=\"/download/mds.exe\">Только программа, без библиотеки модели</a></p>
<p>Архив — подписанная программа и urls.txt. Если на этой ноде лежит onnxruntime.dll, библиотека модели тоже попадает в архив.</p>",
        ),
        None => "<p>Подписанный файл для Windows на этой ноде ещё не выложен. Когда оператор опубликует выпуск, на этой странице появится кнопка «Скачать ноду для Windows». Исходный код будет лежать отдельно, в открытом репозитории.</p>"
            .to_string(),
    };
    article(
        "Скачать ноду",
        &format!(
            r#"{file}
<h3>Как подключить её к поиску</h3>
<p>После запуска нода открывает свой сайт со строкой поиска. Чтобы она вошла в общую сеть, при старте укажите хотя бы один адрес соседней ноды. Без соседа нода отвечает только своим индексом.</p>
<pre>mds node --listen 0.0.0.0:8080 --seeds urls.txt --follow-external --swarm-key секрет --peers http://адрес-соседа:8080</pre>
<ul>
<li>В urls.txt — стартовые адреса сайтов, по одному в строке.</li>
<li>Публичный улей должен быть доступен из интернета по статическому IP или домену.</li>
<li>Пчела дома или на старом компьютере сидит за динамическим IP. Ей белый адрес не нужен: она сама соединяется с ульем.</li>
</ul>
<pre>mds node --seeds urls.txt --follow-external --hive http://адрес-улья:8080 --swarm-key секрет</pre>
<ul>
<li>Своя страница поиска будет на том адресе, который указан в --listen. Для людей снаружи на этот адрес ставят домен.</li>
<li>Телефон к этой кнопке не относится: сборка для телефона здесь не раздаётся, и телефон не является публичным входом.</li>
<li>Обезличенный журнал запросов открывается только с компьютера, где запущена нода, по адресу /operator.</li>
</ul>
<p><a href="/how">Как устроен поиск</a></p>"#
        ),
    )
}

pub fn page_help() -> String {
    article(
        "Помочь проекту",
        r#"<p>Пчела — волонтёрский поиск. Выдачу не продают. Деньги нужны на улей: сервер, диск и нормальный канал, чтобы страницы открывались и обход не вставал.</p>
<p>Один публичный улей — это 4 ядра, 8 ГБ памяти и SSD около 100 ГБ. Такой сервер стоит порядка двух тысяч рублей в месяц. Узкий канал за эти деньги не подходит.</p>
<p>Перевод держит инфраструктуру роя. Рекламы в выдаче от этого не появится.</p>
<p>Прямой кнопки оплаты на этой ноде ещё нет. Пока помощь можно передать через лабораторию <a href="https://dartalab.ru/">DARTA</a>. Когда появятся реквизиты, они будут на этой же странице.</p>"#,
    )
}

fn article(title: &str, body: &str) -> String {
    document("", &format!("<article><h2>{}</h2>{body}</article>", escape(title)), false, false)
}

fn footer() -> String {
    r#"<footer>
<div class="foot">
<a class="help" href="/help">Помочь проекту<span>на сервер и канал улья</span></a>
<nav>
<a href="/">Поиск</a>
<a href="/manifesto">Манифест</a>
<a href="/how">Как устроен</a>
<a href="/for-sites">Сайту</a>
<a href="/llms">llms.md</a>
<a href="/download">Скачать ноду</a>
<a href="https://dartalab.ru/">dartalab.ru</a>
</nav>
<p>Разработка креативной лаборатории <a href="https://dartalab.ru/">DARTA</a>.</p>
</div>
</footer>"#
        .to_string()
}

fn badges(words: bool, meaning: bool, volunteer: bool) -> String {
    let mut out = String::new();
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
        }
    }

    #[test]
    fn query_is_escaped() {
        let html = render("<script>", &[], 1, None);
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("https://dartalab.ru/"));
        assert!(html.contains("<title>Пчела</title>"));
        let home = render("", &[], 1, None);
        assert!(home.contains("class=\"home\""));
        assert!(home.contains("Bee"));
        assert!(home.contains("без фильтра по языку"));
        assert!(home.contains("class=\"c1\""));
        assert!(home.contains("<svg"));
        assert!(home.contains("Помочь проекту"));
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
        assert!(!html.contains("id=\"how\""));
    }

    #[test]
    fn info_pages_explain_llms_and_the_node_file() {
        let llms = page_llms();
        assert!(llms.contains("/llms.txt"));
        assert!(llms.contains("/llms.md"));
        assert!(llms.contains("400 символов"));
        assert!(llms.contains("натуральный мёд"));
        let missing = page_download(None);
        assert!(missing.contains("ещё не выложен"));
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
        };
        let closed = render_results("мёд", &[], &[adult.clone()], false, 1, None);
        assert!(closed.contains("18+"));
        assert!(closed.contains("class=\"results blurred\""));
        assert!(!closed.contains("https://studio.example/v"));
        let open = render_results("мёд", &[], &[adult], true, 1, None);
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
}
