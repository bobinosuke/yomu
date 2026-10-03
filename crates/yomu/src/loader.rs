//! 取得した内容を、種類 (HTML・テキスト・PDF・その他) に応じて表示用の Document にする。
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use crate::pytext;
use crate::doc::{Block, Document, Kind};
use crate::extract::extract;
use crate::fetch::Response;
use crate::pages::{message_page, text_page};
use crate::urls::{percent_bytes, unquote, urlsplit};

static SENTENCE_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[。．.！？!?：:」』）)]$").unwrap());
static SCRIPT_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<script\b([^>]*)>").unwrap());
static SCRIPT_TYPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)\btype\s*=\s*["']?([^"'\s>]+)"#).unwrap());
static NOSCRIPT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<noscript[^>]*>(.*?)</noscript>").unwrap());

/// ブラウザで開くよう案内する目安: 取り出せた本文がこれより短い (空白を除いた文字数)
const BROWSER_MAX_TEXT: usize = 300;
/// <noscript> で JavaScript を求めているページなら、本文がこれより短ければ案内する
const BROWSER_MAX_TEXT_NOSCRIPT: usize = 1000;

/// 本文の文字数 (空白を除く)
pub fn text_len(doc: &Document) -> usize {
    doc.blocks.iter().map(|b| b.text().chars().filter(|c| !c.is_whitespace()).count()).sum()
}

/// JavaScript なしでは読めなさそうなページか (普段のブラウザで開くよう案内する)。JavaScript のスクリプトがあり、
/// 取り出せた本文がほとんどないか、<noscript> で JavaScript を有効にするよう求めていて本文が短いもの
pub fn needs_browser(r: &Response, doc: &Document) -> bool {
    if !is_html(r) {
        return false;
    }
    let html = r.text();
    let has_script = SCRIPT_TAG.captures_iter(html).any(|c| {
        let ty = SCRIPT_TYPE.captures(&c[1]).map(|t| t[1].to_ascii_lowercase()).unwrap_or_default();
        ty.is_empty() || ty.contains("javascript") || ty == "module"
    });
    if !has_script {
        return false;
    }
    let n = text_len(doc);
    let asks = NOSCRIPT.captures_iter(html).any(|c| c[1].to_ascii_lowercase().contains("javascript"));
    n < BROWSER_MAX_TEXT || (asks && n < BROWSER_MAX_TEXT_NOSCRIPT)
}

pub fn downloads_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Downloads")
}

/// catch_unwind で受け止めた panic の内容
pub fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "panic".into())
}

fn is_html(r: &Response) -> bool {
    let mime = r.mime();
    mime == "text/html" || mime == "application/xhtml+xml" || mime.is_empty()
}


/// 取得結果から文書を作る (設定は使わない。テストと同じ結果)
pub fn to_document(r: &Response, full: bool) -> Document {
    to_document_with(r, full, crate::settings::Settings::default())
}

/// 今の設定 (広告・追跡のリンクを消す、画像を表示する) で文書を作る (画面と --dump 用)
pub fn to_document_for_view(r: &Response, full: bool) -> Document {
    build_document(r, full, crate::settings::get(), true)
}

pub fn to_document_with(r: &Response, full: bool, s: crate::settings::Settings) -> Document {
    build_document(r, full, s, false)
}

/// view なら画面で開くとき (ページの入力欄も拾い、画像の設定を使う)
fn build_document(r: &Response, full: bool, s: crate::settings::Settings, view: bool) -> Document {
    let mime = r.mime();
    let mut doc = if is_html(r) && view {
        crate::extract::extract_for_view(r.text(), &r.url, full, s.images)
    } else if is_html(r) {
        extract(r.text(), &r.url, full)
    } else if mime == "application/pdf" {
        pdf_page(r)
    } else if mime.starts_with("text/") || ["json", "xml", "javascript"].iter().any(|e| mime.ends_with(e)) {
        text_page(&r.url, r.text())
    } else {
        save_download(r, &downloads_dir())
    };
    // 設定でオンなら、広告・追跡のリンクを除く
    if s.block_trackers {
        let n = crate::trackers::clean(&mut doc);
        if n > 0 {
            let removed = t!("広告・追跡 {n} 件を除去", n = n);
            doc.note = if doc.note.is_empty() { removed } else { format!("{} | {removed}", doc.note) };
        }
    }
    if r.status >= 400 {
        // サイトのエラーページも本文として見せ、状態行にステータスを出す
        doc.note = if doc.note.is_empty() { format!("HTTP {}", r.status) } else { format!("HTTP {} | {}", r.status, doc.note) };
        if doc.blocks.is_empty() {
            doc.blocks = vec![Block::para(t!("HTTP {status} のエラーが返りました。", status = r.status))];
        }
    }
    doc
}

// ---- PDF

/// PDF から文字を取り出し、ページごとに段落に分けて表示する。
/// 文字の取り出しは pdf-extract (Python 版の pypdf とは、空白や改行の入り方が少し違う)
pub fn pdf_page(r: &Response) -> Document {
    let content = r.content.clone();
    // 壊れた PDF で pdf-extract が panic することがあるので、ここで受け止める
    let result = std::panic::catch_unwind(move || -> Result<(Vec<String>, Option<String>), String> {
        let pages = pdf_extract::extract_text_from_mem_by_pages(&content).map_err(|e| e.to_string())?;
        let title = pdf_extract::Document::load_mem(&content).ok().and_then(|d| pdf_title(&d));
        Ok((pages, title))
    });
    let (pages, title) = match result {
        Ok(Ok(x)) => x,
        Ok(Err(e)) => return message_page(&r.url, &filename(r), &t!("PDF を読めませんでした: {e}", e = e), ""),
        Err(_) => return message_page(&r.url, &filename(r), t!("PDF を読めませんでした: 壊れた PDF です"), ""),
    };
    let title = title.filter(|t| !t.is_empty()).unwrap_or_else(|| filename(r));
    let mut blocks = Vec::new();
    let words = pdf_words(&pages);
    for (n, text) in pages.iter().enumerate() {
        blocks.push(Block::heading(t!("{n} ページ", n = n + 1), 3));
        blocks.extend(pdf_paragraphs(&join_pdf_lines(&clean_pdf_text(text), &words)).into_iter().map(Block::para));
    }
    if !blocks.iter().any(|b| b.kind == Kind::P) {
        blocks.push(Block::para(t!("文字を取り出せませんでした (画像だけの PDF の可能性があります)。")));
    }
    Document { url: r.url.clone(), title, blocks, note: t!("PDF {n} ページ", n = pages.len()), ..Default::default() }
}

fn pdf_title(d: &pdf_extract::Document) -> Option<String> {
    let info = d.trailer.get(b"Info").ok()?;
    let info = match info {
        pdf_extract::Object::Reference(id) => d.get_object(*id).ok()?,
        o => o,
    };
    let title = info.as_dict().ok()?.get(b"Title").ok()?;
    let title = match title {
        pdf_extract::Object::Reference(id) => d.get_object(*id).ok()?,
        o => o,
    };
    let bytes = title.as_str().ok()?;
    Some(decode_pdf_text(bytes))
}

/// PDF の文字列 (UTF-16BE か PDFDocEncoding。後者は Latin-1 とほぼ同じなのでそれで読む)
fn decode_pdf_text(b: &[u8]) -> String {
    if let Some(rest) = b.strip_prefix(&[0xfe, 0xff]) {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = b.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    b.iter().map(|&c| c as char).collect()
}

/// 日本語の文字 (かな・漢字・全角の記号。全角の空白は除く)
fn is_ja(c: char) -> bool {
    matches!(c, '\u{3001}'..='\u{303f}' | '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{9fff}' | '\u{f900}'..='\u{faff}' | '\u{ff01}'..='\u{ffef}')
}

/// PDF から取り出した文字を整える。pdf-extract は字の間隔から空白を入れるので、日本語の文字と、日本語の文字・数字・英字の間に
/// 余計な半角の空白が入る (「令和5年 10 月1日」→「令和5年10月1日」)。全角の空白は意図した字下げなので残す。
/// 表の部分などに混ざる制御文字 (改行・タブ以外) も除く
pub fn clean_pdf_text(text: &str) -> String {
    let chars: Vec<char> = text.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if matches!(c, ' ' | '\u{a0}') {
            let mut j = i;
            while j < chars.len() && matches!(chars[j], ' ' | '\u{a0}') {
                j += 1;
            }
            let (prev, next) = (out.chars().last(), chars.get(j).copied());
            let joinable = |a: char, b: char| (is_ja(a) && (is_ja(b) || b.is_ascii_alphanumeric())) || (a.is_ascii_alphanumeric() && is_ja(b));
            if let (Some(p), Some(n)) = (prev, next)
                && joinable(p, n)
            {
                i = j;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// 文書に出てくる英単語 (小文字)。行末のハイフンを外してつなぐかを決めるのに使う
pub fn pdf_words(pages: &[String]) -> std::collections::HashSet<String> {
    pages
        .iter()
        .flat_map(|p| p.split(|c: char| !c.is_ascii_alphabetic()))
        .filter(|w| w.len() > 1)
        .map(|w| w.to_ascii_lowercase())
        .collect()
}

/// PDF の行のつなぎ方を直す (段落に分ける前)。
/// - pdf-extract は日本語の文書で行と行の間に空行を入れることがあり、そのままでは 1 行ごとに段落が分かれる。
///   前の行が本文の行 (ページの行の長さの中央値の 8 割以上。短い見出しの行は除く) で、文の終わり (「。」など) で終わらず、
///   前後のどちらかが日本語の文字なら、その空行は行の折り返しとして除く
/// - 行末のハイフンで分けた英単語をつなぐ。ハイフンを外した形が文書のどこかにあれば外し (「transduc-」と「tion」→
///   「transduction」)、なければ残す (「English-」と「to-German」→「English-to-German」)。words は pdf_words
pub fn join_pdf_lines(text: &str, words: &std::collections::HashSet<String>) -> String {
    const SENTENCE_END: &str = "。．！？!?」』）)";
    let lines: Vec<&str> = text.split('\n').collect();
    let mut lens: Vec<usize> = lines.iter().map(|l| l.trim().chars().count()).filter(|&n| n > 0).collect();
    lens.sort_unstable();
    let body_len = lens.get(lens.len() / 2).copied().unwrap_or(0) * 4 / 5;
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        // 空行 1 つをはさんで続く日本語の行
        if line.trim().is_empty()
            && let (Some(prev), Some(next)) = (out.last(), lines.get(i + 1))
            && !next.trim().is_empty()
        {
            let (a, b) = (prev.trim_end().chars().last(), next.trim_start().chars().next());
            if let (Some(a), Some(b)) = (a, b)
                && !SENTENCE_END.contains(a)
                && (is_ja(a) || is_ja(b))
                && prev.trim().chars().count() >= body_len
            {
                i += 1;
                continue;
            }
        }
        // 行末のハイフンで分けた英単語をつなぐ
        if let Some(prev) = out.last_mut() {
            let p = prev.trim_end();
            let mut it = p.chars().rev();
            if it.next() == Some('-')
                && it.next().is_some_and(|c| c.is_ascii_alphabetic())
                && line.trim_start().chars().next().is_some_and(|c| c.is_ascii_lowercase())
            {
                let head = &p[..p.len() - 1];
                let first: String = head.chars().rev().take_while(|c| c.is_ascii_alphabetic()).collect::<Vec<_>>().into_iter().rev().collect();
                let rest: String = line.trim_start().chars().take_while(|c| c.is_ascii_alphabetic()).collect();
                let whole = format!("{first}{rest}").to_ascii_lowercase();
                let joined = if words.contains(&whole) { format!("{head}{}", line.trim_start()) } else { format!("{p}{}", line.trim_start()) };
                *prev = joined;
                i += 1;
                continue;
            }
        }
        out.push(line.to_string());
        i += 1;
    }
    out.join("\n")
}

/// Python の str.splitlines の区切り
/// PDF の文字は見た目の行ごとに改行されているので、文の途中の改行はつなぐ。
/// 空行か、句点などで終わる行で段落を区切る。日本語どうしの行はそのまま、英語どうしは空白でつなぐ。
pub fn pdf_paragraphs(text: &str) -> Vec<String> {
    let mut paras = Vec::new();
    let mut cur = String::new();
    for line in pytext::splitlines(text).into_iter().map(pytext::strip) {
        if line.is_empty() {
            if !cur.is_empty() {
                paras.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if cur.chars().last().is_some_and(|c| c.is_ascii()) && line.chars().next().is_some_and(|c| c.is_ascii()) {
            cur.push(' ');
        }
        cur.push_str(line);
        // 短い行が句点で終わるなら段落の終わりとみなす
        if SENTENCE_END.is_match(line) && line.chars().count() < 40 {
            paras.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        paras.push(cur);
    }
    paras
}

// ---- その他のファイル

/// Path(name).name と同じく、最後の要素 (末尾の / は無視する)
fn last_component(p: &str) -> String {
    p.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string()
}

pub fn filename(r: &Response) -> String {
    if !r.disposition.is_empty()
        && let Some(name) = disposition_filename(&r.disposition)
    {
        let name = last_component(&name);
        if !name.is_empty() {
            return name;
        }
    }
    let name = last_component(&unquote(&urlsplit(&r.url, "").path));
    if name.is_empty() { "download".into() } else { name }
}

/// Content-Disposition のファイル名。Python の email.message.get_filename と同じく、
/// filename (引用符付きも可) を先に、なければ filename* (RFC 2231 / 5987。filename*0 などの続きも) を見る
fn disposition_filename(value: &str) -> Option<String> {
    let params = split_params(value);
    if let Some((_, v)) = params.iter().find(|(k, _)| k == "filename") {
        return Some(v.clone());
    }
    let mut parts: Vec<(usize, bool, String)> = Vec::new();
    for (k, v) in &params {
        let Some(rest) = k.strip_prefix("filename*") else { continue };
        let (num, encoded) = match rest {
            "" => (0, true),
            r => match r.strip_suffix('*') {
                Some(n) => (n.parse().ok()?, true),
                None => (r.parse().ok()?, false),
            },
        };
        parts.push((num, encoded, v.clone()));
    }
    if parts.is_empty() {
        return None;
    }
    parts.sort_by_key(|p| p.0);
    let mut charset = String::from("utf-8");
    let mut bytes = Vec::new();
    for (i, (_, encoded, v)) in parts.iter().enumerate() {
        let mut v = v.as_str();
        if *encoded {
            if i == 0 && let Some((cs, rest)) = v.split_once('\'') {
                charset = cs.to_string();
                v = rest.split_once('\'').map_or(rest, |x| x.1);
            }
            bytes.extend(percent_bytes(v));
        } else {
            bytes.extend(v.as_bytes());
        }
    }
    let enc = encoding_rs::Encoding::for_label(charset.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    Some(enc.decode_without_bom_handling(&bytes).0.into_owned())
}

/// "attachment; filename=\"a b.csv\"; x=1" を (小文字の名前, 値) の列に。引用符の中の ; と \ のエスケープを扱う
fn split_params(value: &str) -> Vec<(String, String)> {
    let mut items = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            '\\' if quoted => {
                cur.push(c);
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            ';' if !quoted => items.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    items.push(cur);
    items
        .iter()
        .skip(1)
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            let v = v.trim();
            let v = match v.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
                Some(inner) => {
                    let mut out = String::new();
                    let mut it = inner.chars();
                    while let Some(c) = it.next() {
                        if c == '\\' {
                            if let Some(n) = it.next() {
                                out.push(n);
                            }
                        } else {
                            out.push(c);
                        }
                    }
                    out
                }
                None => v.to_string(),
            };
            Some((k.trim().to_ascii_lowercase(), v))
        })
        .collect()
}

/// 表示できない形式はダウンロードフォルダに保存する (同名のファイルがあれば番号を付ける)
pub fn save_download(r: &Response, folder: &Path) -> Document {
    let name = filename(r);
    let path = Path::new(&name);
    let (stem, suffix) = match (path.file_stem(), path.extension()) {
        (Some(s), Some(e)) if !e.is_empty() => (s.to_string_lossy().into_owned(), format!(".{}", e.to_string_lossy())),
        _ => (name.clone(), String::new()),
    };
    let mut dest = folder.join(&name);
    let mut n = 1;
    while dest.exists() {
        dest = folder.join(format!("{stem} ({n}){suffix}"));
        n += 1;
    }
    let mime = r.mime();
    if let Err(e) = std::fs::create_dir_all(folder).and_then(|_| std::fs::write(&dest, &r.content)) {
        return message_page(&r.url, &name, &t!("この形式 ({mime}) は表示できず、保存にも失敗しました: {e}", mime = mime, e = e), "");
    }
    message_page(
        &r.url,
        &name,
        &t!("この形式 ({mime}) は表示できないので保存しました: {path}", mime = mime, path = dest.display()),
        t!("ダウンロード"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html(body: &str) -> Response {
        Response::new("https://e.com/", 200, "text/html", body.as_bytes().to_vec())
    }

    #[test]
    fn needs_browser_only_for_script_pages_with_little_text() {
        let long = "長い本文です。".repeat(100);
        let page = |b: &str| {
            let r = html(b);
            let d = to_document(&r, false);
            needs_browser(&r, &d)
        };
        assert!(page(r#"<html><body><div id="app"></div><script src="app.js"></script></body></html>"#));
        assert!(!page(r#"<html><body><div id="app"></div></body></html>"#), "スクリプトがなければ動かしても変わらない");
        assert!(!page(&format!(r#"<html><body><p>{long}</p><script src="a.js"></script></body></html>"#)), "本文が十分ある");
        assert!(!page(r#"<html><body><p>x</p><script type="application/ld+json">{}</script></body></html>"#), "データ用のスクリプトだけ");
        let medium = "本文です。".repeat(80);
        assert!(page(&format!(r#"<html><body><p>{medium}</p><noscript>Please enable JavaScript</noscript><script>1</script></body></html>"#)));
    }

    #[test]
    fn disposition_forms() {
        assert_eq!(disposition_filename("attachment; filename=\"report.csv\"").as_deref(), Some("report.csv"));
        assert_eq!(disposition_filename("attachment; filename*=UTF-8''%E5%B0%86%E6%A3%8B.pdf").as_deref(), Some("将棋.pdf"));
        assert_eq!(disposition_filename("inline; filename=a.txt; filename*=UTF-8''b.txt").as_deref(), Some("a.txt"));
        assert_eq!(disposition_filename("attachment").as_deref(), None);
    }

    #[test]
    fn pdf_text_spaces_between_japanese_are_removed() {
        assert_eq!(clean_pdf_text("令和5年 10 月1日現在、1億 2,435 万人"), "令和5年10月1日現在、1億2,435万人");
        assert_eq!(clean_pdf_text("第 1 章 Google の 検索"), "第1章Googleの検索");
        assert_eq!(clean_pdf_text("1　高齢化の現状"), "1　高齢化の現状"); // 全角の空白は残す
        assert_eq!(clean_pdf_text("The model is based on 2 layers."), "The model is based on 2 layers.");
        assert_eq!(clean_pdf_text("総数\u{8}6,386\n男"), "総数6,386\n男");
    }

    #[test]
    fn pdf_lines_are_joined() {
        let none = std::collections::HashSet::new();
        // 日本語の本文の行の間の空行は折り返し。見出しの後・文の終わりの後の空行と、英語の段落の間の空行は残す
        let ja = "1　高齢化の現状\n\n我が国の65歳以上人口は、昭和25年には総\n\n人口の5％に満たなかったが、昭和45年に7％\n\nを超え、さらに、平成6年には14％を超えた。\n\n高齢化率はその後も上昇を続け、令和5年10\n\n月1日現在、29.1％に達している。";
        assert_eq!(
            pdf_paragraphs(&join_pdf_lines(ja, &none)),
            [
                "1　高齢化の現状",
                "我が国の65歳以上人口は、昭和25年には総人口の5％に満たなかったが、昭和45年に7％を超え、さらに、平成6年には14％を超えた。",
                "高齢化率はその後も上昇を続け、令和5年10月1日現在、29.1％に達している。",
            ]
        );
        assert_eq!(join_pdf_lines("1 Introduction\n\nRecurrent networks", &none), "1 Introduction\n\nRecurrent networks");
        // 行末のハイフン: 文書にハイフンなしの形があれば外し、なければ残す
        let words: std::collections::HashSet<String> = ["transduction".to_string()].into();
        assert_eq!(join_pdf_lines("modeling and transduc-\ntion models", &words), "modeling and transduction models");
        assert_eq!(join_pdf_lines("the English-\nto-German task", &words), "the English-to-German task");
        assert_eq!(join_pdf_lines("see Section 3-\nResults", &words), "see Section 3-\nResults");
    }

    #[test]
    fn splitlines_like_python() {
        assert_eq!(pytext::splitlines("a\r\nb\rc\n\nd"), ["a", "b", "c", "", "d"]);
    }
}
