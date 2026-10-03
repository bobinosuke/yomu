//! OpenJTalk の NJD (読みの規則)。pyopenjtalk-plus v0.4.1-post9 の `OpenJTalk._run_njd_from_mecab` と同じ読みを出す。
//!
//! 流れ: mecab2njd → njd_set_pronunciation → apply_original_rule_before_chaining (pyopenjtalk-plus の Python 側の規則)
//! → njd_set_digit。
//! 読み上げのモデル (Supertonic) はカナの文だけを受け取るので、その後のアクセント句・アクセント型・無声化
//! (njd_set_accent_phrase / njd_set_accent_type / njd_set_unvoiced_vowel) は省く。これらは読み (pron) を変えない
//! (無声化は ’ の印を付けるだけ)。アクセントが要る TTS に戻すときは、コミット d3e3023 の njd/accent.rs と
//! njd/unvoiced.rs を戻せば Python 版と同じ結果になる。njd_set_long_vowel は改造版では何もしないので省く。
//! acc・mora_size・chain_* の欄は、読みの規則が参照・更新するので残している。
//! C の実装 (lib/open_jtalk/src の改造版) を、双方向リストを配列の添字で表して、そのまま移植している。
//! 文字列の比較や切り出しは C と同じくバイト単位で行う。
mod digit;
mod original_rule;
mod pronunciation;
mod rules;

#[derive(Clone, Debug, PartialEq)]
pub struct NjdFeature {
    pub string: String,
    pub pos: String,
    pub pos_group1: String,
    pub pos_group2: String,
    pub pos_group3: String,
    pub ctype: String,
    pub cform: String,
    pub orig: String,
    pub read: String,
    pub pron: String,
    pub acc: i32,
    pub mora_size: i32,
    pub chain_rule: String,
    pub chain_flag: i32,
}

/// OpenJTalk の MeCab 出力の行 ("表層形,品詞,...。"記号,空白" は除外済み) から NJD の結果を作る。
pub fn run_njd(mecab_lines: &[String]) -> Vec<NjdFeature> {
    if mecab_lines.is_empty() {
        return Vec::new();
    }
    let mut njd = Njd::default();
    for line in mecab_lines {
        let n = njd.new_node();
        njd.load(n, line);
        njd.push(n);
    }
    pronunciation::set_pronunciation(&mut njd);
    // pyopenjtalk-plus は一度 Python の dict に移して規則を当て、NJD を作り直す。
    // その往復で、値のない欄 (NULL) は "*" という文字列になる
    let mut features = njd.features();
    original_rule::apply(&mut features);
    let mut njd = Njd::from_features(&features);
    digit::set_digit(&mut njd);
    njd.features()
}

pub(crate) type Link = Option<usize>;

#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub string: Option<String>,
    pub pos: Option<String>,
    pub pos_group1: Option<String>,
    pub pos_group2: Option<String>,
    pub pos_group3: Option<String>,
    pub ctype: Option<String>,
    pub cform: Option<String>,
    pub orig: Option<String>,
    pub read: Option<String>,
    pub pron: Option<String>,
    pub acc: i32,
    pub mora_size: i32,
    pub chain_rule: Option<String>,
    pub chain_flag: i32,
    pub prev: Link,
    pub next: Link,
}

impl Default for Node {
    /// NJDNode_initialize
    fn default() -> Self {
        Node {
            string: None,
            pos: None,
            pos_group1: None,
            pos_group2: None,
            pos_group3: None,
            ctype: None,
            cform: None,
            orig: None,
            read: None,
            pron: None,
            acc: 0,
            mora_size: 0,
            chain_rule: None,
            chain_flag: -1,
            prev: None,
            next: None,
        }
    }
}

/// NJDNode_set_*: 空文字列は NULL にする
pub(crate) fn val(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

/// NJDNode_get_*: NULL は "*"
pub(crate) fn get(s: &Option<String>) -> &str {
    s.as_deref().unwrap_or("*")
}

/// NJDNode_add_*
pub(crate) fn add(field: &mut Option<String>, s: &str) {
    match field {
        Some(v) => v.push_str(s),
        None => *field = Some(s.to_string()),
    }
}

impl Node {
    pub fn set_acc(&mut self, acc: i32) {
        self.acc = acc.max(0);
    }
    pub fn set_mora_size(&mut self, size: i32) {
        self.mora_size = size.max(0);
    }
    pub fn add_mora_size(&mut self, size: i32) {
        self.set_mora_size(self.mora_size + size);
    }
    pub fn string(&self) -> &str {
        get(&self.string)
    }
    pub fn pos(&self) -> &str {
        get(&self.pos)
    }
    pub fn pos_group1(&self) -> &str {
        get(&self.pos_group1)
    }
    pub fn pos_group2(&self) -> &str {
        get(&self.pos_group2)
    }
    pub fn pos_group3(&self) -> &str {
        get(&self.pos_group3)
    }
    pub fn cform(&self) -> &str {
        get(&self.cform)
    }
    pub fn orig(&self) -> &str {
        get(&self.orig)
    }
    pub fn read(&self) -> &str {
        get(&self.read)
    }
    pub fn pron(&self) -> &str {
        get(&self.pron)
    }
    pub fn chain_rule(&self) -> &str {
        get(&self.chain_rule)
    }
    /// NJDNode_copy (prev / next 以外)
    fn copy_from(&mut self, o: &Node) {
        let (prev, next) = (self.prev, self.next);
        *self = o.clone();
        self.set_acc(o.acc);
        self.set_mora_size(o.mora_size);
        self.prev = prev;
        self.next = next;
    }
    fn feature(&self) -> NjdFeature {
        NjdFeature {
            string: self.string().into(),
            pos: self.pos().into(),
            pos_group1: self.pos_group1().into(),
            pos_group2: self.pos_group2().into(),
            pos_group3: self.pos_group3().into(),
            ctype: get(&self.ctype).into(),
            cform: self.cform().into(),
            orig: self.orig().into(),
            read: self.read().into(),
            pron: self.pron().into(),
            acc: self.acc,
            mora_size: self.mora_size,
            chain_rule: self.chain_rule().into(),
            chain_flag: self.chain_flag,
        }
    }
}

/// C の atoi
pub(crate) fn atoi(s: &str) -> i32 {
    let s = s.trim_start();
    let (neg, rest) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut n: i64 = 0;
    for b in rest.bytes() {
        if !b.is_ascii_digit() {
            break;
        }
        n = (n * 10 + (b - b'0') as i64).min(i32::MAX as i64 + 1);
    }
    (if neg { -n } else { n }) as i32
}

/// strtopcmp: str が pattern で始まればその長さ、そうでなければ -1
pub(crate) fn strtopcmp(s: &[u8], pattern: &str) -> i32 {
    if s.starts_with(pattern.as_bytes()) { pattern.len() as i32 } else { -1 }
}

/// njd_node.c の get_token_from_string: d までを切り出し、index を d の次へ進める
fn token<'a>(s: &'a str, index: &mut usize, d: u8) -> &'a str {
    let b = s.as_bytes();
    let start = *index;
    while *index < b.len() && b[*index] != d {
        *index += 1;
    }
    let t = &s[start..*index];
    if *index < b.len() {
        *index += 1;
    }
    t
}

/// 双方向リスト。ノードは消しても配列には残し、つながりだけ外す。
#[derive(Default)]
pub(crate) struct Njd {
    pub nodes: Vec<Node>,
    pub head: Link,
    pub tail: Link,
}

impl Njd {
    pub fn new_node(&mut self) -> usize {
        self.nodes.push(Node::default());
        self.nodes.len() - 1
    }

    pub fn from_features(features: &[NjdFeature]) -> Self {
        let mut njd = Njd::default();
        for f in features {
            let n = njd.new_node();
            let node = &mut njd.nodes[n];
            node.string = val(&f.string);
            node.pos = val(&f.pos);
            node.pos_group1 = val(&f.pos_group1);
            node.pos_group2 = val(&f.pos_group2);
            node.pos_group3 = val(&f.pos_group3);
            node.ctype = val(&f.ctype);
            node.cform = val(&f.cform);
            node.orig = val(&f.orig);
            node.read = val(&f.read);
            node.pron = val(&f.pron);
            node.set_acc(f.acc);
            node.set_mora_size(f.mora_size);
            node.chain_rule = val(&f.chain_rule);
            node.chain_flag = f.chain_flag;
            njd.push(n);
        }
        njd
    }

    pub fn features(&self) -> Vec<NjdFeature> {
        let mut out = Vec::new();
        let mut n = self.head;
        while let Some(i) = n {
            out.push(self.nodes[i].feature());
            n = self.nodes[i].next;
        }
        out
    }

    /// NJD_push_node
    pub fn push(&mut self, mut n: usize) {
        match self.tail {
            Some(t) if self.head.is_some() => {
                self.nodes[t].next = Some(n);
                self.nodes[n].prev = Some(t);
            }
            _ => self.head = Some(n),
        }
        while let Some(x) = self.nodes[n].next {
            n = x;
        }
        self.tail = Some(n);
    }

    /// NJD_remove_node: 消して次のノードを返す
    pub fn remove(&mut self, n: usize) -> Link {
        let (prev, next) = (self.nodes[n].prev, self.nodes[n].next);
        let ret = if Some(n) == self.head && Some(n) == self.tail {
            self.head = None;
            self.tail = None;
            None
        } else if Some(n) == self.head {
            self.head = next;
            self.nodes[next.unwrap()].prev = None;
            self.head
        } else if Some(n) == self.tail {
            self.tail = prev;
            self.nodes[prev.unwrap()].next = None;
            None
        } else {
            self.nodes[prev.unwrap()].next = next;
            self.nodes[next.unwrap()].prev = prev;
            next
        };
        self.nodes[n] = Node::default();
        ret
    }

    /// NJD_remove_silent_node: 読みのないノードを消す
    pub fn remove_silent(&mut self) {
        let mut n = self.head;
        while let Some(i) = n {
            n = if self.nodes[i].pron() == "*" { self.remove(i) } else { self.nodes[i].next };
        }
    }

    /// NJDNode_insert: prev と next の間に node (とそれに続くノード) を入れ、その末尾を返す
    pub fn insert(&mut self, prev: usize, next: Link, node: usize) -> usize {
        let mut tail = node;
        while let Some(x) = self.nodes[tail].next {
            tail = x;
        }
        self.nodes[prev].next = Some(node);
        self.nodes[node].prev = Some(prev);
        if let Some(nx) = next {
            self.nodes[nx].prev = Some(tail);
        }
        self.nodes[tail].next = next;
        tail
    }

    /// NJDNode_load: MeCab の素性の行を読み込む。複数語の連結 (アクセントが "1/3:0/4" のもの) はノードを増やす
    pub fn load(&mut self, n: usize, s: &str) {
        if s.len() >= 1024 * 6 {
            return;
        }
        let mut i = 0;
        let string = token(s, &mut i, b',');
        let node = &mut self.nodes[n];
        node.pos = val(token(s, &mut i, b','));
        node.pos_group1 = val(token(s, &mut i, b','));
        node.pos_group2 = val(token(s, &mut i, b','));
        node.pos_group3 = val(token(s, &mut i, b','));
        node.ctype = val(token(s, &mut i, b','));
        node.cform = val(token(s, &mut i, b','));
        let orig = token(s, &mut i, b',');
        let read = token(s, &mut i, b',');
        let pron = token(s, &mut i, b',');
        let acc = token(s, &mut i, b',');
        node.chain_rule = val(token(s, &mut i, b','));
        match token(s, &mut i, b',') {
            "1" => node.chain_flag = 1,
            "0" => node.chain_flag = 0,
            _ => {}
        }
        // 記号
        if acc.contains('*') || !acc.contains('/') {
            node.string = val(string);
            node.orig = val(orig);
            node.read = val(read);
            node.pron = val(pron);
            node.set_acc(0);
            node.set_mora_size(0);
            return;
        }
        let count = acc.bytes().filter(|&b| b == b'/').count();
        if count == 1 {
            node.string = val(string);
            node.orig = val(orig);
            node.read = val(read);
            node.pron = val(pron);
            let mut ia = 0;
            node.set_acc(atoi(token(acc, &mut ia, b'/')));
            node.set_mora_size(atoi(token(acc, &mut ia, b':')));
            return;
        }
        // 複数語の連結
        let mut aligned = orig.bytes().filter(|&b| b == b':').count() == count - 1;
        let (mut io, mut is) = (0, 0);
        if aligned {
            for _ in 0..count - 1 {
                let t = token(orig, &mut io, b':');
                if t.is_empty() || !string.as_bytes()[is..].starts_with(t.as_bytes()) {
                    aligned = false;
                    break;
                }
                is += t.len();
            }
        }
        if aligned && is >= string.len() {
            aligned = false;
        }
        let (mut io, mut is, mut ir, mut ip, mut ia) = (0, 0, 0, 0, 0);
        let mut cur = n;
        for k in 0..count {
            if k > 0 {
                let prev = cur;
                cur = self.new_node();
                let p = self.nodes[prev].clone();
                self.nodes[cur].copy_from(&p);
                self.nodes[cur].chain_flag = 0;
                self.nodes[cur].prev = Some(prev);
                self.nodes[prev].next = Some(cur);
            }
            let t = token(orig, &mut io, b':');
            let node = &mut self.nodes[cur];
            node.orig = val(t);
            if aligned {
                if k + 1 < count {
                    node.string = val(t);
                    is += t.len();
                } else {
                    node.string = val(&string[is..]);
                }
            } else if k == 0 {
                node.string = val(string);
            } else {
                node.string = val(t);
            }
            node.read = val(token(read, &mut ir, b':'));
            node.pron = val(token(pron, &mut ip, b':'));
            node.set_acc(atoi(token(acc, &mut ia, b'/')));
            node.set_mora_size(atoi(token(acc, &mut ia, b':')));
        }
    }
}
