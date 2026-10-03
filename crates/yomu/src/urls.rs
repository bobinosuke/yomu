//! 入力された文字列や URL の扱い。TUI と --dump の両方から使う。
//! URL の分解・組み立て・結合・エンコードは Python 3.12 の urllib.parse と同じ結果にする。
//! url クレート (WHATWG) は日本語の URL を %エンコードしたり、末尾に / を足したりして Python 版と結果が変わるので使わない。
use std::sync::LazyLock;

use regex::Regex;

static SCHEME_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^[a-z][a-z0-9+.-]*://").unwrap());

pub fn looks_like_url(s: &str) -> bool {
    if SCHEME_URL.is_match(s) {
        return true;
    }
    !s.contains(' ') && s.trim_matches('.').contains('.') && s.is_ascii()
}

/// スキームを省いた URL (example.com) に https:// を補う
pub fn to_url(s: &str) -> String {
    if s.contains("://") { s.to_string() } else { format!("https://{s}") }
}

/// 1 階層上 (root なら サイトのトップ) の URL
pub fn parent_url(url: &str, root: bool) -> String {
    let u = urlsplit(url, "");
    let path = if root {
        "/".to_string()
    } else {
        let p = u.path.trim_end_matches('/');
        match p.rsplit_once('/') {
            Some((head, _)) => format!("{head}/"),
            None => "/".to_string(),
        }
    };
    urlunsplit(&Split { path, query: String::new(), fragment: String::new(), ..u })
}

const USES_RELATIVE: &[&str] = &[
    "", "ftp", "http", "gopher", "nntp", "imap", "wais", "file", "https", "shttp", "mms", "prospero", "rtsp", "rtsps",
    "rtspu", "sftp", "svn", "svn+ssh", "ws", "wss",
];
const USES_NETLOC: &[&str] = &[
    "", "ftp", "http", "gopher", "nntp", "telnet", "imap", "wais", "file", "mms", "https", "shttp", "snews", "prospero",
    "rtsp", "rtsps", "rtspu", "rsync", "svn", "svn+ssh", "sftp", "nfs", "git", "git+ssh", "ws", "wss", "itms-services",
];
const USES_PARAMS: &[&str] =
    &["", "ftp", "hdl", "prospero", "http", "imap", "https", "shttp", "rtsp", "rtsps", "rtspu", "sip", "sips", "mms", "sftp", "tel"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Split {
    pub scheme: String,
    pub netloc: String,
    pub path: String,
    pub query: String,
    pub fragment: String,
}

pub fn urlsplit(url: &str, default_scheme: &str) -> Split {
    let is_c0 = |c: char| c <= ' ';
    let mut url: String = url.trim_start_matches(is_c0).chars().filter(|c| !matches!(c, '\t' | '\r' | '\n')).collect();
    let mut scheme: String = default_scheme.trim_matches(is_c0).chars().filter(|c| !matches!(c, '\t' | '\r' | '\n')).collect();
    let (mut netloc, mut query, mut fragment) = (String::new(), String::new(), String::new());
    if let Some(i) = url.find(':')
        && i > 0
        && url.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && url[..i].chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    {
        scheme = url[..i].to_ascii_lowercase();
        url = url[i + 1..].to_string();
    }
    if url.starts_with("//") {
        let rest = &url[2..];
        let delim = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        netloc = rest[..delim].to_string();
        url = rest[delim..].to_string();
    }
    if let Some((a, b)) = url.split_once('#') {
        fragment = b.to_string();
        url = a.to_string();
    }
    if let Some((a, b)) = url.split_once('?') {
        query = b.to_string();
        url = a.to_string();
    }
    Split { scheme, netloc, path: url, query, fragment }
}

pub fn urlunsplit(s: &Split) -> String {
    let mut url = s.path.clone();
    if !s.netloc.is_empty() {
        if !url.is_empty() && !url.starts_with('/') {
            url = format!("/{url}");
        }
        url = format!("//{}{url}", s.netloc);
    } else if url.starts_with("//")
        || (!s.scheme.is_empty() && USES_NETLOC.contains(&s.scheme.as_str()) && (url.is_empty() || url.starts_with('/')))
    {
        url = format!("//{url}");
    }
    if !s.scheme.is_empty() {
        url = format!("{}:{url}", s.scheme);
    }
    if !s.query.is_empty() {
        url = format!("{url}?{}", s.query);
    }
    if !s.fragment.is_empty() {
        url = format!("{url}#{}", s.fragment);
    }
    url
}

/// urlparse: urlsplit に加えて、最後の区切りの後の ;params を分ける
fn urlparse(url: &str, scheme: &str) -> (Split, String) {
    let mut s = urlsplit(url, scheme);
    let mut params = String::new();
    if USES_PARAMS.contains(&s.scheme.as_str()) && s.path.contains(';') {
        let i = match s.path.rfind('/') {
            Some(slash) => s.path[slash..].find(';').map(|j| slash + j),
            None => s.path.find(';'),
        };
        if let Some(i) = i {
            params = s.path[i + 1..].to_string();
            s.path.truncate(i);
        }
    }
    (s, params)
}

fn urlunparse(s: &Split, params: &str) -> String {
    let mut s = s.clone();
    if !params.is_empty() {
        s.path = format!("{};{params}", s.path);
    }
    urlunsplit(&s)
}

pub fn urljoin(base: &str, url: &str) -> String {
    if base.is_empty() {
        return url.to_string();
    }
    if url.is_empty() {
        return base.to_string();
    }
    let (b, bparams) = urlparse(base, "");
    let (mut u, mut params) = urlparse(url, &b.scheme);
    if u.scheme != b.scheme || !USES_RELATIVE.contains(&u.scheme.as_str()) {
        return url.to_string();
    }
    if USES_NETLOC.contains(&u.scheme.as_str()) {
        if !u.netloc.is_empty() {
            return urlunparse(&u, &params);
        }
        u.netloc = b.netloc.clone();
    }
    if u.path.is_empty() && params.is_empty() {
        u.path = b.path.clone();
        params = bparams;
        if u.query.is_empty() {
            u.query = b.query.clone();
        }
        return urlunparse(&u, &params);
    }
    let mut base_parts: Vec<&str> = b.path.split('/').collect();
    if base_parts.last().is_some_and(|p| !p.is_empty()) {
        base_parts.pop();
    }
    let segments: Vec<&str> = if u.path.starts_with('/') {
        u.path.split('/').collect()
    } else {
        let mut segs: Vec<&str> = base_parts;
        segs.extend(u.path.split('/'));
        // 先頭と末尾以外の空の要素を除く (つなぎ直したときに / が重ならないように)
        if segs.len() > 2 {
            let last = segs.len() - 1;
            let mid: Vec<&str> = segs[1..last].iter().copied().filter(|s| !s.is_empty()).collect();
            let mut v = vec![segs[0]];
            v.extend(mid);
            v.push(segs[last]);
            segs = v;
        }
        segs
    };
    let mut resolved: Vec<&str> = Vec::new();
    for seg in &segments {
        match *seg {
            ".." => {
                resolved.pop();
            }
            "." => {}
            s => resolved.push(s),
        }
    }
    if matches!(segments.last(), Some(&".") | Some(&"..")) {
        resolved.push("");
    }
    let joined = resolved.join("/");
    u.path = if joined.is_empty() { "/".to_string() } else { joined };
    urlunparse(&u, &params)
}

/// (# より前, # より後)
pub fn urldefrag(url: &str) -> (String, String) {
    if url.contains('#') {
        let (mut s, params) = urlparse(url, "");
        let frag = std::mem::take(&mut s.fragment);
        (urlunparse(&s, &params), frag)
    } else {
        (url.to_string(), String::new())
    }
}

/// %XX をバイトに戻す (UTF-8 として読むとは限らないもの。Content-Disposition の RFC 2231 など)
pub fn percent_bytes(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = ((b[i + 1] as char).to_digit(16), (b[i + 2] as char).to_digit(16))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// urllib.parse.unquote (UTF-8。壊れたバイト列は U+FFFD にする)
pub fn unquote(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    String::from_utf8_lossy(&percent_bytes(s)).into_owned()
}

/// urllib.parse.quote_plus (safe は英数字と "_.-~")
pub fn quote_plus(s: &str) -> String {
    let mut out = String::new();
    for &b in s.as_bytes() {
        match b {
            b' ' => out.push('+'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// urllib.parse.urlencode (値は quote_plus)
pub fn urlencode(fields: &[(String, String)]) -> String {
    fields.iter().map(|(k, v)| format!("{}={}", quote_plus(k), quote_plus(v))).collect::<Vec<_>>().join("&")
}

/// urllib.parse.parse_qs(query, keep_blank_values) の各キーの最初の値を、キーが最初に出た順に
pub fn parse_qs_first(query: &str, keep_blank: bool) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for nv in query.split('&') {
        if nv.is_empty() {
            continue;
        }
        let (name, value) = match nv.split_once('=') {
            Some((n, v)) => (n, v),
            None if keep_blank => (nv, ""),
            None => continue,
        };
        if value.is_empty() && !keep_blank {
            continue;
        }
        let name = unquote(&name.replace('+', " "));
        let value = unquote(&value.replace('+', " "));
        if !out.iter().any(|(k, _)| *k == name) {
            out.push((name, value));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_join() {
        let u = urlsplit("HTTPS://example.com/a/b?x=1#f", "");
        assert_eq!((u.scheme.as_str(), u.netloc.as_str(), u.path.as_str()), ("https", "example.com", "/a/b"));
        assert_eq!((u.query.as_str(), u.fragment.as_str()), ("x=1", "f"));
        assert_eq!(urlunsplit(&u), "https://example.com/a/b?x=1#f");
        assert_eq!(urldefrag("https://e.com/p#x").0, "https://e.com/p");
    }

    #[test]
    fn joins_like_python() {
        let base = "https://example.com/a/b/c.html?x=1#f";
        assert_eq!(urljoin(base, "d.html"), "https://example.com/a/b/d.html");
        assert_eq!(urljoin(base, "../d"), "https://example.com/a/d");
        assert_eq!(urljoin(base, "/日本語"), "https://example.com/日本語");
        assert_eq!(urljoin(base, "?y=2"), "https://example.com/a/b/c.html?y=2");
        assert_eq!(urljoin(base, "#top"), "https://example.com/a/b/c.html?x=1#top");
        assert_eq!(urljoin(base, "//other.org"), "https://other.org");
        assert_eq!(urljoin("https://example.com", "a"), "https://example.com/a");
        assert_eq!(urljoin(base, "."), "https://example.com/a/b/");
    }

    #[test]
    fn encode_like_python() {
        assert_eq!(quote_plus("将棋 a~*"), "%E5%B0%86%E6%A3%8B+a~%2A");
        assert_eq!(unquote("%E5%B0%86%zz"), "将%zz");
        let q = parse_qs_first("q=%E5%B0%86+x&_seen=&q=2", true);
        assert_eq!(q, [("q".into(), "将 x".into()), ("_seen".into(), String::new())]);
    }
}
