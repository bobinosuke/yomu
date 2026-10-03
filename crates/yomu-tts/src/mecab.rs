//! コンパイル済みの MeCab 辞書をそのまま読む、MeCab 互換の形態素解析。
//!
//! pyopenjtalk-plus 同梱の MeCab (lib/open_jtalk/src/mecab/src) の 1-best 解析と同じ結果を出す:
//! - 辞書 (sys.dic と、`-u` で渡すユーザー辞書) は Darts のダブル配列で共通接頭辞検索する
//! - 先頭の空白 (char.bin で空白と同じ種類の文字) は読み飛ばし、形態素の表層形に含めない
//! - 未知語は char.bin の invoke / group / length の規則で作る
//! - Viterbi で同じコストの候補があるときは、MeCab の連結リストの順 (後から足したものが先) で先に見たものを選ぶ
use memmap2::Mmap;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

const DICTIONARY_MAGIC_ID: u32 = 0xef71_8f77;
const DIC_VERSION: u32 = 102;
const DEFAULT_MAX_GROUPING_SIZE: usize = 24;
const MAX_KEY_LEN: usize = 65535;

fn map(path: &Path) -> io::Result<Mmap> {
    let f = File::open(path).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
    // SAFETY: 辞書ファイルは読み取り専用で開き、解析中に書き換えられないことを前提にする
    unsafe { Mmap::map(&f) }
}

fn broken(path: &Path) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("dictionary file is broken: {}", path.display()))
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}

fn u16_at(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(b[off..off + 2].try_into().unwrap())
}

#[derive(Clone, Copy)]
struct Token {
    lc: u16,
    rc: u16,
    wcost: i16,
    feature: u32,
}

/// sys.dic / unk.dic / ユーザー辞書 (dictionary.cpp の Dictionary)
struct Dictionary {
    mmap: Mmap,
    typ: u32,
    da: (usize, usize),   // (開始位置, 要素数)。要素は base: i32, check: u32
    token: usize,         // Token (16 バイト) の並びの開始位置
    feature: usize,       // 素性の文字列 (NUL 区切り) の開始位置
}

impl Dictionary {
    fn open(path: &Path) -> io::Result<Self> {
        let mmap = map(path)?;
        let b = &mmap[..];
        if b.len() < 100 {
            return Err(broken(path));
        }
        let magic = u32_at(b, 0);
        if (magic ^ DICTIONARY_MAGIC_ID) as usize != b.len() || u32_at(b, 4) != DIC_VERSION {
            return Err(broken(path));
        }
        let typ = u32_at(b, 8);
        let (dsize, tsize, fsize) = (u32_at(b, 24) as usize, u32_at(b, 28) as usize, u32_at(b, 32) as usize);
        let da = 40 + 32; // ヘッダ (10 個の u32) と文字コード名 (32 バイト) の後
        let token = da + dsize;
        let feature = token + tsize;
        if feature + fsize != b.len() {
            return Err(broken(path));
        }
        Ok(Self { mmap, typ, da: (da, dsize / 8), token, feature })
    }

    fn unit(&self, p: usize) -> Option<(i32, u32)> {
        if p >= self.da.1 {
            return None;
        }
        let off = self.da.0 + p * 8;
        Some((u32_at(&self.mmap, off) as i32, u32_at(&self.mmap, off + 4)))
    }

    /// Darts の commonPrefixSearch。(値, 一致した長さ) を短い順に返す。
    fn common_prefix_search(&self, key: &[u8], out: &mut Vec<(i32, usize)>) {
        out.clear();
        let Some((mut b, _)) = self.unit(0) else { return };
        for (i, &k) in key.iter().enumerate() {
            if let Some((n, check)) = self.unit(b as u32 as usize)
                && b as u32 == check
                && n < 0
            {
                out.push((-n - 1, i));
            }
            match self.unit((b as u32 as usize) + k as usize + 1) {
                Some((base, check)) if b as u32 == check => b = base,
                _ => return,
            }
        }
        if let Some((n, check)) = self.unit(b as u32 as usize)
            && b as u32 == check
            && n < 0
        {
            out.push((-n - 1, key.len()));
        }
    }

    fn exact_match_search(&self, key: &[u8]) -> Option<i32> {
        let (mut b, _) = self.unit(0)?;
        for &k in key {
            match self.unit((b as u32 as usize) + k as usize + 1) {
                Some((base, check)) if b as u32 == check => b = base,
                _ => return None,
            }
        }
        let (n, check) = self.unit(b as u32 as usize)?;
        (b as u32 == check && n < 0).then_some(-n - 1)
    }

    /// 検索結果の値から、その表層形の Token の並び (開始番号, 個数)
    fn tokens(value: i32) -> (usize, usize) {
        ((value >> 8) as usize, (value & 0xff) as usize)
    }

    fn token(&self, i: usize) -> Token {
        let off = self.token + i * 16;
        let b = &self.mmap[..];
        Token { lc: u16_at(b, off), rc: u16_at(b, off + 2), wcost: u16_at(b, off + 6) as i16, feature: u32_at(b, off + 8) }
    }

    fn feature(&self, t: Token) -> &str {
        let s = &self.mmap[self.feature + t.feature as usize..];
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        std::str::from_utf8(&s[..end]).unwrap_or("")
    }
}

/// char.bin の文字の種類 (char_property.h の CharInfo。ビットフィールドを 1 つの u32 に詰めたもの)
#[derive(Clone, Copy, Default)]
struct CharInfo(u32);

impl CharInfo {
    fn typ(self) -> u32 {
        self.0 & 0x3ffff
    }
    fn default_type(self) -> u32 {
        (self.0 >> 18) & 0xff
    }
    fn length(self) -> usize {
        ((self.0 >> 26) & 0xf) as usize
    }
    fn group(self) -> bool {
        (self.0 >> 30) & 1 == 1
    }
    fn invoke(self) -> bool {
        (self.0 >> 31) & 1 == 1
    }
    fn is_kind_of(self, c: CharInfo) -> bool {
        self.typ() & c.typ() != 0
    }
}

struct CharProperty {
    mmap: Mmap,
    names: Vec<String>,
    map: usize,
}

impl CharProperty {
    fn open(path: &Path) -> io::Result<Self> {
        let mmap = map(path)?;
        let csize = u32_at(&mmap, 0) as usize;
        if 4 + 32 * csize + 4 * 0xffff != mmap.len() {
            return Err(broken(path));
        }
        let names = (0..csize)
            .map(|i| {
                let s = &mmap[4 + 32 * i..4 + 32 * (i + 1)];
                let end = s.iter().position(|&c| c == 0).unwrap_or(32);
                String::from_utf8_lossy(&s[..end]).into_owned()
            })
            .collect();
        Ok(Self { names, map: 4 + 32 * csize, mmap })
    }

    fn info(&self, ucs: u16) -> CharInfo {
        // map は 0xffff 個 (U+FFFF は範囲外。MeCab も同じく範囲外を読むが、実際には現れない)
        let i = (ucs as usize).min(0xfffe);
        CharInfo(u32_at(&self.mmap, self.map + 4 * i))
    }

    /// ucs.h の utf8_to_ucs2 と同じ。BMP の外の文字は 0 になる。s は空でないこと。
    fn char_info(&self, s: &[u8]) -> (CharInfo, usize) {
        let len = s.len();
        let b0 = s[0];
        let (t, mblen) = if b0 < 0x80 {
            (b0 as u16, 1)
        } else if len >= 2 && b0 & 0xe0 == 0xc0 {
            ((((b0 & 0x1f) as u16) << 6) | (s[1] & 0x3f) as u16, 2)
        } else if len >= 3 && b0 & 0xf0 == 0xe0 {
            ((((b0 & 0x0f) as u16) << 12) | (((s[1] & 0x3f) as u16) << 6) | (s[2] & 0x3f) as u16, 3)
        } else if len >= 4 && b0 & 0xf8 == 0xf0 {
            (0, 4)
        } else if len >= 5 && b0 & 0xfc == 0xf8 {
            (0, 5)
        } else if len >= 6 && b0 & 0xfe == 0xfc {
            (0, 6)
        } else {
            (0, 1)
        };
        (self.info(t), mblen)
    }

    /// seekToOtherType: begin から c と同じ種類の文字が続く間進める。
    /// (止まった位置, 最後に調べた文字の種類, その長さ, 進んだ文字数)。fail / mblen は呼び出し前の値を引き継ぐ。
    fn seek_to_other_type(
        &self,
        s: &[u8],
        begin: usize,
        end: usize,
        mut c: CharInfo,
        fail: &mut CharInfo,
        mblen: &mut usize,
    ) -> (usize, usize) {
        let mut p = begin;
        let mut clen = 0;
        while p != end {
            let (info, l) = self.char_info(&s[p..end]);
            *fail = info;
            *mblen = l;
            if !c.is_kind_of(info) {
                break;
            }
            p += l;
            clen += 1;
            c = info;
        }
        (p, clen)
    }

    /// getCharInfo(begin3, end)。begin3 == end のときは MeCab は文字列の終端の NUL を読むので、それに合わせる。
    fn char_info_at(&self, s: &[u8], pos: usize, end: usize) -> (CharInfo, usize) {
        if pos >= end { (self.info(0), 1) } else { self.char_info(&s[pos..end]) }
    }
}

/// 連接コスト表 (matrix.bin)
struct Connector {
    mmap: Mmap,
    lsize: usize,
}

impl Connector {
    fn open(path: &Path) -> io::Result<Self> {
        let mmap = map(path)?;
        if mmap.len() < 4 {
            return Err(broken(path));
        }
        let (lsize, rsize) = (u16_at(&mmap, 0) as usize, u16_at(&mmap, 2) as usize);
        if 2 * (lsize * rsize + 2) != mmap.len() {
            return Err(broken(path));
        }
        Ok(Self { mmap, lsize })
    }

    fn cost(&self, rc: u16, lc: u16) -> i32 {
        let i = rc as usize + self.lsize * lc as usize;
        u16_at(&self.mmap, 4 + 2 * i) as i16 as i32
    }
}

#[derive(Clone, Copy)]
struct Node {
    begin: usize,   // 表層形の開始位置 (先頭の空白を除く)
    length: usize,  // 表層形のバイト数
    rlength: usize, // 先頭の空白を含むバイト数
    lc: u16,
    rc: u16,
    wcost: i16,
    dict: usize, // 素性を引く辞書 (dicts の番号。usize::MAX は unk.dic)
    token: Token,
    is_unk: bool,
    char_type: u32,
    cost: i64,
    prev: usize,
}

/// 1 つの形態素。BOS / EOS は含めない。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Morph {
    pub surface: String,
    pub feature: String,
    pub is_unk: bool,
    /// MeCab の node->char_type (表層形の最初の文字の char.def の種類の番号)
    pub char_type: u32,
    /// 引いた辞書 (0 がシステム辞書、1.. が渡した順のユーザー辞書)。未知語は None
    pub dict: Option<usize>,
    /// 解析した文字列の中の表層形の位置 (バイト)
    pub begin: usize,
}

impl Morph {
    /// OpenJTalk の Mecab_analysis が返す形 ("表層形,素性")
    pub fn openjtalk_line(&self) -> String {
        format!("{},{}", self.surface, self.feature)
    }
}

/// 素性の文字列を列に分ける。MeCab の tokenizeCSV と同じく、"..." で囲まれた列の中のカンマは区切りにしない
/// ("" は " 1 文字)。
pub fn csv_fields(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    loop {
        let mut field = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            while let Some(c) = chars.next() {
                if c == '"' {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        field.push('"');
                    } else {
                        break;
                    }
                } else {
                    field.push(c);
                }
            }
            // 閉じ引用符の後は次のカンマまで読み飛ばす
            for c in chars.by_ref() {
                if c == ',' {
                    out.push(field);
                    field = String::new();
                    break;
                }
            }
            if chars.peek().is_none() && !field.is_empty() {
                out.push(field);
                return out;
            }
            continue;
        }
        let mut ended = true;
        for c in chars.by_ref() {
            if c == ',' {
                ended = false;
                break;
            }
            field.push(c);
        }
        out.push(field);
        if ended {
            return out;
        }
    }
}

pub struct Tagger {
    dicts: Vec<Dictionary>, // 0 がシステム辞書、1.. がユーザー辞書 (渡した順)
    unk: Dictionary,
    unk_tokens: Vec<(usize, usize)>, // 文字の種類ごとの unk.dic の Token (開始番号, 個数)
    property: CharProperty,
    connector: Connector,
    space: CharInfo,
    max_grouping_size: usize,
}

impl Tagger {
    /// dicdir の sys.dic / unk.dic / char.bin / matrix.bin と、ユーザー辞書 (この順に引く) を開く。
    pub fn open(dicdir: &Path, userdics: &[PathBuf]) -> io::Result<Self> {
        let unk = Dictionary::open(&dicdir.join("unk.dic"))?;
        let property = CharProperty::open(&dicdir.join("char.bin"))?;
        let sys = Dictionary::open(&dicdir.join("sys.dic"))?;
        if sys.typ != 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("not a system dictionary: {}", dicdir.display())));
        }
        let mut dicts = vec![sys];
        for p in userdics {
            let d = Dictionary::open(p)?;
            if d.typ != 1 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("not a user dictionary: {}", p.display())));
            }
            dicts.push(d);
        }
        let unk_tokens = property
            .names
            .iter()
            .map(|name| {
                unk.exact_match_search(name.as_bytes()).map(Dictionary::tokens).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, format!("cannot find UNK category: {name}"))
                })
            })
            .collect::<io::Result<_>>()?;
        let connector = Connector::open(&dicdir.join("matrix.bin"))?;
        let space = property.info(0x20);
        Ok(Self { dicts, unk, unk_tokens, property, connector, space, max_grouping_size: DEFAULT_MAX_GROUPING_SIZE })
    }

    fn dict(&self, i: usize) -> &Dictionary {
        if i == usize::MAX { &self.unk } else { &self.dicts[i] }
    }

    /// tokenizer.cpp の lookup。pos から始まる候補を、MeCab の bnext の順 (後から足したものが先) で返す。
    fn lookup(&self, s: &[u8], pos: usize, results: &mut Vec<(i32, usize)>) -> Vec<Node> {
        let end = s.len().min(pos + MAX_KEY_LEN);
        let mut cinfo = CharInfo::default();
        let mut mblen = 0;
        let (begin2, _) = self.property.seek_to_other_type(s, pos, end, self.space, &mut cinfo, &mut mblen);
        let mut added: Vec<Node> = Vec::new();
        let node = |dict: usize, token: Token, surface_end: usize, is_unk: bool, char_type: u32| Node {
            begin: begin2,
            length: surface_end - begin2,
            rlength: surface_end - pos,
            lc: token.lc,
            rc: token.rc,
            wcost: token.wcost,
            dict,
            token,
            is_unk,
            char_type,
            cost: 0,
            prev: 0,
        };
        for (di, d) in self.dicts.iter().enumerate() {
            d.common_prefix_search(&s[begin2..end], results);
            for &(value, len) in results.iter() {
                let (start, n) = Dictionary::tokens(value);
                for j in 0..n {
                    added.push(node(di, d.token(start + j), begin2 + len, false, cinfo.default_type()));
                }
            }
        }
        if !added.is_empty() && !cinfo.invoke() {
            added.reverse();
            return added;
        }

        let add_unknown = |added: &mut Vec<Node>, begin3: usize| {
            let (start, n) = self.unk_tokens[cinfo.default_type() as usize];
            // 空白で終わる文では begin3 が文末を越える。MeCab と同じくそのまま作る (文末を越えて終わる形態素は
            // EOS につながらないので、最良経路には入らない)
            for k in 0..n {
                added.push(node(usize::MAX, self.unk.token(start + k), begin3, true, cinfo.default_type()));
            }
        };

        let mut begin3 = begin2 + mblen;
        let mut group_begin3 = usize::MAX;
        if begin3 > end {
            add_unknown(&mut added, begin3);
            if !added.is_empty() {
                added.reverse();
                return added;
            }
        }
        if cinfo.group() {
            let tmp = begin3;
            let mut fail = CharInfo::default();
            let (b, clen) = self.property.seek_to_other_type(s, begin3, end, cinfo, &mut fail, &mut mblen);
            if clen <= self.max_grouping_size {
                add_unknown(&mut added, b);
            }
            group_begin3 = b;
            begin3 = tmp;
        }
        for _ in 1..=cinfo.length() {
            if begin3 > end {
                break;
            }
            if begin3 == group_begin3 {
                continue;
            }
            add_unknown(&mut added, begin3);
            let (info, l) = self.property.char_info_at(s, begin3, end);
            if !cinfo.is_kind_of(info) {
                break;
            }
            begin3 += l;
        }
        if added.is_empty() {
            add_unknown(&mut added, begin3);
        }
        added.reverse();
        added
    }

    pub fn parse(&self, text: &str) -> Vec<Morph> {
        let s = text.as_bytes();
        let len = s.len();
        let bos = Node {
            begin: 0,
            length: 0,
            rlength: 0,
            lc: 0,
            rc: 0,
            wcost: 0,
            dict: 0,
            token: Token { lc: 0, rc: 0, wcost: 0, feature: 0 },
            is_unk: false,
            char_type: 0,
            cost: 0,
            prev: usize::MAX,
        };
        let mut nodes = vec![bos];
        // end_nodes[x]: x で終わる形態素。MeCab は連結リストの先頭に足すので、後ろから見る
        let mut end_nodes: Vec<Vec<usize>> = vec![Vec::new(); len + 8];
        end_nodes[0].push(0);
        let mut results = Vec::new();

        let connect = |nodes: &mut Vec<Node>, end_nodes: &mut Vec<Vec<usize>>, pos: usize, mut r: Node| -> usize {
            let mut best: Option<(i64, usize)> = None;
            for &l in end_nodes[pos].iter().rev() {
                let ln = &nodes[l];
                let cost = ln.cost + (self.connector.cost(ln.rc, r.lc) + r.wcost as i32) as i64;
                if best.is_none_or(|(c, _)| cost < c) {
                    best = Some((cost, l));
                }
            }
            let (cost, prev) = best.expect("end_nodes[pos] は空でない");
            r.cost = cost;
            r.prev = prev;
            let id = nodes.len();
            let x = (pos + r.rlength).min(end_nodes.len() - 1);
            nodes.push(r);
            end_nodes[x].push(id);
            id
        };

        for pos in 0..len {
            if end_nodes[pos].is_empty() {
                continue;
            }
            for r in self.lookup(s, pos, &mut results) {
                connect(&mut nodes, &mut end_nodes, pos, r);
            }
        }
        let Some(pos) = (0..=len).rev().find(|&p| !end_nodes[p].is_empty()) else { return Vec::new() };
        let eos = Node { begin: len, prev: usize::MAX, ..nodes[0] };
        let eos_id = connect(&mut nodes, &mut end_nodes, pos, eos);
        let mut i = nodes[eos_id].prev;

        let mut out = Vec::new();
        while i != 0 {
            let n = &nodes[i];
            let end = (n.begin + n.length).min(len);
            out.push(Morph {
                surface: String::from_utf8_lossy(&s[n.begin..end]).into_owned(),
                feature: self.dict(n.dict).feature(n.token).to_owned(),
                is_unk: n.is_unk,
                char_type: n.char_type,
                dict: (n.dict != usize::MAX).then_some(n.dict),
                begin: n.begin,
            });
            i = n.prev;
        }
        out.reverse();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::csv_fields;

    #[test]
    fn csv() {
        assert_eq!(csv_fields("a,b,,c"), ["a", "b", "", "c"]);
        assert_eq!(csv_fields("\"x,y\",z"), ["x,y", "z"]);
        assert_eq!(csv_fields("a,\"q\"\"\""), ["a", "q\""]);
        assert_eq!(csv_fields(""), [""]);
    }
}
