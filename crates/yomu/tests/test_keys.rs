//! tests/test_keys.py の移植 (hint_labels は TUI 側のテストで確かめる)
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use yomu::ime::{normalize_key, read_key};
use yomu::urls::{looks_like_url, parent_url};

#[test]
fn looks_like_url_() {
    assert!(looks_like_url("https://example.com/a b"));
    assert!(looks_like_url("example.com"));
    assert!(!looks_like_url("将棋 ルール"));
    assert!(!looks_like_url("python"));
    assert!(!looks_like_url("例え.jp"));
}

#[test]
fn normalize_key_reads_ime_output() {
    let got: Vec<String> = "ｊＧ？・「」１".chars().map(|c| normalize_key(Some(&c.to_string())).unwrap()).collect();
    assert_eq!(got, ["j", "G", "?", "/", "[", "]", "1"]);
    assert_eq!(normalize_key(None), None);
}

#[test]
fn read_key_names() {
    let k = |code, m| read_key(&KeyEvent::new(code, m));
    assert_eq!(k(KeyCode::Char('ｊ'), KeyModifiers::NONE), ("j".into(), Some("j".into())));
    assert_eq!(k(KeyCode::Char('J'), KeyModifiers::SHIFT), ("J".into(), Some("J".into())));
    assert_eq!(k(KeyCode::Char(' '), KeyModifiers::NONE), ("space".into(), None));
    assert_eq!(k(KeyCode::Char('\u{3000}'), KeyModifiers::NONE), ("space".into(), None));
    assert_eq!(k(KeyCode::Char('n'), KeyModifiers::CONTROL), ("ctrl+n".into(), None));
    assert_eq!(k(KeyCode::Esc, KeyModifiers::NONE), ("escape".into(), None));
    assert_eq!(k(KeyCode::PageDown, KeyModifiers::NONE), ("pagedown".into(), None));
    assert_eq!(k(KeyCode::BackTab, KeyModifiers::SHIFT), ("shift+tab".into(), None));
}

#[test]
fn parent_url_() {
    assert_eq!(parent_url("https://example.com/a/b/c.html?x=1", false), "https://example.com/a/b/");
    assert_eq!(parent_url("https://example.com/a/b/", false), "https://example.com/a/");
    assert_eq!(parent_url("https://example.com/a/b/", true), "https://example.com/");
}
