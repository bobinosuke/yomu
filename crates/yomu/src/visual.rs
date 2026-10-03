//! Vimium のビジュアルモード (v / V / キャレット) の、カーソル移動・選択範囲・コピーする文字列・描画。
//! Python 版の visual.py と同じ。
//!
//! 画面に描かれた行 (折り返し後) の上でカーソルを動かす。位置は (行, 行内の文字の位置)。
//! 日本語は語の区切りに空白がないので、w / b / e はひらがな・カタカナ・漢字・英数字・記号の
//! 文字種が変わるところを語の区切りとみなす。
use crate::pytext::{is_space, lstrip_len, rstrip};
use crate::render::{Line, Seg};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Caret,
    Char,
    Line,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Caret => t!("キャレット"),
            Mode::Char => t!("ビジュアル"),
            Mode::Line => t!("行ビジュアル"),
        }
    }
}

/// 描画済みの 1 行
#[derive(Clone, Debug)]
pub struct VLine {
    pub text: Vec<char>,
    pub segs: Vec<Seg>,
    pub blks: Vec<usize>, // この行にあるブロックの番号 (折り返しでできた行かどうかの判定に使う)
    pub indent: usize,    // 行頭の字下げの文字数 (コピーするときに除く)
}

impl From<&Line> for VLine {
    fn from(l: &Line) -> Self {
        VLine { text: l.text().chars().collect(), segs: l.segs.clone(), blks: l.blks(), indent: l.indent() }
    }
}

type Pos = (usize, usize);

/// 語の区切りを決める文字種。0 は空白
pub fn char_class(ch: char) -> u8 {
    if is_space(ch) {
        return 0;
    }
    let o = ch as u32;
    if (0x3041..=0x309F).contains(&o) {
        return 1; // ひらがな
    }
    if (0x30A0..=0x30FF).contains(&o) || (0xFF66..=0xFF9F).contains(&o) {
        return 2; // カタカナ (長音「ー」を含む)
    }
    if (0x3400..=0x9FFF).contains(&o) || (0xF900..=0xFAFF).contains(&o) || "々〆ヶ".contains(ch) {
        return 3; // 漢字
    }
    if ch.is_alphanumeric() || ch == '_' {
        return 4; // 英数字 (全角を含む)
    }
    5 // 記号・句読点
}

fn is_blank(t: &[char]) -> bool {
    t.iter().all(|&c| is_space(c))
}

pub struct Visual {
    lines: Vec<VLine>,
    flat: Vec<char>, // 行を改行でつないだ 1 本の文字列 (行をまたぐ移動はこの上で行う)
    mode: Mode,
    cursor: Pos,
    anchor: Pos,
    want_col: usize, // j / k で縦に動くときに保つ列
}

impl Visual {
    pub fn new(lines: Vec<Line>, line: usize, mode: Mode) -> Self {
        Self::from_vlines(lines.iter().map(VLine::from).collect(), line, mode)
    }

    pub fn from_vlines(lines: Vec<VLine>, line: usize, mode: Mode) -> Self {
        let mut flat = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            if i > 0 {
                flat.push('\n');
            }
            flat.extend_from_slice(&l.text);
        }
        let mut line = line.min(lines.len().saturating_sub(1));
        // 空行なら次の文字のある行から
        while line + 1 < lines.len() && is_blank(&lines[line].text) {
            line += 1;
        }
        let col = lines.get(line).map_or(0, |l| lstrip_len(&l.text));
        Self { lines, flat, mode, cursor: (line, col), anchor: (line, col), want_col: col }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn cursor(&self) -> Pos {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    fn to_index(&self, pos: Pos) -> usize {
        self.lines[..pos.0].iter().map(|x| x.text.len() + 1).sum::<usize>() + pos.1
    }

    fn to_pos(&self, mut i: usize) -> Pos {
        for (n, x) in self.lines.iter().enumerate() {
            if i <= x.text.len() {
                return (n, i);
            }
            i -= x.text.len() + 1;
        }
        let last = self.lines.len() - 1;
        (last, self.lines[last].text.len())
    }

    /// 行の文字数。描画結果の行末に付く見えない空白は数えない
    fn width(&self, line: usize) -> usize {
        rstrip(&self.lines[line].text).len()
    }

    fn clamp(&self, line: isize, col: isize) -> Pos {
        let line = line.clamp(0, self.lines.len() as isize - 1) as usize;
        let max_col = (self.width(line) as isize - 1).max(0);
        (line, col.clamp(0, max_col) as usize)
    }

    // ---- 移動

    pub fn move_cursor(&mut self, cmd: &str, n: usize) {
        if self.lines.is_empty() {
            return;
        }
        let (line, _col) = self.cursor;
        match cmd {
            "j" | "k" => {
                let d = if cmd == "j" { n as isize } else { -(n as isize) };
                self.cursor = self.clamp(line as isize + d, self.want_col as isize);
                return;
            }
            "h" | "l" => {
                let d: isize = if cmd == "l" { 1 } else { -1 };
                let last = self.flat.len() as isize - 1;
                let mut i = self.to_index(self.cursor) as isize;
                for _ in 0..n {
                    i = (i + d).max(0).min(last);
                    // 行と行の間の改行の位置は飛ばす (空行には止まれる)
                    while 0 < i && i < last && self.flat[i as usize] == '\n' && self.flat[i as usize - 1] != '\n' {
                        i += d;
                    }
                }
                self.cursor = self.to_pos(i.max(0) as usize);
            }
            "0" => self.cursor = (line, 0),
            "^" => self.cursor = (line, lstrip_len(&self.lines[line].text)),
            "$" => self.cursor = self.clamp(line as isize, self.width(line) as isize - 1),
            "gg" => self.cursor = (0, 0),
            "G" => self.cursor = self.clamp(self.lines.len() as isize - 1, 0),
            "w" | "b" | "e" => {
                let mut i = self.to_index(self.cursor);
                for _ in 0..n {
                    i = match cmd {
                        "w" => self.next_word(i),
                        "b" => self.prev_word(i),
                        _ => self.word_end(i),
                    };
                }
                self.cursor = self.to_pos(i);
            }
            "}" | "{" => {
                for _ in 0..n {
                    let d = if cmd == "}" { 1 } else { -1 };
                    self.cursor = (self.next_paragraph(self.cursor.0, d), 0);
                }
            }
            _ => {}
        }
        self.want_col = self.cursor.1;
    }

    fn next_word(&self, mut i: usize) -> usize {
        let s = &self.flat;
        if i + 1 >= s.len() {
            return i;
        }
        let c = char_class(s[i]);
        while i < s.len() && c != 0 && char_class(s[i]) == c {
            i += 1;
        }
        while i < s.len() && char_class(s[i]) == 0 {
            i += 1;
        }
        i.min(s.len() - 1)
    }

    fn prev_word(&self, i: usize) -> usize {
        let s = &self.flat;
        let mut i = i as isize - 1;
        while i > 0 && char_class(s[i as usize]) == 0 {
            i -= 1;
        }
        let c = if i >= 0 { char_class(s[i as usize]) } else { 0 };
        while i > 0 && char_class(s[i as usize - 1]) == c {
            i -= 1;
        }
        i.max(0) as usize
    }

    fn word_end(&self, i: usize) -> usize {
        let s = &self.flat;
        let mut i = i + 1;
        while i < s.len() && char_class(s[i]) == 0 {
            i += 1;
        }
        if i >= s.len() {
            return s.len().saturating_sub(1);
        }
        let c = char_class(s[i]);
        while i + 1 < s.len() && char_class(s[i + 1]) == c {
            i += 1;
        }
        i
    }

    /// { / }: 次 (前) の空行へ
    fn next_paragraph(&self, line: usize, d: isize) -> usize {
        let last = self.lines.len() as isize - 1;
        let mut line = line as isize + d;
        while 0 < line && line < last && !is_blank(&self.lines[line as usize].text) {
            line += d;
        }
        line.clamp(0, last) as usize
    }

    // ---- モード

    pub fn set_mode(&mut self, mode: Mode) {
        if self.mode == Mode::Caret && mode != Mode::Caret {
            self.anchor = self.cursor; // キャレットから選択を始めるときは、今の位置が起点
        }
        self.mode = mode;
    }

    /// マウスのドラッグで選ぶ: anchor から cursor まで (どちらも (行, 文字の位置))
    pub fn select(&mut self, anchor: Pos, cursor: Pos) {
        let clamp = |(l, c): Pos| {
            let l = l.min(self.lines.len().saturating_sub(1));
            (l, c.min(self.width(l).saturating_sub(1)))
        };
        let (anchor, cursor) = (clamp(anchor), clamp(cursor));
        self.mode = Mode::Char;
        self.anchor = anchor;
        self.cursor = cursor;
        self.want_col = self.cursor.1;
    }

    /// o: 選択の反対の端へ移る
    pub fn swap(&mut self) {
        std::mem::swap(&mut self.anchor, &mut self.cursor);
    }

    // ---- 選択範囲

    /// 選択の始まりと終わり (終わりの文字を含む)
    pub fn span(&self) -> (Pos, Pos) {
        let (a, b) = if self.anchor <= self.cursor { (self.anchor, self.cursor) } else { (self.cursor, self.anchor) };
        match self.mode {
            Mode::Line => ((a.0, 0), (b.0, self.width(b.0).saturating_sub(1))),
            Mode::Caret => (self.cursor, self.cursor),
            Mode::Char => (a, b),
        }
    }

    /// n 行目で反転させる文字の範囲 [start, end)
    fn line_range(&self, n: usize) -> Option<(usize, usize)> {
        let ((l0, c0), (l1, c1)) = self.span();
        if !(l0 <= n && n <= l1) {
            return None;
        }
        let start = if n == l0 { c0 } else { 0 };
        let end = (if n == l1 { c1 + 1 } else { self.lines[n].text.len() }).min(self.width(n)); // 行末の空白は反転させない
        Some((start, end.max(start + 1)))
    }

    /// 選んだ文字列。画面の折り返しでできた改行は取り除き、段落の区切りだけ改行にする
    pub fn selected_text(&self) -> String {
        let ((l0, c0), (l1, c1)) = self.span();
        let mut out: Vec<char> = Vec::new();
        for n in l0..=l1 {
            let line = &self.lines[n];
            let text = &line.text;
            let from = (if n == l0 { c0 } else { 0 }).max(line.indent).min(text.len());
            let to = (if n == l1 { c1 + 1 } else { text.len() }).min(text.len());
            let mut part: &[char] = if from < to { &text[from..to] } else { &[] };
            if n > l0 {
                let prev = &self.lines[n - 1];
                let wrapped = !prev.blks.is_empty() && !line.blks.is_empty() && prev.blks.last() == line.blks.first();
                if wrapped {
                    part = &part[lstrip_len(part)..]; // 箇条書きのぶら下げインデントを除く
                    let ascii_alnum = |c: Option<&char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
                    if ascii_alnum(out.last()) && ascii_alnum(part.first()) {
                        out.push(' '); // 英語は折り返しの位置で消えた空白を戻す
                    }
                } else {
                    out.push('\n');
                }
            }
            out.extend_from_slice(if n < l1 { rstrip(part) } else { part });
        }
        out.into_iter().collect()
    }

    // ---- 読み上げの開始位置 (S)

    /// n 行目の各文字がどのブロックの文字か。字下げや箇条書きの記号は None。
    /// 描画の印 (blk) がない行は、字下げの後ろを行のブロックの文字とみなす
    fn line_blocks(&self, n: usize) -> Vec<Option<usize>> {
        let line = &self.lines[n];
        let mut out: Vec<Option<usize>> = Vec::new();
        for s in &line.segs {
            out.extend(std::iter::repeat_n(s.blk, s.text.chars().count()));
        }
        if !out.iter().any(|b| b.is_some())
            && let Some(&b) = line.blks.first()
        {
            let indent = if line.indent > 0 { line.indent } else { lstrip_len(&line.text) };
            out = std::iter::repeat_n(None, indent).chain(std::iter::repeat_n(Some(b), line.text.len().saturating_sub(indent))).collect();
        }
        out
    }

    /// pos にあるブロックの番号と、そのブロックの pos から後ろの文字列。
    /// pos が空行や字下げの上なら、その後ろで最初のブロックの先頭から
    pub fn block_at(&self, pos: Pos) -> Option<(usize, String)> {
        let (line, col) = pos;
        let mut found = None;
        for n in line..self.lines.len() {
            let owners = self.line_blocks(n);
            let from = if n == line { col } else { 0 };
            if let Some(start) = (from..owners.len()).find(|&i| owners[i].is_some()) {
                found = Some((n, start, owners));
                break;
            }
        }
        let (n, start, mut owners) = found?;
        let blk = owners[start];
        let mut rest = String::new();
        for k in n..self.lines.len() {
            if k > n {
                owners = self.line_blocks(k);
            }
            if !owners.contains(&blk) {
                break;
            }
            for (i, &ch) in self.lines[k].text.iter().take(owners.len()).enumerate() {
                if owners[i] == blk && (k > n || i >= start) {
                    rest.push(ch);
                }
            }
        }
        Some((blk.unwrap(), rest))
    }

    /// 選んだ文字列を、ブロックごとに分けたもの (ブロック番号, 文字列)。選んだ範囲だけを読み上げるのに使う。
    /// 字下げや箇条書きの記号は含めない。折り返しで消えた英語の空白は戻す
    pub fn selected_blocks(&self) -> Vec<(usize, String)> {
        let ((l0, c0), (l1, c1)) = self.span();
        let mut out: Vec<(usize, String)> = Vec::new();
        for n in l0..=l1 {
            let owners = self.line_blocks(n);
            let text = &self.lines[n].text;
            let from = if n == l0 { c0 } else { 0 };
            let to = (if n == l1 { c1 + 1 } else { text.len() }).min(owners.len());
            let mut line_start = true;
            for i in from..to {
                let Some(b) = owners[i] else { continue };
                match out.last_mut() {
                    Some((last, s)) if *last == b => {
                        if line_start && s.ends_with(|c: char| c.is_ascii_alphanumeric()) && text[i].is_ascii_alphanumeric() {
                            s.push(' ');
                        }
                        s.push(text[i]);
                    }
                    _ => out.push((b, text[i].to_string())),
                }
                line_start = false;
            }
        }
        out.retain(|(_, s)| !s.trim().is_empty());
        out
    }

    // ---- 描画

    /// 描画済みの行を、選択範囲を反転させて描き直したもの
    pub fn render(&self) -> Vec<Line> {
        self.lines
            .iter()
            .enumerate()
            .map(|(n, line)| match self.line_range(n) {
                None => Line { segs: line.segs.clone() },
                Some((start, end)) => {
                    let mut segs = highlight(&line.segs, start, end);
                    if end > line.text.len() {
                        // 空行の選択は空白を反転させて見せる
                        let mut s = Seg { text: " ".into(), ..Default::default() };
                        s.style.reverse = true;
                        segs.push(s);
                    }
                    Line { segs }
                }
            })
            .collect()
    }
}

/// segs のうち、行内の文字位置 [start, end) の部分を反転させる
pub fn highlight(segs: &[Seg], start: usize, end: usize) -> Vec<Seg> {
    let mut out = Vec::new();
    let mut pos = 0;
    for s in segs {
        let chars: Vec<char> = s.text.chars().collect();
        let (a, b) = (pos, pos + chars.len());
        pos = b;
        if b <= start || a >= end {
            out.push(s.clone());
            continue;
        }
        let (i, j) = (start.max(a) - a, end.min(b) - a);
        let mut sel = s.clone();
        sel.style.reverse = true;
        for (range, seg) in [(0..i, s), (i..j, &sel), (j..chars.len(), s)] {
            if !range.is_empty() {
                out.push(Seg { text: chars[range].iter().collect(), ..seg.clone() });
            }
        }
    }
    out
}
