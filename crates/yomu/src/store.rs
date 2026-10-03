//! 履歴とブックマークの保存と、URL 入力欄 (o / b) の候補選び。
//!
//! ~/.local/share/yomu/ (XDG_DATA_HOME があればその下) に JSON で置く。形式は Python 版と同じで、互いに読める。
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub const HISTORY_LIMIT: usize = 2000;

pub fn data_dir() -> PathBuf {
    match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".local").join("share"),
    }
    .join("yomu")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub kind: String,   // "url" / "search"
    pub target: String, // URL または検索語
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub time: f64,
    #[serde(skip)]
    pub bookmark: bool, // 候補として出すときの印 (保存はしない)
}

impl Item {
    fn is(&self, kind: &str, target: &str) -> bool {
        self.kind == kind && self.target == target
    }
}

/// 新しい順に並んだ Item の列を 1 つの JSON ファイルに保存する
pub struct ItemList {
    pub path: PathBuf,
    pub limit: Option<usize>,
    pub items: Vec<Item>,
}

impl ItemList {
    pub fn open(path: PathBuf, limit: Option<usize>) -> Self {
        let items = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<Item>>(&s).ok())
            .unwrap_or_default();
        Self { path, limit, items }
    }

    /// Python 版と同じく json.dumps(ensure_ascii=False, indent=1) の形で書く。保存できなくても閲覧は続けられるようにする
    pub fn save(&self) {
        let _ = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(self.path.parent().unwrap_or(Path::new(".")))?;
            let tmp = self.path.with_extension("tmp");
            let mut buf = Vec::new();
            let mut ser = serde_json::Serializer::with_formatter(&mut buf, serde_json::ser::PrettyFormatter::with_indent(b" "));
            self.items.serialize(&mut ser).map_err(std::io::Error::other)?;
            std::fs::write(&tmp, buf)?;
            std::fs::rename(&tmp, &self.path)
        })();
    }

    pub fn find(&self, kind: &str, target: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.is(kind, target))
    }

    /// 先頭 (最新) に入れる。同じものがあれば移動する
    pub fn add(&mut self, kind: &str, target: &str, title: &str) {
        self.items.retain(|i| !i.is(kind, target));
        let time = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
        self.items.insert(0, Item { kind: kind.into(), target: target.into(), title: title.into(), time, bookmark: false });
        if let Some(limit) = self.limit {
            self.items.truncate(limit);
        }
        self.save();
    }

    /// time 以降 (UNIX 時刻の秒) に開いたものを消す。消した数を返す
    pub fn remove_since(&mut self, time: f64) -> usize {
        let before = self.items.len();
        self.items.retain(|i| i.time < time);
        let n = before - self.items.len();
        if n > 0 {
            self.save();
        }
        n
    }

    pub fn remove(&mut self, kind: &str, target: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|i| !i.is(kind, target));
        if self.items.len() != before {
            self.save();
            return true;
        }
        false
    }
}

pub struct Store {
    pub history: ItemList,
    pub bookmarks: ItemList,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    /// ~/.local/share/yomu (XDG_DATA_HOME があればその下) のもの
    pub fn new() -> Self {
        Self::open(&data_dir())
    }

    pub fn open(dir: &Path) -> Self {
        Self {
            history: ItemList::open(dir.join("history.json"), Some(HISTORY_LIMIT)),
            bookmarks: ItemList::open(dir.join("bookmarks.json"), None),
        }
    }

    pub fn visit(&mut self, kind: &str, target: &str, title: &str) {
        self.history.add(kind, target, title);
    }

    /// ブックマークに入れる。すでにあれば外す。入れたなら true
    pub fn toggle_bookmark(&mut self, kind: &str, target: &str, title: &str) -> bool {
        if self.bookmarks.remove(kind, target) {
            return false;
        }
        self.bookmarks.add(kind, target, title);
        true
    }

    /// 入力した語をすべて (大文字小文字を区別せず) 含むものを、ブックマーク → 新しい順に返す
    pub fn suggest(&self, query: &str, bookmarks_only: bool, limit: usize) -> Vec<Item> {
        let query = query.to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        let marked = |i: &Item| self.bookmarks.items.iter().any(|b| b.is(&i.kind, &i.target));
        let history = self.history.items.iter().filter(|i| !bookmarks_only && !marked(i)).map(|i| (i, false));
        self.bookmarks
            .items
            .iter()
            .map(|i| (i, true))
            .chain(history)
            .filter(|(i, _)| {
                let hay = format!("{} {}", i.title, i.target).to_lowercase();
                words.iter().all(|w| hay.contains(w))
            })
            .take(limit)
            .map(|(i, bookmark)| Item { bookmark, ..i.clone() })
            .collect()
    }
}
