//! OpenJTalk の frontend (pyopenjtalk の OpenJTalk.run_frontend に当たるもの)。
//! text2mecab → MeCab (OpenJTalk の辞書 + AivisSpeech の辞書) → "記号,空白" を除く → NJD。
//! AivisSpeech の辞書の読み (居飛車 → イビシャ、ChatGPT → チャットジーピーティー など) を引くのに使う。
use crate::mecab::Tagger;
use crate::njd::{NjdFeature, run_njd};
use crate::resources::Resources;
use crate::text2mecab::text2mecab;

pub struct Frontend {
    tagger: Tagger, // OpenJTalk の辞書 + AivisSpeech の辞書
    base: Tagger,   // OpenJTalk の辞書だけ (AivisSpeech の辞書で読みが変わるかを見るのに使う)
}

impl Frontend {
    pub fn open(res: &Resources) -> Result<Self, String> {
        let fail = |e: std::io::Error| format!("OpenJTalk の辞書を開けない: {e}");
        let tagger = Tagger::open(&res.openjtalk_dict(), &res.user_dicts()).map_err(fail)?;
        let base = Tagger::open(&res.openjtalk_dict(), &[]).map_err(fail)?;
        Ok(Self { tagger, base })
    }

    /// MeCab の解析結果 (OpenJTalk の辞書 + AivisSpeech の辞書)。text2mecab は通さない
    pub fn parse(&self, text: &str) -> Vec<crate::mecab::Morph> {
        self.tagger.parse(text)
    }

    /// OpenJTalk の辞書だけで引いた読み (カタカナ)
    pub fn base_reading(&self, text: &str) -> String {
        self.base.parse(text).iter().map(|m| pron(&m.feature).unwrap_or_else(|| m.surface.clone())).collect()
    }

    pub fn run(&self, text: &str) -> Vec<NjdFeature> {
        // text2mecab が半角空白を全角にし、MeCab が "記号,空白" にしたものは除く (NJD で pau が入るため)
        let lines: Vec<String> = self
            .tagger
            .parse(&text2mecab(text))
            .iter()
            .map(|m| m.openjtalk_line())
            .filter(|l| !l.contains("記号,空白"))
            .collect();
        run_njd(&lines)
    }
}

/// OpenJTalk の辞書の素性の発音 (9 番目の欄)。空や * なら None
pub fn pron(feature: &str) -> Option<String> {
    crate::mecab::csv_fields(feature).into_iter().nth(8).filter(|p| !p.is_empty() && p != "*")
}
