//! 読み上げる文章の選び方。Python 版 (yomu-python) の src/yomu/speech.py の、Document から (ブロック番号, 文) の列を作る部分。
//! 文の分け方と言語の判定は yomu_tts::sentences、合成・再生は yomu_tts::speaker。
use std::sync::LazyLock;

use regex::Regex;
use yomu_tts::sentences::split_sentences;

use crate::pytext;
use crate::doc::{Block, Kind};

pub use yomu_tts::sentences::{Lang, detect_lang};

const LINK_RATIO_MAX: f64 = 0.5; // リンクの文字がこれ以上を占めるブロックはナビやタグ一覧とみなして読まない
static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://\S+").unwrap());
static LETTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\W\d_]").unwrap());
static BREADCRUMB_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[>›»/|]\s*$").unwrap());
/// 記事の末尾に付く、本文ではない定番の節
static TAIL_SECTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^\s*(脚注|注釈|出典|参考文献|参考資料|関連項目|関連記事|関連リンク|外部リンク|references?|notes?|footnotes|see also|external links|further reading)\s*$",
    )
    .unwrap()
});

/// 本文として読む文字列。画像の alt と URL は読まない
pub fn block_text(b: &Block) -> String {
    let t: String = b.spans.iter().filter(|s| !s.image).map(|s| s.text.as_str()).collect();
    pytext::strip(&URL.replace_all(&t, "")).to_string()
}

/// 見出しや箇条書きの項目のような、文になっていないものか (読み上げでは漢字を辞書の読みで読む。yomu_tts::katakana)。
/// 見出し (h1〜h6) の文と、ブロックの 1 行まるごとで「。！？」などで終わらない文 (「・ひらがなの語」「主に」、
/// <br> で改行した「【擬音語・擬態語】」など)。text は block_text(b) から split_sentences で分けた文
pub fn is_label(b: &Block, text: &str) -> bool {
    if b.kind == Kind::H {
        return true;
    }
    let ends_sentence =
        text.trim_end_matches(|c: char| "」』）)\"'”’".contains(c)).ends_with(|c: char| "。！？!?．.…".contains(c));
    !ends_sentence && block_text(b).split('\n').any(|line| pytext::strip(line) == text)
}

/// 本文のブロックか。次のものは読まない:
/// コード・表・区切り線 / パンくず・タグ・関連記事のようにリンクだらけのもの (見出しは除く。
/// 本文の見出しが別ページへのリンクになっているサイトがある) / 数字や記号だけのもの (いいね数など) /
/// 「>」などで終わるパンくず
pub fn is_body(b: &Block) -> bool {
    if !matches!(b.kind, Kind::H | Kind::P | Kind::Li | Kind::Quote) {
        return false;
    }
    let text = block_text(b);
    if !LETTER.is_match(&text) || BREADCRUMB_END.is_match(&text) {
        return false;
    }
    if b.kind == Kind::H {
        return true;
    }
    let linked: usize = b.spans.iter().filter(|s| s.link.is_some() && !s.image).map(|s| s.text.chars().count()).sum();
    (linked as f64) / (text.chars().count() as f64) < LINK_RATIO_MAX
}

/// (ブロック番号, 文) の列。本文のブロックだけを読む。下に読む本文がない見出し (記事一覧の見出しなど) も読まない
pub fn chunks_from(blocks: &[Block], start: usize) -> Vec<(usize, String)> {
    let mut body: Vec<bool> = blocks.iter().map(is_body).collect();
    let mut i = 0;
    while i < blocks.len() {
        // 脚注・参考文献などの節は、見出しから次の同格の見出しまで読まない
        let b = &blocks[i];
        if b.kind == Kind::H && TAIL_SECTION.is_match(&b.text()) {
            let mut j = i + 1;
            while j < blocks.len() && !(blocks[j].kind == Kind::H && blocks[j].level <= b.level) {
                body[j] = false;
                j += 1;
            }
            body[i] = false;
            i = j;
        } else {
            i += 1;
        }
    }
    for i in 0..blocks.len() {
        let b = &blocks[i];
        if b.kind == Kind::H && body[i] && !TAIL_SECTION.is_match(&b.text()) {
            let nxt = (i + 1..blocks.len())
                .find(|&j| blocks[j].kind == Kind::H && blocks[j].level <= b.level)
                .unwrap_or(blocks.len());
            let has_sub = (i + 1..nxt).any(|j| blocks[j].kind == Kind::H);
            // 小見出しを持つ見出しは、小見出しの側で判定されるので残す
            body[i] = has_sub || (i + 1..nxt).any(|j| body[j]);
        }
    }
    let mut out = Vec::new();
    for i in start..blocks.len() {
        if body[i] {
            out.extend(split_sentences(&block_text(&blocks[i])).into_iter().map(|s| (i, s)));
        }
    }
    out
}

/// 本文 (body) から読む文を選び、ブロック番号を画面に出ている全体表示 (shown) のものに付け替える。
/// body のブロックは shown のどれかと同じ文字列なので、先頭から順に対応づける
pub fn chunks_for_view(body: &[Block], shown: &[Block], start: usize) -> Vec<(usize, String)> {
    let mut mapping = std::collections::HashMap::new();
    let mut j = 0;
    for (i, b) in body.iter().enumerate() {
        let t = b.text();
        if let Some(k) = (j..shown.len()).find(|&k| shown[k].text() == t) {
            mapping.insert(i, k);
            j = k + 1;
        }
    }
    chunks_from(body, 0)
        .into_iter()
        .filter_map(|(i, s)| mapping.get(&i).filter(|&&k| k >= start).map(|&k| (k, s)))
        .collect()
}

/// ビジュアルモードのカーソルから読むとき: blk の文のうち、カーソルのある文から読む。
/// rest は画面上のブロックのカーソルから後ろの文字列。読む文と画面の文字列は画像の alt や URL の分だけ
/// 違うことがあるので、空白を除いてから rest の出だしを探し (複数あれば末尾からの長さで見積もった位置に
/// 近いもの)、見つからなければその見積もりの位置から読む
pub fn start_at(chunks: &[(usize, String)], blk: usize, rest: &str) -> Vec<(usize, String)> {
    let norm = |s: &str| -> Vec<char> { s.chars().filter(|c| !pytext::is_space(*c)).collect() };
    let sents: Vec<&String> = chunks.iter().filter(|(i, _)| *i == blk).map(|(_, s)| s).collect();
    let whole: Vec<char> = sents.iter().flat_map(|s| norm(s)).collect();
    let tail = norm(rest);
    let guess = whole.len().saturating_sub(tail.len());
    let mut found: Vec<usize> = Vec::new();
    for n in [12, 6, 3] {
        // 出だしに alt などが入っていることもあるので、短くしながら探す
        let head = &tail[..n.min(tail.len())];
        if !head.is_empty() {
            found = find_all(&whole, head);
        }
        if !found.is_empty() {
            break;
        }
    }
    let pos = found.iter().copied().min_by_key(|&x| x.abs_diff(guess)).unwrap_or(guess);
    let (mut skip, mut end) = (0, 0);
    for s in &sents {
        end += norm(s).len();
        if end > pos {
            break;
        }
        skip += 1;
    }
    let k = chunks.iter().position(|(i, _)| *i == blk).unwrap_or(0);
    chunks[..k].iter().chain(chunks[(k + skip).min(chunks.len())..].iter()).cloned().collect()
}

/// Python の \s (空白) と同じ
/// 重ならない一致の位置 (re.finditer と同じ)
fn find_all(hay: &[char], needle: &[char]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if hay[i..i + needle.len()] == *needle {
            out.push(i);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}
