//! lxml と同じ形の木。要素だけを節にし、文字列は要素の text (最初の子の前) と tail (要素の後ろ) に持たせる。
//! Python 版は lxml (libxml2) で読んでいたが、ここでは html5ever (dom_query。rs-trafilatura と同じもの) で読む。
//! コメントなどは読むときに捨てる (Python 版の prune_always がコメントを消し、前後の文字列をつなぐのと同じ)。

pub type Id = usize;

#[derive(Clone, Debug, Default)]
pub struct Node {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub parent: Option<Id>,
    pub children: Vec<Id>,
    pub text: String,
    pub tail: String,
}

#[derive(Clone, Debug, Default)]
pub struct Tree {
    pub nodes: Vec<Node>,
}

impl Tree {
    /// HTML を読む。根は html 要素
    pub fn parse(html: &str) -> Tree {
        use html5ever::tendril::TendrilSink;
        // ブラウザと同じくスクリプトが動く前提で読む (noscript の中身は要素にせず文字列のまま。どうせ消す)。
        // dom_query の Document::from はスクリプトなしで読むので、<p> の中の <noscript><p>…</p></noscript> の
        // 内側の <p> が外側の <p> を閉じ、noscript の外に出てしまう
        let opts = html5ever::ParseOpts {
            tree_builder: html5ever::tree_builder::TreeBuilderOpts { scripting_enabled: true, ..Default::default() },
            ..Default::default()
        };
        let doc = html5ever::parse_document(dom_query::Document::default(), opts).one(html);
        let mut tree = Tree::default();
        let root = doc.root();
        let html_el = root.children().into_iter().find(|c| c.is_element());
        match html_el {
            Some(h) => {
                tree.convert(&h, None);
            }
            None => {
                tree.nodes.push(Node { tag: "html".into(), ..Default::default() });
            }
        }
        tree
    }

    fn convert(&mut self, n: &dom_query::NodeRef, parent: Option<Id>) -> Id {
        let id = self.nodes.len();
        let attrs = n.attrs().iter().map(|a| (a.name.local.to_string(), a.value.to_string())).collect();
        self.nodes.push(Node {
            tag: n.node_name().map(|t| t.to_string()).unwrap_or_default().to_lowercase(),
            attrs,
            parent,
            ..Default::default()
        });
        let mut last: Option<Id> = None;
        for c in n.children() {
            if c.is_element() {
                let cid = self.convert(&c, Some(id));
                self.nodes[id].children.push(cid);
                last = Some(cid);
            } else if c.is_text() {
                let t = c.text().to_string();
                match last {
                    Some(l) => self.nodes[l].tail.push_str(&t),
                    None => self.nodes[id].text.push_str(&t),
                }
            }
        }
        id
    }

    pub fn root(&self) -> Id {
        0
    }

    pub fn tag(&self, id: Id) -> &str {
        &self.nodes[id].tag
    }

    pub fn get(&self, id: Id, name: &str) -> Option<&str> {
        self.nodes[id].attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn parent(&self, id: Id) -> Option<Id> {
        self.nodes[id].parent
    }

    pub fn children(&self, id: Id) -> &[Id] {
        &self.nodes[id].children
    }

    /// 自分とすべての子孫 (文書順)。lxml の el.iter()
    pub fn iter(&self, id: Id) -> Vec<Id> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            out.push(n);
            for &c in self.nodes[n].children.iter().rev() {
                stack.push(c);
            }
        }
        out
    }

    /// 子孫 (自分を含まない、文書順)
    pub fn descendants(&self, id: Id) -> Vec<Id> {
        let mut v = self.iter(id);
        v.remove(0);
        v
    }

    pub fn ancestors(&self, id: Id) -> impl Iterator<Item = Id> + '_ {
        std::iter::successors(self.nodes[id].parent, move |&p| self.nodes[p].parent)
    }

    /// lxml の text_content(): 自分の text と、子孫の text・tail (自分の tail は含まない)
    pub fn text_content(&self, id: Id) -> String {
        let mut s = String::new();
        self.collect_text(id, &mut s);
        s
    }

    fn collect_text(&self, id: Id, s: &mut String) {
        s.push_str(&self.nodes[id].text);
        for &c in &self.nodes[id].children {
            self.collect_text(c, s);
            s.push_str(&self.nodes[c].tail);
        }
    }

    /// 要素を消す。後ろに続く文字列 (tail) は残す (Python 版の remove)
    pub fn remove(&mut self, id: Id) {
        let Some(parent) = self.nodes[id].parent else { return };
        let pos = self.nodes[parent].children.iter().position(|&c| c == id).unwrap();
        let tail = std::mem::take(&mut self.nodes[id].tail);
        if !tail.is_empty() {
            if pos > 0 {
                let prev = self.nodes[parent].children[pos - 1];
                self.nodes[prev].tail.push_str(&tail);
            } else {
                self.nodes[parent].text.push_str(&tail);
            }
        }
        self.nodes[parent].children.remove(pos);
        self.nodes[id].parent = None;
    }

    /// 最初の子要素のうち tag のもの (lxml の find("tag"))
    pub fn find_child(&self, id: Id, tag: &str) -> Option<Id> {
        self.nodes[id].children.iter().copied().find(|&c| self.nodes[c].tag == tag)
    }
}
