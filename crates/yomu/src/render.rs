//! Document を端末向けの行 (折り返し済み・スタイル付き) と Markdown (--dump) に描画する。
//! Python 版の render.py (Rich で描画) と同じ見た目にする。
//! TUI とパイプでない --dump は、この行をそれぞれ ratatui と ANSI エスケープで出す。
use std::collections::HashMap;

use crate::doc::{Block, Document, Kind, Span};

mod cell_table;
mod rich;

pub use rich::{cell_len, set_cell_size};
use rich::{Justify, RText, Renderable, SStyle, Table, render_lines};

pub const READING_WIDTH: usize = 80; // 本文の幅の上限 (全角40字)。これより長い行は目で追いにくい

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

// ---- 色 (白い背景・黒い背景の両方でコントラスト比 約 4.5 以上)
pub const HEADING_COLOR: Rgb = Rgb(196, 84, 10);
pub const LINK_COLOR: Rgb = Rgb(42, 116, 212);
pub const CODE_COLOR: Rgb = Rgb(152, 85, 212);
/// 対訳の訳文の色 (見出し・リンク・コードの色と見分けられる青緑)
pub const TRANSLATION_COLOR: Rgb = Rgb(20, 150, 120);
/// 対訳で原文の下に置いた訳文の頭に付ける印
const TRANSLATION_MARK: &str = "↳ ";
pub const MARK_BG: Rgb = Rgb(255, 200, 0);
pub const BLACK: Rgb = Rgb(0, 0, 0);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub dim: bool,
    pub reverse: bool,
    pub url: Option<String>, // 端末のハイパーリンク (OSC 8)
}

/// 行の中の、同じスタイルの文字列。描画結果の各行から、どのブロック・どのリンクがそこにあるかを逆引きするための印を持つ
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seg {
    pub text: String,
    pub style: Style,
    pub blk: Option<usize>,  // ブロック番号
    pub link: Option<usize>, // Document.links のインデックス
    pub occ: Option<usize>,  // リンクの出現番号 (同じ URL でも出現ごとに別の番号)
    pub indent: bool,        // 見出しの階層に合わせた字下げの空白 (ビジュアルモードでコピーするときに除く)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    pub segs: Vec<Seg>,
}

impl Line {
    pub fn text(&self) -> String {
        self.segs.iter().map(|s| s.text.as_str()).collect()
    }
    /// この行にあるリンクの出現番号と、そのリンクの Document.links のインデックス (出てくる順)
    pub fn occs(&self) -> Vec<(usize, usize)> {
        let mut out: Vec<(usize, usize)> = Vec::new();
        for s in &self.segs {
            if let (Some(o), Some(l)) = (s.occ, s.link)
                && !out.iter().any(|x| x.0 == o)
            {
                out.push((o, l));
            }
        }
        out
    }
    /// この行にあるブロックの番号 (出てくる順)
    pub fn blks(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for b in self.segs.iter().filter_map(|s| s.blk) {
            if !out.contains(&b) {
                out.push(b);
            }
        }
        out
    }
    /// 行頭の字下げの文字数
    pub fn indent(&self) -> usize {
        self.segs.iter().take_while(|s| s.indent).map(|s| s.text.chars().count()).sum()
    }
}

/// 描画時だけの状態
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub hints: Option<HashMap<usize, String>>, // リンクの出現番号 → ヒントのラベル
    pub typed: String,                         // ヒントモードで入力済みの文字
    pub find: String,                          // ページ内検索の語
    pub numbers: bool, // リンクの後ろに [番号] を付ける (--dump を端末に出すとき)
}

/// TUI の本文 (Python 版の to_rich を width マスで描いたもの。Console.render_lines(pad=False) の結果)
pub fn render(doc: &Document, view: &View, width: usize) -> Vec<Line> {
    render_wide(doc, view, width, width)
}

/// render と同じ。ただしコードと表は wide マス (端末の幅) まで使う (本文だけを読みやすい幅で折り返す)
pub fn render_wide(doc: &Document, view: &View, width: usize, wide: usize) -> Vec<Line> {
    lines_of_wide(&to_rich_parts(doc, view), width, wide)
}

/// --dump を端末に出すとき用 (Python 版の to_terminal)。TUI と同じ見た目に、リンクの番号と URL の一覧を足す
pub fn to_terminal(doc: &Document, width: usize) -> Vec<Line> {
    to_terminal_wide(doc, width, width)
}

/// to_terminal と同じ。ただしコードと表は wide マスまで使う
pub fn to_terminal_wide(doc: &Document, width: usize, wide: usize) -> Vec<Line> {
    let dim = SStyle { dim: true, ..Default::default() };
    let mut parts = vec![(text(&doc.url, dim.clone()), false), (text("", SStyle::default()), false)];
    parts.extend(to_rich_parts(doc, &View { numbers: true, ..Default::default() }));
    if !doc.links.is_empty() {
        parts.push((text("", SStyle::default()), false));
        parts.push((text("---", SStyle::default()), false));
        for (i, u) in doc.links.iter().enumerate() {
            parts.push((text(&format!("[{}] {u}", i + 1), dim.clone()), false));
        }
    }
    lines_of_wide(&parts, width, wide)
}

/// (描くもの, 端末の幅まで使うか) を行にする
fn lines_of_wide(parts: &[(Renderable, bool)], width: usize, wide: usize) -> Vec<Line> {
    parts
        .iter()
        .flat_map(|(p, is_wide)| render_lines(p, if *is_wide { wide } else { width }, Justify::Default, false))
        .map(|segs| Line { segs: segs.into_iter().map(to_seg).collect() })
        .collect()
}

fn to_seg(s: rich::Seg) -> Seg {
    let st = s.style;
    Seg {
        text: s.text,
        style: Style {
            fg: st.fg,
            bg: st.bg,
            bold: st.bold,
            italic: st.italic,
            underline: st.underline,
            strike: st.strike,
            dim: st.dim,
            reverse: st.reverse,
            url: st.url,
        },
        blk: st.blk,
        link: st.link,
        occ: st.occ,
        indent: st.indent,
    }
}

/// 行を ANSI エスケープ付きの文字列にする (--dump を端末に出すとき)。http(s) のリンクには OSC 8 のハイパーリンクも付ける
pub fn to_ansi(lines: &[Line]) -> String {
    let mut out = String::new();
    for line in lines {
        for s in &line.segs {
            let codes = sgr(&s.style);
            if let Some(u) = &s.style.url {
                out.push_str(&format!("\x1b]8;;{u}\x1b\\"));
            }
            if codes.is_empty() {
                out.push_str(&s.text);
            } else {
                out.push_str(&format!("\x1b[{codes}m{}\x1b[0m", s.text));
            }
            if s.style.url.is_some() {
                out.push_str("\x1b]8;;\x1b\\");
            }
        }
        out.push('\n');
    }
    out
}

fn sgr(st: &Style) -> String {
    let mut c: Vec<String> = Vec::new();
    for (on, code) in [(st.bold, "1"), (st.dim, "2"), (st.italic, "3"), (st.underline, "4"), (st.reverse, "7"), (st.strike, "9")] {
        if on {
            c.push(code.into());
        }
    }
    if let Some(Rgb(r, g, b)) = st.fg {
        c.push(if (r, g, b) == (0, 0, 0) { "30".into() } else { format!("38;2;{r};{g};{b}") });
    }
    if let Some(Rgb(r, g, b)) = st.bg {
        c.push(format!("48;2;{r};{g};{b}"));
    }
    c.join(";")
}

// ---- Rich の Text に当たるものの組み立て (render.py の to_rich)

const QUOTE_BAR: &str = "\u{258e}"; // ▎
const INDENT: usize = 2;
const MAX_INDENT: usize = 8; // 階層ごとの字下げと、その上限

fn h1_style() -> SStyle {
    SStyle { bold: true, italic: true, underline: true, fg: Some(HEADING_COLOR), ..Default::default() }
}
fn h_style() -> SStyle {
    SStyle { bold: true, fg: Some(HEADING_COLOR), ..Default::default() }
}
fn find_style() -> SStyle {
    SStyle { fg: Some(BLACK), bg: Some(MARK_BG), ..Default::default() }
}
fn hint_style() -> SStyle {
    SStyle { bold: true, fg: Some(BLACK), bg: Some(MARK_BG), ..Default::default() }
}
fn hint_typed_style() -> SStyle {
    SStyle { bold: true, fg: Some(Rgb(120, 120, 120)), bg: Some(MARK_BG), ..Default::default() }
}

fn text(s: &str, style: SStyle) -> Renderable {
    Renderable::Text(RText::new(s, style))
}

pub fn alpha(mut n: usize) -> String {
    let mut t = String::new();
    while n > 0 {
        n -= 1;
        t.insert(0, (b'a' + (n % 26) as u8) as char);
        n /= 26;
    }
    t
}

pub fn roman(mut n: usize) -> String {
    let mut t = String::new();
    for (v, r) in [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")] {
        while n >= v {
            t.push_str(r);
            n -= v;
        }
    }
    t
}

/// 番号付きの箇条書きの番号 ("3." の 3)。番号でなければ None
fn marker_number(marker: &str) -> Option<usize> {
    let mut chars = marker.chars();
    chars.next_back()?;
    let digits = chars.as_str();
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// 箇条書きの記号。記号は深さによらず "-"。番号付きは 1. → a. → i. (深さ1/2/3段目)
pub fn list_marker(b: &Block) -> String {
    if b.marker.is_empty() {
        return String::new();
    }
    let Some(n) = marker_number(&b.marker) else { return "-".into() };
    match b.level {
        1 => alpha(n) + ".",
        2 => roman(n) + ".",
        _ => b.marker.clone(),
    }
}

/// 各ブロックの字下げ。見出しは階層ごとに2字ずつ、本文はその見出しよりさらに2字下げる。
/// 階層は見出しのレベルそのものではなく、ページに出てくるレベルの順位で数える (h2 と h4 だけなら2段)
pub fn indents(blocks: &[Block]) -> Vec<usize> {
    let mut levels: Vec<u32> = blocks.iter().filter(|b| b.kind == Kind::H).map(|b| b.level).collect();
    levels.sort_unstable();
    levels.dedup();
    let rank = |lv: u32| levels.iter().position(|&l| l == lv).unwrap();
    let mut cur: Option<usize> = None;
    blocks
        .iter()
        .map(|b| {
            if b.kind == Kind::H {
                let r = rank(b.level);
                cur = Some(r);
                (r * INDENT).min(MAX_INDENT)
            } else {
                cur.map_or(0, |c| ((c + 1) * INDENT).min(MAX_INDENT))
            }
        })
        .collect()
}

/// ブロックの前に空行を入れるか。箇条書きが続くときは詰め、それ以外は段落の間を1行あける。
/// 対訳の訳文は原文のすぐ下に置く
pub fn gap_before(prev: Option<&Block>, b: &Block) -> bool {
    !b.translation && prev.is_some_and(|p| !(p.kind == Kind::Li && b.kind == Kind::Li))
}

/// 描画中の状態 (Python 版の View のうち、描きながら変わるもの)
struct Ctx<'a> {
    view: &'a View,
    links: &'a [String],
    occ: isize, // リンクの出現番号
    blk: usize, // 描画中のブロック番号
}

fn highlight(t: &mut RText, view: &View) {
    if !view.find.is_empty() {
        t.highlight_word(&view.find, &find_style());
    }
}

fn spans_text(spans: &[Span], cx: &mut Ctx, base: SStyle) -> RText {
    let mut t = RText::new("", base);
    for (i, s) in spans.iter().enumerate() {
        let mut st = SStyle {
            bold: s.bold,
            italic: s.italic || s.image,
            strike: s.strike,
            dim: s.minor || s.image,
            ..Default::default()
        };
        if s.code {
            st.fg = Some(CODE_COLOR);
        }
        if s.translated {
            st.fg = Some(TRANSLATION_COLOR);
        }
        if s.link.is_some() {
            st.fg = Some(LINK_COLOR);
        }
        st.blk = Some(cx.blk);
        if let Some(l) = s.link {
            let start = i == 0 || spans[i - 1].link != s.link;
            if start {
                cx.occ += 1;
            }
            st.link = Some(l);
            st.occ = Some(cx.occ as usize);
            let label = cx.view.hints.as_ref().and_then(|h| h.get(&(cx.occ as usize)));
            if let (Some(label), true) = (label, start) {
                let n = cx.view.typed.chars().count().min(label.chars().count());
                let typed: String = label.chars().take(n).collect();
                let rest: String = label.chars().skip(n).collect();
                t.append(&typed, hint_typed_style());
                t.append(&rest, hint_style());
            }
            if let Some(url) = cx.links.get(l)
                && (url.starts_with("http://") || url.starts_with("https://"))
            {
                st.url = Some(url.clone()); // 端末のハイパーリンク (OSC 8)
            }
        }
        t.append(&s.text, st);
        if cx.view.numbers
            && let Some(l) = s.link
            && (i + 1 == spans.len() || spans[i + 1].link != s.link)
        {
            t.append(&format!("[{}]", l + 1), SStyle { dim: true, ..Default::default() });
        }
    }
    highlight(&mut t, cx.view);
    t
}

fn block_rich(b: &Block, cx: &mut Ctx) -> Renderable {
    match b.kind {
        Kind::H => Renderable::Text(spans_text(&b.spans, cx, if b.level == 1 { h1_style() } else { h_style() })),
        Kind::Li => {
            // 折り返した2行目以降は記号の後ろ (本文の位置) にそろえる
            let marker = if b.translation { TRANSLATION_MARK.trim_end().to_string() } else { list_marker(b) };
            let head = " ".repeat(2 * b.level as usize) + &if marker.is_empty() { "  ".to_string() } else { marker + " " };
            let rest = " ".repeat(cell_len(&head));
            let style = if b.translation { SStyle { fg: Some(TRANSLATION_COLOR), ..Default::default() } } else { SStyle::default() };
            Renderable::Prefixed { inner: Box::new(Renderable::Text(spans_text(&b.spans, cx, SStyle::default()))), first: head, rest, style }
        }
        Kind::Quote => {
            let base = SStyle { italic: true, dim: true, ..Default::default() };
            let first = format!("{QUOTE_BAR} ");
            Renderable::Prefixed {
                inner: Box::new(Renderable::Text(spans_text(&b.spans, cx, base))),
                rest: first.clone(),
                first,
                style: SStyle { dim: true, ..Default::default() },
            }
        }
        Kind::Pre => {
            let src = b.spans.first().map_or("", |s| s.text.as_str());
            let mut t = RText::new(src, SStyle { fg: Some(CODE_COLOR), blk: Some(cx.blk), ..Default::default() });
            highlight(&mut t, cx.view);
            Renderable::Text(t)
        }
        Kind::Hr => text("---", SStyle::default()),
        Kind::Table => table_rich(b, cx),
        Kind::P => Renderable::Text(spans_text(&b.spans, cx, if b.caption { SStyle { dim: true, ..Default::default() } } else { SStyle::default() })),
    }
}

fn table_rich(b: &Block, cx: &mut Ctx) -> Renderable {
    let rows = b.rows.as_deref().unwrap_or(&[]);
    let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    // 先頭行がすべて <th> (太字) なら見出し行として中央寄せにする
    let header = rows.len() > 1 && rows[0].iter().all(|c| !c.is_empty() && c.iter().all(|s| s.bold));
    let mut columns: Vec<Vec<RText>> = vec![Vec::new(); width];
    if header {
        for (i, col) in columns.iter_mut().enumerate() {
            let mut t = match rows[0].get(i) {
                Some(cell) if !cell.is_empty() => spans_text(cell, cx, SStyle::default()),
                _ => RText::new("", SStyle::default()),
            };
            t.justify = Some(Justify::Center);
            col.push(t);
        }
    }
    for row in if header { &rows[1..] } else { rows } {
        for (i, col) in columns.iter_mut().enumerate() {
            col.push(match row.get(i) {
                Some(cell) => spans_text(cell, cx, SStyle::default()),
                None => RText::new("", SStyle::default()),
            });
        }
    }
    Renderable::Table(Table { show_header: header, columns })
}

/// 記事のタイトルを見出し1として本文の先頭に出す。本文の最初の見出しがタイトルと同じなら出さない
/// (ヘルプなどの組み込みページにも出さない)
fn title_heading(doc: &Document) -> Option<Renderable> {
    if doc.title.is_empty() || !doc.url.starts_with("http") {
        return None;
    }
    if let Some(first) = doc.blocks.iter().take(3).find(|b| b.kind == Kind::H) {
        let t = first.text();
        let t = crate::pytext::strip(&t);
        if !t.is_empty() && doc.title.contains(t) {
            return None;
        }
    }
    Some(text(&doc.title, h1_style()))
}

/// (描くもの, 端末の幅まで使うか)。コード (pre) と表は端末の幅まで使う
fn to_rich_parts(doc: &Document, view: &View) -> Vec<(Renderable, bool)> {
    let mut cx = Ctx { view, links: &doc.links, occ: -1, blk: 0 };
    let mut parts = Vec::new();
    if let Some(t) = title_heading(doc) {
        parts.push((t, false)); // ブロックではないので blk の印は付けない (ヒント・読み上げの位置に影響しない)
    }
    let mut prev: Option<&Block> = None;
    for ((i, b), ind) in doc.blocks.iter().enumerate().zip(indents(&doc.blocks)) {
        cx.blk = i;
        if gap_before(prev, b) || (prev.is_none() && !parts.is_empty()) {
            parts.push((text("", SStyle::default()), false));
        }
        let mut r = block_rich(b, &mut cx);
        if b.translation && b.kind != Kind::Li {
            // 箇条書きは記号の位置に印を付ける (block_rich)
            let style = SStyle { fg: Some(TRANSLATION_COLOR), ..Default::default() };
            r = Renderable::Prefixed { inner: Box::new(r), first: TRANSLATION_MARK.into(), rest: "  ".into(), style };
        }
        let wide = matches!(b.kind, Kind::Pre | Kind::Table);
        parts.push((if ind > 0 {
            let pad = " ".repeat(ind);
            Renderable::Prefixed { inner: Box::new(r), first: pad.clone(), rest: pad, style: SStyle { indent: true, ..Default::default() } }
        } else {
            r
        }, wide));
        prev = Some(b);
    }
    if parts.is_empty() {
        parts.push((text(t!("(表示できる本文がありません。a で全体表示に切り替えられます)"), SStyle { dim: true, ..Default::default() }), false));
    }
    parts
}

// ---- Markdown

fn spans_md(spans: &[Span]) -> String {
    let mut out = String::new();
    for (i, s) in spans.iter().enumerate() {
        let mut t = s.text.replace('\n', "  \n");
        if s.code {
            t = format!("`{t}`");
        } else if !crate::pytext::strip(&t).is_empty() {
            if s.bold {
                t = format!("**{t}**");
            }
            if s.italic {
                t = format!("*{t}*");
            }
            if s.strike {
                t = format!("~~{t}~~");
            }
        }
        out.push_str(&t);
        if let Some(l) = s.link
            && spans.get(i + 1).is_none_or(|n| n.link != s.link)
        {
            out.push_str(&format!("[{}]", l + 1));
        }
    }
    out
}

pub fn to_markdown(doc: &Document) -> String {
    let mut lines: Vec<String> = vec![format!("# {}", doc.title), format!("<{}>", doc.url), String::new()];
    let mut prev: Option<&Block> = None;
    for b in &doc.blocks {
        if gap_before(prev, b) {
            lines.push(String::new());
        }
        match b.kind {
            Kind::H => lines.push("#".repeat((b.level as usize + 1).min(6)) + " " + &spans_md(&b.spans)),
            Kind::Li => {
                let marker = if marker_number(&b.marker).is_some() { b.marker.clone() } else { "-".into() };
                lines.push("  ".repeat(b.level as usize) + &marker + " " + &spans_md(&b.spans));
            }
            Kind::Quote => lines.push("> ".to_string() + &spans_md(&b.spans)),
            Kind::Pre => {
                lines.push("```".into());
                lines.push(b.spans.first().map_or(String::new(), |s| s.text.clone()));
                lines.push("```".into());
            }
            Kind::Hr => lines.push("---".into()),
            Kind::Table => {
                let rows = b.rows.as_deref().unwrap_or(&[]);
                let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
                for (i, row) in rows.iter().enumerate() {
                    let mut cells: Vec<String> = row.iter().map(|c| spans_md(c).replace('|', "\\|").replace("  \n", " ")).collect();
                    cells.resize(width, String::new());
                    lines.push(format!("| {} |", cells.join(" | ")));
                    if i == 0 {
                        lines.push("|".to_string() + &"---|".repeat(width));
                    }
                }
            }
            Kind::P => lines.push(spans_md(&b.spans)),
        }
        prev = Some(b);
    }
    if !doc.links.is_empty() {
        lines.extend(["".into(), "---".into(), "".into()]);
        lines.extend(doc.links.iter().enumerate().map(|(i, u)| format!("[{}]: {u}", i + 1)));
    }
    lines.join("\n") + "\n"
}

/// 幅 width マスで折り返したときの 1 行目 (Textual の 1 行の欄に長い文字列を入れたときの見え方)
pub fn first_line(text: &str, width: usize) -> String {
    let t = rich::RText::new(text, rich::SStyle::default());
    t.wrap(width.max(1), rich::Justify::Default).first().map(|l| l.plain().trim_end().to_string()).unwrap_or_default()
}
