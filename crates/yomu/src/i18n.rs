//! 画面の言語 (日本語・英語・韓国語)。文言は日本語をそのまま鍵にし (gettext と同じ)、t! で訳す。
//! 訳は i18n/en.rs・i18n/ko.rs の表 (日本語 → 訳)。表にない文言は日本語のまま出す。
//! 言語は起動したときに決め (detect)、設定画面で変えられる。何も決めていないとき (テストなど) は日本語
use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

mod en;
mod ko;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lang {
    #[serde(rename = "ja")]
    Ja,
    #[serde(rename = "en")]
    En,
    #[serde(rename = "ko")]
    Ko,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::Ja, Lang::En, Lang::Ko];

    /// ISO 639-1 の言語コード
    pub fn code(self) -> &'static str {
        match self {
            Lang::Ja => "ja",
            Lang::En => "en",
            Lang::Ko => "ko",
        }
    }

    /// その言語での言語の名前 (設定画面に出す。どの言語の画面でも訳さない)
    pub fn name(self) -> &'static str {
        match self {
            Lang::Ja => "日本語",
            Lang::En => "English",
            Lang::Ko => "한국어",
        }
    }

    /// 「ja_JP.UTF-8」「en-US」などの先頭の言語コードから
    pub fn from_locale(s: &str) -> Option<Lang> {
        let code = s.split(['_', '-', '.', '@']).next().unwrap_or("").to_ascii_lowercase();
        Lang::ALL.into_iter().find(|l| l.code() == code)
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

/// 今の画面の言語
pub fn lang() -> Lang {
    Lang::ALL[LANG.load(Ordering::Relaxed) as usize]
}

pub fn set_lang(l: Lang) {
    LANG.store(Lang::ALL.iter().position(|x| *x == l).unwrap_or(0) as u8, Ordering::Relaxed);
}

/// 設定で言語を決めていないときの言語。環境変数 (LC_ALL → LC_MESSAGES → LANG) → macOS の言語設定の順に見て、
/// 対応している言語が見つからなければ英語
pub fn detect() -> Lang {
    for k in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        // C・POSIX は言語を決めていない (次を見る)
        let v = std::env::var(k).unwrap_or_default();
        if v.is_empty() || v == "C" || v.starts_with("C.") || v == "POSIX" {
            continue;
        }
        return Lang::from_locale(&v).unwrap_or(Lang::En);
    }
    #[cfg(target_os = "macos")]
    if let Some(l) = mac_preferred().as_deref().and_then(Lang::from_locale) {
        return l;
    }
    Lang::En
}

/// macOS の言語設定の一番目 (「ja-JP」など)
#[cfg(target_os = "macos")]
fn mac_preferred() -> Option<String> {
    let out = std::process::Command::new("defaults").args(["read", "-g", "AppleLanguages"]).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    s.split('"').nth(1).map(str::to_string)
}

static EN: LazyLock<HashMap<&str, &str>> = LazyLock::new(|| en::TABLE.iter().copied().collect());
static KO: LazyLock<HashMap<&str, &str>> = LazyLock::new(|| ko::TABLE.iter().copied().collect());

/// 日本語の文言を今の言語に訳す (訳がなければ日本語のまま)
pub fn tr(ja: &'static str) -> &'static str {
    let table = match lang() {
        Lang::Ja => return ja,
        Lang::En => &EN,
        Lang::Ko => &KO,
    };
    table.get(ja).copied().unwrap_or(ja)
}

/// 文言の {名前} に値を入れる
pub fn fill(s: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut out = s.to_string();
    for (k, v) in args {
        out = out.replace(&format!("{{{k}}}"), &v.to_string());
    }
    out
}

/// 日本語の文言を今の言語に訳す。t!("{n} 件", n = x) のように {名前} に値を入れられる (訳の側でも同じ名前を使う)
#[macro_export]
macro_rules! t {
    ($s:literal) => {
        $crate::i18n::tr($s)
    };
    ($s:literal, $($k:ident = $v:expr),+ $(,)?) => {
        $crate::i18n::fill($crate::i18n::tr($s), &[$((stringify!($k), &$v as &dyn std::fmt::Display)),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_locale_codes() {
        assert_eq!(Lang::from_locale("ja_JP.UTF-8"), Some(Lang::Ja));
        assert_eq!(Lang::from_locale("en-US"), Some(Lang::En));
        assert_eq!(Lang::from_locale("ko_KR.UTF-8"), Some(Lang::Ko));
        assert_eq!(Lang::from_locale("fr_FR.UTF-8"), None);
    }

    #[test]
    fn fills_named_values() {
        assert_eq!(fill("{n} 件 ({n})、{s}", &[("n", &3), ("s", &"x")]), "3 件 (3)、x");
    }

    /// コードの t!("…") の文言がどれも英語と韓国語の表にあり、表に使われていない訳が残っていない
    #[test]
    fn every_message_is_translated() {
        let used = messages_in_source();
        for (name, table) in [("en", en::TABLE), ("ko", ko::TABLE)] {
            let keys: std::collections::BTreeSet<&str> = table.iter().map(|(k, _)| *k).collect();
            assert_eq!(keys.len(), table.len(), "{name}: 同じ文言が二度ある");
            let missing: Vec<&String> = used.iter().filter(|m| !keys.contains(m.as_str())).collect();
            assert!(missing.is_empty(), "{name} に訳がない: {missing:#?}");
            let unused: Vec<&&str> = keys.iter().filter(|k| !used.contains(**k)).collect();
            assert!(unused.is_empty(), "{name} の使われていない訳: {unused:#?}");
            // {名前} の数と名前が原文と同じ
            for (ja, tr) in table {
                assert_eq!(placeholders(ja), placeholders(tr), "{name}: {ja} → {tr}");
            }
        }
    }

    fn placeholders(s: &str) -> std::collections::BTreeSet<String> {
        regex::Regex::new(r"\{(\w+)\}").unwrap().captures_iter(s).map(|c| c[1].to_string()).collect()
    }

    /// src の下の .rs から t!("…") の文言を集める (i18n の表とこのファイルは除く)
    fn messages_in_source() -> std::collections::BTreeSet<String> {
        let re = regex::Regex::new(r#"\bt!\(\s*"((?:[^"\\]|\\.)*)""#).unwrap();
        let mut out = std::collections::BTreeSet::new();
        let mut dirs = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(d) = dirs.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    if !p.ends_with("i18n") {
                        dirs.push(p);
                    }
                } else if p.extension().is_some_and(|x| x == "rs") && !p.ends_with("i18n.rs") {
                    let s = std::fs::read_to_string(&p).unwrap();
                    out.extend(re.captures_iter(&s).map(|c| c[1].replace("\\\"", "\"").replace("\\n", "\n")));
                }
            }
        }
        out
    }
}
