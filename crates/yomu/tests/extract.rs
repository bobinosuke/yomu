//! 本文抽出のテスト (Python 版の tests/test_extract.py と tests/test_pages.py)。
use std::io::Read;
use std::path::PathBuf;

use yomu::doc::{Block, Kind, Span};
use yomu::extract::{compact_links, dedupe, extract, normalize_spans, select_by_reference};

const PARA: &str = "将棋は二人で行うボードゲームの一種で、盤上の駒を交互に動かして相手の玉将を詰ませた方が勝ちとなる。";

fn html() -> String {
    let para = PARA.repeat(3);
    format!(
        r#"<html><head><title>将棋入門</title></head><body>
<nav><a href="/">ホーム</a> <a href="/about">サイト案内</a></nav>
<div class="breadcrumbs"><a href="/">トップ</a> &gt; 将棋</div>
<article>
<h1>将棋のルール</h1>
<p>{para}</p>
<p><ruby>本将棋<rt>ほんしょうぎ</rt></ruby>は<a href="/koma">駒</a>を使う。{para}</p>
<p><img src="a.png" alt="将棋盤の図"></p>
<p>{para}詳しくは<a href="https://example.org/rules#top">ルール集</a>を参照。</p>
<ol><li>一つ目の手順を説明する文章です。<ul><li>入れ子の項目です。</li></ul></li><li>二つ目の手順です。</li></ol>
<pre>print("hello")</pre>
<table><tr><th>駒</th><th>動き</th></tr><tr><td>歩</td><td>前に1マス</td></tr></table>
</article>
<footer><a href="/privacy">プライバシーポリシー</a></footer>
</body></html>"#
    )
}

fn all_text(blocks: &[Block]) -> String {
    blocks.iter().map(|b| b.text()).collect::<Vec<_>>().join("\n")
}

fn span(text: &str, link: Option<usize>) -> Span {
    Span { link, ..Span::new(text) }
}

fn p(text: &str) -> Block {
    Block::new(Kind::P, vec![Span::new(text)])
}

#[test]
fn extract_keeps_body_and_drops_chrome() {
    let doc = extract(&html(), "https://example.com/shogi", false);
    let text = all_text(&doc.blocks);
    assert!(doc.note.starts_with("本文抽出"));
    assert_eq!(doc.title, "将棋入門");
    assert!(text.contains("盤上の駒を交互に動かして"));
    assert!(!text.contains("サイト案内") && !text.contains("プライバシーポリシー"));
}

#[test]
fn extract_keeps_links_images_and_ruby() {
    let doc = extract(&html(), "https://example.com/shogi", false);
    let spans: Vec<&Span> = doc.blocks.iter().flat_map(|b| &b.spans).collect();
    assert!(spans.iter().any(|s| s.image && s.text == "［画像: 将棋盤の図］"));
    assert!(doc.links.contains(&"https://example.com/koma".to_string()));
    assert!(doc.links.contains(&"https://example.org/rules#top".to_string()));
    assert!(doc.blocks.iter().any(|b| b.text().contains("本将棋（ほんしょうぎ）")));
    // 残ったブロックで使うリンクだけに番号が振り直されている
    let mut used: Vec<usize> = spans.iter().filter_map(|s| s.link).collect();
    used.sort();
    used.dedup();
    assert_eq!(used, (0..doc.links.len()).collect::<Vec<_>>());
}

#[test]
fn extract_structure() {
    let doc = extract(&html(), "https://example.com/shogi", false);
    for k in [Kind::H, Kind::P, Kind::Li, Kind::Pre, Kind::Table] {
        assert!(doc.blocks.iter().any(|b| b.kind == k), "{k:?} がない");
    }
    let lis: Vec<&Block> = doc.blocks.iter().filter(|b| b.kind == Kind::Li).collect();
    assert_eq!((lis[0].marker.as_str(), lis[0].level), ("1.", 0));
    assert_eq!((lis[1].marker.as_str(), lis[1].level), ("・", 1));
    let table = doc.blocks.iter().find(|b| b.kind == Kind::Table).unwrap();
    assert!(table.rows.as_ref().unwrap()[0][0][0].bold); // th は太字
}

#[test]
fn full_view_keeps_everything() {
    let doc = extract(&html(), "https://example.com/shogi", true);
    let text = all_text(&doc.blocks);
    assert!(doc.note == "全体表示" && doc.full);
    assert!(text.contains("サイト案内") && text.contains("プライバシーポリシー"));
}

#[test]
fn normalize_joins_japanese_across_newlines() {
    let spans = normalize_spans(&[Span::new("  日本\n語の"), span(" 文章 ", Some(0)), Span::new("\n")]);
    assert_eq!(spans.iter().map(|s| s.text.as_str()).collect::<String>(), "日本語の文章");
    assert_eq!(spans.last().unwrap().link, Some(0));
}

#[test]
fn select_by_reference_prefers_dense_run() {
    let body: Vec<Block> = (0..5).map(|i| p(&format!("本文の段落その{i}です。内容のある文章が続きます。"))).collect();
    let nav = p("本文の段落その1です。"); // 本文と同じ文言のナビが離れた場所にある
    let junk: Vec<Block> = (0..4).map(|i| p(&format!("まったく関係のないフッターの文言{i}です。"))).collect();
    let mut blocks = vec![nav];
    blocks.extend(junk.clone());
    blocks.extend(body.clone());
    blocks.extend(junk);
    let reference: String = body.iter().map(|b| b.text()).collect();
    assert_eq!(select_by_reference(&blocks, &reference), Some(body));
}

#[test]
fn compact_links_renumbers() {
    let blocks = vec![Block::new(Kind::P, vec![span("a", Some(3)), span("b", Some(1)), span("c", Some(3))])];
    let links: Vec<String> = ["u0", "u1", "u2", "u3"].iter().map(|s| s.to_string()).collect();
    let (blocks, links) = compact_links(blocks, &links);
    assert_eq!(links, ["u3", "u1"]);
    assert_eq!(blocks[0].spans.iter().map(|s| s.link).collect::<Vec<_>>(), [Some(0), Some(1), Some(0)]);
}

#[test]
fn dedupe_title_repeats_and_consecutive_duplicates() {
    let title = "疑問符の使い方 | 校正サイト";
    let h = |level| Block { level, ..Block::new(Kind::H, vec![Span::new("疑問符の使い方")]) };
    let blocks = vec![
        Block { marker: "・".into(), ..Block::new(Kind::Li, vec![Span::new("疑問符の使い方")]) }, // パンくずの最後の項目
        h(1),
        h(2),
        p("本文です。"),
        p("本文です。"), // 同じ段落が続く
        p("次の段落です。"),
    ];
    let got: Vec<(Kind, String)> = dedupe(blocks, title).iter().map(|b| (b.kind, b.text())).collect();
    assert_eq!(
        got,
        [(Kind::H, "疑問符の使い方".into()), (Kind::P, "本文です。".into()), (Kind::P, "次の段落です。".into())]
    );
}

#[test]
fn xhtml_with_xml_declaration() {
    let html = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        "\n",
        r#"<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Strict//EN" "http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd">"#,
        "\n",
        r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>世界史</title></head>"#,
        "<body><p>ローマ帝国は地中海世界を統一した。</p></body></html>"
    );
    let doc = extract(html, "https://example.com/", true);
    assert_eq!(doc.title, "世界史");
    assert!(doc.blocks.iter().any(|b| b.text().contains("地中海世界")));
}

#[test]
fn noscript_inside_paragraph_is_dropped() {
    let doc = extract("<p>本文<noscript><p>広告</p></noscript>です。</p>", "https://example.com/", true);
    assert!(!all_text(&doc.blocks).contains("広告"));
}

// ---- 実在の日本語ページ (2026-09-28 に保存) での本文抽出の回帰テスト。
// ページの HTML は第三者の著作物なのでリポジトリに入れず、手元に置いたディレクトリ (pages.json と <id>.html.gz) を
// YOMU_TEST_PAGES で渡したときだけ確かめる

#[test]
fn real_pages() {
    let Some(dir) = std::env::var_os("YOMU_TEST_PAGES").map(PathBuf::from) else { return };
    let pages: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("pages.json")).unwrap()).unwrap();
    for page in pages.as_array().unwrap() {
        let id = page["id"].as_str().unwrap();
        let gz = std::fs::read(dir.join(format!("{id}.html.gz"))).unwrap();
        let mut html = String::new();
        flate2::read::GzDecoder::new(&gz[..]).read_to_string(&mut html).unwrap();
        let doc = extract(&html, page["url"].as_str().unwrap(), false);
        let text = all_text(&doc.blocks);
        for s in page["must_contain"].as_array().unwrap() {
            assert!(text.contains(s.as_str().unwrap()), "{id}: 本文が抜けている: {s}");
        }
        for s in page["must_not_contain"].as_array().unwrap() {
            assert!(!text.contains(s.as_str().unwrap()), "{id}: ナビ・広告などが入っている: {s}");
        }
        if let Some(m) = page.get("max_count").and_then(|m| m.as_object()) {
            for (s, n) in m {
                let count = doc.blocks.iter().filter(|b| b.text() == *s).count();
                assert!(count as u64 <= n.as_u64().unwrap(), "{id}: 繰り返しが残っている: {s}");
            }
        }
    }
}
