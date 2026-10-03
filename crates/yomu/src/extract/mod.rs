//! HTML から本文を探し、Block 列に変換する。Python 版の extract.py の移植。
//!
//! 本文の判定は rs-trafilatura に任せる。ページの種類 (記事・一覧・商品など) ごとにルールを切り替えるので、一覧ページにも強い。
//! ただし rs-trafilatura が返す本文は属性 (href・alt など) が消えているので、そのままは使えない。
//! そこで元のページ全体を Block に分け、rs-trafilatura の本文テキストに含まれるブロックだけを残す。
//! ナビなど同じ文言がページ内の別の場所にもあるときに拾わないよう、一致したブロックが最も密集した連続範囲に限る。
//!
//! 照合の前に、確実にナビとわかる要素 (nav・navbox・パンくずなど) は消しておく。
//!
//! rs-trafilatura は記事本文を取りこぼして関連記事の一覧などを本文とすることがある (ITmedia の記事で確認)。
//! そこで自前のルール (itemprop=articleBody → article → main → 句読点密度スコア) でも本文を探して比べ、
//! 「ほとんど重ならず、自前の方がずっと文章が多い」ときは自前の方を使う。
//! 文章の量は、リンクを除いた、句点で終わる文の文字数で測る (関連記事の見出しはリンクなので数えない)。
//!
//! rs-trafilatura が本文を返さなかったとき・元のページと照合できなかったときは、ページ全体を出す。
pub mod dom;

use crate::pytext::{is_space, strip};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

use crate::doc::{Block, Document, Form, Kind, Span};
use dom::{Id, Tree};
use crate::urls::{urldefrag, urljoin};

const SHINGLE: usize = 4; // テキストの一致判定に使う文字 n-gram の長さ
const MIN_COVERAGE: f64 = 0.5; // ブロックの n-gram のうち、本文テキストに含まれる割合がこれ以上なら本文とみなす
// rs-trafilatura の結果より自前のルールの結果を使う条件 (実ページで調整。tests/extract.rs の real_pages)
const SWITCH_MIN_PROSE: usize = 300; // 自前の方の文章がこの文字数以上
const SWITCH_RATIO: f64 = 1.5; // 自前の方の文章が rs-trafilatura の何倍以上か
const SWITCH_MAX_OVERLAP: f64 = 0.3; // 自前の方の文のうち、rs-trafilatura の結果にもある割合がこれ未満
const TITLE_WINDOW: usize = 10; // 記事タイトルの繰り返しを探す、本文の先頭からのブロック数

static SENTENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^。！？!?\n]{8,}[。！？!?]").unwrap());
static TITLE_SEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+[|｜\-–—:：]\s+|[|｜]").unwrap()); // 「記事名 | サイト名」の区切り

// どのモードでも消す要素
const DROP_TAGS: &[&str] = &[
    "script", "style", "noscript", "template", "svg", "iframe", "form", "button", "input", "select", "textarea", "canvas",
    "object", "embed", "link", "meta", "dialog", "head",
];
// 自前のルールで本文を探すときに消す、ナビ・広告などの要素
const CHROME_TAGS: &[&str] = &["nav", "aside", "footer"];
const CHROME_ROLES: &[&str] = &["navigation", "banner", "contentinfo", "complementary", "search", "dialog"];
static BOILER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:^|[\s_-])(?:nav|navbar|navbox|navigation|g?menu|breadcrumbs?|footer|sidebar|side-?bar|share|sharing|social|sns|related|recommend(?:ed|ation)?s?|ranking|popular|ads?|advert\w*|banner|promo|sponsored?|comments?|cookie|consent|popup|modal|subscribe|newsletter|pagination|pager|toc|mw-editsection|editsection|noprint|catlinks|printfooter|skip-?link)(?:$|[\s_-])",
    )
    .unwrap()
});
// rs-trafilatura に渡す前に消す、確実にナビとわかる要素 (コメント欄などは本文になりうるので含めない)
static SURE_CHROME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:^|\s)(?:navbox\S*|sidebar|vertical-navbox|catlinks|printfooter|mw-editsection|noprint|breadcrumbs?|skip-?link)(?:$|\s)",
    )
    .unwrap()
});
// XHTML の先頭の <?xml ... encoding="..."?>
static XML_DECL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\s*<\?xml[^>]*\?>").unwrap());
static HIDDEN_STYLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)display\s*:\s*none|visibility\s*:\s*hidden").unwrap());

// ブロックとして扱う要素 (それ以外はインライン)
const BLOCK_TAGS: &[&str] = &[
    "p", "div", "section", "article", "main", "header", "center", "figure", "figcaption", "address", "details", "body", "html",
    "table", "tbody", "thead", "tfoot", "tr", "td", "th", "caption", "ul", "ol", "dl", "li", "dt", "dd", "blockquote", "pre",
    "hr", "summary", "h1", "h2", "h3", "h4", "h5", "h6", "nav", "aside", "footer",
];


fn is_cjk(c: char) -> bool {
    matches!(c, '\u{3000}'..='\u{30ff}' | '\u{3400}'..='\u{9fff}' | '\u{f900}'..='\u{faff}' | '\u{ff00}'..='\u{ffef}')
}

/// 空白の並びを 1 つの空白にする (re.sub(r"\s+", " ", s))
fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_ws = false;
    for c in s.chars() {
        if is_space(c) {
            if !in_ws {
                out.push(' ');
            }
            in_ws = true;
        } else {
            out.push(c);
            in_ws = false;
        }
    }
    out
}


/// 一致判定用に、記号と空白を除いて小文字にする
fn norm(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

fn textlen(t: &Tree, id: Id) -> usize {
    t.text_content(id).chars().filter(|&c| !is_space(c)).count()
}

fn link_density(t: &Tree, id: Id) -> f64 {
    let n = textlen(t, id);
    if n == 0 {
        return 1.0;
    }
    let links: usize = t.iter(id).into_iter().filter(|&e| t.tag(e) == "a").map(|a| textlen(t, a)).sum();
    links as f64 / n as f64
}

fn is_hidden(t: &Tree, id: Id) -> bool {
    t.get(id, "hidden").is_some()
        || t.get(id, "aria-hidden") == Some("true")
        || HIDDEN_STYLE.is_match(t.get(id, "style").unwrap_or(""))
}

fn prune_always(t: &mut Tree, root: Id) {
    for el in t.iter(root) {
        if DROP_TAGS.contains(&t.tag(el)) || is_hidden(t, el) {
            t.remove(el);
        }
    }
}

/// ナビ・広告・関連記事などを消す。本文の半分以上を占める要素は誤爆とみなして残す
fn prune_boilerplate(t: &mut Tree, root: Id) {
    let total = textlen(t, root).max(1) as f64;
    for el in t.iter(root) {
        if el == root || t.parent(el).is_none() {
            continue;
        }
        // 祖先がすでに消えていれば孤立しているので飛ばす
        if !t.ancestors(el).any(|a| a == root) {
            continue;
        }
        let attrs: Vec<&str> = [t.get(el, "class"), t.get(el, "id")].into_iter().flatten().filter(|s| !s.is_empty()).collect();
        let tag = t.tag(el);
        let chrome = CHROME_TAGS.contains(&tag)
            || t.get(el, "role").is_some_and(|r| CHROME_ROLES.contains(&r))
            || (tag == "header" && !t.descendants(el).iter().any(|&d| t.tag(d) == "h1"))
            || BOILER.is_match(&attrs.join(" "));
        if chrome && (textlen(t, el) as f64) < total * 0.5 {
            t.remove(el);
        }
    }
}

fn prune_sure_chrome(t: &mut Tree, root: Id) {
    for el in t.iter(root) {
        if t.parent(el).is_none() {
            continue;
        }
        if t.tag(el) == "nav" || t.get(el, "role") == Some("navigation") || SURE_CHROME.is_match(t.get(el, "class").unwrap_or("")) {
            t.remove(el);
        }
    }
}

/// 本文を含む要素を 1 つ選ぶ
fn find_main(t: &Tree, body: Id) -> Id {
    let desc = t.descendants(body);
    let finders: [&dyn Fn(Id) -> bool; 4] = [
        &|e| t.get(e, "itemprop") == Some("articleBody"),
        &|e| t.tag(e) == "article",
        &|e| t.tag(e) == "main",
        &|e| t.get(e, "role") == Some("main"),
    ];
    for f in finders {
        let cands: Vec<Id> = desc.iter().copied().filter(|&e| f(e)).collect();
        if !cands.is_empty() {
            let best = max_first(&cands, |&e| textlen(t, e) as f64);
            if textlen(t, best) >= 200 {
                return best;
            }
        }
    }
    // 句読点密度スコア (Readability の簡易版)。<p> を使わず <br> で段落を作るサイトもあるので、
    // 直下のテキストが長い要素をすべて「段落」として数える
    let mut order: Vec<Id> = Vec::new();
    let mut scores: HashMap<Id, f64> = HashMap::new();
    for el in t.iter(body) {
        let mut own = t.nodes[el].text.clone();
        for &c in t.children(el) {
            own.push_str(&t.nodes[c].tail);
        }
        let n = own.chars().filter(|&c| !is_space(c)).count();
        if n < 30 {
            continue;
        }
        let punct = own.chars().filter(|c| "、。，,．".contains(*c)).count();
        let s = 1.0 + punct as f64 + (n as f64 / 100.0).min(3.0);
        let parent = t.parent(el);
        let grand = parent.and_then(|p| t.parent(p));
        for (anc, w) in [(Some(el), 1.0), (parent, 0.5), (grand, 0.25)] {
            if let Some(a) = anc {
                let e = scores.entry(a).or_insert_with(|| {
                    order.push(a);
                    0.0
                });
                *e += s * w;
            }
        }
    }
    if order.is_empty() {
        return body;
    }
    max_first(&order, |&e| scores[&e] * (1.0 - link_density(t, e)))
}

/// Python の max(): 最大のもののうち最初のもの
fn max_first<T: Copy>(items: &[T], key: impl Fn(&T) -> f64) -> T {
    let mut best = items[0];
    let mut bk = key(&best);
    for it in &items[1..] {
        let k = key(it);
        if k > bk {
            best = *it;
            bk = k;
        }
    }
    best
}

/// Python の re.search(f"[{CJK}]$", s): 末尾 (末尾が改行ならその前も) が日本語の文字か
fn ends_with_cjk(s: &str) -> bool {
    let mut it = s.chars().rev();
    match it.next() {
        Some(c) if is_cjk(c) => true,
        Some('\n') => it.next().is_some_and(is_cjk),
        _ => false,
    }
}

/// 空白を詰める。日本語の文字どうしの間に入った改行由来の空白は消す
pub fn normalize_spans(spans: &[Span]) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut prev = String::new();
    for s in spans {
        let mut t = if s.text == "\n" { s.text.clone() } else { collapse_ws(&s.text) };
        if t == "\n" && (strip(&prev).is_empty() || prev.ends_with("\n\n")) {
            continue; // 先頭や 3 つ以上続く <br> は捨てる
        }
        if t.starts_with(' ') && (prev.is_empty() || prev.ends_with([' ', '\n'])) {
            t.remove(0);
        }
        if t.starts_with(' ') && t.chars().nth(1).is_some_and(is_cjk) && ends_with_cjk(&prev) {
            t.remove(0);
        }
        let t = remove_cjk_gaps(&t);
        if !t.is_empty() {
            prev.push_str(&t);
            out.push(Span { text: t, ..s.clone() });
        }
    }
    while out.last().is_some_and(|s| strip(&s.text).is_empty()) {
        out.pop();
    }
    if let Some(last) = out.last_mut() {
        last.text = last.text.trim_end_matches(is_space).to_string();
    }
    if strip(&out.iter().map(|s| s.text.as_str()).collect::<String>()).is_empty() {
        return vec![];
    }
    out
}

/// 日本語の文字に挟まれた半角空白を消す
fn remove_cjk_gaps(t: &str) -> String {
    let chars: Vec<char> = t.chars().collect();
    let mut out = String::with_capacity(t.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == ' ' && i > 0 && is_cjk(chars[i - 1]) && chars.get(i + 1).is_some_and(|&n| is_cjk(n)) {
            continue;
        }
        out.push(c);
    }
    out
}

/// 今あふれているインライン要素をどの種類のブロックとして確定するか
#[derive(Clone, Debug, Default)]
struct Ctx {
    kind: Kind,
    level: u32,
    marker: Option<String>,
    caption: bool,
    list: Option<&'static str>, // "ul" / "ol"
    n: usize,
    depth: u32,
}

#[derive(Clone, Debug, Default)]
struct St {
    link: Option<usize>,
    bold: bool,
    code: bool,
    italic: bool,
    strike: bool,
    minor: bool,
    rt_paren: bool,
}

/// DOM をたどって Block の列を組み立てる
struct BlockBuilder<'a> {
    t: &'a Tree,
    base: String,
    page: String,
    blocks: Vec<Block>,
    links: Vec<String>,
    link_idx: HashMap<String, usize>,
    cur: Vec<Span>,
    pending_anchors: Vec<String>, // 次に確定するブロックに付ける id
    ctx: Vec<Ctx>,
}

impl<'a> BlockBuilder<'a> {
    fn new(t: &'a Tree, base: &str) -> Self {
        Self {
            t,
            base: base.to_string(),
            page: urldefrag(base).0,
            blocks: vec![],
            links: vec![],
            link_idx: HashMap::new(),
            cur: vec![],
            pending_anchors: vec![],
            ctx: vec![Ctx::default()],
        }
    }

    // ---- インライン
    fn link_index(&mut self, href: Option<&str>) -> Option<usize> {
        let href = href?;
        if href.is_empty() || href.starts_with("javascript:") || href.starts_with("mailto:") || href.starts_with("tel:") {
            return None;
        }
        let url = urljoin(&self.base, strip(href));
        if url == self.page {
            return None; // 自分自身へのリンク (# なし) は番号を振らない
        }
        if let Some(&i) = self.link_idx.get(&url) {
            return Some(i);
        }
        let i = self.links.len();
        self.link_idx.insert(url.clone(), i);
        self.links.push(url);
        Some(i)
    }

    fn add(&mut self, text: &str, st: &St) {
        if !text.is_empty() {
            self.cur.push(Span {
                text: text.to_string(),
                link: st.link,
                bold: st.bold,
                code: st.code,
                italic: st.italic,
                strike: st.strike,
                minor: st.minor,
                image: false,
                translated: false,
                src: None,
            });
        }
    }

    fn image(&mut self, el: Id, st: &St) {
        let alt = collapse_ws(self.t.get(el, "alt").unwrap_or(""));
        let alt = strip(&alt);
        // 画像を表示する設定のときだけ、画像の URL を持たせる (alt のない画像も「［画像］」として入れる)
        let src = if WITH_IMAGES.get() { image_src(self.t, el).map(|s| urljoin(&self.base, s)) } else { None };
        if alt.is_empty() {
            if src.is_some() {
                self.cur.push(Span { text: t!("［画像］").into(), link: st.link, image: true, src, ..Default::default() });
            }
            return;
        }
        if alt.chars().count() <= 2 && !alt.chars().all(|c| c.is_alphanumeric()) {
            self.add(alt, st); // 絵文字画像はそのまま文字として出す
        } else {
            self.cur.push(Span { text: t!("［画像: {alt}］", alt = alt), link: st.link, image: true, src, ..Default::default() });
        }
    }

    // ---- ブロック
    fn emit(&mut self, mut block: Block) {
        block.anchors.append(&mut self.pending_anchors);
        self.blocks.push(block);
    }

    fn flush(&mut self) {
        let spans = normalize_spans(&self.cur);
        self.cur.clear();
        if spans.is_empty() {
            return;
        }
        let c = self.ctx.last_mut().unwrap();
        let block = if c.kind == Kind::Li {
            Block { kind: Kind::Li, spans, level: c.level, marker: c.marker.take().unwrap_or_default(), ..Default::default() }
        } else {
            Block { kind: c.kind, spans, level: c.level, caption: c.caption, ..Default::default() }
        };
        self.emit(block);
    }

    fn push(&mut self, c: Ctx) {
        self.flush();
        self.ctx.push(c);
    }

    fn pop(&mut self) {
        self.flush();
        self.ctx.pop();
    }

    fn walk(&mut self, el: Id, st: &St) {
        let t = self.t;
        let tag = t.tag(el);
        let anchor = t.get(el, "id").filter(|s| !s.is_empty()).or_else(|| if tag == "a" { t.get(el, "name") } else { None });
        if let Some(a) = anchor.filter(|s| !s.is_empty()) {
            self.pending_anchors.push(a.to_string());
        }
        match tag {
            "br" => self.add("\n", st),
            "img" => self.image(el, st),
            "ruby" => {
                // ふりがな: 漢字（かんじ）。括弧 (rp) を持つルビは自前で括弧を足さない
                let has_rp = t.descendants(el).iter().any(|&d| t.tag(d) == "rp" || t.get(d, "class").is_some_and(|c| c.contains("rp")));
                self.children(el, &St { rt_paren: !has_rp, ..st.clone() });
            }
            "rt" => {
                let text = t.text_content(el);
                let x = strip(&text);
                let s = if st.rt_paren { format!("（{x}）") } else { x.to_string() };
                self.add(&s, st);
            }
            "hr" => {
                self.flush();
                self.emit(Block::new(Kind::Hr, vec![]));
            }
            "pre" => {
                self.flush();
                let text = t.text_content(el);
                let txt = text.trim_matches('\n');
                if !strip(txt).is_empty() {
                    self.emit(Block::new(Kind::Pre, vec![Span { text: txt.to_string(), code: true, ..Default::default() }]));
                }
            }
            "table" if self.is_data_table(el) => self.table(el, st),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.push(Ctx { kind: Kind::H, level: tag[1..].parse().unwrap(), ..Default::default() });
                self.children(el, st);
                self.pop();
            }
            "ul" | "ol" => {
                let depth = self.ctx.iter().filter(|c| c.list.is_some()).count() as u32;
                let top = self.ctx.last().unwrap();
                let c = Ctx {
                    kind: top.kind,
                    level: top.level,
                    list: Some(if tag == "ul" { "ul" } else { "ol" }),
                    depth,
                    ..Default::default()
                };
                self.push(c);
                self.children(el, st);
                self.pop();
            }
            "li" => {
                let (ordered, n, depth) = match self.ctx.iter_mut().rev().find(|c| c.list.is_some()) {
                    Some(l) => {
                        l.n += 1;
                        (l.list == Some("ol"), l.n, l.depth)
                    }
                    None => (false, 1, 0),
                };
                let marker = if ordered { format!("{n}.") } else { "・".to_string() };
                self.push(Ctx { kind: Kind::Li, level: depth, marker: Some(marker), ..Default::default() });
                self.children(el, st);
                self.pop();
            }
            "blockquote" => {
                self.push(Ctx { kind: Kind::Quote, ..Default::default() });
                self.children(el, st);
                self.pop();
            }
            "figcaption" => {
                self.push(Ctx { kind: Kind::P, caption: true, ..Default::default() });
                self.children(el, st);
                self.pop();
            }
            "dt" | "summary" | "caption" => {
                self.flush();
                self.children(el, &St { bold: true, ..st.clone() });
                self.flush();
            }
            "dd" => {
                self.push(Ctx { kind: Kind::Li, level: 1, marker: Some(String::new()), ..Default::default() });
                self.children(el, st);
                self.pop();
            }
            _ if BLOCK_TAGS.contains(&tag) => {
                self.flush();
                self.children(el, st);
                self.flush();
            }
            _ => {
                let mut st = st.clone();
                match tag {
                    "a" => st.link = self.link_index(t.get(el, "href")),
                    "b" | "strong" => st.bold = true,
                    "em" | "i" | "cite" | "dfn" => st.italic = true,
                    "del" | "s" | "strike" => st.strike = true,
                    "small" => st.minor = true,
                    "code" | "kbd" | "samp" | "tt" => st.code = true,
                    _ => {}
                }
                self.children(el, &st);
            }
        }
    }

    fn children(&mut self, el: Id, st: &St) {
        let t = self.t;
        self.add(&t.nodes[el].text, st);
        for &c in t.children(el) {
            self.walk(c, st);
            self.add(&t.nodes[c].tail, st);
        }
    }

    // ---- 表
    fn rows(&self, el: Id) -> Vec<Id> {
        // ./tr|./*/tr を文書順に
        let t = self.t;
        let mut out = Vec::new();
        for &c in t.children(el) {
            if t.tag(c) == "tr" {
                out.push(c);
            }
            for &g in t.children(c) {
                if t.tag(g) == "tr" {
                    out.push(g);
                }
            }
        }
        out
    }

    fn cells(&self, tr: Id) -> Vec<Id> {
        self.t.children(tr).iter().copied().filter(|&c| matches!(self.t.tag(c), "td" | "th")).collect()
    }

    /// レイアウト目的の表 (古い日本語サイトに多い) は普通のブロックとして流す
    fn is_data_table(&self, el: Id) -> bool {
        let t = self.t;
        let desc = t.descendants(el);
        if desc.iter().any(|&d| t.tag(d) == "table") {
            return false;
        }
        let rows = self.rows(el);
        if rows.is_empty() || rows.iter().map(|&r| self.cells(r).len()).max().unwrap_or(0) < 2 {
            return false;
        }
        desc.iter().filter(|&&d| matches!(t.tag(d), "td" | "th")).all(|&c| textlen(t, c) <= 300)
    }

    fn table(&mut self, el: Id, st: &St) {
        self.flush();
        let mut rows = Vec::new();
        for tr in self.rows(el) {
            let mut cells = Vec::new();
            for td in self.cells(tr) {
                let saved = std::mem::take(&mut self.cur);
                let bold = st.bold || self.t.tag(td) == "th";
                self.children(td, &St { bold, ..st.clone() });
                cells.push(normalize_spans(&self.cur));
                self.cur = saved;
            }
            if cells.iter().any(|c| !c.is_empty()) {
                rows.push(cells);
            }
        }
        if !rows.is_empty() {
            self.emit(Block { kind: Kind::Table, rows: Some(rows), ..Default::default() });
        }
    }
}

fn page_title(t: &Tree) -> String {
    let all = t.iter(t.root());
    let og = all
        .iter()
        .find(|&&e| t.tag(e) == "meta" && t.get(e, "property") == Some("og:title") && t.get(e, "content").is_some())
        .and_then(|&e| t.get(e, "content"));
    let title = match og {
        Some(o) => o.to_string(),
        None => all.iter().find(|&&e| t.tag(e) == "title").map(|&e| t.nodes[e].text.clone()).unwrap_or_default(),
    };
    strip(&collapse_ws(&title)).to_string()
}

/// ページが宣言した言語 (<html lang="…">、なければ <meta http-equiv="content-language">)。なければ空
fn page_lang(t: &Tree) -> String {
    let all = t.iter(t.root());
    let html = all.iter().find(|&&e| t.tag(e) == "html").and_then(|&e| t.get(e, "lang"));
    let meta = || {
        all.iter()
            .find(|&&e| t.tag(e) == "meta" && t.get(e, "http-equiv").is_some_and(|v| v.eq_ignore_ascii_case("content-language")))
            .and_then(|&e| t.get(e, "content"))
    };
    html.or_else(meta).map(|l| l.trim().to_string()).unwrap_or_default()
}

/// rs-trafilatura が本文と判定したテキスト。判定できなければ None
fn reference_text(html: &str, url: &str) -> Option<String> {
    let options = rs_trafilatura::Options {
        url: Some(url.to_string()),
        include_tables: true,
        include_images: true,
        ..rs_trafilatura::Options::default()
    };
    // 壊れた HTML などで落ちてもページ全体を出して続ける
    let r = std::panic::catch_unwind(|| rs_trafilatura::extract_with_options(html, &options)).ok()?.ok()?;
    Some(r.content_text).filter(|s| !s.is_empty())
}

pub fn select_by_reference(blocks: &[Block], ref_text: &str) -> Option<Vec<Block>> {
    let r = norm(ref_text);
    let rc: Vec<char> = r.chars().collect();
    if rc.len() < 50 {
        return None;
    }
    let grams: HashSet<String> = rc.windows(SHINGLE).map(|w| w.iter().collect()).collect();
    let matched = |b: &Block| -> Option<bool> {
        let t = norm(&b.text());
        if t.is_empty() || b.image_only() {
            // 区切り線や画像など、本文テキストと照合できないブロック
            return None;
        }
        let tc: Vec<char> = t.chars().collect();
        if tc.len() < SHINGLE {
            return Some(r.contains(&t));
        }
        let g: Vec<String> = tc.windows(SHINGLE).map(|w| w.iter().collect()).collect();
        let hit = g.iter().filter(|x| grams.contains(*x)).count();
        Some(hit as f64 / g.len() as f64 >= MIN_COVERAGE)
    };
    let flags: Vec<Option<bool>> = blocks.iter().map(matched).collect();
    // 一致すれば +文字数、しなければ -文字数 として、合計が最大になる連続範囲を選ぶ
    let (mut best, mut best_range, mut cur, mut start) = (0i64, None, 0i64, 0usize);
    for (i, (b, f)) in blocks.iter().zip(&flags).enumerate() {
        let w = match f {
            None => 0,
            Some(m) => {
                let n = norm(&b.text()).chars().count() as i64;
                if *m { n } else { -n }
            }
        };
        if cur <= 0 {
            cur = w;
            start = i;
        } else {
            cur += w;
        }
        if cur > best {
            best = cur;
            best_range = Some((start, i));
        }
    }
    let (lo, hi) = best_range?;
    let mut kept = Vec::new();
    for i in lo..=hi {
        match flags[i] {
            Some(true) => kept.push(blocks[i].clone()),
            None => {
                // 照合できないブロックは、前後のブロックが両方とも本文なら残す
                let prev = flags[lo..i].iter().rev().find_map(|f| *f).unwrap_or(false);
                let next = flags[i + 1..=hi].iter().find_map(|f| *f).unwrap_or(false);
                if prev && next {
                    kept.push(blocks[i].clone());
                }
            }
            Some(false) => {}
        }
    }
    (!kept.is_empty()).then_some(kept)
}

/// 段落・箇条書き・引用の中の、リンクを除いた文 (句点で終わる 8 文字以上)
fn sentences(blocks: &[Block]) -> Vec<String> {
    let mut out = Vec::new();
    for b in blocks {
        if matches!(b.kind, Kind::P | Kind::Li | Kind::Quote) {
            let text: String = b.spans.iter().filter(|s| s.link.is_none() && !s.image).map(|s| s.text.as_str()).collect();
            out.extend(SENTENCE.find_iter(&text).map(|m| m.as_str().to_string()));
        }
    }
    out
}

/// rs-trafilatura が本文を取りこぼしていて、自前のルールの結果の方がよいか
fn prefer_own(rs_blocks: &[Block], own_blocks: &[Block]) -> bool {
    let rs_prose: usize = sentences(rs_blocks).iter().map(|s| s.chars().count()).sum();
    let own = sentences(own_blocks);
    let own_prose: usize = own.iter().map(|s| s.chars().count()).sum();
    if own_prose < SWITCH_MIN_PROSE || (own_prose as f64) < rs_prose as f64 * SWITCH_RATIO {
        return false;
    }
    let rs_text: String = rs_blocks.iter().map(|b| norm(&b.text())).collect();
    let overlap: usize = own
        .iter()
        .filter(|x| rs_text.contains(&norm(x).chars().take(20).collect::<String>()))
        .map(|x| x.chars().count())
        .sum();
    (overlap as f64 / own_prose as f64) < SWITCH_MAX_OVERLAP
}

/// 本文の先頭に記事タイトルが何度も出るのをまとめ、同じ文言が続くブロックを 1 つにする。
/// 先頭付近でタイトル (か「記事名 | サイト名」の記事名) と同じ文言のブロックが複数あれば、
/// 最も上位の見出しを 1 つだけ残す (見出しがなければ残さない。TUI の先頭には記事タイトルが出る)
pub fn dedupe(blocks: Vec<Block>, title: &str) -> Vec<Block> {
    let names: HashSet<String> = std::iter::once(title)
        .chain(TITLE_SEP.split(title))
        .map(norm)
        .filter(|n| n.chars().count() >= 4)
        .collect();
    let repeats: Vec<usize> =
        blocks.iter().take(TITLE_WINDOW).enumerate().filter(|(_, b)| names.contains(&norm(&b.text()))).map(|(i, _)| i).collect();
    let mut drop = HashSet::new();
    if repeats.len() > 1 {
        let headings: Vec<usize> = repeats.iter().copied().filter(|&i| blocks[i].kind == Kind::H).collect();
        let keep = headings.iter().copied().min_by_key(|&i| blocks[i].level);
        drop = repeats.iter().copied().filter(|&i| Some(i) != keep).collect();
    }
    let mut out: Vec<Block> = Vec::new();
    for (i, b) in blocks.into_iter().enumerate() {
        let t = norm(&b.text());
        if drop.contains(&i) || (!t.is_empty() && out.last().is_some_and(|p| norm(&p.text()) == t)) {
            continue;
        }
        out.push(b);
    }
    out
}

/// 残ったブロックで使われているリンクだけに番号を振り直す
pub fn compact_links(mut blocks: Vec<Block>, links: &[String]) -> (Vec<Block>, Vec<String>) {
    let mut remap: HashMap<usize, usize> = HashMap::new();
    let mut new_links = Vec::new();
    let mut fix = |spans: &mut Vec<Span>| {
        for s in spans {
            if let Some(l) = s.link {
                let n = *remap.entry(l).or_insert_with(|| {
                    new_links.push(links[l].clone());
                    new_links.len() - 1
                });
                s.link = Some(n);
            }
        }
    };
    for b in &mut blocks {
        fix(&mut b.spans);
        for row in b.rows.iter_mut().flatten() {
            for cell in row {
                fix(cell);
            }
        }
    }
    (blocks, new_links)
}

fn to_blocks(t: &Tree, root: Id, url: &str) -> (Vec<Block>, Vec<String>) {
    let mut b = BlockBuilder::new(t, url);
    b.walk(root, &St::default());
    b.flush();
    (b.blocks, b.links)
}

/// HTML を読み、スクリプトや隠れた要素を消して (木, タイトル, body 要素) を返す
fn parse(html: &str) -> (Tree, String, String, Id) {
    // 文字コードはもう判定して文字列にしてあるので、XML 宣言 (encoding を含む) は取り除いてよい
    let html = XML_DECL.replace(html, "");
    let html = if html.is_empty() { "<html></html>".into() } else { html };
    let mut t = Tree::parse(&html);
    let (title, lang) = (page_title(&t), page_lang(&t));
    // フォームは消す前に拾う (画面で開いたときだけ)
    if WITH_FORMS.get() {
        FORMS.set(collect_forms(&t, PAGE_URL.with_borrow(|u| u.clone()).as_str()));
    }
    let root = t.root();
    prune_always(&mut t, root);
    let body = t.find_child(t.root(), "body").unwrap_or(t.root());
    (t, title, lang, body)
}

fn doc(url: &str, title: &str, lang: &str, (blocks, links): (Vec<Block>, Vec<String>), note: &str, full: bool) -> Document {
    let (url, title, note, lang) = (url.to_string(), title.to_string(), note.to_string(), lang.to_string());
    Document { url, title, blocks, links, note, full, lang, forms: vec![] }
}

thread_local! {
    /// 画像の URL も拾うか (extract_for_view で画像を表示する設定のときだけ true)
    static WITH_IMAGES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// ページの入力欄も拾うか (extract_for_view の間だけ true)。拾ったものは FORMS に入れる
    static WITH_FORMS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FORMS: std::cell::RefCell<Vec<Form>> = const { std::cell::RefCell::new(Vec::new()) };
    static PAGE_URL: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// 文字を入力する欄の type
const TEXT_TYPES: &[&str] = &["", "text", "search", "email", "url", "tel", "number"];

/// ページのフォームのうち、文字を入力する欄があるもの。値はブラウザがそのまま送ったときと同じにする
/// (隠し項目・選択肢の初期値・チェック済みの項目・名前のある最初の送信ボタン)
fn collect_forms(t: &Tree, url: &str) -> Vec<Form> {
    let mut out = Vec::new();
    for f in t.iter(t.root()).into_iter().filter(|&e| t.tag(e) == "form") {
        let get = !t.get(f, "method").is_some_and(|m| m.trim().eq_ignore_ascii_case("post"));
        let action = t.get(f, "action").map(str::trim).filter(|a| !a.is_empty() && !a.starts_with("javascript:"));
        let action = action.map_or_else(|| url.to_string(), |a| urljoin(url, a));
        let (mut fields, mut target, mut submit) = (Vec::new(), None, None);
        let mut label = String::new();
        for e in t.descendants(f) {
            let name = t.get(e, "name").unwrap_or("").to_string();
            let value = t.get(e, "value").unwrap_or("").to_string();
            let disabled = t.get(e, "disabled").is_some();
            match t.tag(e) {
                _ if disabled => {}
                "input" => {
                    let ty = t.get(e, "type").unwrap_or("").trim().to_ascii_lowercase();
                    match ty.as_str() {
                        ty if TEXT_TYPES.contains(&ty) && !name.is_empty() => {
                            if target.is_none() {
                                target = Some(fields.len());
                                label = field_label(t, f, e);
                                fields.push((name, None));
                            } else {
                                fields.push((name, Some(value)));
                            }
                        }
                        "hidden" if !name.is_empty() => fields.push((name, Some(value))),
                        "checkbox" | "radio" if !name.is_empty() && t.get(e, "checked").is_some() => {
                            fields.push((name, Some(if value.is_empty() { "on".into() } else { value })))
                        }
                        "submit" | "image" if submit.is_none() => submit = Some((name, value, e)),
                        _ => {}
                    }
                }
                "textarea" if !name.is_empty() => {
                    if target.is_none() {
                        target = Some(fields.len());
                        label = field_label(t, f, e);
                        fields.push((name, None));
                    } else {
                        fields.push((name, Some(t.text_content(e))));
                    }
                }
                "select" if !name.is_empty() => {
                    let opts: Vec<Id> = t.descendants(e).into_iter().filter(|&o| t.tag(o) == "option").collect();
                    let chosen = opts.iter().copied().find(|&o| t.get(o, "selected").is_some()).or(opts.first().copied());
                    if let Some(o) = chosen {
                        let v = t.get(o, "value").map(str::to_string).unwrap_or_else(|| strip(&t.text_content(o)).to_string());
                        fields.push((name, Some(v)));
                    }
                }
                "button" if submit.is_none() && !t.get(e, "type").is_some_and(|ty| !ty.eq_ignore_ascii_case("submit")) => {
                    submit = Some((name, value, e))
                }
                _ => {}
            }
        }
        if target.is_none() {
            continue;
        }
        if let Some((name, value, e)) = submit {
            if !name.is_empty() {
                fields.push((name, Some(value.clone())));
            }
            if label.is_empty() {
                label = strip(&collapse_ws(&t.text_content(e))).to_string();
                if label.is_empty() {
                    label = value;
                }
            }
        }
        // 同じフォームが複数あれば (パソコン用とスマートフォン用など) 1 つにまとめる
        let f = Form { action, get, fields, label };
        if !out.contains(&f) {
            out.push(f);
        }
    }
    out
}

/// 入力欄の説明。placeholder → aria-label → <label for> → title の順
fn field_label(t: &Tree, form: Id, e: Id) -> String {
    let attr = ["placeholder", "aria-label", "title"].iter().find_map(|a| t.get(e, a).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string));
    let by_for = || {
        let id = t.get(e, "id")?;
        let l = t.iter(form).into_iter().find(|&l| t.tag(l) == "label" && t.get(l, "for") == Some(id))?;
        Some(strip(&collapse_ws(&t.text_content(l))).to_string()).filter(|s| !s.is_empty())
    };
    attr.or_else(by_for).unwrap_or_default()
}

/// 画面で開くときの抽出 (画像の URL は images が true のときだけ、ページの入力欄は必ず拾う)
pub fn extract_for_view(html: &str, url: &str, full: bool, images: bool) -> Document {
    WITH_IMAGES.set(images);
    WITH_FORMS.set(true);
    PAGE_URL.set(url.to_string());
    let mut doc = extract(html, url, full);
    WITH_IMAGES.set(false);
    WITH_FORMS.set(false);
    doc.forms = FORMS.take();
    doc
}

/// 画像の大きさの候補 (srcset) から選ぶ目安の幅 (ピクセル)。本文の幅 (80 文字前後) を、画像をきれいに描ける端末で
/// 描くのに足りる大きさ。これより大きい画像は端末では生かせず、通信が無駄になる
const IMAGE_TARGET_WIDTH: f64 = 800.0;

/// <img> の画像の URL。大きさの候補 (srcset) があればそこから選び (choose_from_srcset)、なければ遅れて読み込む画像
/// (data-src など)、src の順に見る。埋め込みの data: と SVG は描けないので None
fn image_src(t: &Tree, el: Id) -> Option<&str> {
    let usable = |s: &&str| !s.trim().is_empty() && !s.starts_with("data:") && !s.split(['?', '#']).next().unwrap_or("").ends_with(".svg");
    let from_srcset = |a: &str| choose_from_srcset(t.get(el, a)?, &usable);
    from_srcset("data-srcset")
        .or_else(|| from_srcset("srcset"))
        .or_else(|| ["data-src", "data-lazy-src", "data-original", "data-lazy", "src"].iter().filter_map(|a| t.get(el, a)).find(usable))
        .map(str::trim)
}

/// srcset (「URL 640w, URL 1280w」や「URL 1x, URL 2x」) から、描くのに足りる一番小さいものを選ぶ。
/// 幅で書かれていれば IMAGE_TARGET_WIDTH 以上のうち一番小さいもの (なければ一番大きいもの)、
/// 倍率で書かれていれば 2 倍までのうち一番大きいもの
fn choose_from_srcset<'a>(srcset: &'a str, usable: &dyn Fn(&&str) -> bool) -> Option<&'a str> {
    let mut by_width: Vec<(f64, &str)> = Vec::new();
    let mut by_density: Vec<(f64, &str)> = Vec::new();
    for c in srcset.split(',') {
        let mut it = c.split_whitespace();
        let Some(url) = it.next().filter(|u| usable(u)) else { continue };
        let d = it.next().unwrap_or("1x");
        match (d.strip_suffix('w'), d.strip_suffix('x')) {
            (Some(w), _) => by_width.extend(w.parse().ok().map(|w| (w, url))),
            (_, Some(x)) => by_density.extend(x.parse().ok().map(|x| (x, url))),
            _ => {}
        }
    }
    let smallest_enough = by_width.iter().filter(|(w, _)| *w >= IMAGE_TARGET_WIDTH).min_by(|a, b| a.0.total_cmp(&b.0));
    let largest = || by_width.iter().max_by(|a, b| a.0.total_cmp(&b.0));
    let density = || by_density.iter().filter(|(x, _)| *x <= 2.0).max_by(|a, b| a.0.total_cmp(&b.0));
    smallest_enough.or_else(largest).or_else(density).map(|(_, u)| *u)
}


/// full=false で本文抽出、true で (スクリプト等を除いた) ページ全体を出す
pub fn extract(html: &str, url: &str, full: bool) -> Document {
    let (mut t, title, lang, body) = parse(html);
    if full {
        return doc(url, &title, &lang, to_blocks(&t, body, url), t!("全体表示"), true);
    }
    prune_sure_chrome(&mut t, body);
    let (blocks, links) = to_blocks(&t, body, url);
    let kept = reference_text(html, url).and_then(|r| select_by_reference(&blocks, &r));
    let Some(kept) = kept else {
        return doc(url, &title, &lang, (blocks, links), t!("全体表示 (本文を判定できませんでした)"), true);
    };
    // rs-trafilatura が記事本文を取りこぼしていないか、自前のルールの結果と比べる
    let main = find_main(&t, body);
    prune_boilerplate(&mut t, main);
    let (own_blocks, own_links) = to_blocks(&t, main, url);
    if prefer_own(&kept, &own_blocks) {
        return doc(url, &title, &lang, compact_links(dedupe(own_blocks, &title), &own_links), t!("本文抽出 (自前)"), false);
    }
    doc(url, &title, &lang, compact_links(dedupe(kept, &title), &links), t!("本文抽出"), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_forms_are_collected_with_default_values() {
        let html = r#"<html><body><header><form action="/search" role="search">
            <input type="hidden" name="lang" value="ja"><label for="q">サイト内検索</label><input id="q" name="q">
            <select name="cat"><option value="all">すべて</option><option value="news" selected>ニュース</option></select>
            <input type="checkbox" name="exact" checked><input type="checkbox" name="old">
            <button type="submit">検索</button></form>
            <form method="post" action="/login"><input name="user" placeholder="ユーザー名"></form></header>
            <p>本文</p></body></html>"#;
        let d = extract_for_view(html, "https://ex.com/page?x=1", true, false);
        assert_eq!(d.forms.len(), 2);
        let f = &d.forms[0];
        assert!(f.get && f.label == "サイト内検索" && f.action == "https://ex.com/search");
        assert_eq!(f.url("将棋 ルール"), "https://ex.com/search?lang=ja&q=%E5%B0%86%E6%A3%8B+%E3%83%AB%E3%83%BC%E3%83%AB&cat=news&exact=on");
        assert!(!d.forms[1].get && d.forms[1].label == "ユーザー名");
        // 画面で開くとき以外は拾わない (Python 版と比べる抽出の結果を変えない)
        assert!(extract(html, "https://ex.com/", true).forms.is_empty());
    }

    #[test]
    fn srcset_picks_the_smallest_image_wide_enough() {
        let ok = |s: &&str| !s.is_empty();
        assert_eq!(choose_from_srcset("a.jpg 400w, b.jpg 800w, c.jpg 1600w", &ok), Some("b.jpg"));
        assert_eq!(choose_from_srcset("a.jpg 300w, b.jpg 600w", &ok), Some("b.jpg"));
        assert_eq!(choose_from_srcset("a.jpg 1.5x, b.jpg 2x, c.jpg 3x", &ok), Some("b.jpg"));
        assert_eq!(choose_from_srcset("a.jpg", &ok), Some("a.jpg"));
    }
}
