//! yomu (Yuto's Oreore Minimal User-interface browser): 本文だけを抜き出して読む、日本語向けの軽量 CLI ブラウザ。
//! Python 版 (yomu-python。今は開発していない) から移植した。
#[macro_use]
pub mod i18n; // t! をほかのモジュールで使うので最初に置く
pub mod doc;
pub mod dump;
pub mod extract;
pub mod fetch;
pub mod history;
pub mod images;
pub mod ime;
pub mod loader;
pub mod pages;
pub mod pytext;
pub mod render;
pub mod search;
pub mod settings;
pub mod speech;
pub mod store;
pub mod translate;
pub mod trackers;
pub mod tor;
pub mod tui;
pub mod urls;
pub mod visual;
