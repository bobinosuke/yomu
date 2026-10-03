//! tests/test_search.py の移植 (偽の DuckDuckGo で確かめる)
use std::cell::RefCell;

use yomu::fetch::HttpError;
use yomu::search::{Fields, GOTO, PAGE_SCHEME, next_label, page_fields, prev_label, search_with};
use yomu::urls::parse_qs_first;

fn results_page(urls: &[&str], next_s: Option<&str>) -> String {
    let items: String = urls
        .iter()
        .map(|u| format!(r#"<div class="result"><a class="result__a" href="{u}">題 {u}</a><a class="result__snippet">説明 {u}</a></div>"#))
        .collect();
    let nxt = next_s.map_or(String::new(), |s| {
        format!(
            r#"<div class="nav-link"><form action="/html/" method="post"><input type="submit" value="Next"><input type="hidden" name="q" value="将棋"><input type="hidden" name="s" value="{s}"><input type="hidden" name="vqd" value="abc"></form></div>"#
        )
    });
    format!("<html><body>{items}{nxt}</body></html>")
}

fn pairs(v: &[(&str, &str)]) -> Fields {
    v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

/// フォームの値を URL エンコードして送る所まで本物と同じにし、送った値を記録する
fn fake(sent: &RefCell<Vec<Fields>>) -> impl Fn(&Fields) -> Result<String, HttpError> + '_ {
    move |form| {
        let form = parse_qs_first(&yomu::urls::urlencode(form), false);
        let second = form.iter().any(|(k, v)| k == "s" && v == "10");
        sent.borrow_mut().push(form);
        Ok(if second {
            // 2 ページ目: 1 ページ目と同じ結果が 1 件混ざる
            results_page(&["https://a.jp/2", "https://c.jp/", "https://d.jp/"], None)
        } else {
            results_page(&["https://a.jp/1", "https://a.jp/2"], Some("10"))
        })
    }
}

#[test]
fn next_page_link_and_skips_seen_results() {
    let sent = RefCell::new(vec![]);
    let first = search_with(fake(&sent), "将棋", None, true).unwrap();
    assert_eq!(first.links[..2], ["https://a.jp/1", "https://a.jp/2"]);
    assert_eq!(first.blocks.last().unwrap().text(), next_label());
    assert!(first.links.last().unwrap().starts_with(PAGE_SCHEME));
    assert_eq!(sent.borrow()[0], pairs(&[("q", "将棋"), ("kl", "jp-jp")]));

    let fields = page_fields(first.links.last().unwrap());
    let q = yomu::search::field(&fields, "q").unwrap().to_string();
    let second = search_with(fake(&sent), &q, Some(&fields), true).unwrap();
    assert_eq!(second.title, "検索: 将棋 (2 ページ目)");
    let results: Vec<&String> = second.links.iter().filter(|u| !u.starts_with(PAGE_SCHEME)).collect();
    assert_eq!(results, ["https://c.jp/", "https://d.jp/"]); // 1 ページ目にあったものは除く
    assert_eq!(sent.borrow()[1], pairs(&[("q", "将棋"), ("s", "10"), ("vqd", "abc")])); // 内部用の値は送らない
    assert_eq!(second.blocks.last().unwrap().text(), prev_label()); // 最後のページには「次」は付けず、「前」だけ
    assert_eq!(page_fields(second.links.last().unwrap()), pairs(&[("q", "将棋"), (GOTO, "1")]));
    assert!(!first.blocks.last().unwrap().text().contains(prev_label())); // 1 ページ目には「前」を付けない
}

#[test]
fn dump_has_no_next_link() {
    let sent = RefCell::new(vec![]);
    let doc = search_with(fake(&sent), "将棋", None, false).unwrap();
    assert!(!doc.blocks.iter().any(|b| b.text() == next_label()));
}

#[test]
fn ads_and_redirects() {
    let html = r#"<div class="result result--ad"><a class="result__a" href="https://ad.example/">広告</a></div>
        <div class="result"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2F%E5%B0%86&rut=x"> 題 </a></div>"#;
    let doc = search_with(|_| Ok(html.to_string()), "x", None, true).unwrap();
    assert_eq!(doc.links, ["https://example.com/将"]);
    assert_eq!(doc.blocks[0].text(), "題");
}
