//! 日本語入力 (IME) の扱い。
//!
//! 日本語入力のままだと f や j が変換待ちの文字としてターミナルに描かれ、アプリに届かない。
//! そこで macOS では、操作モードの間は英数入力に、入力欄を開いている間と終了後はかなに切り替える。
//!
//! 切り替えは JIS キーボードの「英数」「かな」キーを押したのと同じキーイベントを送って行う
//! (ターミナルにアクセシビリティの許可が必要)。
//! 許可がなければ、入力ソースの API (TISSelectInputSource) で英数 (ASCII を打てるキーボード配列) と、
//! 使っていた日本語入力を選ぶ。こちらは許可がいらないが、メニューバーの表示は変わっても最前面のターミナルの
//! 入力は切り替わらないことがある (日本語・中国語などの入力ソースでよく知られた macOS の問題) ので、
//! 許可があればキーイベントを使う。
//! 今の入力ソースは HIToolbox の設定から読む (TISCopyCurrentKeyboardInputSource は、
//! 起動し続けるプロセスの中では更新されず古い値を返し続ける)。
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_normalization::UnicodeNormalization;

/// IME で確定してしまった記号を Vimium のキーに読み替える (全角英数字は NFKC で半角になる)
fn ja_key(c: char) -> char {
    match c {
        '・' => '/',
        '「' => '[',
        '」' => ']',
        '。' => '.',
        '、' => ',',
        'ー' => '-',
        '￥' => '\\',
        c => c,
    }
}

pub fn normalize_key(ch: Option<&str>) -> Option<String> {
    let ch = ch?;
    if ch.is_empty() {
        return Some(String::new());
    }
    let mapped: String = ch.chars().map(ja_key).collect();
    Some(mapped.nfkc().collect())
}

/// crossterm のキーイベントを (キー名, 文字) にする。キー名は Python 版 (Textual) の名前に合わせる。
/// IME で確定した全角文字も半角のキーとして扱う。文字が空白なら ("space", None)
pub fn read_key(ev: &KeyEvent) -> (String, Option<String>) {
    let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
    let alt = ev.modifiers.contains(KeyModifiers::ALT);
    let shift = ev.modifiers.contains(KeyModifiers::SHIFT);
    let named = |name: &str| {
        let mut k = String::new();
        if ctrl {
            k.push_str("ctrl+");
        }
        if alt {
            k.push_str("alt+");
        }
        if shift {
            k.push_str("shift+");
        }
        (k + name, None)
    };
    match ev.code {
        KeyCode::Char(c) if ctrl || alt => named(&c.to_lowercase().to_string()),
        KeyCode::Char(c) => {
            let ch = normalize_key(Some(&c.to_string())).unwrap_or_default();
            if ch == " " {
                ("space".into(), None) // 全角の空白も NFKC で半角になる
            } else if ch.is_empty() || ch.chars().any(char::is_control) {
                (c.to_string(), None)
            } else {
                (ch.clone(), Some(ch))
            }
        }
        KeyCode::Esc => ("escape".into(), None),
        KeyCode::Enter => named("enter"),
        KeyCode::Tab => named("tab"),
        KeyCode::BackTab => ("shift+tab".into(), None),
        KeyCode::Backspace => named("backspace"),
        KeyCode::Delete => named("delete"),
        KeyCode::Up => named("up"),
        KeyCode::Down => named("down"),
        KeyCode::Left => named("left"),
        KeyCode::Right => named("right"),
        KeyCode::Home => named("home"),
        KeyCode::End => named("end"),
        KeyCode::PageUp => named("pageup"),
        KeyCode::PageDown => named("pagedown"),
        KeyCode::Insert => named("insert"),
        KeyCode::F(n) => named(&format!("f{n}")),
        _ => ("unknown".into(), None),
    }
}

/// 今の入力が日本語 (かな) 入力か。英字モード (…Roman) と ABC などは false
pub fn is_japanese() -> bool {
    japanese_source().is_some()
}

/// 今の入力が日本語 (かな) 入力なら、その (Bundle ID, Input Mode)。HIToolbox の設定から読む
fn japanese_source() -> Option<(String, String)> {
    let out = run_with_timeout(
        Command::new("defaults").args(["read", "com.apple.HIToolbox", "AppleSelectedInputSources"]),
        Duration::from_secs(2),
    )?;
    parse_japanese_source(&out)
}

fn parse_japanese_source(out: &str) -> Option<(String, String)> {
    let bundle = regex::Regex::new(r#""Bundle ID" = "([^"]+)""#).unwrap();
    let mode = regex::Regex::new(r#""Input Mode" = "([^"]+)""#).unwrap();
    // 設定は入力ソースごとの { … } の並び
    out.split('}').find_map(|entry| {
        let m = mode.captures(entry)?[1].to_string();
        (!m.ends_with("Roman")).then(|| (bundle.captures(entry).map(|b| b[1].to_string()).unwrap_or_default(), m))
    })
}

fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> Option<String> {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut s = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut s).ok()?;
    Some(s)
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;

    pub const KEY_EISU: u16 = 0x66; // kVK_JIS_Eisu
    pub const KEY_KANA: u16 = 0x68; // kVK_JIS_Kana
    const HID_EVENT_TAP: u32 = 0;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn CGEventCreateKeyboardEvent(source: *const c_void, keycode: u16, keydown: bool) -> *mut c_void;
        fn CGEventPost(tap: u32, event: *mut c_void);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    pub fn trusted() -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    /// 入力ソースの API
    pub mod tis {
        use std::ffi::c_void;

        use core_foundation::array::{CFArray, CFArrayRef};
        use core_foundation::base::{CFTypeRef, TCFType};
        use core_foundation::boolean::{CFBoolean, CFBooleanRef};
        use core_foundation::dictionary::CFDictionaryRef;
        use core_foundation::string::{CFString, CFStringRef};

        #[link(name = "Carbon", kind = "framework")]
        unsafe extern "C" {
            fn TISCreateInputSourceList(props: CFDictionaryRef, include_all_installed: bool) -> CFArrayRef;
            fn TISCopyCurrentASCIICapableKeyboardLayoutInputSource() -> *const c_void;
            fn TISSelectInputSource(source: *const c_void) -> i32;
            fn TISGetInputSourceProperty(source: *const c_void, key: CFStringRef) -> CFTypeRef;
            static kTISPropertyInputSourceID: CFStringRef;
            static kTISPropertyInputSourceIsSelectCapable: CFStringRef;
            static kTISPropertyInputSourceIsASCIICapable: CFStringRef;
        }

        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            fn CFRelease(cf: *const c_void);
        }

        /// 英数にする。日本語入力自身の英数モード (ことえり・Google 日本語入力・ATOK の「英数」) があればそれを、
        /// なければ ASCII を打てるキーボード配列 (ABC など) を選ぶ
        pub fn select_ascii(bundle: &str) {
            if let Some(id) = super::super::ascii_mode(&sources(), bundle) {
                return select(&id);
            }
            unsafe {
                let s = TISCopyCurrentASCIICapableKeyboardLayoutInputSource();
                if !s.is_null() {
                    TISSelectInputSource(s);
                    CFRelease(s);
                }
            }
        }

        /// 使っていた日本語入力に戻すときの入力ソースの ID
        pub fn japanese_id(bundle: &str, mode: &str) -> Option<String> {
            super::super::japanese_mode(&sources(), bundle, mode)
        }

        /// ID の入力ソースを選ぶ
        pub fn select(id: &str) {
            unsafe {
                let arr = TISCreateInputSourceList(std::ptr::null(), false);
                if arr.is_null() {
                    return;
                }
                let arr: CFArray<*const c_void> = CFArray::wrap_under_create_rule(arr);
                if let Some(s) = arr.iter().find(|s| source_id(**s) == id) {
                    TISSelectInputSource(*s);
                }
            }
        }

        /// 使っている入力ソースの (ID, 選べるか, ASCII を打てるか)
        pub fn sources() -> Vec<(String, bool, bool)> {
            unsafe {
                let arr = TISCreateInputSourceList(std::ptr::null(), false);
                if arr.is_null() {
                    return Vec::new();
                }
                let arr: CFArray<*const c_void> = CFArray::wrap_under_create_rule(arr);
                arr.iter()
                    .map(|s| {
                        let flag = |key: CFStringRef| {
                            let v = TISGetInputSourceProperty(*s, key) as CFBooleanRef;
                            !v.is_null() && CFBoolean::wrap_under_get_rule(v) == CFBoolean::true_value()
                        };
                        (source_id(*s), flag(kTISPropertyInputSourceIsSelectCapable), flag(kTISPropertyInputSourceIsASCIICapable))
                    })
                    .collect()
            }
        }

        fn source_id(s: *const c_void) -> String {
            unsafe {
                let v = TISGetInputSourceProperty(s, kTISPropertyInputSourceID);
                if v.is_null() { String::new() } else { CFString::wrap_under_get_rule(v as CFStringRef).to_string() }
            }
        }
    }

    pub fn press(keycode: u16) {
        for down in [true, false] {
            unsafe {
                let ev = CGEventCreateKeyboardEvent(std::ptr::null(), keycode, down);
                if !ev.is_null() {
                    CGEventPost(HID_EVENT_TAP, ev);
                    CFRelease(ev);
                }
            }
        }
    }
}

/// 入力ソースの一覧 (ID, 選べるか, ASCII を打てるか) から、日本語入力 bundle 自身の英数モードを選ぶ
/// (ことえり …Roman、Google 日本語入力 com.google.inputmethod.Japanese.Roman など。全角英数は除く)
fn ascii_mode(sources: &[(String, bool, bool)], bundle: &str) -> Option<String> {
    sources
        .iter()
        .filter(|(id, select, ascii)| *select && *ascii && !bundle.is_empty() && id.starts_with(bundle) && id.as_str() != bundle)
        .map(|(id, _, _)| id)
        .find(|id| id.ends_with(".Roman"))
        .cloned()
}

/// 入力ソースの一覧から、使っていた日本語入力 (bundle の、ASCII を打てない選べるモード) を選ぶ。
/// Input Mode の最後の部分で終わるもの (ことえり …Japanese)、ひらがなのモード (Google 日本語入力 …base、
/// ATOK …Japanese) の順に優先し、カタカナなどのモードは最後にする
fn japanese_mode(sources: &[(String, bool, bool)], bundle: &str, mode: &str) -> Option<String> {
    let last = mode.rsplit('.').next().unwrap_or(mode);
    let ids: Vec<&String> =
        sources.iter().filter(|(id, select, ascii)| *select && !*ascii && id.starts_with(bundle)).map(|(id, _, _)| id).collect();
    let ends = |suffix: &str| ids.iter().find(|id| id.ends_with(suffix)).map(|id| id.to_string());
    ends(&format!(".{last}")).or_else(|| ends(".base")).or_else(|| ends(".Japanese")).or_else(|| ids.first().map(|id| id.to_string()))
}

pub struct InputSource {
    pub enabled: bool,
    /// キーイベントで切り替えるか (アクセシビリティの許可がある)。なければ入力ソースの API で切り替える
    pub key_events: bool,
    pub japanese: bool,              // 日本語入力を使っていたか (入力欄と終了時にかなへ戻すかどうか)
    japanese_id: Option<String>,     // 使っていた日本語入力の ID (API で戻すときに使う)
    bundle: String,                  // 使っていた日本語入力の Bundle ID (API で英数モードを探すのに使う)
}

impl Default for InputSource {
    fn default() -> Self {
        Self::new()
    }
}

impl InputSource {
    pub fn new() -> Self {
        #[cfg(target_os = "macos")]
        let (enabled, key_events) = (true, mac::trusted());
        #[cfg(not(target_os = "macos"))]
        let (enabled, key_events) = (false, false);
        Self { enabled, key_events, japanese: false, japanese_id: None, bundle: String::new() }
    }

    /// 英数入力にする (操作モード)。日本語入力だったかどうかを覚えておく
    pub fn to_ascii(&mut self) {
        if !self.enabled {
            return;
        }
        // 一度日本語入力と分かれば、以後は調べ直さない (調べるたびに外部コマンドを動かすので)
        if !self.japanese
            && let Some((bundle, _mode)) = japanese_source()
        {
            self.japanese = true;
            #[cfg(target_os = "macos")]
            if !self.key_events {
                self.japanese_id = mac::tis::japanese_id(&bundle, &_mode);
            }
            self.bundle = bundle;
        }
        #[cfg(target_os = "macos")]
        if self.key_events {
            mac::press(mac::KEY_EISU); // すでに英数でも害はないので必ず押す
        } else {
            mac::tis::select_ascii(&self.bundle);
        }
    }

    /// 日本語入力を使っていたなら、かなに戻す (入力欄・終了時)
    pub fn restore(&self) {
        if !(self.enabled && self.japanese) {
            return;
        }
        #[cfg(target_os = "macos")]
        if self.key_events {
            mac::press(mac::KEY_KANA);
        } else if let Some(id) = &self.japanese_id {
            mac::tis::select(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_japanese_source_from_hitoolbox() {
        let out = r#"(
        {
        "Bundle ID" = "com.apple.PressAndHold";
        InputSourceKind = "Non Keyboard Input Method";
    },
        {
        "Bundle ID" = "com.apple.inputmethod.Kotoeri.RomajiTyping";
        "Input Mode" = "com.apple.inputmethod.Japanese";
        InputSourceKind = "Input Mode";
    }
)"#;
        assert_eq!(
            parse_japanese_source(out),
            Some(("com.apple.inputmethod.Kotoeri.RomajiTyping".into(), "com.apple.inputmethod.Japanese".into()))
        );
        assert_eq!(parse_japanese_source(&out.replace("Japanese\"", "Roman\"")), None);
    }

    fn list(v: &[(&str, bool, bool)]) -> Vec<(String, bool, bool)> {
        v.iter().map(|(id, s, a)| (id.to_string(), *s, *a)).collect()
    }

    #[test]
    fn chooses_modes_for_each_ime() {
        // ことえり (ABC と一緒に使う。自身の英数モードは入力ソースに出ない)
        let kotoeri = list(&[
            ("com.apple.keylayout.ABC", true, true),
            ("com.apple.inputmethod.Kotoeri.RomajiTyping.Japanese", true, false),
            ("com.apple.inputmethod.Kotoeri.RomajiTyping", false, true),
        ]);
        let b = "com.apple.inputmethod.Kotoeri.RomajiTyping";
        assert_eq!(ascii_mode(&kotoeri, b), None); // ABC などのキーボード配列を選ぶ
        assert_eq!(japanese_mode(&kotoeri, b, "com.apple.inputmethod.Japanese").as_deref(), Some("com.apple.inputmethod.Kotoeri.RomajiTyping.Japanese"));
        // Google 日本語入力 (ABC を外していることが多い)
        let google = list(&[
            ("com.google.inputmethod.Japanese", false, true),
            ("com.google.inputmethod.Japanese.Katakana", true, false),
            ("com.google.inputmethod.Japanese.base", true, false),
            ("com.google.inputmethod.Japanese.FullWidthRoman", true, false),
            ("com.google.inputmethod.Japanese.Roman", true, true),
        ]);
        let b = "com.google.inputmethod.Japanese";
        assert_eq!(ascii_mode(&google, b).as_deref(), Some("com.google.inputmethod.Japanese.Roman"));
        assert_eq!(japanese_mode(&google, b, "com.apple.inputmethod.Japanese").as_deref(), Some("com.google.inputmethod.Japanese.base"));
    }
}
