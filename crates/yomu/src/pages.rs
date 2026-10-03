//! yomu が自分で作るページ (ヘルプ・新しいタブ・エラーなど)。
use crate::doc::{Block, Document, Kind, Span};

/// ヘルプに出すキー操作 (見出し, [(キー, 説明)])
pub fn help() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    vec![
        (t!("ページ内の移動"), vec![
            ("j / k", t!("下 / 上にスクロール")), ("h / l", t!("左 / 右にスクロール")),
            ("d / u", t!("半ページ下 / 上")), ("gg / G", t!("先頭 / 末尾")),
            ("f", t!("リンクを開く (ラベルを入力)")), ("F", t!("リンクを新しいタブで開く")),
            ("yf", t!("リンクの URL をコピー")), ("yy", t!("今のページの URL をコピー")),
            ("[[ / ]]", t!("「前へ」/「次へ」のリンクを開く")), ("r", t!("再読み込み")),
        ]),
        (t!("URL を開く"), vec![
            ("o / O", t!("URL を開く・検索。履歴とブックマークから候補を出す (O は新しいタブ)")),
            ("b / B", t!("ブックマークから開く (B は新しいタブ)")),
            ("↑↓ / Tab", t!("入力欄で候補を選ぶ")),
            ("ge / gE", t!("今の URL を編集して開く (gE は新しいタブ)")),
            ("p / P", t!("クリップボードの URL・語を開く (P は新しいタブ)")),
            ("gu / gU", t!("URL を1階層上 / サイトのトップへ")),
        ]),
        (t!("ページ内検索"), vec![("/", t!("検索")), ("n / N", t!("次 / 前の一致"))]),
        (t!("ビジュアルモード"), vec![
            ("v", t!("カーソルだけ動かす (キャレット)。もう一度 v でその位置から選択を始める")),
            ("V", t!("行単位で選択を始める")), ("c", t!("選択中にキャレットへ戻る (選択し直す)")),
            ("h j k l", t!("左 下 上 右")), ("w / b / e", t!("次の語 / 前の語 / 語の終わり (日本語は文字種の変わり目で区切る)")),
            ("0 / ^ / $", t!("行頭 / 行の最初の文字 / 行末")), ("gg / G", t!("先頭 / 末尾")), ("{ / }", t!("前 / 次の段落")),
            ("o", t!("選択の反対の端へ")), ("y", t!("コピーして終了")), ("E", t!("選んだ文だけを訳して、選んだ所の下に出す (ページの対訳と同じく原文は残す)")), ("S", t!("選んだ文だけを読み上げて終了 (言語は選んだ文から決める。キャレットならカーソルの文から最後まで)")), ("Esc", t!("終了")),
        ]),
        (t!("履歴"), vec![("H / L", t!("戻る / 進む"))]),
        (t!("タブ"), vec![
            ("t", t!("新しいタブ")), ("J gT / K gt", t!("左 / 右のタブ")), ("g0 / g$", t!("最初 / 最後のタブ")),
            ("^", t!("直前に見ていたタブ")), ("yt", t!("タブを複製")), ("x / X", t!("タブを閉じる / 閉じたタブを戻す")),
            ("<< / >>", t!("タブを左 / 右へ移動")),
        ]),
        (t!("マーク"), vec![(t!("m + 英字"), t!("位置を記録 (大文字はページをまたぐ)")), (t!("` + 英字"), t!("記録した位置へ")), ("``", t!("ジャンプ前の位置へ"))]),
        (t!("その他"), vec![(t!("数字 + コマンド"), t!("回数指定 (5j など)")), ("Esc", t!("取り消し")), ("?", t!("このヘルプ"))]),
        (t!("yomu 独自"), vec![
            ("s", t!("検索エンジンで検索 (結果の末尾の「次の検索結果 →」か ]] で続き、「← 前の検索結果」か [[ で前へ)")), ("a", t!("本文抽出 ⇔ ページ全体")),
            ("gb", t!("今のページをブックマークに追加 / 削除")),
            ("gx", t!("今のページを普段のブラウザで開く (JavaScript がないと読めないページ用)")),
            ("gp", t!("プライベートモード (Tor) に切り替える / 戻る (yomu を起動し直す。普段のタブは戻ったときに開き直す)")),
            ("gi", t!("ページの入力欄 (検索のフォームなど) に文字を入れて送る。入力欄が複数あれば選ぶ")),
            ("gh", t!("履歴の画面 (選んで開く。1 時間以内・昨日と今日・すべての履歴を消せる)")),
            ("gs", t!("設定 (言語・Cookie の保存・広告と追跡のリンクを消す・画像の表示)")),
            ("S", t!("画面の位置から読み上げ / 停止 (Supertonic 3。初回に辞書とモデルをダウンロードする)")),
            ("e", t!("ページを日本語に翻訳 (原文を訳文に置き換える。Google 翻訳)。もう一度 e で原文に戻す")),
            ("E", t!("原文の下に訳文を並べる (対訳)。もう一度 E で原文に戻す")),
            ("q", t!("終了")),
        ]),
    ]
}

pub fn help_page() -> Document {
    let mut blocks = vec![Block::heading(t!("yomu のキー操作 (Vimium 準拠)"), 1)];
    for (section, items) in help() {
        blocks.push(Block::heading(section, 3));
        for (k, d) in items {
            // Python の str.ljust(14) と同じく文字数でそろえる
            let pad = 14usize.saturating_sub(k.chars().count());
            let key = Span { bold: true, ..Span::new(format!("{k}{}", " ".repeat(pad))) };
            blocks.push(Block::new(Kind::Li, vec![key, Span::new(d)]));
        }
    }
    Document { url: "about:help".into(), title: t!("ヘルプ").into(), blocks, note: t!("ヘルプ (Esc / ? で閉じる)").into(), ..Default::default() }
}

/// 入力欄の一覧 (gi で入力欄が複数あったとき)。選ぶと、その入力欄に入力する (yomu-form:番号)
pub fn forms_page(forms: &[crate::doc::Form]) -> Document {
    let mut blocks = vec![Block::heading(t!("入力欄"), 1), Block::para(t!("入力する欄を f のラベルかクリックで選んでください。"))];
    let mut links = Vec::new();
    for (i, f) in forms.iter().enumerate() {
        let label = if f.label.is_empty() { t!("(説明のない入力欄)").to_string() } else { f.label.clone() };
        let mut spans = Vec::new();
        if f.get {
            links.push(format!("yomu-form:{i}"));
            spans.push(Span { link: Some(links.len() - 1), ..Span::new(label) });
        } else {
            spans.push(Span::new(label));
            spans.push(Span { minor: true, ..Span::new(format!("  {}", t!("(POST で送るフォームは未対応)"))) });
        }
        spans.push(Span { minor: true, ..Span::new(format!("  → {}", crate::urls::unquote(&f.action))) });
        blocks.push(Block::new(Kind::Li, spans));
    }
    Document { url: "about:forms".into(), title: t!("入力欄").into(), blocks, links, note: t!("入力欄 (Esc で閉じる)").into(), ..Default::default() }
}

pub fn blank_page() -> Document {
    let body = vec![Block::para(t!("o で URL を開くか検索、? でキー操作の一覧を表示します。"))];
    Document { url: "about:blank".into(), title: t!("新しいタブ").into(), blocks: body, note: t!("新しいタブ").into(), ..Default::default() }
}

pub fn message_page(url: &str, title: &str, message: &str, note: &str) -> Document {
    Document { url: url.into(), title: title.into(), blocks: vec![Block::para(message)], note: note.into(), ..Default::default() }
}

pub fn text_page(url: &str, text: &str) -> Document {
    let block = Block::new(Kind::Pre, vec![Span { code: true, ..Span::new(text) }]);
    Document { url: url.into(), title: url.into(), blocks: vec![block], note: t!("テキスト").into(), ..Default::default() }
}
