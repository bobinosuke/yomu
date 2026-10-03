//! tests/test_render.py の移植。
use yomu::doc::{Block, Document, Kind, Span};
use yomu::render::{View, cell_len, indents, list_marker, render, to_markdown};

fn span(t: &str) -> Span {
    Span::new(t)
}

fn block(kind: Kind, spans: Vec<Span>) -> Block {
    Block::new(kind, spans)
}

fn doc() -> Document {
    let mut h = block(Kind::H, vec![span("見出し")]);
    h.level = 1;
    let mut li = block(Kind::Li, vec![span("項目")]);
    li.marker = "・".into();
    let mut li2 = block(Kind::Li, vec![span("番号")]);
    li2.level = 1;
    li2.marker = "2.".into();
    let bold = |t: &str| Span { bold: true, ..span(t) };
    let mut table = block(Kind::Table, vec![]);
    table.rows = Some(vec![vec![vec![bold("A")], vec![bold("B")]], vec![vec![span("1")], vec![span("2")]]]);
    Document {
        url: "https://example.com/".into(),
        title: "題名".into(),
        blocks: vec![
            h,
            block(Kind::P, vec![span("本文と"), Span { link: Some(0), ..span("リンク") }, span("と"), Span { code: true, ..span("code") }]),
            li,
            li2,
            block(Kind::Quote, vec![span("引用")]),
            block(Kind::Pre, vec![Span { code: true, ..span("x = 1") }]),
            block(Kind::Hr, vec![]),
            table,
        ],
        links: vec!["https://example.com/a".into()],
        ..Default::default()
    }
}

fn plain(d: &Document, view: &View, width: usize) -> String {
    render(d, view, width).iter().map(|l| l.text() + "\n").collect()
}

#[test]
fn markdown() {
    let md = to_markdown(&doc());
    assert!(md.starts_with("# 題名\n<https://example.com/>\n"));
    assert!(md.contains("## 見出し"));
    assert!(md.contains("本文とリンク[1]と`code`"));
    assert!(md.contains("- 項目\n  2. 番号"));
    assert!(md.contains("| **A** | **B** |\n|---|---|\n| 1 | 2 |"));
    assert!(md.trim_end().ends_with("[1]: https://example.com/a"));
}

#[test]
fn terminal_rendering_follows_claude_code() {
    let out = plain(&doc(), &View::default(), 40);
    assert!(out.contains("- 項目") && out.contains("  b. 番号"));
    assert!(out.contains("▎ 引用"));
    assert!(out.contains('┌') && out.contains('├'));
    assert!(out.contains("---"));
}

#[test]
fn list_markers() {
    let li = |marker: &str, level| Block { kind: Kind::Li, marker: marker.into(), level, ..Default::default() };
    assert_eq!(list_marker(&li("3.", 0)), "3.");
    assert_eq!(list_marker(&li("3.", 1)), "c.");
    assert_eq!(list_marker(&li("4.", 2)), "iv.");
    assert_eq!(list_marker(&li("・", 2)), "-");
}

#[test]
fn hint_labels_are_drawn_before_links_and_tagged() {
    let view = View { hints: Some([(0, "sa".to_string())].into()), typed: "s".into(), ..Default::default() };
    assert!(plain(&doc(), &view, 40).contains("saリンク"));
    let lines = render(&doc(), &View::default(), 40);
    let seg = lines.iter().flat_map(|l| &l.segs).find(|s| s.text == "リンク").unwrap();
    assert_eq!((seg.link, seg.occ, seg.blk), (Some(0), Some(0), Some(1)));
}

#[test]
fn list_items_use_hanging_indent() {
    let mut li = block(Kind::Li, vec![span(&"あ".repeat(30))]);
    li.level = 1;
    li.marker = "・".into();
    let d = Document { url: "https://e.com".into(), title: "t".into(), blocks: vec![li], ..Default::default() };
    let out = plain(&d, &View::default(), 24);
    let lines: Vec<&str> = out.lines().filter(|x| !x.trim().is_empty()).skip(1).collect();
    assert!(lines[0].starts_with("  - ") && lines[1..].iter().all(|x| x.starts_with("    ")));
}

#[test]
fn japanese_wraps_between_characters_with_kinsoku() {
    let d = Document {
        url: "about:x".into(),
        title: "t".into(),
        blocks: vec![block(Kind::P, vec![span("日本語の文章に English が混ざると、空白のない日本語の塊が次の行へ送られていた。")])],
        ..Default::default()
    };
    let out = plain(&d, &View::default(), 20);
    let lines: Vec<&str> = out.lines().map(|x| x.trim_end()).collect();
    // 英単語は途中で切らずに次の行へ送り、空白のない日本語は行幅いっぱいまで詰める
    assert_eq!(lines[1], "English が混ざると、");
    assert_eq!(cell_len(lines[2]), 20);
    assert!(!lines.iter().any(|x| x.starts_with('、') || x.starts_with('。'))); // 句読点を行頭に置かない
}

#[test]
fn article_title_is_shown_once() {
    let d = Document { url: "https://e.com".into(), title: "記事の題".into(), blocks: vec![block(Kind::P, vec![span("本文")])], ..Default::default() };
    assert_eq!(plain(&d, &View::default(), 80).lines().next(), Some("記事の題"));
    let mut h = block(Kind::H, vec![span("記事の題")]);
    h.level = 1;
    let dup = Document { url: "https://e.com".into(), title: "記事の題 | サイト".into(), blocks: vec![h], ..Default::default() };
    assert_eq!(plain(&dup, &View::default(), 80).matches("記事の題").count(), 1);
}

#[test]
fn indent_follows_heading_hierarchy() {
    let h = |t: &str, level| Block { level, ..block(Kind::H, vec![span(t)]) };
    let mut li = block(Kind::Li, vec![span("b")]);
    li.marker = "・".into();
    let blocks = vec![block(Kind::P, vec![span("前文")]), h("大", 2), block(Kind::P, vec![span("a")]), h("小", 4), li, h("大2", 2)];
    // h2 と h4 だけのページなら2段: 見出しは 0 / 2 字、本文はその見出しより 2 字下げる
    assert_eq!(indents(&blocks), [0, 0, 2, 2, 4, 0]);
    let out = plain(&Document { url: "about:x".into(), title: "t".into(), blocks, ..Default::default() }, &View::default(), 80);
    assert!(out.contains("\n  a\n") && out.contains("\n    - b\n"));
}

#[test]
fn italic_and_strike_in_markdown() {
    let d = Document {
        url: "about:x".into(),
        title: "t".into(),
        blocks: vec![block(Kind::P, vec![Span { italic: true, ..span("強調") }, span("と"), Span { strike: true, ..span("削除") }])],
        ..Default::default()
    };
    assert!(to_markdown(&d).contains("*強調*と~~削除~~"));
}
