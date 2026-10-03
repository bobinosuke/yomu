//! 抽出結果の中間表現。TUI の描画・Markdown 出力・読み上げはすべてここから作る。
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Span {
    pub text: String,
    pub link: Option<usize>, // Document.links のインデックス (0 始まり)
    pub bold: bool,
    pub code: bool,
    pub image: bool,  // alt から作った画像の代替テキスト
    pub italic: bool, // <em> <i> など
    pub strike: bool, // <del> <s> (削除された文字)
    pub minor: bool,  // <small> など、本文より目立たせない注記
    /// 対訳表示で足した訳文 (translate.rs)。訳文とわかる色で描く
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub translated: bool,
    /// 画像の URL (設定で画像を表示するときだけ入れる。image が true の span)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
}

impl Span {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), ..Default::default() }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    H,
    #[default]
    P,
    Li,
    Pre,
    Quote,
    Table,
    Hr,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Block {
    pub kind: Kind,
    pub spans: Vec<Span>,
    pub level: u32,     // 見出しレベル、または箇条書きの深さ
    pub marker: String, // 箇条書きの記号 ("・" や "3.")
    pub rows: Option<Vec<Vec<Vec<Span>>>>, // table 用: 行 → セル → spans
    pub anchors: Vec<String>, // このブロックから始まる要素の id (#見出し への移動に使う)
    pub caption: bool,  // 図のキャプション (<figcaption>)。本文より目立たせない
    /// 対訳表示で原文の後に足した訳文 (translate.rs)。原文との間に空行を入れない
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub translation: bool,
}

impl Block {
    pub fn new(kind: Kind, spans: Vec<Span>) -> Self {
        Self { kind, spans, ..Default::default() }
    }

    /// 文字列 1 つの見出し
    pub fn heading(text: impl Into<String>, level: u32) -> Self {
        Self { level, ..Self::new(Kind::H, vec![Span::new(text)]) }
    }

    /// 文字列 1 つの段落
    pub fn para(text: impl Into<String>) -> Self {
        Self::new(Kind::P, vec![Span::new(text)])
    }

    pub fn text(&self) -> String {
        match &self.rows {
            Some(rows) => rows
                .iter()
                .map(|row| row.iter().map(|c| plain(c)).collect::<Vec<_>>().join(" "))
                .collect::<Vec<_>>()
                .join(" / "),
            None => plain(&self.spans),
        }
    }

    /// 画像の alt だけのブロック
    pub fn image_only(&self) -> bool {
        self.spans.iter().any(|s| s.image) && self.spans.iter().all(|s| s.image || s.text.trim().is_empty())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Document {
    pub url: String,
    pub title: String,
    pub blocks: Vec<Block>,
    pub links: Vec<String>,
    pub note: String, // 状態行に出す説明 (「本文抽出」「全体表示」など)
    pub full: bool,   // 本文抽出せずにページ全体を出したもの
    pub lang: String, // ページが宣言した言語 (<html lang="…"> など。なければ空。読み上げの言語の判定に使う)
    /// ページの入力欄 (検索のフォームなど)。画面で開いたときだけ拾う (gi で入力して送る)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub forms: Vec<Form>,
}

/// ページのフォーム (入力欄が 1 つ以上あるもの)
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Form {
    /// 送り先 (ページの URL で解決したもの)
    pub action: String,
    /// GET で送るか (POST は未対応)
    pub get: bool,
    /// 送る値 (文書の順)。値が None のものが、入力してもらう欄
    pub fields: Vec<(String, Option<String>)>,
    /// 入力欄の説明 (placeholder・label など)
    pub label: String,
}

impl Form {
    /// 入力した文字で送る URL (GET)。送り先の URL の ? 以降は、フォームの値で置き換える (ブラウザと同じ)
    pub fn url(&self, value: &str) -> String {
        let base = self.action.split('#').next().unwrap_or("").split('?').next().unwrap_or("");
        let q: Vec<String> = self
            .fields
            .iter()
            .map(|(k, v)| format!("{}={}", crate::urls::quote_plus(k), crate::urls::quote_plus(v.as_deref().unwrap_or(value))))
            .collect();
        format!("{base}?{}", q.join("&"))
    }
}

pub fn plain(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}
