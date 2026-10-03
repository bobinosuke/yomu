//! tests/test_visual.py の移植。
use yomu::doc::{Block, Document, Kind, Span};
use yomu::render::{Seg, Style, View, render};
use yomu::visual::{Mode, VLine, Visual, highlight};

/// (文字列, ブロック番号) の行。同じブロック番号が続く行は折り返しでできた行
fn lines(rows: &[(&str, isize)]) -> Vec<VLine> {
    rows.iter()
        .map(|&(t, b)| VLine {
            text: t.chars().collect(),
            segs: vec![Seg { text: t.into(), ..Default::default() }],
            blks: if t.trim().is_empty() { vec![] } else { vec![b as usize] },
            indent: 0,
        })
        .collect()
}

fn page() -> Vec<VLine> {
    lines(&[("将棋は二人で行うボードゲームの", 0), ("一種である。", 0), ("", -1), ("- Python と asyncio を", 1), ("  使う例です。", 1)])
}

fn rest_of(line: &VLine, col: usize) -> String {
    line.text[col..].iter().collect()
}

#[test]
fn starts_at_first_text_of_top_line() {
    let v = Visual::from_vlines(page(), 2, Mode::Char); // 空行から始めると次の文字のある行へ
    assert_eq!(v.cursor(), (3, 0));
}

#[test]
fn words_split_by_script_in_japanese() {
    let p = page();
    let mut v = Visual::from_vlines(p.clone(), 0, Mode::Char);
    v.move_cursor("w", 1);
    assert!(rest_of(&p[0], v.cursor().1).starts_with('は')); // 漢字「将棋」→ ひらがな「は」
    v.move_cursor("w", 1);
    assert!(rest_of(&p[0], v.cursor().1).starts_with("二人"));
    v.move_cursor("e", 1);
    assert_eq!(p[0].text[v.cursor().1], '人');
    v.move_cursor("b", 1);
    assert!(rest_of(&p[0], v.cursor().1).starts_with("二人"));
}

#[test]
fn moves_across_lines_and_keeps_column() {
    let p = page();
    let mut v = Visual::from_vlines(p.clone(), 0, Mode::Char);
    v.move_cursor("$", 1);
    assert_eq!(v.cursor(), (0, p[0].text.len() - 1));
    v.move_cursor("l", 1);
    assert_eq!(v.cursor(), (1, 0)); // 行末の次は次の行の先頭
    v.move_cursor("k", 1);
    v.move_cursor("0", 1);
    v.move_cursor("j", 3);
    assert_eq!(v.cursor(), (3, 0));
    v.move_cursor("}", 1);
    assert_eq!(v.cursor().0, 4); // 次の空行 (なければ末尾) へ
    v.move_cursor("gg", 1);
    assert_eq!(v.cursor(), (0, 0));
}

#[test]
fn copy_joins_wrapped_lines() {
    let mut v = Visual::from_vlines(page(), 0, Mode::Char);
    v.move_cursor("j", 1);
    v.move_cursor("$", 1);
    assert_eq!(v.selected_text(), "将棋は二人で行うボードゲームの一種である。");
    let mut v = Visual::from_vlines(page(), 3, Mode::Line);
    v.move_cursor("j", 1);
    assert_eq!(v.selected_text(), "- Python と asyncio を使う例です。"); // ぶら下げインデントは除く
}

#[test]
fn copy_keeps_paragraph_breaks_and_swap() {
    let mut v = Visual::from_vlines(page(), 0, Mode::Char);
    v.move_cursor("G", 1);
    v.move_cursor("$", 1);
    assert_eq!(v.selected_text().split('\n').collect::<Vec<_>>(), ["将棋は二人で行うボードゲームの一種である。", "", "- Python と asyncio を使う例です。"]);
    v.swap();
    assert_eq!(v.cursor(), (0, 0));
}

#[test]
fn caret_mode_selects_only_cursor() {
    let mut v = Visual::from_vlines(page(), 0, Mode::Caret);
    v.move_cursor("l", 2);
    assert_eq!(v.selected_text(), "は");
    v.set_mode(Mode::Char); // キャレットから選択を始めると今の位置が起点
    v.move_cursor("l", 1);
    assert_eq!(v.selected_text(), "は二");
}

#[test]
fn highlight_splits_segments() {
    let bold = Style { bold: true, ..Default::default() };
    let segs = vec![Seg { text: "ab".into(), style: bold, ..Default::default() }, Seg { text: "cd".into(), ..Default::default() }];
    let out = highlight(&segs, 1, 3);
    assert_eq!(out.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), ["a", "b", "c", "d"]);
    assert!(out[1].style.reverse && out[1].style.bold);
    assert_eq!(out[2].style, Style { reverse: true, ..Default::default() });
}

#[test]
fn copy_skips_indent() {
    let row = |t: &str| VLine { text: t.chars().collect(), segs: vec![Seg { text: t.into(), ..Default::default() }], blks: vec![0], indent: 4 };
    let mut v = Visual::from_vlines(vec![row("    本文の一行目で"), row("    続きです。")], 0, Mode::Line);
    v.move_cursor("j", 1);
    assert_eq!(v.selected_text(), "本文の一行目で続きです。");
}

#[test]
fn block_at_cursor_for_speech() {
    let v = Visual::from_vlines(page(), 0, Mode::Char);
    assert_eq!(v.block_at((0, 3)), Some((0, "二人で行うボードゲームの一種である。".into())));
    assert_eq!(v.block_at((2, 0)), Some((1, "- Python と asyncio を使う例です。".into()))); // 空行なら次のブロックの先頭から
    assert_eq!(v.block_at((4, 0)), Some((1, "使う例です。".into()))); // 字下げの上なら本文の先頭から
}

#[test]
fn selected_blocks_for_speech() {
    let mut v = Visual::from_vlines(page(), 0, Mode::Line);
    v.move_cursor("G", 1);
    assert_eq!(
        v.selected_blocks(),
        [(0, "将棋は二人で行うボードゲームの一種である。".into()), (1, "- Python と asyncio を使う例です。".into())]
    );
    let mut v = Visual::from_vlines(lines(&[("Shogi is a", 0), ("game.", 0)]), 0, Mode::Line);
    v.move_cursor("j", 1);
    assert_eq!(v.selected_blocks(), [(0, "Shogi is a game.".into())]); // 折り返しで消えた空白を戻す
}

/// 実際の描画結果では、箇条書きの記号や字下げはブロックの文字に含めない
#[test]
fn block_at_uses_render_meta() {
    let mut li = Block::new(Kind::Li, vec![Span::new("項目の文です。")]);
    li.marker = "・".into();
    let d = Document {
        url: "u".into(),
        title: "t".into(),
        blocks: vec![Block::new(Kind::P, vec![Span::new("一文目です。二文目です。")]), li],
        ..Default::default()
    };
    let rendered = render(&d, &View::default(), 40);
    let v = Visual::new(rendered.clone(), 0, Mode::Char);
    let li = rendered.iter().position(|x| x.blks().contains(&1)).unwrap();
    assert_eq!(v.block_at((li, 0)), Some((1, "項目の文です。".into())));
    let p = rendered.iter().position(|x| x.blks().contains(&0)).unwrap();
    let col = rendered[p].text().chars().position(|c| c == '二').unwrap();
    assert_eq!(v.block_at((p, col)), Some((0, "二文目です。".into())));
}
