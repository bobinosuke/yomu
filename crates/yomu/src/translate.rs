//! ページの翻訳 (画面の言語へ)。Firefox の拡張機能 TWP (Translate Web Pages) と Immersive Translate を参考にした。
//!
//! - 翻訳は Google 翻訳の translate_a/t (client=gtx。キーはいらない)。段落を q としてまとめて送る
//!   (Immersive Translate と同じく 1 回に 50 段落まで。TWP・Immersive Translate の文字数の上限に近い長さで区切る)
//! - 段落の中のリンク・太字などは、TWP と同じく書式ごとに <a i=番号> で囲んで送り、返ってきた訳文を番号で元の書式に
//!   戻す。Google は訳すときにタグの順番を入れ替えるので、訳文の語順のまま書式だけを戻す
//! - 訳すのは見出し・段落・箇条書き・引用 (コード・表・区切り線は訳さない)。すでに訳し先の言語の段落 (日本語なら
//!   かな、韓国語ならハングルを含むもの。英語はページが lang="en" ならページごと) と、数字や記号だけの段落は送らない。
//!   Google が訳し先の言語と判定した段落も原文のまま
//! - 対訳 (Bilingual) は Immersive Translate と同じく、原文の段落のすぐ後に訳文を置く。短いもの (24 文字以下・
//!   4 語以下) は同じ行の後ろに付ける
use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::doc::{Block, Document, Kind, Span};
use crate::fetch::Client;

const ENDPOINT: &str = "https://translate.googleapis.com/translate_a/t?client=gtx&dt=t&format=html&sl=auto&tl=";

/// 訳し先の言語 (画面の言語)
pub fn target_lang() -> &'static str {
    crate::i18n::lang().code()
}
/// 1 回に送る段落の数と文字数の上限
const MAX_ITEMS: usize = 50;
const MAX_CHARS: usize = 4000;
/// 対訳で、原文と同じ行の後ろに訳文を付ける短いものの上限 (Immersive Translate の blockMinTextCount / blockMinWordCount)
const INLINE_MAX_CHARS: usize = 24;
const INLINE_MAX_WORDS: usize = 4;

static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<a\s+i=(\d+)>([^<]*)</a>").unwrap());
static ANY_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"</?[a-zA-Z][^>]*>").unwrap());
static LETTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\W\d_]").unwrap());
static URL_ONLY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(https?://|www\.)\S*\s*$").unwrap());
/// 句読点 (直後が空白か文末のもの) と閉じ括弧の前の空白
static SPACE_BEFORE_CLOSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+([,.;:!?](?:\s|$)|[)\]）」』、。])").unwrap());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// 原文を訳文に置き換える
    Replace,
    /// 原文の後に訳文を置く (対訳)
    Bilingual,
}

/// 翻訳する段落の番号。見出し・段落・箇条書き・引用のうち、訳し先の言語でなく、文字 (数字・記号以外) を含むもの
pub fn targets(doc: &Document) -> Vec<usize> {
    let lang = crate::i18n::lang();
    // 英語は文字の種類で見分けられないので、ページの宣言で決める (宣言がなければ Google の判定に任せる)
    if lang == crate::i18n::Lang::En && crate::i18n::Lang::from_locale(&doc.lang) == Some(lang) {
        return Vec::new();
    }
    doc.blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| matches!(b.kind, Kind::H | Kind::P | Kind::Li | Kind::Quote) && !b.image_only())
        .filter(|(_, b)| {
            let t = b.text();
            LETTER.is_match(&t) && !URL_ONLY.is_match(&t) && !in_lang(&t, lang)
        })
        .map(|(i, _)| i)
        .collect()
}

/// 文字の種類から見て、その言語の段落か (日本語はかな、韓国語はハングルを含む。英語は見分けない)
fn in_lang(text: &str, lang: crate::i18n::Lang) -> bool {
    use crate::i18n::Lang;
    match lang {
        Lang::Ja => text.chars().any(|c| matches!(c, '\u{3041}'..='\u{3096}' | '\u{30a1}'..='\u{30fa}')),
        Lang::Ko => text.chars().any(|c| matches!(c, '\u{ac00}'..='\u{d7a3}' | '\u{1100}'..='\u{11ff}' | '\u{3131}'..='\u{318e}')),
        Lang::En => false,
    }
}

/// 同じ書式が続く span をまとめたもの。送るときは 1 つずつ <a i=番号> で囲む
fn groups(spans: &[Span]) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    for s in spans {
        match out.last_mut() {
            Some(last) if Span { text: String::new(), ..last.clone() } == Span { text: String::new(), ..s.clone() } => {
                last.text.push_str(&s.text)
            }
            _ => out.push(s.clone()),
        }
    }
    out
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

/// 送る文字列。書式が 1 つだけならそのまま、複数なら書式ごとに <a i=番号> で囲む (TWP と同じ)
pub fn request_text(b: &Block) -> String {
    let g = groups(&b.spans);
    if g.len() <= 1 {
        return escape(b.text().trim());
    }
    g.iter().enumerate().map(|(i, s)| format!("<a i={i}>{}</a>", escape(&s.text))).collect()
}

/// 訳文を span に戻す。<a i=番号> の中は元のその書式にし、タグの外の文字は書式なしにする。訳文の語順のまま並べる
pub fn response_spans(b: &Block, translated: &str) -> Vec<Span> {
    let g = groups(&b.spans);
    if g.len() <= 1 {
        let base = g.into_iter().next().unwrap_or_default();
        let mut out = vec![Span { text: unescape(&ANY_TAG.replace_all(translated, "")), ..base }];
        tidy_spaces(&mut out);
        return out;
    }
    let mut out = Vec::new();
    let plain = |text: &str, out: &mut Vec<Span>| {
        let text = unescape(&ANY_TAG.replace_all(text, ""));
        if !text.is_empty() {
            out.push(Span::new(text));
        }
    };
    let mut pos = 0;
    for m in TAG.captures_iter(translated) {
        let whole = m.get(0).unwrap();
        plain(&translated[pos..whole.start()], &mut out);
        pos = whole.end();
        let text = unescape(&m[2]);
        match m[1].parse::<usize>().ok().and_then(|i| g.get(i)) {
            Some(style) => out.push(Span { text, ..style.clone() }),
            None => out.push(Span::new(text)),
        }
    }
    plain(&translated[pos..], &mut out);
    tidy_spaces(&mut out);
    out
}

/// Google はタグの前後に空白を入れて返すので、リンクの後ろの読点などの前に空白が残る ("safety , and")。
/// 句読点と閉じ括弧の前の空白を詰める。点の直後に文字が続くもの (.NET など) は句読点とみなさない
fn tidy_spaces(spans: &mut Vec<Span>) {
    let closes = |text: &str| {
        let mut it = text.trim_start().chars();
        match (it.next(), it.next()) {
            (Some(')' | ']' | '）' | '」' | '』' | '、' | '。'), _) => true,
            (Some(',' | '.' | ';' | ':' | '!' | '?'), next) => next.is_none_or(|c| c.is_whitespace() || c.is_ascii_punctuation()),
            _ => false,
        }
    };
    for i in 1..spans.len() {
        if !closes(&spans[i].text) {
            continue;
        }
        spans[i].text = spans[i].text.trim_start().to_string();
        // 前の span の末尾の空白も詰める (空白だけの span は空になる)
        for j in (0..i).rev() {
            spans[j].text = spans[j].text.trim_end().to_string();
            if !spans[j].text.is_empty() {
                break;
            }
        }
    }
    // span の中の空白も詰める ("concurrency ." など)
    for s in spans.iter_mut() {
        if SPACE_BEFORE_CLOSE.is_match(&s.text) {
            s.text = SPACE_BEFORE_CLOSE.replace_all(&s.text, "$1").into_owned();
        }
    }
    spans.retain(|s| !s.text.is_empty());
}

/// 段落を、1 回に送る組に分ける (順番はそのまま)
pub fn batches(doc: &Document, order: &[usize]) -> Vec<Vec<(usize, String)>> {
    let mut out: Vec<Vec<(usize, String)>> = Vec::new();
    let mut chars = 0;
    for &i in order {
        let t = request_text(&doc.blocks[i]);
        let n = t.chars().count();
        if out.last().is_none_or(|b| b.len() >= MAX_ITEMS || (chars + n > MAX_CHARS && !b.is_empty())) {
            out.push(Vec::new());
            chars = 0;
        }
        chars += n;
        out.last_mut().unwrap().push((i, t));
    }
    out
}

/// 1 組を訳す。段落ごとに (訳文, Google が判定した元の言語) を返す
pub fn translate(client: &Client, texts: &[String]) -> Result<Vec<(String, String)>, String> {
    let form: Vec<(&str, &str)> = texts.iter().map(|t| ("q", t.as_str())).collect();
    let r = client.post(&format!("{ENDPOINT}{}", target_lang())).form(&form).send().map_err(|e| e.0)?;
    let status = r.status();
    let body = r.text();
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    parse_response(&body, texts.len())
}

/// 応答 ([["訳文","en"], …]。sl=auto なので段落ごとに判定した言語が付く) を読む
pub fn parse_response(body: &str, n: usize) -> Result<Vec<(String, String)>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| t!("翻訳の応答を読めない: {e}", e = e))?;
    let items = v.as_array().ok_or(t!("翻訳の応答が配列でない"))?;
    let out: Vec<(String, String)> = items
        .iter()
        .map(|it| match it {
            Value::String(s) => (s.clone(), String::new()),
            Value::Array(a) => (
                a.first().and_then(Value::as_str).unwrap_or_default().to_string(),
                a.get(1).and_then(Value::as_str).unwrap_or_default().to_string(),
            ),
            _ => (String::new(), String::new()),
        })
        .collect();
    if out.len() != n {
        return Err(t!("翻訳の応答の数が合わない ({sent} 件送って {got} 件)", sent = n, got = out.len()));
    }
    Ok(out)
}

/// 選んだ文章 (段落ごと) を訳す。訳し先の言語と判定されたものは原文のまま。段落の間は空行でつなぐ
pub fn translate_selection(client: &Client, texts: &[String]) -> Result<String, String> {
    let req: Vec<String> = texts.iter().map(|t| escape(t)).collect();
    let res = translate(client, &req)?;
    Ok(texts.iter().zip(res).map(|(src, (t, lang))| if lang == target_lang() { src.clone() } else { unescape(&t) }).collect::<Vec<_>>().join("\n\n"))
}

/// 訳文を当てはめた文書。done は段落の番号 → 訳文 (Google が訳し先の言語と判定したものは入れない)。
/// 対訳では原文の後に訳文の段落を足すので、元の段落の番号 → 新しい文書での番号 も返す
pub fn apply(source: &Document, done: &HashMap<usize, Vec<Span>>, mode: Mode) -> (Document, Vec<usize>) {
    let mut blocks = Vec::with_capacity(source.blocks.len() + done.len());
    let mut index = Vec::with_capacity(source.blocks.len());
    for (i, b) in source.blocks.iter().enumerate() {
        index.push(blocks.len());
        let Some(spans) = done.get(&i) else {
            blocks.push(b.clone());
            continue;
        };
        match mode {
            Mode::Replace => blocks.push(Block { spans: spans.clone(), ..b.clone() }),
            Mode::Bilingual if is_short(b) => {
                let mut spans_all = b.spans.clone();
                spans_all.push(Span::new("  "));
                spans_all.extend(spans.iter().map(|s| Span { translated: true, ..s.clone() }));
                blocks.push(Block { spans: spans_all, ..b.clone() });
            }
            Mode::Bilingual => {
                blocks.push(b.clone());
                let marker = if b.kind == Kind::Li { String::new() } else { b.marker.clone() };
                blocks.push(Block {
                    spans: spans.iter().map(|s| Span { translated: true, ..s.clone() }).collect(),
                    marker,
                    anchors: Vec::new(),
                    translation: true,
                    ..b.clone()
                });
            }
        }
    }
    (Document { blocks, ..source.clone() }, index)
}

/// 対訳で同じ行の後ろに訳文を付ける短いものか
fn is_short(b: &Block) -> bool {
    let t = b.text();
    let t = t.trim();
    t.chars().count() <= INLINE_MAX_CHARS && t.split_whitespace().count() <= INLINE_MAX_WORDS && !t.contains('\n')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linked(t: &str, l: usize) -> Span {
        Span { link: Some(l), ..Span::new(t) }
    }

    #[test]
    fn tags_map_back_by_index_in_translated_order() {
        let b = Block::new(Kind::P, vec![Span::new("Read the "), linked("documentation", 0), Span::new(" before you start.")]);
        assert_eq!(request_text(&b), "<a i=0>Read the </a><a i=1>documentation</a><a i=2> before you start.</a>");
        let spans = response_spans(&b, "<a i=2>始める前に</a><a i=1>ドキュメント</a><a i=0>をお読みください</a>。");
        assert_eq!(spans.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), ["始める前に", "ドキュメント", "をお読みください", "。"]);
        assert_eq!(spans[1].link, Some(0));
        assert_eq!(spans[0].link, None);
    }

    /// Google がタグの前後に入れた空白を、句読点と閉じ括弧の前では詰める
    #[test]
    fn spaces_before_punctuation_are_removed() {
        let link = |t: &str| Span { link: Some(0), ..Span::new(t) };
        let b = Block::new(Kind::P, vec![Span::new("メモリ"), link("安全性"), Span::new("、並行性")]);
        let text = |spans: Vec<Span>| spans.iter().map(|s| s.text.as_str()).collect::<String>();
        let got = response_spans(&b, "<a i=0>memory </a><a i=1>safety</a> <a i=2>, and concurrency .</a>");
        assert_eq!(text(got), "memory safety, and concurrency.");
        let got = response_spans(&b, "<a i=0>uses </a><a i=1>.NET</a><a i=2> (see [2] )</a>");
        assert_eq!(text(got), "uses .NET (see [2])");
    }

    #[test]
    fn single_style_is_sent_as_plain_text() {
        let b = Block::para("A < B & C");
        assert_eq!(request_text(&b), "A &lt; B &amp; C");
        assert_eq!(response_spans(&b, "A &lt; B &amp; C です")[0].text, "A < B & C です");
    }

    #[test]
    fn response_with_detected_language() {
        let r = parse_response(r#"[["こんにちは世界","en"],["これはテストです。","ja"]]"#, 2).unwrap();
        assert_eq!(r[1], ("これはテストです。".into(), "ja".into()));
        assert!(parse_response(r#"["a"]"#, 2).is_err());
    }

    #[test]
    fn targets_skip_japanese_code_and_symbols() {
        let doc = Document {
            blocks: vec![
                Block::heading("Introduction", 2),
                Block::para("これは日本語です。"),
                Block::new(Kind::Pre, vec![Span::new("let x = 1;")]),
                Block::para("123 / 456"),
                Block::para("https://example.com/a"),
                Block::para("Hello world."),
            ],
            ..Default::default()
        };
        assert_eq!(targets(&doc), [0, 5]);
    }

    #[test]
    fn bilingual_puts_translation_after_original() {
        let doc = Document {
            blocks: vec![Block::heading("Introduction", 2), Block::para("This is a long paragraph that should be translated below.")],
            ..Default::default()
        };
        let done = HashMap::from([(0, vec![Span::new("はじめに")]), (1, vec![Span::new("これは訳文です。")])]);
        let (d, index) = apply(&doc, &done, Mode::Bilingual);
        assert_eq!(d.blocks[0].text(), "Introduction  はじめに"); // 短いものは同じ行に
        assert_eq!(d.blocks[2].text(), "これは訳文です。");
        assert!(d.blocks[2].translation && d.blocks[2].spans[0].translated);
        assert!(d.blocks[0].spans.last().unwrap().translated && !d.blocks[0].spans[0].translated);
        assert_eq!(index, [0, 1]);
        let (d, _) = apply(&doc, &done, Mode::Replace);
        assert_eq!(d.blocks.iter().map(|b| b.text()).collect::<Vec<_>>(), ["はじめに", "これは訳文です。"]);
        assert!(!d.blocks[0].spans[0].translated); // 置き換えは全体が訳文なので色を付けない
    }

    #[test]
    fn batches_respect_limits() {
        let doc = Document { blocks: (0..120).map(|i| Block::para(format!("Paragraph {i}."))).collect(), ..Default::default() };
        let order: Vec<usize> = (0..120).collect();
        let b = batches(&doc, &order);
        assert_eq!(b.iter().map(Vec::len).collect::<Vec<_>>(), [50, 50, 20]);
    }
}
