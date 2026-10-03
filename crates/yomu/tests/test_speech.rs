//! tests/test_speech.py の移植 (音素と声の選び方は yomu-tts のテストで確かめる)
use yomu::doc::{Block, Kind, Span};
use yomu::speech::{Lang, block_text, chunks_for_view, chunks_from, detect_lang, is_body, is_label, start_at};
use yomu_tts::sentences::split_sentences;

fn p(text: &str) -> Block {
    Block::new(Kind::P, vec![Span::new(text)])
}

fn h(text: &str, level: u32) -> Block {
    Block { level, ..Block::new(Kind::H, vec![Span::new(text)]) }
}

fn linked(text: &str) -> Span {
    Span { link: Some(0), ..Span::new(text) }
}

fn owned(v: &[(usize, &str)]) -> Vec<(usize, String)> {
    v.iter().map(|(i, s)| (*i, s.to_string())).collect()
}

#[test]
fn block_text_skips_images_and_urls() {
    let b = Block::new(Kind::P, vec![Span::new("本文 https://example.com を見る"), Span { image: true, ..Span::new("［画像: 図］") }]);
    assert_eq!(block_text(&b), "本文  を見る");
}

#[test]
fn is_body_() {
    assert!(is_body(&p("将棋は二人で行う。")));
    assert!(!is_body(&p("123"))); // いいね数など
    assert!(!is_body(&p("トップ >"))); // パンくず
    assert!(!is_body(&Block::new(Kind::P, vec![linked("関連記事のタイトル")]))); // リンクだらけ
    assert!(is_body(&Block { level: 2, ..Block::new(Kind::H, vec![linked("見出しのリンク")]) })); // 見出しはリンクでも読む
    assert!(!is_body(&Block::new(Kind::Pre, vec![Span { code: true, ..Span::new("code") }])));
}

#[test]
fn chunks_skip_tail_sections_and_empty_headings() {
    let blocks = vec![
        h("本題", 2),
        p("本文です。次の文です！"),
        h("記事一覧", 2),
        Block { marker: "・".into(), ..Block::new(Kind::Li, vec![linked("別の記事")]) },
        h("脚注", 2),
        p("注釈の文です。"),
    ];
    assert_eq!(chunks_from(&blocks, 0), owned(&[(0, "本題"), (1, "本文です。"), (1, "次の文です！")]));
    assert_eq!(chunks_from(&blocks, 1), owned(&[(1, "本文です。"), (1, "次の文です！")]));
}

#[test]
fn split_long_sentence_at_commas() {
    let s = "あ、".repeat(100);
    let parts = split_sentences(&s);
    assert!(parts.iter().all(|x| x.chars().count() <= 150) && parts.concat() == s);
}

#[test]
fn chunks_for_view_maps_body_to_full_view() {
    let body = vec![p("本文の一文目です。"), p("本文の二文目です。")];
    let shown = vec![p("メニュー"), body[0].clone(), p("広告"), body[1].clone()];
    assert_eq!(chunks_for_view(&body, &shown, 0), owned(&[(1, "本文の一文目です。"), (3, "本文の二文目です。")]));
    assert_eq!(chunks_for_view(&body, &shown, 2), owned(&[(3, "本文の二文目です。")]));
}

#[test]
fn detect_lang_by_page() {
    assert_eq!(detect_lang(&["将棋は二人で行うボードゲームである。", "Python の asyncio を使う。"], ""), Some(Lang::JA));
    // 英語のページに日本語が少し混ざっていても英語
    assert_eq!(detect_lang(&["Shogi (将棋) is a two-player strategy board game.", "It is played on a 9x9 board."], ""), Some(Lang::EN));
    assert_eq!(detect_lang(&["123 456"], ""), None); // 文字がなければ決められない
}

#[test]
fn detect_lang_uses_declared_and_detected_languages() {
    let fr = ["Le shogi est un jeu de société traditionnel japonais.", "Il se joue à deux sur un plateau de neuf cases."];
    let code = |texts: &[&str], declared: &str| detect_lang(texts, declared).map(Lang::code);
    // 宣言があればそれ (地域の部分は見ない)
    assert_eq!(code(&fr, "fr-FR"), Some("fr"));
    // 宣言がなければ本文から推定する
    assert_eq!(code(&fr, ""), Some("fr"));
    assert_eq!(code(&["장기는 두 사람이 하는 보드 게임이다. 한국의 전통 놀이이다."], ""), Some("ko"));
    // 日本語の文章は、英語のテンプレートのまま lang="en" と宣言していても日本語
    assert_eq!(code(&["将棋は二人で行うボードゲームである。"], "en"), Some("ja"));
    // 読めない言語の宣言があっても、本文が読める言語ならその言語
    let en = ["This page is written in English, although the site declares that it is in Chinese.",
              "The rest of the article explains the rules of the game and how the pieces move on the board."];
    assert_eq!(code(&en, "zh-CN"), Some("en"));
}

#[test]
fn unsupported_languages_are_not_read() {
    let code = |texts: &[&str], declared: &str| detect_lang(texts, declared).map(Lang::code);
    // かなのない漢字だけの文章 (中国語) は日本語にしない
    let zh = ["象棋是一种两人对弈的中国传统棋类游戏。", "棋盘由九条竖线和十条横线组成。"];
    assert_eq!(code(&zh, "zh-CN"), None);
    assert_eq!(code(&zh, ""), None);
    // タイ語
    assert_eq!(code(&["หมากรุกญี่ปุ่นเป็นเกมกระดานสำหรับผู้เล่นสองคน"], "th"), None);
}

#[test]
fn english_sentences_are_split() {
    assert_eq!(split_sentences("Shogi is a game. It has 40 pieces! Is it hard?"), ["Shogi is a game.", "It has 40 pieces!", "Is it hard?"]);
    let long = "word ".repeat(60);
    assert!(split_sentences(&long).iter().all(|x| x.chars().count() <= 150 && !x.starts_with(' ')));
}

#[test]
fn labels_are_headings_and_whole_line_fragments() {
    let blocks =
        [h("概要", 2), p("主に"), p("一文目です。二文目"), p("これは文です。"), p("「引用です。」"), p("【擬音語・擬態語】\nざーざー・ぺらぺらなど")];
    let labels: Vec<(usize, String)> = chunks_from(&blocks, 0).into_iter().filter(|(i, t)| is_label(&blocks[*i], t)).collect();
    assert_eq!(labels, [(0, "概要".into()), (1, "主に".into()), (5, "【擬音語・擬態語】".into()), (5, "ざーざー・ぺらぺらなど".into())]);
}

#[test]
fn colon_inside_parentheses_does_not_split() {
    assert_eq!(
        split_sentences("混血語（こんけつご、英: hybrid、独: hybrides Wort）とは合成語のこと。例: 荷物"),
        ["混血語（こんけつご、英: hybrid、独: hybrides Wort）とは合成語のこと。", "例:", "荷物"]
    );
}

#[test]
fn start_at_sentence_of_cursor() {
    let chunks = owned(&[(0, "前の段落です。"), (1, "一文目です。"), (1, "二文目です。"), (1, "三文目です。"), (2, "次の段落です。")]);
    let without = |skip: &[usize]| -> Vec<(usize, String)> {
        chunks.iter().enumerate().filter(|(i, _)| !skip.contains(i)).map(|(_, c)| c.clone()).collect()
    };
    assert_eq!(start_at(&chunks, 1, "二文目です。三文目です。"), without(&[1]));
    assert_eq!(start_at(&chunks, 1, "目です。三文目です。"), without(&[1])); // 文の途中なら、その文から
    assert_eq!(start_at(&chunks, 1, "一文目です。二文目です。三文目です。"), chunks);
    // 画面には画像の alt などが余分に出ていても、出だしが一致する所を探す
    assert_eq!(start_at(&chunks, 1, "三文目です。［画像: 図］"), without(&[1, 2]));
    // 読まないブロック (コードなど) なら何も飛ばさない
    assert_eq!(start_at(&chunks[4..], 3, "x = 1"), chunks[4..].to_vec());
}
