//! Python の文字列操作と同じ結果を出す関数。Python 版 (yomu-python) と同じ結果にするために使う。

/// str.isspace と同じ空白 (Rust の is_whitespace に加えて \x1c-\x1f)
pub fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

/// str.strip()
pub fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// 先頭の空白の文字数 (文字の列で)
pub fn lstrip_len(t: &[char]) -> usize {
    t.iter().take_while(|&&c| is_space(c)).count()
}

/// str.rstrip() (文字の列で)
pub fn rstrip(t: &[char]) -> &[char] {
    let mut end = t.len();
    while end > 0 && is_space(t[end - 1]) {
        end -= 1;
    }
    &t[..end]
}

/// str.splitlines()
pub fn splitlines(s: &str) -> Vec<&str> {
    let is_break = |c: char| matches!(c, '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}');
    let mut out = Vec::new();
    let mut start = 0;
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if is_break(c) {
            out.push(&s[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r' && it.peek().is_some_and(|&(_, n)| n == '\n') {
                it.next();
                end += 1;
            }
            start = end;
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_python() {
        assert_eq!(strip("\u{1c} a b \n"), "a b");
        assert_eq!(splitlines("a\r\nb\rc\u{2028}d\n"), ["a", "b", "c", "d"]);
        let t: Vec<char> = "  ab ".chars().collect();
        assert_eq!((lstrip_len(&t), rstrip(&t).len()), (2, 4));
    }
}
