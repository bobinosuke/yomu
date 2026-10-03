//! yomu のコマンド。引数と挙動は Python 版の cli.py と同じ。
use std::process::ExitCode;

use yomu::t;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const USAGE: &str = "usage: yomu [-h] [--dump] [--full] [--keep-ime] [--private] [target ...]";
/// --help の説明 (t! で訳す)
fn help() -> &'static str {
    t!("本文だけを抜き出して読む、日本語向けの軽量 CLI ブラウザ

positional arguments:
  target      URL または検索語 (URL でなければ DuckDuckGo で検索)

options:
  -h, --help  show this help message and exit
  --dump      対話画面を開かずに本文を出力する (端末には整形して、パイプには Markdown で)
  --full      本文抽出せずページ全体を出す
  --keep-ime  IME を自動で切り替えない (既定では操作モードの間だけ英数入力にする)
  --private   プライベートモード: すべての通信を Tor に通し、履歴・タブ・Cookie を残さない (翻訳は使えない)")
}

fn main() -> ExitCode {
    yomu::i18n::set_lang(yomu::settings::ui_lang(&yomu::settings::get()));
    let (mut dump, mut full, mut keep_ime, mut private, mut words) = (false, false, false, false, Vec::new());
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--dump" => dump = true,
            "--full" => full = true,
            "--keep-ime" => keep_ime = true,
            "--private" => private = true,
            "-h" | "--help" => {
                println!("{USAGE}\n\n{}", help());
                return ExitCode::SUCCESS;
            }
            s if s.starts_with("--") => return usage_error(&format!("unrecognized arguments: {s}")),
            _ => words.push(a),
        }
    }
    let target = words.join(" ");
    if private {
        yomu::fetch::set_private();
    }
    if dump {
        if private {
            yomu::tor::start(|m| eprintln!("{m}"));
        }
        if target.is_empty() {
            return usage_error(t!("--dump には URL か検索語が必要です"));
        }
        return ExitCode::from(yomu::dump::dump(&target, full) as u8);
    }
    let mut app = yomu::tui::App::new((!target.is_empty()).then_some(target), full, !keep_ime);
    if let Err(e) = app.run() {
        eprintln!("yomu: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn usage_error(msg: &str) -> ExitCode {
    eprintln!("{USAGE}\nyomu: error: {msg}");
    ExitCode::from(2)
}
