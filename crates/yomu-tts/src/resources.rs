//! 辞書・モデルの置き場所。どれも ~/.cache/yomu の下に置く。
use std::path::PathBuf;

#[derive(Clone)]
pub struct Resources {
    pub cache: PathBuf,
}

impl Default for Resources {
    fn default() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        Self { cache: home.join(".cache").join("yomu") }
    }
}

impl Resources {
    /// 読み上げのモデル Supertonic 3 (onnx/ と voice_styles/)
    pub fn supertonic(&self) -> PathBuf {
        self.cache.join("supertonic-3")
    }
    /// OpenJTalk のシステム辞書 (pyopenjtalk-plus 同梱のもの。sys.dic, unk.dic, char.bin, matrix.bin)
    pub fn openjtalk_dict(&self) -> PathBuf {
        self.cache.join("openjtalk-dict")
    }
    /// 言語の判定 (lingua) の統計データ。言語の ISO 639-1 のコードごとのディレクトリ
    pub fn lingua(&self) -> PathBuf {
        self.cache.join("lingua")
    }
    /// AivisSpeech のユーザー辞書 (コンパイル済みの MeCab ユーザー辞書)。読み込む順 (名前順) に返す。
    pub fn user_dicts(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(self.cache.join("aivis-dict"))
            .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();
        v.retain(|p| p.extension().is_some_and(|e| e == "dic"));
        v.sort();
        v
    }
}

