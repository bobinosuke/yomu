//! 本文の画像 (設定の images)。画像だけの段落の画像を裏で取ってきて、段落の下に場所を空けて描く。
use std::collections::HashMap;
use std::sync::Arc;

use image::DynamicImage;

use crate::doc::Document;
use crate::render::{Line, Seg};

/// 取ってくる画像の大きさの上限 (バイト)
const MAX_BYTES: usize = 15 << 20;
/// 取ってきた画像はこの幅 (ピクセル) まで縮めて持つ
const MAX_PIXELS_WIDE: u32 = 1600;
/// 1 枚の画像に使う行数の上限
pub const MAX_ROWS: u16 = 24;
/// 同時に取ってくる数
const WORKERS: usize = 4;

pub enum State {
    Loading,
    Failed,
    Ready(Arc<DynamicImage>),
}

/// 画像を描く場所 (描画結果の行での位置)
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub top: usize,
    pub rows: u16,
    pub cols: u16,
    pub src: String,
}

/// 画像の描き方と 1 文字の大きさ。端末への問い合わせ (返事を読むために端末の入力を横取りする) は、操作中に行うと
/// キー入力を奪ったり入力の設定を壊したりするので使わず、環境変数と端末の画面の大きさ (ピクセル) から決める。
/// わからなければ半角ブロックの色で描く (どの端末でも出る)
pub fn picker() -> ratatui_image::picker::Picker {
    use ratatui_image::picker::{Picker, ProtocolType};
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    let font = crossterm::terminal::window_size()
        .ok()
        .filter(|w| w.width > 0 && w.height > 0 && w.columns > 0 && w.rows > 0)
        .map_or((10, 20), |w| (w.width / w.columns, w.height / w.rows));
    // 非推奨 (問い合わせを勧める) だが、上の理由で問い合わせは使えない
    #[allow(deprecated)]
    let mut p = Picker::from_fontsize(ratatui_image::FontSize { width: font.0, height: font.1 });
    let (term, program) = (var("TERM"), var("TERM_PROGRAM"));
    let protocol = if term.starts_with("tmux") || term.starts_with("screen") || program == "tmux" {
        ProtocolType::Halfblocks // tmux の中では端末の画像の機能が素通しにならないことが多い
    } else if !var("KITTY_WINDOW_ID").is_empty() || term.contains("kitty") || term.contains("ghostty") || program.contains("ghostty") {
        ProtocolType::Kitty
    } else if ["iTerm", "WezTerm", "vscode", "WarpTerminal", "rio", "Tabby", "Hyper", "mintty"].iter().any(|t| program.contains(t))
        || var("LC_TERMINAL").contains("iTerm")
    {
        ProtocolType::Iterm2
    } else {
        ProtocolType::Halfblocks
    };
    p.set_protocol_type(protocol);
    p
}

/// 画像だけの段落にある画像 (段落の番号, URL)。文の途中の小さな画像は文字のまま出す
pub fn image_srcs(doc: &Document) -> Vec<(usize, String)> {
    doc.blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| b.image_only())
        .flat_map(|(i, b)| b.spans.iter().filter_map(move |s| s.src.clone().map(|u| (i, u))))
        .collect()
}

/// 画像を取ってくる。page を Referer として送る (直リンクを断るサイトがあるので。送り方は referer)。
/// ヘッダーは Firefox (ESR 140) が <img> の画像を取るときと同じにする (プライベートモードで Tor Browser と見分けがつかないように)
pub fn fetch(client: &crate::fetch::Client, url: &str, page: &str) -> Result<DynamicImage, String> {
    let mut req = client
        .get_for(url, page)
        .header("accept", "image/avif,image/webp,image/png,image/svg+xml,image/*;q=0.8,*/*;q=0.5")
        .header("sec-fetch-dest", "image")
        .header("sec-fetch-mode", "no-cors")
        .header("sec-fetch-site", sec_fetch_site(page, url))
        .header("priority", "u=5, i");
    if let Some(r) = referer(page, url) {
        req = req.header("referer", &r);
    }
    let r = req.send().map_err(|e| e.0)?;
    if !r.status().is_success() {
        return Err(format!("HTTP {}", r.status().as_u16()));
    }
    let bytes = r.bytes();
    if bytes.len() > MAX_BYTES {
        return Err(t!("大きすぎる").into());
    }
    let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
    Ok(if img.width() > MAX_PIXELS_WIDE { img.resize(MAX_PIXELS_WIDE, u32::MAX, image::imageops::FilterType::Triangle) } else { img })
}

/// URL の (scheme, ホストとポート)。小文字にし、ユーザー名とパスワードは除く
fn origin(s: &crate::urls::Split) -> (String, String) {
    (s.scheme.clone(), s.netloc.rsplit('@').next().unwrap_or("").to_ascii_lowercase())
}

/// ホストとポートからホストだけ
fn host_only(netloc: &str) -> &str {
    match netloc.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => netloc.split(':').next().unwrap_or(""),
    }
}

/// 画像を取るときに送る Referer。ブラウザの既定 (strict-origin-when-cross-origin) と同じく、同じサイト (オリジン) には
/// ページの URL を、よそのサイトにはオリジン (https://example.com/) だけを送り、https から http へは送らない。
/// よその画像のサーバー (CDN・アクセス解析など) に、どのページを読んでいるかまで知らせないように
fn referer(page: &str, url: &str) -> Option<String> {
    use crate::urls::{Split, urlsplit, urlunsplit};
    let (p, u) = (urlsplit(page, ""), urlsplit(url, ""));
    if !matches!(p.scheme.as_str(), "http" | "https") || (p.scheme == "https" && u.scheme != "https") {
        return None;
    }
    let (scheme, netloc) = origin(&p);
    if (scheme.as_str(), netloc.as_str()) == (u.scheme.as_str(), origin(&u).1.as_str()) {
        return Some(urlunsplit(&Split { netloc, fragment: String::new(), ..p }));
    }
    Some(format!("{scheme}://{netloc}/"))
}

/// sec-fetch-site。同じオリジンなら same-origin、scheme と登録できるドメイン (example.co.jp など。Public Suffix List で決める)
/// が同じなら same-site、ほかは cross-site
fn sec_fetch_site(page: &str, url: &str) -> &'static str {
    let (p, u) = (origin(&crate::urls::urlsplit(page, "")), origin(&crate::urls::urlsplit(url, "")));
    if p == u {
        return "same-origin";
    }
    let site = |h: &str| psl::domain_str(h).unwrap_or(h).to_string();
    let (ph, uh) = (host_only(&p.1), host_only(&u.1));
    if p.0 == u.0 && (ph == uh || site(ph) == site(uh)) { "same-site" } else { "cross-site" }
}

/// urls を WORKERS 個ずつ並行して取ってくる。1 枚取れるたびに done を呼ぶ
pub fn fetch_all(urls: Vec<String>, page: String, done: impl Fn(String, Result<DynamicImage, String>) + Send + Sync + 'static) {
    let queue = Arc::new(std::sync::Mutex::new(urls));
    let done = Arc::new(done);
    for _ in 0..WORKERS {
        let (queue, done, page) = (queue.clone(), done.clone(), page.clone());
        std::thread::spawn(move || {
            let client = crate::fetch::client();
            while let Some(url) = queue.lock().unwrap().pop() {
                let r = fetch(&client, &url, &page);
                done(url, r);
            }
        });
    }
}

/// 画像を描く大きさ (列, 行)。幅は max_cols まで、行は MAX_ROWS まで。font は 1 文字の大きさ (ピクセル)
pub fn size_in_cells(img: &DynamicImage, max_cols: u16, font: (u16, u16)) -> (u16, u16) {
    let (fw, fh) = (font.0.max(1) as f64, font.1.max(1) as f64);
    let (iw, ih) = (img.width().max(1) as f64, img.height().max(1) as f64);
    let mut cols = (iw / fw).ceil().min(max_cols as f64).max(1.0);
    let mut rows = (cols * fw * ih / iw / fh).ceil();
    if rows > MAX_ROWS as f64 {
        rows = MAX_ROWS as f64;
        cols = (rows * fh * iw / ih / fw).floor().max(1.0);
    }
    (cols as u16, rows.max(1.0) as u16)
}

/// 描画結果の行に、画像の場所 (空行) を足す。画像は段落の最後の行の下に置く。足した後の行と、画像の場所を返す
pub fn place(lines: &[Line], doc: &Document, states: &HashMap<String, State>, max_cols: u16, font: (u16, u16)) -> (Vec<Line>, Vec<Place>) {
    let mut by_block: HashMap<usize, Vec<(String, u16, u16)>> = HashMap::new();
    for (blk, src) in image_srcs(doc) {
        if let Some(State::Ready(img)) = states.get(&src) {
            let (cols, rows) = size_in_cells(img, max_cols, font);
            by_block.entry(blk).or_default().push((src, cols, rows));
        }
    }
    if by_block.is_empty() {
        return (lines.to_vec(), vec![]);
    }
    // 段落ごとの最後の行
    let mut last: HashMap<usize, usize> = HashMap::new();
    for (i, l) in lines.iter().enumerate() {
        for b in l.blks() {
            last.insert(b, i);
        }
    }
    let mut after: HashMap<usize, Vec<(String, u16, u16)>> = HashMap::new();
    for (blk, imgs) in by_block {
        if let Some(&i) = last.get(&blk) {
            after.entry(i).or_default().extend(imgs);
        }
    }
    let mut out = Vec::with_capacity(lines.len());
    let mut places = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        out.push(l.clone());
        if let Some(imgs) = after.get(&i) {
            let blk = l.blks().last().copied();
            for (src, cols, rows) in imgs {
                places.push(Place { top: out.len(), rows: *rows, cols: *cols, src: src.clone() });
                // 空行にも段落の番号を付けておく (読み上げの位置合わせなどで、段落の続きとして扱う)
                out.extend((0..*rows).map(|_| Line { segs: vec![Seg { blk, ..Default::default() }] }));
            }
        }
    }
    (out, places)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn referer_is_origin_only_for_other_sites() {
        let page = "https://user:pw@Example.com/a/b?q=1#frag";
        assert_eq!(referer(page, "https://example.com/img.png").as_deref(), Some("https://example.com/a/b?q=1"));
        assert_eq!(referer(page, "https://cdn.example.net/img.png").as_deref(), Some("https://example.com/"));
        assert_eq!(referer(page, "https://example.com:8443/img.png").as_deref(), Some("https://example.com/"));
        assert_eq!(referer(page, "http://example.com/img.png"), None);
        assert_eq!(referer("http://a.onion/p", "http://b.onion/i.png").as_deref(), Some("http://a.onion/"));
        assert_eq!(referer("file:///tmp/a.html", "https://example.com/i.png"), None);
    }

    #[test]
    fn sec_fetch_site_uses_registrable_domain() {
        let page = "https://www.example.co.jp/a";
        assert_eq!(sec_fetch_site(page, "https://www.example.co.jp/i.png"), "same-origin");
        assert_eq!(sec_fetch_site(page, "https://img.example.co.jp/i.png"), "same-site");
        assert_eq!(sec_fetch_site(page, "https://other.co.jp/i.png"), "cross-site");
        assert_eq!(sec_fetch_site(page, "http://img.example.co.jp/i.png"), "cross-site");
        assert_eq!(sec_fetch_site("https://a.github.io/", "https://b.github.io/i.png"), "cross-site");
    }
}
