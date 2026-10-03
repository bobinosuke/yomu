//! 設定 (gs の設定画面で切り替える)。どれも最初はオフ。~/.local/share/yomu/settings.json に保存する。
use std::sync::{LazyLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::doc::{Block, Document, Kind, Span};

/// 設定画面の項目のリンク (yomu-setting:名前)。開くと切り替える
pub const SETTING_SCHEME: &str = "yomu-setting:";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// Cookie をファイルに残す (期限つきのものだけ。終了するときに保存し、次に起動したときに読み込む)
    #[serde(default)]
    pub save_cookies: bool,
    /// 広告・追跡のリンクを本文から除き、URL から追跡用の値 (utm_* など) を除く
    #[serde(default)]
    pub block_trackers: bool,
    /// 画像を端末の中に出す (Kitty・iTerm2・Sixel に対応した端末で)
    #[serde(default)]
    pub images: bool,
    /// 画面の言語 (None は自動。環境変数と OS の言語設定から決める)
    #[serde(default)]
    pub lang: Option<crate::i18n::Lang>,
}

/// オン・オフの項目 (名前, 項目, 説明)
fn items() -> [(&'static str, &'static str, &'static str); 3] {
    [
        ("cookies", t!("Cookie をファイルに残す"), t!("Cookie の同意やログインを次に起動したときも覚えておく (期限つきの Cookie だけ。cookies.json に平文で保存する)")),
        ("trackers", t!("広告・追跡のリンクを消す"), t!("本文から広告・追跡のサービスへのリンクを除き、URL から追跡用の値 (utm_source など) を除く")),
        ("images", t!("画像を表示する"), t!("本文の画像を端末の中に出す (Kitty・iTerm2・Sixel に対応した端末のみ。Mac 標準のターミナルでは出ない)")),
    ]
}

static CURRENT: LazyLock<RwLock<Settings>> = LazyLock::new(|| RwLock::new(load()));

fn path() -> std::path::PathBuf {
    crate::store::data_dir().join("settings.json")
}

fn load() -> Settings {
    std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// 今の設定
pub fn get() -> Settings {
    *CURRENT.read().unwrap()
}

/// 名前の項目を切り替えて保存する。切り替えた後の値を返す (知らない名前なら None)
pub fn toggle(name: &str) -> Option<bool> {
    let mut s = CURRENT.write().unwrap();
    let v = match name {
        "cookies" => &mut s.save_cookies,
        "trackers" => &mut s.block_trackers,
        "images" => &mut s.images,
        _ => return None,
    };
    *v = !*v;
    let on = *v;
    save(&s);
    Some(on)
}

/// 画面の言語を決めて保存する (None は自動)。新しい画面の言語を返す
pub fn set_lang(lang: Option<crate::i18n::Lang>) -> crate::i18n::Lang {
    let mut s = CURRENT.write().unwrap();
    s.lang = lang;
    save(&s);
    ui_lang(&s)
}

/// 設定の画面の言語 (自動なら環境変数と OS の言語設定から)
pub fn ui_lang(s: &Settings) -> crate::i18n::Lang {
    s.lang.unwrap_or_else(crate::i18n::detect)
}

fn save(s: &Settings) {
    let p = path();
    let _ = std::fs::create_dir_all(p.parent().unwrap());
    let _ = std::fs::write(&p, serde_json::to_vec_pretty(s).unwrap_or_default());
}

fn value(s: &Settings, name: &str) -> bool {
    match name {
        "cookies" => s.save_cookies,
        "trackers" => s.block_trackers,
        "images" => s.images,
        _ => false,
    }
}

/// 設定画面。項目を f のラベルかクリックで開くと切り替わる (言語は yomu-setting:lang:コード。auto は自動)
pub fn settings_page() -> Document {
    let s = get();
    let mut blocks = vec![Block::heading(t!("設定"), 1), Block::para(t!("f のラベルかクリックで項目を選びます。オン・オフの項目は切り替わり、言語は選んだものになります。"))];
    let mut links = Vec::new();
    // 画面の言語。名前と説明の下の行に選べるものを並べる。選んでいるものは [x] を付けた太字にし、リンクにしない
    // (選んでいないものだけがリンクの色で、f のラベルが付く)
    use crate::i18n::Lang;
    let desc = t!("画面の言語。翻訳の訳し先と検索の地域もこの言語に合わせる。自動なら環境変数 (LANG など) と OS の言語設定から決める");
    let head = vec![Span { bold: true, ..Span::new(t!("言語 (Language)")) }, Span { minor: true, ..Span::new(format!("  {desc}")) }];
    blocks.push(Block::new(Kind::Li, head));
    let auto = format!("{} ({})", t!("自動"), crate::i18n::detect().name());
    let mut options = Vec::new();
    for (lang, label) in std::iter::once((None, auto)).chain(Lang::ALL.map(|l| (Some(l), l.name().to_string()))) {
        if !options.is_empty() {
            options.push(Span::new("   "));
        }
        if s.lang == lang {
            options.push(Span { bold: true, ..Span::new(format!("[x] {label}")) });
        } else {
            links.push(format!("{SETTING_SCHEME}lang:{}", lang.map_or("auto", Lang::code)));
            options.push(Span { link: Some(links.len() - 1), ..Span::new(format!("[ ] {label}")) });
        }
    }
    blocks.push(Block { level: 1, ..Block::new(Kind::Li, options) });
    for (name, label, desc) in items() {
        let on = value(&s, name);
        links.push(format!("{SETTING_SCHEME}{name}"));
        let mark = Span { bold: true, link: Some(links.len() - 1), ..Span::new(format!("[{}] {label}", if on { t!("オン") } else { t!("オフ") })) };
        blocks.push(Block::new(Kind::Li, vec![mark, Span { minor: true, ..Span::new(format!("  {desc}")) }]));
    }
    // プライベートモード (保存する設定ではなく、切り替えると起動し直す)
    let on = crate::fetch::is_private();
    links.push(format!("{SETTING_SCHEME}private"));
    let mark = Span { bold: true, link: Some(links.len() - 1), ..Span::new(format!("[{}] {}", if on { t!("オン") } else { t!("オフ") }, t!("プライベートモード (Tor)"))) };
    let desc = t!("すべての通信を Tor に通し、履歴・タブ・Cookie を残さない。切り替えると yomu を起動し直す (gp でも切り替えられる)");
    blocks.push(Block::new(Kind::Li, vec![mark, Span { minor: true, ..Span::new(format!("  {desc}")) }]));
    Document { url: "about:settings".into(), title: t!("設定").into(), blocks, links, note: t!("設定 (Esc で閉じる)").into(), ..Default::default() }
}
