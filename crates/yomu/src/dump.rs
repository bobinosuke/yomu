//! --dump: 対話画面を開かずに本文を出力する (端末には TUI と同じ見た目で、パイプやファイルには Markdown で)。
use std::io::{IsTerminal, Write};

use crate::doc::Document;
use crate::fetch::{HttpError, client, fetch};
use crate::loader::{panic_message, to_document_with};
use crate::settings::Settings;
use crate::render::{READING_WIDTH, to_ansi, to_markdown, to_terminal_wide};
use crate::search::search;
use crate::urls::{looks_like_url, to_url};

/// 終了コードを返す
pub fn dump(target: &str, full: bool) -> i32 {
    let doc = match load(target, full) {
        Ok(Ok(doc)) => doc,
        Ok(Err(e)) => {
            eprintln!("yomu: {}", t!("読み込めませんでした: {e}", e = e));
            return 1;
        }
        Err(msg) => {
            // 壊れたページで抽出が落ちたときも、理由だけを出す
            eprintln!("yomu: {}", t!("このページを表示できませんでした ({msg})", msg = msg));
            return 1;
        }
    };
    let mut out = std::io::stdout().lock();
    let text = if out.is_terminal() {
        let width = crossterm::terminal::size().map(|(w, _)| w as usize).unwrap_or(READING_WIDTH);
        let mut s = to_ansi(&to_terminal_wide(&doc, width.min(READING_WIDTH), width));
        if !s.ends_with('\n') {
            s.push('\n');
        }
        s
    } else {
        to_markdown(&doc)
    };
    let _ = out.write_all(text.as_bytes());
    0
}

/// 外側の Err は抽出中の panic (その内容)
fn load(target: &str, full: bool) -> Result<Result<Document, HttpError>, String> {
    if crate::fetch::CookieJar::uses_file() {
        crate::fetch::COOKIES.load();
    }
    let c = client();
    let target = target.to_string();
    let run = move || -> Result<Document, HttpError> {
        if looks_like_url(&target) {
            let r = fetch(&c, &to_url(&target))?;
            // 広告・追跡のリンクの設定は使う (画像は Markdown に出さない)
            Ok(to_document_with(&r, full, Settings { images: false, ..crate::settings::get() }))
        } else {
            search(&target, None, false)
        }
    };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).map_err(|p| panic_message(&*p))
}
