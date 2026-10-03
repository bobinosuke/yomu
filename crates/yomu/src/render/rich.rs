//! Rich (15.0.0) の描画のうち yomu が使う部分の移植。Python 版の見た目と同じ行を出すため、
//! Text の折り返し (render.py が差し替えた、日本語を 1 文字ずつ折り返す単語分割を含む)・文字幅・
//! 行への分割と切り詰め・Prefixed・Table (box.SQUARE) の列幅の決め方を写す。
use crate::pytext::{is_space, rstrip, splitlines};
use std::sync::LazyLock;

use regex::Regex;

use super::Rgb;
use super::cell_table::{NARROW_TO_WIDE, WIDTHS};

// ---- スタイル

/// Rich の Style のうち yomu が使うもの。meta (blk / link / occ / indent) も持つ
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SStyle {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub reverse: bool,
    pub url: Option<String>,
    pub blk: Option<usize>,
    pub link: Option<usize>,
    pub occ: Option<usize>,
    pub indent: bool,
}

impl SStyle {
    /// Style.combine (後のものが上書きする。属性は足し合わせ、meta は後の値で更新)
    pub fn add(&self, o: &SStyle) -> SStyle {
        SStyle {
            fg: o.fg.or(self.fg),
            bg: o.bg.or(self.bg),
            bold: self.bold || o.bold,
            dim: self.dim || o.dim,
            italic: self.italic || o.italic,
            underline: self.underline || o.underline,
            strike: self.strike || o.strike,
            reverse: self.reverse || o.reverse,
            url: o.url.clone().or_else(|| self.url.clone()),
            blk: o.blk.or(self.blk),
            link: o.link.or(self.link),
            occ: o.occ.or(self.occ),
            indent: self.indent || o.indent,
        }
    }

    /// Style.null() か (Text.append で span を足さない)
    pub fn is_null(&self) -> bool {
        *self == SStyle::default()
    }
}

// ---- 文字幅 (rich.cells)

fn char_width(c: char) -> usize {
    let cp = c as u32;
    if (cp != 0 && cp < 32) || (0x7f..0xa0).contains(&cp) {
        return 0;
    }
    let last = WIDTHS[WIDTHS.len() - 1];
    if cp > last.1 {
        return 1;
    }
    match WIDTHS.binary_search_by(|&(s, e, _)| {
        if cp < s {
            std::cmp::Ordering::Greater
        } else if cp > e {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    }) {
        Ok(i) => WIDTHS[i].2 as usize,
        Err(_) => 1,
    }
}

fn narrow_to_wide(c: char) -> bool {
    NARROW_TO_WIDE.binary_search(&(c as u32)).is_ok()
}

pub fn cell_len_chars(text: &[char]) -> usize {
    if !text.iter().any(|&c| c == '\u{200d}' || c == '\u{fe0f}') {
        return text.iter().map(|&c| char_width(c)).sum();
    }
    let mut total = 0;
    let mut last: Option<char> = None;
    for &c in text {
        if c == '\u{200d}' || c == '\u{fe0f}' {
            if c == '\u{fe0f}'
                && let Some(l) = last
            {
                total += narrow_to_wide(l) as usize;
                last = None;
            }
        } else {
            let w = char_width(c);
            if w > 0 {
                last = Some(c);
                total += w;
            }
        }
    }
    total
}

pub fn cell_len(text: &str) -> usize {
    let chars: Vec<char> = text.chars().collect();
    cell_len_chars(&chars)
}

/// split_graphemes: (始まり, 終わり, 幅) の列 (文字の位置)
fn graphemes(text: &[char]) -> Vec<(usize, usize, usize)> {
    let n = text.len();
    let mut spans: Vec<(usize, usize, usize)> = Vec::new();
    let mut last: Option<char> = None;
    let mut i = 0;
    while i < n {
        let c = text[i];
        if c == '\u{200d}' || c == '\u{fe0f}' {
            if spans.is_empty() {
                spans.push((i, i + 1, 0));
                i += 1;
                continue;
            }
            if c == '\u{200d}' {
                i += if i < n - 1 { 2 } else { 1 };
                spans.last_mut().unwrap().1 = i;
            } else {
                i += 1;
                let sp = spans.last_mut().unwrap();
                if let Some(l) = last
                    && narrow_to_wide(l)
                {
                    last = None;
                    sp.2 += 1;
                }
                sp.1 = i;
            }
            continue;
        }
        let w = char_width(c);
        if w > 0 {
            last = Some(c);
            spans.push((i, i + 1, w));
        } else if let Some(sp) = spans.last_mut() {
            sp.1 = i + 1;
        } else {
            spans.push((i, i + 1, 0));
        }
        i += 1;
    }
    spans
}

/// set_cell_size: 幅 total に切るか空白で埋める。全角文字の途中で切れるときはその文字を空白にする
pub fn set_cell_size(text: &[char], total: usize) -> Vec<char> {
    let size = cell_len_chars(text);
    if size == total {
        return text.to_vec();
    }
    if size < total {
        let mut v = text.to_vec();
        v.extend(std::iter::repeat_n(' ', total - size));
        return v;
    }
    if total == 0 {
        return vec![];
    }
    let mut left = 0;
    for (s, _e, w) in graphemes(text) {
        if left == total {
            return text[..s].to_vec();
        }
        if left + w > total {
            let mut v = text[..s].to_vec();
            v.push(' ');
            return v;
        }
        left += w;
    }
    text.to_vec()
}

/// chop_cells: 幅 width ごとに切る
fn chop_cells(text: &[char], width: usize) -> Vec<Vec<char>> {
    let mut lines = Vec::new();
    let (mut size, mut offset) = (0, 0);
    for (s, _e, w) in graphemes(text) {
        if size + w > width {
            lines.push(text[offset..s].to_vec());
            offset = s;
            size = 0;
        }
        size += w;
    }
    if size > 0 {
        lines.push(text[offset..].to_vec());
    }
    lines
}

// ---- 折り返し (rich._wrap。render.py の re_word)


const JA: &str = r"\x{3000}-\x{30ff}\x{3400}-\x{9fff}\x{f900}-\x{faff}\x{ff00}-\x{ffef}";
const JA_CLOSE: &str = "、。，．・：；？！゛゜ヽヾゝゞ々ー」』）】〕］｝〉》ぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮヵヶ";
const JA_OPEN: &str = "「『（【〔［｛〈《";
const SPACE: &str = r"\s\x1c-\x1f";

static RE_WORD: LazyLock<Regex> = LazyLock::new(|| {
    let esc = |s: &str| s.chars().map(|c| regex::escape(&c.to_string())).collect::<String>();
    Regex::new(&format!(
        r"^[{SPACE}]*(?:[{open}]*[{JA}][{close}]*|[^{SPACE}{JA}]+)[{SPACE}]*",
        open = esc(JA_OPEN),
        close = esc(JA_CLOSE)
    ))
    .unwrap()
});

/// words: (始まり, 終わり) の列 (文字の位置)。語は右側の空白を含む
fn words(text: &[char]) -> Vec<(usize, usize)> {
    let s: String = text.iter().collect();
    // バイト位置 → 文字位置
    let mut char_at = vec![0usize; s.len() + 1];
    let mut ci = 0;
    for (bi, c) in s.char_indices() {
        char_at[bi] = ci;
        ci += 1;
        let _ = c;
    }
    char_at[s.len()] = ci;
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(m) = RE_WORD.find(&s[pos..]) {
        let (a, b) = (pos + m.start(), pos + m.end());
        out.push((char_at[a], char_at[b]));
        if b == pos {
            break;
        }
        pos = b;
    }
    out
}


/// divide_line: 幅 width に収めるための改行位置 (文字の位置)
fn divide_line(text: &[char], width: usize) -> Vec<usize> {
    let mut breaks = Vec::new();
    let mut cell_offset: isize = 0;
    for (start, end) in words(text) {
        let word = &text[start..end];
        let word_length = cell_len_chars(rstrip(word)) as isize;
        let remaining = width as isize - cell_offset;
        if remaining >= word_length {
            cell_offset += cell_len_chars(word) as isize;
        } else if word_length > width as isize {
            let folded = chop_cells(word, width);
            let mut start = start;
            let n = folded.len();
            for (i, line) in folded.iter().enumerate() {
                if start > 0 {
                    breaks.push(start);
                }
                if i + 1 == n {
                    cell_offset = cell_len_chars(line) as isize;
                } else {
                    start += line.len();
                }
            }
        } else if cell_offset > 0 && start > 0 {
            breaks.push(start);
            cell_offset = cell_len_chars(word) as isize;
        }
    }
    breaks
}

// ---- Text

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Justify {
    #[default]
    Default,
    Left,
    Center,
}

#[derive(Clone, Debug)]
pub struct TSpan {
    pub start: usize,
    pub end: usize,
    pub style: SStyle,
}

#[derive(Clone, Debug, Default)]
pub struct RText {
    pub chars: Vec<char>,
    pub style: SStyle,
    pub spans: Vec<TSpan>,
    pub justify: Option<Justify>,
}

/// strip_control_codes (ベル・バックスペース・垂直タブ・改ページ・復帰を消す)
fn sanitize(text: &str) -> impl Iterator<Item = char> + '_ {
    text.chars().filter(|c| !matches!(*c as u32, 7 | 8 | 11 | 12 | 13))
}

impl RText {
    pub fn new(text: &str, style: SStyle) -> Self {
        Self { chars: sanitize(text).collect(), style, spans: vec![], justify: None }
    }

    pub fn plain(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn append(&mut self, text: &str, style: SStyle) {
        let chars: Vec<char> = sanitize(text).collect();
        if text.is_empty() {
            return;
        }
        let start = self.chars.len();
        self.chars.extend(chars);
        if !style.is_null() {
            self.spans.push(TSpan { start, end: self.chars.len(), style });
        }
    }

    fn set_plain(&mut self, chars: Vec<char>) {
        if chars != self.chars {
            self.chars = chars;
            let max = self.chars.len();
            self.spans.retain(|s| s.start < max);
            for s in &mut self.spans {
                s.end = s.end.min(max);
            }
        }
    }

    fn right_crop(&mut self, amount: usize) {
        let max = self.chars.len() - amount;
        self.spans.retain(|s| s.start < max);
        for s in &mut self.spans {
            s.end = s.end.min(max);
        }
        self.chars.truncate(max);
    }

    fn rstrip_end(&mut self, size: usize) {
        let len = self.chars.len();
        if len > size {
            let excess = len - size;
            let ws = self.chars.iter().rev().take_while(|&&c| is_space(c)).count();
            if ws > 0 {
                self.right_crop(ws.min(excess));
            }
        }
    }

    pub fn truncate(&mut self, max_width: usize, pad: bool) {
        let length = cell_len_chars(&self.chars);
        if length > max_width {
            let new = set_cell_size(&self.chars, max_width);
            self.set_plain(new);
        }
        if pad && length < max_width {
            self.chars.extend(std::iter::repeat_n(' ', max_width - length));
        }
    }

    fn pad_left(&mut self, n: usize) {
        if n > 0 {
            let mut v = vec![' '; n];
            v.extend_from_slice(&self.chars);
            self.chars = v;
            for s in &mut self.spans {
                s.start += n;
                s.end += n;
            }
        }
    }

    fn pad_right(&mut self, n: usize) {
        self.chars.extend(std::iter::repeat_n(' ', n));
    }

    /// divide: offsets の位置で分ける
    fn divide(&self, offsets: &[usize]) -> Vec<RText> {
        if offsets.is_empty() {
            return vec![self.clone()];
        }
        let mut bounds = vec![0];
        bounds.extend_from_slice(offsets);
        bounds.push(self.chars.len());
        let mut lines: Vec<RText> = bounds
            .windows(2)
            .map(|w| RText {
                chars: self.chars[w[0]..w[1]].to_vec(),
                style: self.style.clone(),
                spans: vec![],
                justify: self.justify,
            })
            .collect();
        for sp in &self.spans {
            for (i, w) in bounds.windows(2).enumerate() {
                let (ls, le) = (w[0], w[1]);
                let ns = sp.start.saturating_sub(ls);
                let ne = (sp.end as isize - ls as isize).min((le - ls) as isize);
                if ne > ns as isize && sp.start <= le && sp.end >= ls {
                    lines[i].spans.push(TSpan { start: ns, end: ne as usize, style: sp.style.clone() });
                }
            }
        }
        lines
    }

    /// split("\n", allow_blank=True)
    fn split_lines(&self) -> Vec<RText> {
        if !self.chars.contains(&'\n') {
            return vec![self.clone()];
        }
        let mut offsets = Vec::new();
        for (i, &c) in self.chars.iter().enumerate() {
            if c == '\n' {
                offsets.push(i);
                offsets.push(i + 1);
            }
        }
        self.divide(&offsets).into_iter().filter(|l| l.chars != ['\n']).collect()
    }

    /// expand_tabs (タブ幅 8)
    fn expand_tabs(&mut self) {
        if !self.chars.contains(&'\t') {
            return;
        }
        let mut out = RText { style: self.style.clone(), justify: self.justify, ..Default::default() };
        let mut cell_position = 0;
        // タブの後ろで区切る (改行は split_lines で除いてあるので行は 1 つ)
        let mut offsets = Vec::new();
        for (i, &c) in self.chars.iter().enumerate() {
            if c == '\t' && i + 1 < self.chars.len() {
                offsets.push(i + 1);
            }
        }
        for mut part in self.divide(&offsets) {
            if part.chars.last() == Some(&'\t') {
                *part.chars.last_mut().unwrap() = ' ';
                cell_position += cell_len_chars(&part.chars);
                let rem = cell_position % 8;
                if rem > 0 {
                    let spaces = 8 - rem;
                    let end = part.chars.len();
                    for s in &mut part.spans {
                        if s.end >= end {
                            s.end += spaces;
                        }
                    }
                    part.chars.extend(std::iter::repeat_n(' ', spaces));
                    cell_position += spaces;
                }
            } else {
                cell_position += cell_len_chars(&part.chars);
            }
            let off = out.chars.len();
            if !part.style.is_null() {
                out.spans.push(TSpan { start: off, end: off + part.chars.len(), style: part.style.clone() });
            }
            out.spans.extend(part.spans.iter().map(|s| TSpan { start: s.start + off, end: s.end + off, style: s.style.clone() }));
            out.chars.extend(part.chars);
        }
        // Text("").join の結果の spans を持ち、元の基本スタイルはそのまま
        self.chars = out.chars;
        self.spans = out.spans;
    }

    /// highlight_words([word], style, case_sensitive=False)
    pub fn highlight_word(&mut self, word: &str, style: &SStyle) {
        if word.is_empty() {
            // Python の re.finditer("") は各位置で空の一致を返すが、長さ 0 の span は描画に影響しない
            return;
        }
        let re = Regex::new(&format!("(?i){}", regex::escape(word))).unwrap();
        let s = self.plain();
        let byte_to_char: Vec<usize> = {
            let mut v = vec![0; s.len() + 1];
            let mut ci = 0;
            for (bi, _) in s.char_indices() {
                v[bi] = ci;
                ci += 1;
            }
            v[s.len()] = ci;
            v
        };
        for m in re.find_iter(&s) {
            self.spans.push(TSpan { start: byte_to_char[m.start()], end: byte_to_char[m.end()], style: style.clone() });
        }
    }

    /// wrap (justify・overflow は fold)
    pub fn wrap(&self, width: usize, justify: Justify) -> Vec<RText> {
        let justify = self.justify.unwrap_or(justify);
        let mut out = Vec::new();
        for mut line in self.split_lines() {
            line.expand_tabs();
            let offsets = divide_line(&line.chars, width);
            let mut new_lines = line.divide(&offsets);
            for l in &mut new_lines {
                l.rstrip_end(width);
            }
            match justify {
                Justify::Default => {}
                Justify::Left => {
                    for l in &mut new_lines {
                        l.truncate(width, true);
                    }
                }
                Justify::Center => {
                    for l in &mut new_lines {
                        let n = rstrip(&l.chars).len();
                        let stripped = l.chars[..n].to_vec();
                        l.set_plain(stripped);
                        l.truncate(width, false);
                        let pad = (width.saturating_sub(cell_len_chars(&l.chars))) / 2;
                        l.pad_left(pad);
                        let rest = width.saturating_sub(cell_len_chars(&l.chars));
                        l.pad_right(rest);
                    }
                }
            }
            for l in &mut new_lines {
                l.truncate(width, false);
            }
            out.extend(new_lines);
        }
        out
    }

    /// 1 行分を、スタイルの変わり目で区切った文字列にする (Text.render)
    pub fn segments(&self) -> Vec<Seg> {
        let len = self.chars.len();
        if self.spans.is_empty() {
            return if len == 0 { vec![] } else { vec![Seg { text: self.plain(), style: self.style.clone() }] };
        }
        let mut cuts: Vec<usize> = vec![0, len];
        for s in &self.spans {
            cuts.push(s.start.min(len));
            cuts.push(s.end.min(len));
        }
        cuts.sort_unstable();
        cuts.dedup();
        let mut out = Vec::new();
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if b <= a {
                continue;
            }
            let mut st = self.style.clone();
            for s in &self.spans {
                if s.start <= a && s.end >= b {
                    st = st.add(&s.style);
                }
            }
            out.push(Seg { text: self.chars[a..b].iter().collect(), style: st });
        }
        out
    }

    /// __rich_measure__: (最小, 最大)。最小は最も長い語、最大は最も長い行の幅
    fn measure(&self) -> (usize, usize) {
        let s = self.plain();
        let lines = splitlines(&s);
        let max_w = lines.iter().map(|l| cell_len(l)).max().unwrap_or(0);
        let min_w = s.split(is_space).filter(|w| !w.is_empty()).map(cell_len).max().unwrap_or(max_w);
        (min_w, max_w)
    }
}


// ---- 描画結果

#[derive(Clone, Debug)]
pub struct Seg {
    pub text: String,
    pub style: SStyle,
}

pub type RLine = Vec<Seg>;

fn line_len(line: &RLine) -> usize {
    line.iter().map(|s| cell_len(&s.text)).sum()
}

/// adjust_line_length: 幅 length に切るか、pad なら空白で埋める
fn adjust_line_length(line: RLine, length: usize, pad: bool) -> RLine {
    let n = line_len(&line);
    if n < length {
        let mut line = line;
        if pad {
            line.push(Seg { text: " ".repeat(length - n), style: SStyle::default() });
        }
        line
    } else if n > length {
        let mut out = Vec::new();
        let mut used = 0;
        for s in line {
            let w = cell_len(&s.text);
            if used + w < length {
                used += w;
                out.push(s);
            } else {
                let chars: Vec<char> = s.text.chars().collect();
                out.push(Seg { text: set_cell_size(&chars, length - used).into_iter().collect(), style: s.style });
                break;
            }
        }
        out
    } else {
        line
    }
}

// ---- 描画できるもの

pub enum Renderable {
    Text(RText),
    /// 折り返した各行の先頭に印を付ける (1 行目は first、2 行目以降は rest)
    Prefixed { inner: Box<Renderable>, first: String, rest: String, style: SStyle },
    Table(Table),
}

/// Console.render_lines(renderable, width, pad): justify は Text の既定の寄せ
pub fn render_lines(r: &Renderable, width: usize, justify: Justify, pad: bool) -> Vec<RLine> {
    if width < 1 {
        return vec![];
    }
    let lines: Vec<RLine> = match r {
        Renderable::Text(t) => t.wrap(width, justify).iter().map(|l| l.segments()).collect(),
        Renderable::Prefixed { inner, first, rest, style } => {
            let w = cell_len(first).max(cell_len(rest));
            render_lines(inner, width.saturating_sub(w), justify, false)
                .into_iter()
                .enumerate()
                .map(|(i, line)| {
                    let mut out = vec![Seg { text: if i == 0 { first.clone() } else { rest.clone() }, style: style.clone() }];
                    out.extend(line);
                    out
                })
                .collect()
        }
        Renderable::Table(t) => t.render(width),
    };
    lines.into_iter().map(|l| adjust_line_length(l, width, pad)).collect()
}

// ---- 表 (box.SQUARE、show_lines=True、show_edge=True、列の overflow は fold、セルの余白は左右 1)

pub struct Table {
    pub show_header: bool,
    /// 列ごとのセル (見出し行を出すときは先頭が見出し)
    pub columns: Vec<Vec<RText>>,
}

const PAD: usize = 2; // セルの左右の余白の合計

type Measure = (usize, usize);

fn normalize(m: (isize, isize)) -> Measure {
    let (mn, mx) = m;
    let mn = mn.max(0).min(mx);
    (mn.max(0) as usize, mn.max(mx).max(0) as usize)
}

fn with_maximum(m: Measure, w: usize) -> Measure {
    (m.0.min(w), m.1.min(w))
}

/// Measurement.get(Padding(text, (0, 1)))
fn measure_cell(t: &RText, max_width: usize) -> Measure {
    if max_width < 1 {
        return (0, 0);
    }
    // Padding.__rich_measure__
    let m = if (max_width as isize) - (PAD as isize) < 1 {
        (max_width, max_width)
    } else {
        // Measurement.get(text)
        let (mn, mx) = t.measure();
        let inner = with_maximum(normalize((mn as isize, mx as isize)), max_width);
        let inner = if inner.1 < 1 { (0, 0) } else { normalize((inner.0 as isize, inner.1 as isize)) };
        with_maximum((inner.0 + PAD, inner.1 + PAD), max_width)
    };
    let m = with_maximum(normalize((m.0 as isize, m.1 as isize)), max_width);
    if m.1 < 1 { (0, 0) } else { normalize((m.0 as isize, m.1 as isize)) }
}

fn ratio_reduce(total: usize, ratios: &[usize], maximums: &[usize], values: &[usize]) -> Vec<usize> {
    let ratios: Vec<usize> = ratios.iter().zip(maximums).map(|(&r, &m)| if m > 0 { r } else { 0 }).collect();
    let mut total_ratio: usize = ratios.iter().sum();
    if total_ratio == 0 {
        return values.to_vec();
    }
    let mut remaining = total as f64;
    let mut out = Vec::new();
    for ((&r, &m), &v) in ratios.iter().zip(maximums).zip(values) {
        if r > 0 && total_ratio > 0 {
            let d = (m as f64).min((r as f64 * remaining / total_ratio as f64).round_ties_even());
            out.push((v as f64 - d) as usize);
            remaining -= d;
            total_ratio -= r;
        } else {
            out.push(v);
        }
    }
    out
}


impl Table {
    fn extra_width(&self) -> usize {
        2 + self.columns.len().saturating_sub(1)
    }

    fn measure_column(&self, col: usize, max_width: usize) -> Measure {
        if max_width < 1 {
            return (0, 0);
        }
        let cells = &self.columns[col];
        let ms: Vec<Measure> = cells.iter().map(|c| measure_cell(c, max_width)).collect();
        let m = (
            ms.iter().map(|m| m.0).max().unwrap_or(1),
            ms.iter().map(|m| m.1).max().unwrap_or(max_width),
        );
        with_maximum(m, max_width)
    }

    fn column_widths(&self, max_width: usize) -> Vec<usize> {
        let n = self.columns.len();
        let ranges: Vec<Measure> = (0..n).map(|i| self.measure_column(i, max_width)).collect();
        let mut widths: Vec<usize> = ranges.iter().map(|r| if r.1 == 0 { 1 } else { r.1 }).collect();
        let table_width: usize = widths.iter().sum();
        if table_width > max_width {
            widths = collapse_widths(widths, max_width);
            let tw: usize = widths.iter().sum();
            if tw > max_width {
                let excess = tw - max_width;
                widths = ratio_reduce(excess, &vec![1; n], &widths.clone(), &widths);
            }
            widths = (0..n).map(|i| self.measure_column(i, widths[i]).1).collect();
        }
        widths
    }

    fn render(&self, max_width: usize) -> Vec<RLine> {
        let extra = self.extra_width();
        let widths = self.column_widths(max_width.saturating_sub(extra));
        let border = |s: String| vec![Seg { text: s, style: SStyle::default() }];
        let row_line = |l: char, m: char, r: char| {
            let mut s = String::from(l);
            for (i, &w) in widths.iter().enumerate() {
                if i > 0 {
                    s.push(m);
                }
                s.extend(std::iter::repeat_n('─', w));
            }
            s.push(r);
            s
        };
        let mut out = vec![border(row_line('┌', '┬', '┐'))];
        let rows = self.columns.first().map_or(0, |c| c.len());
        for r in 0..rows {
            let header_row = r == 0 && self.show_header;
            let last = r + 1 == rows;
            let cells: Vec<Vec<RLine>> = (0..self.columns.len())
                .map(|c| render_cell(&self.columns[c][r], widths[c]))
                .collect();
            let height = cells.iter().map(|c| c.len()).max().unwrap_or(0).max(1);
            let cells: Vec<Vec<RLine>> = cells
                .into_iter()
                .zip(&widths)
                .map(|(lines, &w)| {
                    let extra = height - lines.len();
                    let blank = vec![Seg { text: " ".repeat(w), style: SStyle::default() }];
                    let mut lines = if header_row {
                        let mut v = vec![blank; extra];
                        v.extend(lines);
                        v
                    } else {
                        let mut v = lines;
                        v.extend(std::iter::repeat_n(blank, extra));
                        v
                    };
                    for l in &mut lines {
                        *l = adjust_line_length(std::mem::take(l), w, true);
                    }
                    lines
                })
                .collect();
            for i in 0..height {
                let mut line = border("│".into());
                for (c, cell) in cells.iter().enumerate() {
                    if c > 0 {
                        line.extend(border("│".into()));
                    }
                    line.extend(cell[i].iter().cloned());
                }
                line.extend(border("│".into()));
                out.push(line);
            }
            if header_row || !last {
                out.push(border(row_line('├', '┼', '┤')));
            }
        }
        out.push(border(row_line('└', '┴', '┘')));
        out
    }
}

/// セルを Padding(text, (0, 1)) として幅 width で描く (左寄せ。見出しは中央寄せ)
fn render_cell(t: &RText, width: usize) -> Vec<RLine> {
    if width < 1 {
        return vec![];
    }
    let inner = width.saturating_sub(PAD);
    let pad = |s: &str| Seg { text: s.into(), style: SStyle::default() };
    render_lines(&Renderable::Text(t.clone()), inner, Justify::Left, true)
        .into_iter()
        .map(|line| {
            let mut out = vec![pad(" ")];
            out.extend(line);
            out.push(pad(" "));
            adjust_line_length(out, width, true)
        })
        .collect()
}

fn collapse_widths(mut widths: Vec<usize>, max_width: usize) -> Vec<usize> {
    let mut total: usize = widths.iter().sum();
    let mut excess = total as isize - max_width as isize;
    while total > 0 && excess > 0 {
        let max_col = *widths.iter().max().unwrap();
        let second = widths.iter().map(|&w| if w != max_col { w } else { 0 }).max().unwrap_or(0);
        let diff = max_col - second;
        let ratios: Vec<usize> = widths.iter().map(|&w| (w == max_col) as usize).collect();
        if !ratios.iter().any(|&r| r > 0) || diff == 0 {
            break;
        }
        let max_reduce = vec![(excess as usize).min(diff); widths.len()];
        widths = ratio_reduce(excess as usize, &ratios, &max_reduce, &widths);
        total = widths.iter().sum();
        excess = total as isize - max_width as isize;
    }
    widths
}

