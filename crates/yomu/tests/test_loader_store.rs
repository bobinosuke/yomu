//! tests/test_loader_store.py の移植
use std::path::PathBuf;

use yomu::fetch::Response;
use yomu::loader::{filename, pdf_paragraphs, save_download, to_document};
use yomu::store::Store;

/// 文字を 1 行だけ含む最小限の PDF
fn minimal_pdf(text: &str) -> Vec<u8> {
    let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = vec![];
    for (n, body) in objs.iter().enumerate() {
        offsets.push(out.len());
        out += &format!("{} 0 obj\n{body}\nendobj\n", n + 1);
    }
    let xref = out.len();
    out += &format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1);
    for o in offsets {
        out += &format!("{o:010} 00000 n \n");
    }
    out += &format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1);
    out.into_bytes()
}

fn resp(content: &[u8], ctype: &str, status: u16, url: &str) -> Response {
    Response::new(url, status, ctype, content.to_vec())
}

fn tmp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("yomu-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn error_page_shows_body_and_status() {
    let body = "<html><body><h1>ページが見つかりません</h1><p>お探しのページは移動しました。</p></body></html>";
    let doc = to_document(&resp(body.as_bytes(), "text/html; charset=utf-8", 404, "https://example.com/a/doc"), false);
    assert!(doc.note.starts_with("HTTP 404"));
    assert!(doc.blocks.iter().any(|b| b.text().contains("移動しました")));
}

#[test]
fn pdf_text_is_shown_by_page() {
    let doc = to_document(&resp(&minimal_pdf("Hello yomu PDF."), "application/pdf", 200, "https://example.com/a/doc"), false);
    assert_eq!(doc.note, "PDF 1 ページ");
    let texts: Vec<String> = doc.blocks.iter().map(|b| b.text()).collect();
    assert_eq!(texts, ["1 ページ", "Hello yomu PDF."]);
}

#[test]
fn broken_pdf_is_a_message() {
    let doc = to_document(&resp(b"%PDF-1.4 broken", "application/pdf", 200, "https://example.com/x.pdf"), false);
    assert!(doc.blocks[0].text().starts_with("PDF を読めませんでした"));
}

#[test]
fn pdf_paragraphs_join_wrapped_lines() {
    let text = "日本語の文章が行の途中で\n改行されている。\n\nThis English line\ncontinues here.";
    assert_eq!(pdf_paragraphs(text), ["日本語の文章が行の途中で改行されている。", "This English line continues here."]);
}

#[test]
fn other_files_are_saved_with_unique_names() {
    let dir = tmp_dir("save");
    let r = resp(b"PK\x03\x04", "application/zip", 200, "https://example.com/files/data.zip");
    let (first, second) = (save_download(&r, &dir), save_download(&r, &dir));
    assert_eq!(std::fs::read(dir.join("data.zip")).unwrap(), b"PK\x03\x04");
    assert!(dir.join("data (1).zip").exists());
    assert!(first.blocks[0].text().contains("保存しました") && second.blocks[0].text().contains("data (1).zip"));
}

#[test]
fn filename_prefers_content_disposition() {
    let mut r = resp(b"", "application/octet-stream", 200, "https://example.com/a/doc");
    r.disposition = "attachment; filename=\"report.csv\"".into();
    assert_eq!(filename(&r), "report.csv");
    let r = resp(b"", "application/octet-stream", 200, "https://example.com/dl/%E5%B0%86%E6%A3%8B.pdf?x=1");
    assert_eq!(filename(&r), "将棋.pdf");
    assert_eq!(filename(&resp(b"", "x/y", 200, "https://example.com/")), "download");
}

#[test]
fn history_and_bookmarks() {
    let dir = tmp_dir("store");
    let mut s = Store::open(&dir);
    s.visit("url", "https://example.com/shogi", "将棋入門");
    s.visit("search", "将棋 ルール", "検索: 将棋 ルール");
    s.visit("url", "https://example.com/go", "囲碁入門");
    assert!(s.toggle_bookmark("url", "https://example.com/go", "囲碁入門"));
    let targets = |items: Vec<yomu::store::Item>| items.into_iter().map(|i| i.target).collect::<Vec<_>>();
    // ブックマークが先、そのあと新しい順。すべての語を含むものだけ
    assert_eq!(targets(s.suggest("", false, 10)), ["https://example.com/go", "将棋 ルール", "https://example.com/shogi"]);
    assert_eq!(targets(s.suggest("将棋 入門", false, 10)), ["https://example.com/shogi"]);
    assert_eq!(targets(s.suggest("", true, 10)), ["https://example.com/go"]);
    // ファイルに残り、次に起動したときも読める
    let mut again = Store::open(&dir);
    assert_eq!(again.history.items[0].target, "https://example.com/go");
    assert!(!again.toggle_bookmark("url", "https://example.com/go", "囲碁入門"));
    assert!(again.suggest("", true, 10).is_empty());
}

#[test]
fn store_reads_python_format() {
    let dir = tmp_dir("pyformat");
    std::fs::write(
        dir.join("history.json"),
        "[\n {\n  \"kind\": \"url\",\n  \"target\": \"https://e.com/\",\n  \"title\": \"題\",\n  \"time\": 1759200000\n },\n {\"kind\": \"search\", \"target\": \"語\"}\n]",
    )
    .unwrap();
    let s = Store::open(&dir);
    assert_eq!(s.history.items.len(), 2);
    assert_eq!((s.history.items[0].time, s.history.items[1].title.as_str()), (1759200000.0, ""));
}
