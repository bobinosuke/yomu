//! yomu の読み上げ。合成は Supertonic 3 (日本語も英語も同じ声)。
//!
//! 日本語の文の中の英単語は、辞書の読み (text2mecab → mecab (OpenJTalk の辞書 + AivisSpeech の辞書) → njd) か、
//! 辞書になければ kanalizer でカタカナにしてから読む (katakana)。
pub mod download;
pub mod frontend;
pub mod katakana;
pub mod mecab;
pub mod njd;
pub mod resources;
pub mod sentences;
pub mod speaker;
pub mod supertonic;
pub mod text2mecab;
