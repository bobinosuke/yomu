//! DuckDuckGo の HTML 版で検索し、結果を Document として返す (API キー不要)。
//!
//! 結果の末尾に「次の検索結果 →」のリンクを置く。DuckDuckGo の次のページは、結果ページにある「Next」フォームの
//! 値をそのまま送ると返ってくる。そのフォームの値を yomu-search: で始まる内部用の URL に詰めてリンクにする。
//! 次のページには前のページと同じ結果が混ざる (2026-09 に試すと 15 件中 4 件) ので、表示済みの URL は除く。
//!
//! 2 ページ目以降には「← 前の検索結果」も置く。前のページを取り直すにはそのページのフォームの値が要り、
//! リンクに詰めていくとページを進めるたびに入れ子で長くなるので、リンクには「何ページ目へ戻るか」(GOTO) だけを入れ、
//! TUI がタブの履歴から同じ検索語のそのページを探して開く。
use std::sync::LazyLock;

use scraper::{ElementRef, Html, Selector};

use crate::pytext;
use crate::doc::{Block, Document, Kind, Span};
use crate::fetch::{Client, HttpError};
use crate::urls::{parse_qs_first, quote_plus, urlencode, urlsplit};

pub const ENDPOINT: &str = "https://html.duckduckgo.com/html/";
/// DuckDuckGo の .onion の窓口。プライベートモード (Tor) ではこちらを使う (Tor Browser の既定の検索と同じ。
/// html.duckduckgo.com は Tor の出口からの要求を断る)
const ONION: &str = "duckduckgogg42xjoc72x3sjasowoarfbgcmvfimaftt6twagswzczad.onion";

/// 検索に使う窓口
fn endpoint() -> String {
    if crate::fetch::is_private() { format!("https://{ONION}/html/") } else { ENDPOINT.to_string() }
}
pub const PAGE_SCHEME: &str = "yomu-search:"; // 次のページのフォームの値を詰めた内部用の URL
pub const SEEN: &str = "_seen"; // 表示済みの URL (改行区切り)。DuckDuckGo には送らない
pub const PAGE: &str = "_page"; // 何ページ目か。DuckDuckGo には送らない
pub const GOTO: &str = "_goto"; // 「前の検索結果」のリンクで、戻る先のページ番号
/// 「次の検索結果」のリンクの文言
pub fn next_label() -> &'static str {
    t!("次の検索結果 →")
}

/// 「前の検索結果」のリンクの文言
pub fn prev_label() -> &'static str {
    t!("← 前の検索結果")
}
/// 検索の地域 (DuckDuckGo の kl)。画面の言語に合わせる。英語は特定の国に寄せない (wt-wt は地域なし)
fn region() -> &'static str {
    match crate::i18n::lang() {
        crate::i18n::Lang::Ja => "jp-jp",
        crate::i18n::Lang::En => "wt-wt",
        crate::i18n::Lang::Ko => "kr-kr",
    }
}

/// フォームの値。Python の dict と同じく、キーが最初に入った順を保つ
pub type Fields = Vec<(String, String)>;

fn sel(s: &str) -> Selector {
    Selector::parse(s).unwrap()
}

static RESULT: LazyLock<Selector> = LazyLock::new(|| sel("div.result"));
static RESULT_A: LazyLock<Selector> = LazyLock::new(|| sel("a.result__a"));
static SNIPPET: LazyLock<Selector> = LazyLock::new(|| sel(".result__snippet"));
static FORM: LazyLock<Selector> = LazyLock::new(|| sel("form"));
static NEXT_SUBMIT: LazyLock<Selector> = LazyLock::new(|| sel(r#"input[type="submit"][value="Next"]"#));
static HIDDEN: LazyLock<Selector> = LazyLock::new(|| sel(r#"input[type="hidden"]"#));

fn set(fields: &mut Fields, k: &str, v: String) {
    match fields.iter_mut().find(|(key, _)| key == k) {
        Some(x) => x.1 = v,
        None => fields.push((k.to_string(), v)),
    }
}

fn pop(fields: &mut Fields, k: &str) -> Option<String> {
    let i = fields.iter().position(|(key, _)| key == k)?;
    Some(fields.remove(i).1)
}

/// //duckduckgo.com/l/?uddg=... のリダイレクトを外す
pub fn real_url(href: &str) -> String {
    let full = if href.contains("://") { href.to_string() } else { format!("https:{href}") };
    let u = urlsplit(&full, "");
    if (u.netloc.ends_with("duckduckgo.com") || u.netloc == ONION)
        && u.path.starts_with("/l/")
        && let Some((_, v)) = parse_qs_first(&u.query, false).into_iter().find(|(k, _)| k == "uddg")
    {
        return v;
    }
    href.to_string()
}

pub fn page_url(fields: &Fields) -> String {
    format!("{PAGE_SCHEME}?{}", urlencode(fields))
}

pub fn page_fields(url: &str) -> Fields {
    parse_qs_first(&urlsplit(url, "").query, true)
}

/// フォームの値の k の値
pub fn field<'a>(fields: &'a Fields, k: &str) -> Option<&'a str> {
    fields.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str())
}

fn text_content(e: ElementRef) -> String {
    e.text().collect()
}

/// 結果ページにある「Next」フォームの値
fn next_form(d: &Html) -> Option<Fields> {
    for form in d.select(&FORM) {
        if form.select(&NEXT_SUBMIT).next().is_some() {
            let mut out = Fields::new();
            for i in form.select(&HIDDEN) {
                if let Some(name) = i.value().attr("name").filter(|n| !n.is_empty()) {
                    set(&mut out, name, i.value().attr("value").unwrap_or("").to_string());
                }
            }
            return Some(out);
        }
    }
    None
}

/// query を検索する。fields は「次の検索結果」のリンクに詰めたフォームの値 (2 ページ目以降)。
/// with_next=false なら「次の検索結果」のリンクを付けない (--dump 用)。
pub fn search(
    query: &str,
    fields: Option<&Fields>,
    with_next: bool,
) -> Result<Document, HttpError> {
    // 検索は指紋をブラウザに似せない接続で送る (fetch::plain_client)
    static CLIENT: std::sync::LazyLock<Client> = std::sync::LazyLock::new(crate::fetch::plain_client);
    search_with(
        |form| {
            let mut req = CLIENT.post(&endpoint()).header("content-type", "application/x-www-form-urlencoded");
            if crate::fetch::is_private() {
                // Tor Browser で検索の窓口 (/html/) のフォームから送ったときと同じにする
                req = req
                    .header("origin", &format!("https://{ONION}"))
                    .header("referer", &endpoint())
                    .header("sec-fetch-site", "same-origin");
            }
            let r = req.body(urlencode(form)).send()?;
            let status = r.status();
            if status.is_client_error() || status.is_server_error() {
                return Err(HttpError(format!("HTTP {} ({})", status.as_u16(), endpoint())));
            }
            Ok(r.text())
        },
        query,
        fields,
        with_next,
    )
}

/// search の本体。post はフォームの値を DuckDuckGo に送って結果ページの HTML を返す
pub fn search_with(
    post: impl FnOnce(&Fields) -> Result<String, HttpError>,
    query: &str,
    fields: Option<&Fields>,
    with_next: bool,
) -> Result<Document, HttpError> {
    let mut fields = fields.cloned().unwrap_or_default();
    let mut seen: Vec<String> = Vec::new();
    for u in pop(&mut fields, SEEN).unwrap_or_default().split('\n').filter(|u| !u.is_empty()) {
        if !seen.iter().any(|s| s == u) {
            seen.push(u.to_string());
        }
    }
    let page: usize = pop(&mut fields, PAGE).filter(|p| !p.is_empty()).and_then(|p| p.parse().ok()).unwrap_or(1);
    let form = if fields.is_empty() {
        vec![("q".to_string(), query.to_string()), ("kl".to_string(), region().to_string())]
    } else {
        fields
    };
    let html = post(&form)?;
    let d = Html::parse_document(&html);
    let mut blocks = Vec::new();
    let mut links: Vec<String> = Vec::new();
    for res in d.select(&RESULT) {
        if res.value().attr("class").unwrap_or("").contains("result--ad") {
            continue;
        }
        let Some(a) = res.select(&RESULT_A).next() else { continue };
        let url = real_url(a.value().attr("href").unwrap_or(""));
        if seen.contains(&url) || links.contains(&url) {
            continue;
        }
        let idx = links.len();
        links.push(url.clone());
        let title = Span { link: Some(idx), ..Span::new(pytext::strip(&text_content(a)).to_string()) };
        blocks.push(Block { level: 3, ..Block::new(Kind::H, vec![title]) });
        blocks.push(Block::new(Kind::Quote, vec![Span::new(url)]));
        if let Some(snip) = res.select(&SNIPPET).next() {
            blocks.push(Block::para(pytext::strip(&text_content(snip))));
        }
    }
    if blocks.is_empty() {
        // 結果ゼロのほか、アクセス過多で DuckDuckGo が確認ページを返したときもここに来る
        blocks.push(Block::para(t!("検索結果がありませんでした。時間をおいて再度お試しください。")));
    }
    let mut nav = Vec::new();
    if with_next && page > 1 {
        nav.push(Span { link: Some(links.len()), ..Span::new(prev_label()) });
        links.push(page_url(&vec![("q".into(), query.into()), (GOTO.into(), (page - 1).to_string())]));
    }
    if with_next
        && !links.is_empty()
        && let Some(mut nxt) = next_form(&d)
    {
        let shown = links.iter().filter(|u| !u.starts_with(PAGE_SCHEME)).cloned();
        let all: Vec<String> = seen.iter().cloned().chain(shown).collect();
        set(&mut nxt, SEEN, all.join("\n"));
        set(&mut nxt, PAGE, (page + 1).to_string());
        if !nav.is_empty() {
            nav.push(Span::new("  |  "));
        }
        nav.push(Span { link: Some(links.len()), ..Span::new(next_label()) });
        links.push(page_url(&nxt));
    }
    if !nav.is_empty() {
        blocks.push(Block::new(Kind::Hr, vec![]));
        blocks.push(Block::new(Kind::P, nav));
    }
    let title = if page > 1 { t!("検索: {query} ({page} ページ目)", query = query, page = page) } else { t!("検索: {target}", target = query) };
    Ok(Document {
        url: format!("{}?q={}", endpoint(), quote_plus(query)),
        title,
        blocks,
        links,
        note: "DuckDuckGo".into(),
        ..Default::default()
    })
}
