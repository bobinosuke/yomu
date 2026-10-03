//! 読む単位への分け方と言語の判定。文の分け方は Python 版 (yomu-python) の split_sentences と同じ。
//! 言語は、読み上げのモデル (Supertonic 3) が読める 31 言語から決める。
use std::sync::LazyLock;

use lingua::{LanguageDetector, LanguageDetectorBuilder};

/// 読み上げのモデルが読める言語 (ISO 639-1 のコード, 名前)
const LANGS: [(&str, &str); 31] = [
    ("ja", "日本語"), ("en", "英語"), ("ko", "韓国語"), ("ar", "アラビア語"),
    ("bg", "ブルガリア語"), ("cs", "チェコ語"), ("da", "デンマーク語"), ("de", "ドイツ語"),
    ("el", "ギリシャ語"), ("es", "スペイン語"), ("et", "エストニア語"), ("fi", "フィンランド語"),
    ("fr", "フランス語"), ("hi", "ヒンディー語"), ("hr", "クロアチア語"), ("hu", "ハンガリー語"),
    ("id", "インドネシア語"), ("it", "イタリア語"), ("lt", "リトアニア語"), ("lv", "ラトビア語"),
    ("nl", "オランダ語"), ("pl", "ポーランド語"), ("pt", "ポルトガル語"), ("ro", "ルーマニア語"),
    ("ru", "ロシア語"), ("sk", "スロバキア語"), ("sl", "スロベニア語"), ("sv", "スウェーデン語"),
    ("tr", "トルコ語"), ("uk", "ウクライナ語"), ("vi", "ベトナム語"),
];

/// ページの言語 (読み上げのモデルが読める言語のどれか)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lang(usize);

impl Lang {
    pub const JA: Lang = Lang(0);
    pub const EN: Lang = Lang(1);
    pub const KO: Lang = Lang(2);

    /// 言語のコード (ja, en-US, pt_BR など) から。読めない言語なら None
    pub fn from_code(code: &str) -> Option<Lang> {
        let primary = code.trim().split(['-', '_']).next()?.to_ascii_lowercase();
        LANGS.iter().position(|(c, _)| *c == primary).map(Lang)
    }
    pub fn code(self) -> &'static str {
        LANGS[self.0].0
    }
    pub fn name(self) -> &'static str {
        LANGS[self.0].1
    }
}

/// 1 回に合成する長さ。長すぎると最初の音が出るまで待たされる
const MAX_CHARS: usize = 150;
/// 本文の文字 (かな・漢字 + アルファベット) のうち、日本語がこの割合以上なら日本語のページ
const JA_MIN_RATIO: f64 = 0.2;
/// 日本語の文字のうち、かながこの割合以上なら日本語 (かなのない漢字だけの文章は中国語とみなす)
const KANA_MIN_RATIO: f64 = 0.05;
/// 言語を推定するのに使う本文の長さ (文字)
const DETECT_CHARS: usize = 4000;
/// 推定した言語の確からしさ (0〜1) がこれ未満なら使わない (「hello」の 1 語だけなどは当てられない)
const DETECT_MIN_CONFIDENCE: f64 = 0.3;

/// 言語の推定 (lingua)。候補は読める 31 言語と、読めない言語のうち読める言語と取り違えやすいもの
/// (漢字だけの中国語、アラビア文字のペルシャ語・ウルドゥー語) とタイ語。
/// 候補にない文字 (ヘブライ文字など) の文章は、どの候補の確からしさも 0 になる。
/// 統計データは読み上げの準備 (Speaker::open) でダウンロードしたものを読む。まだないときは文字の種類だけで判定する
static DETECTOR: LazyLock<LanguageDetector> = LazyLock::new(|| {
    use lingua::Language::*;
    lingua::set_models_directory(&crate::resources::Resources::default().lingua());
    LanguageDetectorBuilder::from_languages(&[
        Japanese, English, Korean, Arabic, Bulgarian, Czech, Danish, German, Greek, Spanish, Estonian, Finnish, French,
        Hindi, Croatian, Hungarian, Indonesian, Italian, Lithuanian, Latvian, Dutch, Polish, Portuguese, Romanian, Russian,
        Slovak, Slovene, Swedish, Turkish, Ukrainian, Vietnamese, Chinese, Thai, Persian, Urdu,
    ])
    .build()
});

fn is_kana(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30ff}' | '\u{ff66}'..='\u{ff9f}')
}

fn is_ja_char(c: char) -> bool {
    is_kana(c) || ('\u{3400}'..='\u{9fff}').contains(&c)
}

/// ページの言語。declared はページが宣言した言語 (<html lang="…">。なければ空)。読み上げられない言語なら None。
/// 1. かなを含む日本語の文章 (かな・漢字が本文の文字の 2 割以上) なら日本語。英語のテンプレートのまま
///    lang="en" になっている日本語のブログもあるので、宣言より先に見る
/// 2. 宣言した言語が読める言語なら、それ (日本語の宣言は 1 で日本語にならなかったので使わない)
/// 3. 本文から推定した言語 (lingua の確からしさが DETECT_MIN_CONFIDENCE 以上のとき)。読めない言語なら None
/// 4. 宣言した言語が読めない言語 (中国語・タイ語など) なら None
/// 5. どれでもなければ、アルファベットが多ければ英語、そうでなければ None
pub fn detect_lang<S: AsRef<str>>(texts: &[S], declared: &str) -> Option<Lang> {
    let (mut ja, mut kana, mut latin, mut letters) = (0usize, 0usize, 0usize, 0usize);
    for t in texts {
        for c in t.as_ref().chars() {
            if is_ja_char(c) {
                ja += 1;
                kana += usize::from(is_kana(c));
            } else if c.is_ascii_alphabetic() {
                latin += 1;
            }
            letters += usize::from(c.is_alphabetic());
        }
    }
    if ja > 0 && ja as f64 / (ja + latin) as f64 >= JA_MIN_RATIO && kana as f64 / ja as f64 >= KANA_MIN_RATIO {
        return Some(Lang::JA);
    }
    if let Some(l) = Lang::from_code(declared).filter(|l| *l != Lang::JA) {
        return Some(l);
    }
    let sample: String = texts.iter().map(|t| t.as_ref()).collect::<Vec<_>>().join("\n").chars().take(DETECT_CHARS).collect();
    if let Some((lang, conf)) = DETECTOR.compute_language_confidence_values(&sample).into_iter().next()
        && conf >= DETECT_MIN_CONFIDENCE
    {
        return Lang::from_code(&lang.iso_code_639_1().to_string());
    }
    if !declared.trim().is_empty() && Lang::from_code(declared).is_none() {
        return None;
    }
    (letters > 0 && latin * 2 >= letters).then_some(Lang::EN)
}

/// Python の str.isspace と同じ空白
fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

/// 文に分ける。「。！？!?」(とヒンディー語の「।」、アラビア語の「؟」) の後 (空白は捨てる)、「.;:」の後の空白、改行で区切る (英語はピリオドの後の空白で区切る)。
/// ただし括弧の中の「.;:」では区切らない (「混血語（こんけつご、英: hybrid、独: hybrides Wort）」を 3 つに分けないように)。
/// MAX_CHARS 文字を超える文は cut_long で切る
pub fn split_sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut sents: Vec<String> = Vec::new();
    let (mut start, mut i) = (0, 0);
    let mut depth = 0usize; // 開いている丸括弧の数
    while i <= chars.len() {
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        match prev {
            Some('(' | '（') => depth += 1,
            Some(')' | '）') => depth = depth.saturating_sub(1),
            _ => {}
        }
        let mut j = i;
        let after_stop = prev.is_some_and(|p| "。！？!?।؟".contains(p));
        let after_period = depth == 0 && prev.is_some_and(|p| ".;:".contains(p)) && i < chars.len() && is_space(chars[i]);
        let hit = if after_stop || after_period {
            while j < chars.len() && is_space(chars[j]) {
                j += 1;
            }
            true
        } else if i < chars.len() && chars[i] == '\n' {
            depth = 0; // 閉じ忘れた括弧が次の行に響かないように
            while j < chars.len() && chars[j] == '\n' {
                j += 1;
            }
            true
        } else {
            false
        };
        if hit {
            sents.push(chars[start..i].iter().collect());
            start = j;
        }
        i = if j > i { j } else { i + 1 };
    }
    if start <= chars.len() {
        sents.push(chars[start..].iter().collect());
    }
    sents.iter().map(|s| s.trim_matches(is_space)).filter(|s| !s.is_empty()).flat_map(|s| cut_long(s, MAX_CHARS)).collect()
}

/// max_len 文字を超える文を、読点 (英語はカンマ、なければ空白) の後で切る。どれもなければ max_len 文字で切る
pub fn cut_long(text: &str, max_len: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut sent: Vec<char> = text.chars().collect();
    while sent.len() > max_len {
        let head = &sent[..max_len];
        let comma = head.iter().rposition(|&c| c == '、').map_or(-1, |p| p as i64);
        let comma_en = head.windows(2).rposition(|w| w == [',', ' ']).map_or(-1, |p| p as i64);
        let mut cut = comma.max(comma_en);
        if cut <= 0 {
            cut = head.iter().rposition(|&c| c == ' ').map_or(-1, |p| p as i64);
        }
        let cut = if cut > 0 { cut as usize + 1 } else { max_len };
        out.push(sent[..cut].iter().collect());
        sent.drain(..cut);
    }
    if !sent.is_empty() {
        out.push(sent.into_iter().collect());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_python() {
        assert_eq!(split_sentences("本文です。次の文です！"), ["本文です。", "次の文です！"]);
        assert_eq!(split_sentences("A b. C d? e\n\nf"), ["A b.", "C d?", "e", "f"]);
        assert_eq!(split_sentences("3.14 は円周率"), ["3.14 は円周率"]);
    }

    #[test]
    fn long_sentence_is_cut_at_commas() {
        let s = "あ、".repeat(100);
        let parts = split_sentences(&s);
        assert!(parts.iter().all(|p| p.chars().count() <= 150));
        assert_eq!(parts.concat(), s);
    }
}
