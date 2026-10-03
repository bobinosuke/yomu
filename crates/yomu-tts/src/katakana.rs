//! 読み上げのモデルに渡す文を作る (text_for_tts)。日本語の文は、渡す前に一部をカタカナにする。
//!
//! - AivisSpeech の辞書で読みが変わる語は、その読みにする (dictionary_words_to_kana)
//! - 英単語はカタカナにする (日本語の声で英語の発音が混ざらないように。english_to_kana)
//! - 句読点・括弧など以外の記号は、OpenJTalk と同じく辞書の読み (＋ → タス、50% → 50パーセント) か、
//!   読みがなければ間 (、) にする (symbols_to_kana)。モデルに任せると、全角の記号も NFKD で ASCII になるので
//!   英語で読んだり (+ → plus)、区切りを無視して語をつなげたり (荷物・場所) する
//! - 見出しや箇条書きの項目のような、文になっていない短いもの (label) は、漢字を含む語をすべて辞書の読みの
//!   カタカナにする (kanji_to_kana。ひらがな・カタカナの語や助詞はそのまま残し、抑揚をつけやすくする)。
//!   短く文脈がないので、読み上げのモデルに漢字の読みを任せると揺れる (目次 → モクツジ・メツギ)。
//!   また、モデルは合成する長さを文字の数から見積もるので、漢字の多いもの (参考文献 = 4 文字で 8 拍) は
//!   長さが足りず、ときどき途中で切れる (サンコウっ)。カナなら文字の数と拍の数がそろう。ただしモデルはカタカナを
//!   短く見積もるので、合成する長さは speaker.rs の LABEL_STRETCH で伸ばす。よくある見出し 20 個を 5 回ずつ
//!   合成すると、漢字のままでは 7/100 回切れ、カタカナにして長さを伸ばすと切れなかった
//!
//! 辞書に読みのある英単語は、その読みにする (AivisSpeech の辞書で ChatGPT → チャットジーピーティー など)。
//! 辞書にない英単語 (OpenJTalk がアルファベットを1文字ずつ読むもの) は、AivisSpeech Engine と同じく kanalizer で
//! 英単語としてのカタカナ読みにする (Python → パイソン)。
use std::sync::LazyLock;

use regex::Regex;

use crate::frontend::{Frontend, pron};
use crate::sentences::Lang;
use crate::text2mecab::text2mecab;

/// 英語の部分 (英字で始まり、英数字・アポストロフィ・ピリオド・ハイフンが続く語の、空白区切りの並び)
static EN_RUN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z][A-Za-z0-9'’.\-]*(?:\s+[A-Za-z][A-Za-z0-9'’.\-]*)*").unwrap());
/// OpenJTalk が辞書にない英単語に付ける品詞
const UNKNOWN_POS: &str = "フィラー";

/// 読み上げのモデルに渡す文。label は見出しや箇条書きの項目のような、文になっていない短いもの
pub fn text_for_tts(frontend: &Frontend, text: &str, lang: Lang, label: bool) -> String {
    match (lang == Lang::JA, label) {
        (true, true) => kanji_to_kana(frontend, &symbols_to_kana(frontend, text)),
        (true, false) => english_to_kana(frontend, &dictionary_words_to_kana(frontend, &symbols_to_kana(frontend, text))),
        (false, _) => text.to_string(),
    }
}

/// モデルがそのまま扱える記号 (句読点・括弧・引用符・長音など。文の区切りや抑揚としてモデルに任せる)
const KEEP: &str = "。、，．,.！？!?：；:;「」『』（）()［］[]｛｝{}〈〉《》“”\"'’‘`…‥ー〜~−-／/＿_";
/// 間 (、) を重ねない記号
const PAUSE: &str = "。、，．,.！？!?：；:;";

fn is_symbol(c: char) -> bool {
    !c.is_alphanumeric() && !c.is_whitespace() && !KEEP.contains(c)
}

/// 記号 (KEEP 以外) を、OpenJTalk の辞書の読みのカタカナにする。読みのない記号は、OpenJTalk と同じく間 (、) にする。
/// 数字の後の % (パーセント) のように前後で読みが変わるので、文ごと MeCab にかける
fn symbols_to_kana(frontend: &Frontend, text: &str) -> String {
    if !text.chars().any(is_symbol) {
        return text.to_string();
    }
    // OpenJTalk と同じく ASCII を全角にして MeCab に渡す。1 文字ずつ変えて、元の文字列での位置を覚えておく
    let (mut z, mut origin) = (String::new(), Vec::new()); // origin: (z での位置, text での位置)
    for (i, c) in text.char_indices() {
        origin.push((z.len(), i));
        let t = if c.is_ascii() { text2mecab(c.encode_utf8(&mut [0; 4])) } else { String::new() };
        if t.is_empty() { z.push(c) } else { z += &t }
    }
    origin.push((z.len(), text.len()));
    let at = |zpos: usize| origin.binary_search_by_key(&zpos, |p| p.0).ok().map(|k| origin[k].1);
    let mut out = String::new();
    let mut pos = 0;
    for m in frontend.parse(&z) {
        let (Some(b), Some(e)) = (at(m.begin), at(m.begin + m.surface.len())) else { continue };
        if !text[b..e].chars().all(is_symbol) {
            continue;
        }
        out += &text[pos..b];
        pos = e;
        match pron(&m.feature).map(|p| p.replace('’', "")) {
            Some(p) if p.chars().all(|c| matches!(c, 'ァ'..='ヶ' | 'ー')) => out += &p,
            // 読みのない記号は間にする。文の頭や、前後がすでに句読点のときは何も入れない
            _ => {
                let prev = out.trim_end().chars().last();
                let next = text[e..].trim_start().chars().next();
                if prev.is_some_and(|c| !PAUSE.contains(c)) && next.is_some_and(|c| !PAUSE.contains(c) && !is_symbol(c)) {
                    out.push('、');
                }
            }
        }
    }
    out + &text[pos..]
}

/// 文の拍の数と、文の中の区切り (句読点) の数。辞書の読みで数える (合成する長さの下限を決めるのに使う。speaker.rs)
pub fn count_moras(frontend: &Frontend, text: &str) -> (usize, usize) {
    let r = reading(frontend, &symbols_to_kana(frontend, text));
    let moras = r.chars().filter(|c| matches!(c, 'ア'..='ヶ' | 'ー') && !"ァィゥェォャュョヮ".contains(*c)).count();
    let pauses = r.trim_end_matches(|c| PAUSE.contains(c)).chars().filter(|c| PAUSE.contains(*c)).count();
    (moras, pauses)
}

/// AivisSpeech の辞書で読みが変わる語 (固有名詞・将棋の用語など。例: 居飛車 → イビシャ、羽生善治 → ハブヨシハル) を、
/// その読みのカタカナにする。ほかの語は漢字のまま残し、読みと抑揚は読み上げのモデルに任せる
/// (全部をカナにすると抑揚が不自然になる)
fn dictionary_words_to_kana(frontend: &Frontend, text: &str) -> String {
    let mut out = String::new();
    let mut pos = 0;
    for m in frontend.parse(text) {
        let from_aivis = m.dict.is_some_and(|d| d > 0);
        if !from_aivis || !m.surface.chars().any(is_kanji) {
            continue;
        }
        let Some(reading) = crate::frontend::pron(&m.feature) else { continue };
        let reading: String = reading.chars().filter(|&c| c != '’' && c != ':').collect();
        if reading.is_empty() || reading == frontend.base_reading(&m.surface).replace('’', "") {
            continue;
        }
        out += &text[pos..m.begin];
        out += &reading;
        pos = m.begin + m.surface.len();
    }
    out + &text[pos..]
}

fn is_kana(c: char) -> bool {
    matches!(c, 'ぁ'..='ゖ' | 'ァ'..='ヺ' | 'ー' | 'ゝ' | 'ゞ' | 'ヽ' | 'ヾ')
}

fn is_kanji(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{9fff}' | '\u{f900}'..='\u{faff}' | '々')
}

/// 文の中の英語の部分をカタカナにする
fn english_to_kana(frontend: &Frontend, text: &str) -> String {
    EN_RUN.replace_all(text, |m: &regex::Captures| reading(frontend, &m[0])).into_owned()
}

/// 文全体を辞書の読み (OpenJTalk の辞書 + AivisSpeech の辞書) のカタカナにする。辞書にない英単語は kanalizer で読む。
/// 句読点・括弧など (KEEP) はそのまま残す (読み上げのモデルに任せる)
fn reading(frontend: &Frontend, text: &str) -> String {
    let mut out = String::new();
    for w in frontend.run(text) {
        let s = hankaku(&w.string);
        if w.pos == UNKNOWN_POS && s.chars().all(|c| c.is_ascii_alphabetic()) {
            out += &english_to_katakana(&s);
        } else if w.pos == "記号" && w.string.chars().all(|c| KEEP.contains(c)) {
            out += &w.string;
        } else {
            out += &w.pron.replace('’', ""); // ’ (辞書の発音にある無声化の印) は渡さない
        }
    }
    out
}

/// かなだけの語 (ひらがな・カタカナの語や助詞) は元の字のまま残し、ほかの語 (漢字・数字・英単語・読みのある記号) は
/// 辞書の読みのカタカナにする (訓読みの漢字 → クンヨミのカンジ、3.5倍 → サンテンゴバイ)
fn kanji_to_kana(frontend: &Frontend, text: &str) -> String {
    let mut out = String::new();
    for w in frontend.run(text) {
        let s = hankaku(&w.string);
        if w.pos == UNKNOWN_POS && s.chars().all(|c| c.is_ascii_alphabetic()) {
            out += &english_to_katakana(&s);
        } else if w.string.chars().all(is_kana) {
            out += &w.string;
        } else {
            // 読みがカナでないもの (句読点・括弧など) は元の字のまま
            let p = w.pron.replace('’', "");
            out += if !p.is_empty() && p.chars().all(is_kana) { &p } else { &w.string };
        }
    }
    out
}

/// 全角の英字を半角にする
fn hankaku(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'Ａ'..='Ｚ' | 'ａ'..='ｚ' => char::from_u32(c as u32 - 0xFEE0).unwrap(),
            _ => c,
        })
        .collect()
}

/// OpenJTalk で使っているアルファベット → カタカナの対応表 (AivisSpeech Engine の katakana_english.py と同じ)
fn letter_kana(c: char) -> &'static str {
    const T: [&str; 26] = [
        "エー", "ビー", "シー", "ディー", "イー", "エフ", "ジー", "エイチ", "アイ", "ジェー", "ケー", "エル", "エム", "エヌ", "オー",
        "ピー", "キュー", "アール", "エス", "ティー", "ユー", "ブイ", "ダブリュー", "エックス", "ワイ", "ズィー",
    ];
    T[(c.to_ascii_uppercase() as u8 - b'A') as usize]
}

/// 英単語をカタカナ読みにする (AivisSpeech Engine の convert_english_to_katakana)。
/// 大文字で区切って語に分け (VoiceVox → Voice / Vox)、1文字の語と全部大文字の語はアルファベットを1文字ずつ読む
pub fn english_to_katakana(s: &str) -> String {
    // 正規表現 [a-zA-Z][a-z]* で語に分ける (大文字は常に語の始まり)
    let mut words: Vec<String> = Vec::new();
    for c in s.chars() {
        match words.last_mut() {
            Some(w) if c.is_ascii_lowercase() => w.push(c),
            _ => words.push(c.to_string()),
        }
    }
    let mut kana = String::new();
    for w in words {
        let spell = w.len() == 1 || w == w.to_ascii_uppercase();
        let converted = if spell {
            None
        } else {
            kanalizer::convert(&w.to_ascii_lowercase()).with_error_on_incomplete(false).perform().ok()
        };
        match converted {
            Some(k) => kana += &k,
            None => kana += &w.chars().map(letter_kana).collect::<String>(),
        }
    }
    kana
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_words() {
        assert_eq!(english_to_katakana("LLM"), "エルエルエム"); // 全部大文字はアルファベット読み
        assert_eq!(english_to_katakana("b"), "ビー");
        assert_eq!(english_to_katakana("python"), "パイソン");
        assert_eq!(english_to_katakana("GitHub"), "ギットハブ"); // Git / Hub に分けて読む
    }
}
