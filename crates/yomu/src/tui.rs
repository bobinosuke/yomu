//! Vimium と同じキー操作の対話型ビューア。Python 版の tui.py (Textual) を ratatui で書き直したもの。
//!
//! リンクは f で画面内のリンクに文字ラベルを出して選ぶ。タブ・ページ内検索・マーク・カウント (5j など) も
//! Vimium の既定キーに合わせている。Vimium にない機能 (検索・本文/全体の切り替え・読み上げ・終了) は
//! Vimium が使っていないキー (s / a / S / q) に置いた。
//!
//! 読み込みと読み上げの準備は別スレッドで行い、結果はチャネルで画面のスレッドに送る。
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use crossterm::event::{
    self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste, EnableFocusChange,
    EnableMouseCapture, Event, KeyEvent, KeyEventKind, MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style as TStyle};
use ratatui::text::{Line as TLine, Span as TSpan};
use ratatui::widgets::Paragraph;
use regex::Regex;
use unicode_width::UnicodeWidthStr;
use yomu_tts::resources::Resources;
use yomu_tts::sentences::{Lang, detect_lang, split_sentences};
use yomu_tts::speaker::{Chunk, Speaker};
use yomu_tts::supertonic::DEFAULT_VOICE;

use crate::doc::{Document, Span as DocSpan};
use crate::fetch::{self, Response};
use crate::ime::{self, InputSource};
use crate::loader::to_document_for_view;
use crate::pages::{blank_page, help_page, message_page};
use crate::render::{self, HEADING_COLOR, LINK_COLOR, Line, READING_WIDTH, Rgb, View};
use crate::search::{self, GOTO, PAGE, PAGE_SCHEME};
use crate::speech;
use crate::translate;
use crate::store::{Item, Store};
use serde::{Deserialize, Serialize};
use crate::urls::{looks_like_url, parent_url, to_url, unquote, urldefrag};
use crate::visual::{Mode, Visual};

const HINT_CHARS: &str = "sadfjklewcmpgh"; // Vimium の既定
const TAB_MIN: usize = 12; // タブの一覧で1つのタブに使う幅 (マス)
const TAB_MAX: usize = 28;
const SCROLL_STEP: usize = 3; // j/k で動く行数
const PREFIX_KEYS: &str = "gym`[]<>"; // 2打鍵目を待つキー
const SUGGEST_MAX: usize = 11; // 候補の欄の高さの上限

/// [[ / ]] で開く「前へ」「次へ」らしいリンクの文字列
static NEXT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*(次へ|次のページ|次の記事|次|다음|next|more|›|»|>>?|→)").unwrap());
static PREV_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*(前へ|前のページ|前の記事|前|이전|prev(ious)?|‹|«|<<?|←)").unwrap());

#[derive(Clone, Copy, PartialEq, Eq)]
enum HintAction {
    Open,
    Tab,
    Yank,
}

impl HintAction {
    fn label(self) -> &'static str {
        match self {
            Self::Open => t!("リンクを開く"),
            Self::Tab => t!("新しいタブで開く"),
            Self::Yank => t!("URL をコピー"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PromptMode {
    Open,
    Search,
    Find,
    Bookmark,
    /// ページの入力欄 (gi)
    Form,
}

impl PromptMode {
    fn label(self) -> &'static str {
        match self {
            Self::Open => t!("URL または検索語"),
            Self::Search => t!("検索語"),
            Self::Find => "/",
            Self::Bookmark => t!("ブックマーク"),
            Self::Form => t!("ページの入力欄"),
        }
    }
    /// 入力中に履歴・ブックマークの候補を出す入力欄
    fn suggests(self) -> bool {
        matches!(self, Self::Open | Self::Bookmark)
    }
}

/// Vimium と同じ方法で、互いに前方一致しないラベルを n 個作る
pub fn hint_labels(n: usize) -> Vec<String> {
    let chars: Vec<char> = HINT_CHARS.chars().collect();
    let mut hints = vec![String::new()];
    let mut offset = 0;
    while hints.len() - offset < n || hints.len() == 1 {
        let hint = hints[offset].clone();
        offset += 1;
        for c in &chars {
            hints.push(format!("{c}{hint}"));
        }
    }
    let mut out: Vec<String> = hints[offset..offset + n].to_vec();
    out.sort();
    out.into_iter().map(|h| h.chars().rev().collect()).collect()
}

/// タブの一覧の行。今のタブは反転させ、ほかは薄く出す。読み上げているタブには 🔊 を付ける。
/// 入りきらないときは今のタブの周りだけ出し、端の外にタブがあることを ‹ › で示す
/// タブの一覧の行。(文字列, スタイル, タブの番号 (区切りや ‹ › は None))
pub fn tab_bar(titles: &[String], cur: usize, width: usize, speaking: Option<usize>) -> Vec<(String, TStyle, Option<usize>)> {
    let size = TAB_MAX.min(TAB_MIN.max(width / titles.len().max(1)));
    let dim = TStyle::default().add_modifier(Modifier::DIM);
    let cells: Vec<(String, TStyle)> = titles
        .iter()
        .enumerate()
        .map(|(i, title)| {
            let text = format!(" {}{} {title}", i + 1, if Some(i) == speaking { " 🔊" } else { "" });
            let mut cell = truncate(&text, size - 1);
            cell.push(' ');
            let style =
                if i == cur { TStyle::default().add_modifier(Modifier::REVERSED | Modifier::BOLD) } else { dim };
            (cell, style)
        })
        .collect();
    let room = width.saturating_sub(2); // 両端の ‹ › の分
    let (mut lo, mut hi, mut used) = (cur, cur + 1, cells[cur].0.width());
    loop {
        // 今のタブから右・左の順に、入るだけ広げる
        let mut grew = false;
        for side in [hi as isize, lo as isize - 1] {
            if side >= 0 && (side as usize) < cells.len() {
                let w = cells[side as usize].0.width();
                if used + w < room {
                    used += w + 1;
                    lo = lo.min(side as usize);
                    hi = hi.max(side as usize + 1);
                    grew = true;
                }
            }
        }
        if !grew {
            break;
        }
    }
    let mut out = vec![(if lo > 0 { "‹" } else { " " }.to_string(), dim, None)];
    for (i, cell) in cells.iter().enumerate().take(hi).skip(lo) {
        if i > lo {
            out.push(("│".to_string(), dim, None));
        }
        out.push((cell.0.clone(), cell.1, Some(i)));
    }
    if hi < cells.len() {
        out.push(("›".to_string(), dim, None));
    }
    out
}

/// 幅 width マスごとに折り返す (改行はそのまま改行)。英語は語の途中で切らないよう、空白の後で折り返す
fn wrap_cells(s: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for para in s.split('\n') {
        let mut line = String::new();
        for word in para.split_inclusive(' ') {
            for c in word.chars() {
                let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                // 語が収まらなければ前で折り返す (語が 1 行より長ければ語の途中で)
                if line.width() + cw > width || (c == word.chars().next().unwrap() && word.is_ascii() && line.width() + word.trim_end().width() > width && !line.is_empty() && word.width() <= width) {
                    out.push(line.trim_end().to_string());
                    line.clear();
                }
                line.push(c);
            }
        }
        out.push(line.trim_end().to_string());
    }
    out
}

/// 幅 width マスに収まるよう、はみ出す分を … で省く (Rich の truncate(overflow="ellipsis"))
fn truncate(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw > width.saturating_sub(1) {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

fn clipboard_get() -> String {
    std::process::Command::new("pbpaste")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}


static NEXT_ID: AtomicU64 = AtomicU64::new(1);
fn new_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryKind {
    Url,
    Search,
    Blank,
}

impl EntryKind {
    fn name(self) -> &'static str {
        match self {
            Self::Url => "url",
            Self::Search => "search",
            Self::Blank => "blank",
        }
    }
    fn from_name(s: &str) -> Self {
        match s {
            "search" => Self::Search,
            "blank" => Self::Blank,
            _ => Self::Url,
        }
    }
}

#[derive(Clone, Debug)]
struct Entry {
    id: u64,
    kind: EntryKind,
    target: String, // URL または検索語
    scroll: usize,
    fields: Option<search::Fields>, // 検索の2ページ目以降: DuckDuckGo に送るフォームの値
}

impl Entry {
    fn new(kind: EntryKind, target: impl Into<String>) -> Self {
        Self { id: new_id(), kind, target: target.into(), scroll: 0, fields: None }
    }
    /// 同じ中身の別の項目 (Python の dataclasses.replace)
    fn copy(&self) -> Self {
        Self { id: new_id(), ..self.clone() }
    }
}

struct Tab {
    id: u64,
    history: Vec<Entry>,
    pos: Option<usize>,
    doc: Option<Arc<Document>>,
    status: String,
    jump_from: Option<usize>, // `` で戻る位置
    translation: Option<Translation>, // 翻訳中・翻訳済みのページ (e / E)
    /// 前回の終了時から開き直したが、まだ読み込んでいないタブのページ名 (切り替えたときに読み込む)
    unloaded: Option<String>,
}

/// 終了したときに開いていたタブ。次に起動したときに開き直す (~/.local/share/yomu/session.json)
#[derive(Default, Serialize, Deserialize)]
struct Session {
    tabs: Vec<SavedTab>,
    cur: usize,
}

#[derive(Serialize, Deserialize)]
struct SavedTab {
    title: String,
    history: Vec<SavedEntry>,
    pos: Option<usize>,
}

#[derive(Serialize, Deserialize)]
struct SavedEntry {
    kind: String,
    target: String,
    scroll: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fields: Option<search::Fields>,
}

fn session_path() -> std::path::PathBuf {
    crate::store::data_dir().join("session.json")
}

/// タブのページの翻訳。訳文を表示している間は、doc に source へ訳文を当てはめた文書を入れる。
/// 原文に戻しても訳文は覚えておき、もう一度訳すときは訳していない段落だけを送る
struct Translation {
    mode: Option<translate::Mode>,       // None なら原文を表示している
    source: Arc<Document>,               // 原文
    targets: Vec<usize>,                 // 訳す段落の番号
    done: HashMap<usize, Vec<DocSpan>>,  // 原文の段落の番号 → 訳文
    finished: HashSet<usize>,            // 訳し終えた段落 (日本語と判定されて訳さなかったものも含む)
    index: Vec<usize>,                   // 原文の段落の番号 → 表示している文書での番号
    cancel: Arc<AtomicBool>,             // 送っている途中の翻訳をやめる
}

impl Tab {
    fn new() -> Self {
        Self {
            id: new_id(),
            history: vec![],
            pos: None,
            doc: None,
            status: String::new(),
            jump_from: None,
            translation: None,
            unloaded: None,
        }
    }
    fn entry(&self) -> Option<&Entry> {
        self.pos.and_then(|p| self.history.get(p))
    }
    fn entry_mut(&mut self) -> Option<&mut Entry> {
        self.pos.and_then(|p| self.history.get_mut(p))
    }
}

/// 別スレッドから画面のスレッドへの知らせ
enum Msg {
    Status(u64, String),
    /// 今見ているタブの状態行に出す (Tor の接続の進み具合など)
    Notice(String),
    Loaded { tab: u64, entry: u64, doc: Arc<Document> },
    SpeechStarted(u64, Lang),
    SpeechBlock { tab: u64, doc: usize, blk: usize },
    SpeechDone(u64),
    Translated { tab: u64, source: usize, items: Vec<(usize, Option<Vec<DocSpan>>)> },
    TranslateFailed { tab: u64, source: usize, error: String },
    /// 本文の画像が届いた (取れなければ Err)
    Image { url: String, result: Result<Arc<image::DynamicImage>, String> },
    /// 選んだ文の訳文 (訳せなければ Err)
    SelectionTranslated { tab: u64, doc: usize, result: Result<String, String> },
}

/// 選んだ文の訳文を、選んだ所の近くに枠で出すもの
struct Popup {
    text: String,
    lines: (usize, usize), // 選んだ所の最初と最後の行 (描画結果の行)
}

/// 読み上げる予定のもの (ページ, (ブロック番号, 文) の列, 言語の決め方)
type PendingSpeech = (Arc<Document>, Vec<(usize, String)>, Detect);

/// y/n で聞いていること
enum Confirm {
    /// 読み上げのデータをダウンロードして、読み上げを始める
    Download(PendingSpeech),
    /// 履歴を消す
    ClearHistory(crate::history::ClearRange),
    /// 翻訳する (初めて翻訳するときに、本文を Google に送ってよいかを聞く)
    Translate(PendingTranslation),
}

/// 同意を待っている翻訳
enum PendingTranslation {
    Page(translate::Mode),
    Selection(Vec<String>),
}

/// 読み上げる言語の決め方 (読み上げの準備の後に detect_lang で決める)
struct Detect {
    texts: Vec<String>,
    /// ページが宣言した言語 (なければ空)
    declared: String,
    /// 読めない言語だったときに出すもの
    unsupported: String,
}

/// 読み込みに使うもの。読み込みのスレッドと共有する
struct Loader {
    http: fetch::Client,
    raw: Mutex<HashMap<String, Arc<Response>>>, // 開いた URL (# より前) → 取得結果
    // 検索結果のページ。DuckDuckGo は同じ検索でも毎回少し違う結果を返すので、戻ったときは同じものを見せる
    searches: Mutex<HashMap<(String, search::Fields), Arc<Document>>>,
}

impl Loader {
    fn fetch_doc(&self, entry: &Entry, reload: bool, full: bool, say: &dyn Fn(String)) -> Arc<Document> {
        if entry.kind == EntryKind::Blank {
            return Arc::new(blank_page());
        }
        let key = urldefrag(&entry.target).0;
        let error = |e: &dyn std::fmt::Display| {
            Arc::new(message_page(&entry.target, t!("エラー"), &t!("読み込めませんでした: {e}", e = e), t!("エラー")))
        };
        if entry.kind == EntryKind::Search {
            let mut fields = entry.fields.clone().unwrap_or_default();
            fields.sort();
            let skey = (entry.target.clone(), fields);
            if !reload && let Some(d) = self.searches.lock().unwrap().get(&skey) {
                return d.clone();
            }
            say(t!("検索中: {target}", target = entry.target));
            return match search::search(&entry.target, entry.fields.as_ref(), true) {
                Ok(d) => {
                    let d = Arc::new(d);
                    self.searches.lock().unwrap().insert(skey, d.clone());
                    d
                }
                Err(e) => error(&e),
            };
        }
        let cached = self.raw.lock().unwrap().get(&key).cloned();
        let r = match cached {
            Some(r) if !reload => r,
            _ => {
                say(t!("読み込み中: {target}", target = entry.target));
                match fetch::fetch(&self.http, &key) {
                    Ok(r) => {
                        let r = Arc::new(r);
                        self.raw.lock().unwrap().insert(key.clone(), r.clone());
                        r
                    }
                    Err(e) => return error(&e),
                }
            }
        };
        // 壊れたページで抽出が落ちても、アプリは落とさない
        let extract = |r: &Response| std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| to_document_for_view(r, full)));
        match extract(&r) {
            Ok(mut d) => {
                // JavaScript なしでは本文がほとんど取れないページは、普段のブラウザで開くよう案内する
                if crate::loader::needs_browser(&r, &d) {
                    add_note(&mut d, t!("本文がほとんど取れませんでした (gx でブラウザで開けます)"));
                }
                Arc::new(d)
            }
            Err(e) => {
                let msg = crate::loader::panic_message(&*e);
                Arc::new(message_page(
                    &entry.target,
                    t!("表示できませんでした"),
                    &t!("このページを表示できませんでした ({msg})。r で再読み込み、a で全体表示に切り替えられます。", msg = msg),
                    t!("エラー"),
                ))
            }
        }
    }
}

fn add_note(d: &mut Document, s: &str) {
    d.note = if d.note.is_empty() { s.into() } else { format!("{} | {s}", d.note) };
}

/// 読み上げ。初めて S を押したときに、別スレッドで辞書・モデルを用意する
#[derive(Default)]
struct Speech {
    speaker: Arc<Mutex<Option<Speaker>>>,
    preparing: Arc<AtomicBool>, // 用意している間 (まだ読み始めていない)
    cancel: Arc<AtomicBool>,    // 用意している間に止められた
    tab: Option<u64>,           // 読み上げているタブ (タブを移っても読み続ける)
    blk: Option<usize>,         // 今読んでいるブロック (そのタブを表示している間に読んだもの)
    lang: Option<Lang>,         // 読み上げているページの言語
    last_used: Option<std::time::Instant>, // 最後に読み上げていた時刻 (モデルを読み込んである間)
}

/// プライベートモードで翻訳を使おうとしたとき
fn private_no_translate() -> &'static str {
    t!("プライベートモードでは翻訳は使えません (本文が Google に送られるため)")
}

/// 読み上げをこれだけ使わなければ、読み上げのモデル (約 500MB) を手放す。次の S でまた読み込む (1 秒ほど)
const SPEECH_IDLE: Duration = Duration::from_secs(180);

impl Speech {
    fn speaking(&self) -> bool {
        if self.preparing.load(Ordering::Relaxed) {
            return true;
        }
        match self.speaker.try_lock() {
            Ok(g) => g.as_ref().is_some_and(|s| s.speaking()),
            Err(_) => true,
        }
    }
    /// しばらく読み上げていなければ、裏で読み上げのモデルを捨ててメモリを返す
    fn unload_if_idle(&mut self) {
        if self.speaking() {
            self.last_used = Some(std::time::Instant::now());
            return;
        }
        if self.last_used.is_none_or(|t| t.elapsed() < SPEECH_IDLE) {
            return;
        }
        self.last_used = None;
        let speaker = self.speaker.clone();
        std::thread::spawn(move || {
            if let Ok(mut g) = speaker.lock()
                && g.as_ref().is_some_and(|s| !s.speaking())
            {
                *g = None;
            }
            fetch::release_memory();
        });
    }

    fn stop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Ok(mut g) = self.speaker.try_lock()
            && let Some(s) = g.as_mut()
        {
            s.stop();
        }
        self.tab = None;
    }
}

/// 描いたものの位置。マウスのクリックがどこに当たったかを見るのに使う
#[derive(Default)]
struct Hit {
    tabs: Vec<(u16, u16, usize)>, // タブの一覧の各タブ (左端, 右端 (含まない), タブの番号)。一覧の行は 0
    title_y: u16,                 // 題名 (URL) の行
    suggest: (u16, u16),          // 候補の一覧 (最初の行, 行数)
    prompt: Option<(u16, usize)>, // 入力欄 (行, 左に隠した幅)
}

/// 描画結果の行のキャッシュ
/// 描画結果を使い回せるか見分けるもの: (Document のポインタ, (本文の幅, コード・表の幅))
type ViewKey = (usize, (usize, usize));

struct Layout {
    key: ViewKey,
    images: u64, // 画像の状態の版 (画像が届くたびに作り直す)
    lines: Arc<Vec<Line>>,
    places: Vec<crate::images::Place>, // 画像を描く場所
}

pub struct App {
    initial: Option<String>,
    full: bool,
    ime: Option<InputSource>,
    loader: Arc<Loader>,
    store: Store, // 履歴とブックマーク
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    // タブ
    tabs: Vec<Tab>,
    cur: usize,
    prev_tab: Option<u64>,
    closed: Vec<Tab>,
    overlay: Option<Arc<Document>>, // ヘルプ
    // キー入力
    pending: String, // 2打鍵目を待っている1打鍵目
    count: String,   // 5j の 5
    hints: Vec<(String, usize)>, // ラベル → リンクの出現番号
    hint_typed: String,
    hint_action: HintAction,
    prompt: Option<Prompt>,
    find_q: String,
    last_find: String,
    visual: Option<Visual>,
    visual_count: String,
    visual_pending: String,
    marks: HashMap<(String, char), usize>,
    global_marks: HashMap<char, (Entry, usize)>,
    speech: Speech,
    // 画面
    scroll_y: usize,
    scroll_x: usize,
    size: (u16, u16),
    /// 状態行 (行, 中身)。draw で描いたものと、端末にまるごと書き直したもの
    status_line: (u16, String),
    status_written: (u16, String),
    /// 入力欄のカーソルの位置 (入力欄がなければ None)
    cursor: Option<(u16, u16)>,
    /// 描いたものの位置 (マウスのクリックがどこに当たったかを見る)
    hit: Hit,
    /// 本文の上で左ボタンを押した位置 (行, 文字の位置) と、そこからドラッグして選んでいるか
    press: Option<(usize, usize)>,
    dragging: bool,
    /// マウスの位置 (端末の外に出たら None) と、その下にあるリンク (出現番号, Document.links のインデックス)
    mouse: Option<(u16, u16)>,
    hover: Option<(usize, usize)>,
    /// 選んだ文の訳文 (キーかクリックで閉じる)
    popup: Option<Popup>,
    /// 設定画面で本文の作り方に関わる設定を変えた (設定画面を閉じたらページを作り直す)
    settings_changed: bool,
    /// y/n で聞いていること (y で実行する)
    confirm: Option<Confirm>,
    /// 終了した後、プライベートモードを切り替えて起動し直す (gp)
    restart: bool,
    /// 入力しているページの入力欄 (gi) と、入力欄の一覧を出したときのページ
    form: Option<crate::doc::Form>,
    form_source: Option<Arc<Document>>,
    layout: Option<Layout>,
    shown: Option<(ViewKey, View, u64, Arc<Vec<Line>>)>, // 表示中の行 (ヒント・検索語の強調つき)
    /// 本文の画像 (設定の images)。URL → 状態。images_version は状態が変わるたびに増やす
    images: HashMap<String, crate::images::State>,
    images_version: u64,
    /// 画像の描き方 (端末に問い合わせて決める。画像を表示する設定のときだけ作る)
    picker: Option<ratatui_image::picker::Picker>,
    /// 描く画像 (URL, 列, 行, 切り出した上端) → 端末に送る形
    image_protocols: HashMap<(String, u16, u16, u32), ratatui_image::protocol::Protocol>,
    occ_links: HashMap<usize, usize>, // リンクの出現番号 → Document.links のインデックス
    quit: bool,
}

/// 入力欄 (o / O / ge / gE / s / / / b)
struct Prompt {
    mode: PromptMode,
    new_tab: bool,
    text: Vec<char>,
    cursor: usize,
    suggestions: Vec<Item>,
    sel: Option<usize>, // 選んでいる候補 (None は入力した文字のまま)
}

impl Prompt {
    fn value(&self) -> String {
        self.text.iter().collect()
    }
}

impl App {
    pub fn new(target: Option<String>, full: bool, switch_ime: bool) -> Self {
        let (tx, rx) = channel();
        Self {
            initial: target,
            full,
            ime: switch_ime.then(InputSource::new),
            loader: Arc::new(Loader {
                http: fetch::client(),
                raw: Mutex::new(HashMap::new()),
                searches: Mutex::new(HashMap::new()),
            }),
            store: Store::new(),
            tx,
            rx,
            tabs: vec![Tab::new()],
            cur: 0,
            prev_tab: None,
            closed: vec![],
            overlay: None,
            pending: String::new(),
            count: String::new(),
            hints: vec![],
            hint_typed: String::new(),
            hint_action: HintAction::Open,
            prompt: None,
            find_q: String::new(),
            last_find: String::new(),
            visual: None,
            visual_count: String::new(),
            visual_pending: String::new(),
            marks: HashMap::new(),
            global_marks: HashMap::new(),
            speech: Speech::default(),
            scroll_y: 0,
            scroll_x: 0,
            size: (80, 24),
            status_line: (0, String::new()),
            status_written: (0, String::new()),
            cursor: None,
            hit: Hit::default(),
            press: None,
            dragging: false,
            mouse: None,
            hover: None,
            popup: None,
            settings_changed: false,
            confirm: None,
            restart: false,
            form: None,
            form_source: None,
            layout: None,
            shown: None,
            images: HashMap::new(),
            images_version: 0,
            picker: None,
            image_protocols: HashMap::new(),
            occ_links: HashMap::new(),
            quit: false,
        }
    }

    /// 画面を開いて、終了するまで動かす
    pub fn run(&mut self) -> io::Result<()> {
        enable_raw_mode()?;
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste, EnableFocusChange)?;
        let mut term = Terminal::new(CrosstermBackend::new(out))?;
        if fetch::CookieJar::uses_file() {
            fetch::COOKIES.load();
        }
        // プライベートモードでは、裏で Tor につなぎ始める (つながるまで、ページの読み込みは待つ)
        if fetch::is_private() {
            let tx = self.tx.clone();
            crate::tor::start(move |m| {
                let _ = tx.send(Msg::Notice(m));
            });
        }
        let result = self.main_loop(&mut term);
        if fetch::CookieJar::uses_file() {
            fetch::COOKIES.save();
        }
        disable_raw_mode()?;
        execute!(io::stdout(), DisableFocusChange, DisableBracketedPaste, DisableMouseCapture, LeaveAlternateScreen)?;
        term.show_cursor()?;
        result?;
        if self.restart {
            return Err(self.exec_switched());
        }
        Ok(())
    }

    /// gp: プライベートモードを切り替える。普段とプライベートでメモリの中の Cookie や読み込んだページを共有しないよう、
    /// yomu を起動し直す。普段のタブは終了するときに保存してあるので、普段に戻ると開き直す。
    /// プライベートのタブと Cookie は残さない
    fn toggle_private(&mut self) {
        self.restart = true;
        self.quit = true;
    }

    /// プライベートモードを切り替えた引数で、今のプロセスを起動し直す (戻るのは失敗したときだけ)
    fn exec_switched(&self) -> io::Error {
        use std::os::unix::process::CommandExt;
        let mut args: Vec<&str> = Vec::new();
        if self.full {
            args.push("--full");
        }
        if self.ime.is_none() {
            args.push("--keep-ime");
        }
        if !fetch::is_private() {
            args.push("--private");
        }
        match std::env::current_exe() {
            Ok(exe) => std::process::Command::new(exe).args(args).exec(),
            Err(e) => e,
        }
    }

    fn main_loop(&mut self, term: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
        let s = term.size()?;
        self.size = (s.width, s.height);
        self.ime_normal();
        // 前回のタブを開き直す。URL や検索語を渡されたら、その後ろに新しいタブで開く
        let restored = self.restore_session();
        match self.initial.take() {
            Some(t) => self.go(&t, restored, false),
            None if !restored => self.navigate(Entry::new(EntryKind::Blank, ""), false, false),
            None => {}
        }
        let mut dirty = true; // 描き直しが要る (何もなければ描かない)
        while !self.quit {
            // 画像を表示する設定なら、画像の描き方と文字の大きさを決める
            if self.picker.is_none() && crate::settings::get().images {
                self.picker = Some(crate::images::picker());
                self.images_version += 1;
                dirty = true;
            }
            if dirty {
                term.draw(|f| self.draw(f))?;
                self.rewrite_status()?;
                dirty = false;
            }
            if event::poll(Duration::from_millis(50))? {
                dirty = true;
                // たまっている入力はまとめて処理してから描く
                loop {
                    match event::read()? {
                        Event::Key(k) if k.kind != KeyEventKind::Release => self.on_key(k),
                        Event::Paste(s) => self.on_paste(&s),
                        Event::Resize(w, h) => self.size = (w, h),
                        Event::FocusLost => self.mouse = None,
                        Event::FocusGained => {
                            // 別のペインやアプリで日本語入力にしてから戻ってきたときも、操作モードなら英数にする。
                            // (フォーカスを失ったときは、送ったキーが移った先のアプリに届いてしまうので何もしない)
                            if self.prompt.is_none() {
                                self.ime_normal();
                            }
                        }
                        Event::Mouse(m) => match m.kind {
                            MouseEventKind::ScrollDown => self.scroll_by(SCROLL_STEP as isize),
                            MouseEventKind::ScrollUp => self.scroll_by(-(SCROLL_STEP as isize)),
                            MouseEventKind::Down(b) => self.on_click(b, m.column, m.row, m.modifiers),
                            MouseEventKind::Drag(MouseButton::Left) => self.on_drag(m.column, m.row),
                            MouseEventKind::Up(MouseButton::Left) => self.on_release(),
                            MouseEventKind::Moved => self.mouse = Some((m.column, m.row)),
                            _ => {}
                        },
                        _ => {}
                    }
                    if self.quit || !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
            }
            while let Ok(m) = self.rx.try_recv() {
                self.on_msg(m);
                dirty = true;
            }
            self.speech.unload_if_idle();
        }
        self.stop_speech(None);
        self.ime_restore(); // 終了したら元の入力ソースに戻す
        self.save_session();
        Ok(())
    }

    // ---- よく使うもの
    fn tab(&self) -> &Tab {
        &self.tabs[self.cur]
    }
    fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.cur]
    }
    fn tab_index(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }
    fn doc(&self) -> Option<Arc<Document>> {
        self.overlay.clone().or_else(|| self.tab().doc.clone())
    }

    /// 本文の欄の大きさ (幅は本文の幅、高さは行数)
    fn body_rect(&self) -> Rect {
        let (w, h) = self.size;
        let top = if self.tabs.len() > 1 { 2 } else { 1 };
        let bottom = 1 + self.suggest_height() + u16::from(self.prompt.is_some());
        Rect::new(1, top, w.saturating_sub(2), h.saturating_sub(top + bottom))
    }
    /// 本文を折り返す幅 (読みやすい幅まで)
    fn page_width(&self) -> usize {
        (self.body_rect().width as usize).clamp(1, READING_WIDTH)
    }
    /// コードと表を描く幅 (端末の幅まで)
    fn wide_width(&self) -> usize {
        (self.body_rect().width as usize).max(1)
    }
    fn widths(&self) -> (usize, usize) {
        (self.page_width(), self.wide_width())
    }
    fn height(&self) -> usize {
        self.body_rect().height as usize
    }
    fn max_scroll_y(&mut self) -> usize {
        let n = self.shown_lines().len();
        n.saturating_sub(self.height())
    }

    fn remember_scroll(&mut self) {
        let y = self.scroll_y;
        // 最後のタブを閉じて終了するときはタブがない
        if let Some(e) = self.tabs.get_mut(self.cur).and_then(|t| t.entry_mut()) {
            e.scroll = y;
        }
    }

    /// jump なら `` で戻れるよう今の位置を覚えておく
    fn scroll_to(&mut self, y: usize, jump: bool) {
        if jump {
            let from = self.scroll_y;
            self.tab_mut().jump_from = Some(from);
        }
        let max = self.max_scroll_y();
        self.scroll_y = y.min(max);
    }

    fn scroll_by(&mut self, d: isize) {
        let y = (self.scroll_y as isize + d).max(0) as usize;
        self.scroll_to(y, false);
    }

    fn say(&mut self, tab: u64, msg: impl Into<String>) {
        if let Some(i) = self.tab_index(tab) {
            self.tabs[i].status = msg.into();
        }
    }
    fn say_here(&mut self, msg: impl Into<String>) {
        let id = self.tab().id;
        self.say(id, msg);
    }

    // ---- IME
    fn ime_normal(&mut self) {
        if let Some(ime) = &mut self.ime {
            ime.to_ascii();
        }
    }
    fn ime_restore(&mut self) {
        if let Some(ime) = &mut self.ime {
            ime.restore();
        }
    }

    // ---- 描画
    /// less と同じく、今どこまで読んだかを割合で返す
    fn position(&mut self) -> String {
        let max = self.max_scroll_y();
        let h = self.height();
        if max == 0 {
            return t!("全体").into();
        }
        if self.scroll_y == 0 {
            return t!("先頭").into();
        }
        if self.scroll_y >= max {
            return t!("末尾").into();
        }
        // Python の round と同じく、ちょうど半分なら偶数に丸める
        format!("{}%", (100.0 * (self.scroll_y + h) as f64 / (max + h) as f64).round_ties_even() as i64)
    }

    /// 描画結果を行に分けたもの (ヒント・検索語の強調なし)。ヒント・ページ内検索・読み上げの位置合わせに使う
    fn layout(&mut self) -> Arc<Vec<Line>> {
        let Some(d) = self.doc() else { return Arc::new(vec![]) };
        let key = (Arc::as_ptr(&d) as usize, self.widths());
        if self.layout.as_ref().is_none_or(|l| l.key != key || l.images != self.images_version) {
            let lines = render::render_wide(&d, &View::default(), key.1.0, key.1.1);
            let (lines, places) = self.with_images(&d, lines);
            self.occ_links.clear();
            for line in &lines {
                for (o, l) in line.occs() {
                    self.occ_links.insert(o, l);
                }
            }
            self.layout = Some(Layout { key, images: self.images_version, lines: Arc::new(lines), places });
        }
        self.layout.as_ref().unwrap().lines.clone()
    }

    /// 表示する行 (ヒントのラベル・検索語の強調つき)
    fn shown_lines(&mut self) -> Arc<Vec<Line>> {
        let Some(d) = self.doc() else { return Arc::new(vec![]) };
        if self.hints.is_empty() && self.find_q.is_empty() {
            return self.layout();
        }
        let hints: HashMap<usize, String> = self
            .hints
            .iter()
            .filter(|(lab, _)| lab.starts_with(&self.hint_typed))
            .map(|(lab, o)| (*o, lab.clone()))
            .collect();
        let view = View {
            hints: (!self.hints.is_empty()).then_some(hints),
            typed: self.hint_typed.clone(),
            find: self.find_q.clone(),
            numbers: false,
        };
        let key = (Arc::as_ptr(&d) as usize, self.widths());
        if let Some((k, v, ver, lines)) = &self.shown
            && *k == key
            && *v == view
            && *ver == self.images_version
        {
            return lines.clone();
        }
        let lines = render::render_wide(&d, &view, key.1.0, key.1.1);
        let lines = Arc::new(self.with_images(&d, lines).0);
        self.shown = Some((key, view, self.images_version, lines.clone()));
        lines
    }

    /// 描画結果の行に、届いた画像の場所を空ける (画像を表示する設定のときだけ)
    fn with_images(&self, d: &Document, lines: Vec<Line>) -> (Vec<Line>, Vec<crate::images::Place>) {
        if self.images.is_empty() || !crate::settings::get().images {
            return (lines, vec![]);
        }
        let font = self.picker.as_ref().map_or((10, 20), |p| {
            let f = p.font_size();
            (f.width, f.height)
        });
        crate::images::place(&lines, d, &self.images, self.page_width() as u16, font)
    }

    /// ページの画像を裏で取ってくる (画像を表示する設定のとき)
    fn fetch_images(&mut self, doc: &Document) {
        if !crate::settings::get().images {
            return;
        }
        let mut urls: Vec<String> = Vec::new();
        for (_, u) in crate::images::image_srcs(doc) {
            if !self.images.contains_key(&u) && !urls.contains(&u) {
                urls.push(u);
            }
        }
        if urls.is_empty() {
            return;
        }
        for u in &urls {
            self.images.insert(u.clone(), crate::images::State::Loading);
        }
        let tx = self.tx.clone();
        crate::images::fetch_all(urls, doc.url.clone(), move |url, result| {
            let _ = tx.send(Msg::Image { url, result: result.map(Arc::new) });
        });
    }

    /// 画像が届いた。場所を空けると下の行がずれるので、画面の一番上の段落が同じ位置に見えるようにスクロールを合わせる
    fn image_loaded(&mut self, url: String, result: Result<Arc<image::DynamicImage>, String>) {
        let anchor = (self.overlay.is_none() && self.visual.is_none())
            .then(|| self.layout().get(self.scroll_y).and_then(|l| l.blks().first().copied()))
            .flatten()
            .and_then(|b| self.block_lines().get(&b).map(|&l| (b, self.scroll_y.saturating_sub(l))));
        let state = match result {
            Ok(img) => crate::images::State::Ready(img),
            Err(_) => crate::images::State::Failed,
        };
        self.images.insert(url, state);
        self.images_version += 1;
        if let Some((b, off)) = anchor
            && let Some(&l) = self.block_lines().get(&b)
        {
            self.scroll_y = l + off;
        }
    }

    /// 見えている画像を描く。一部だけ見えているときは、見えている部分を切り出して描く
    fn draw_images(&mut self, f: &mut ratatui::Frame, body: Rect) {
        let Some(places) = self.layout.as_ref().map(|l| l.places.clone()) else { return };
        if places.is_empty() || self.scroll_x > 0 || self.overlay.is_some() {
            return;
        }
        let Some(picker) = &self.picker else { return };
        let (y0, h) = (self.scroll_y, body.height as usize);
        if self.image_protocols.len() > 64 {
            self.image_protocols.clear();
        }
        for p in places {
            let (top, bot) = (p.top.max(y0), (p.top + p.rows as usize).min(y0 + h));
            if top >= bot {
                continue;
            }
            let Some(crate::images::State::Ready(img)) = self.images.get(&p.src) else { continue };
            let rows = (bot - top) as u16;
            let area = Rect::new(body.x, body.y + (top - y0) as u16, p.cols.min(body.width), rows);
            // 見えている行に当たる部分を切り出す
            let (cut_top, cut_h) = if rows == p.rows {
                (0, img.height())
            } else {
                let px = |r: usize| (img.height() as u64 * r as u64 / p.rows as u64) as u32;
                (px(top - p.top), px(bot - p.top) - px(top - p.top))
            };
            let key = (p.src.clone(), area.width, area.height, cut_top);
            if !self.image_protocols.contains_key(&key) {
                let part = if cut_h == img.height() { (**img).clone() } else { img.crop_imm(0, cut_top, img.width(), cut_h.max(1)) };
                match picker.new_protocol(part, area.as_size(), ratatui_image::Resize::Fit(None)) {
                    Ok(proto) => {
                        self.image_protocols.insert(key.clone(), proto);
                    }
                    Err(_) => continue,
                }
            }
            if let Some(proto) = self.image_protocols.get(&key) {
                f.render_widget(ratatui_image::Image::new(proto), area);
            }
        }
    }

    /// ブロック番号 → そのブロックが始まる行
    fn block_lines(&mut self) -> HashMap<usize, usize> {
        let mut first = HashMap::new();
        for (i, line) in self.layout().iter().enumerate() {
            for b in line.blks() {
                first.entry(b).or_insert(i);
            }
        }
        first
    }

    fn draw(&mut self, f: &mut ratatui::Frame) {
        self.hit = Hit::default();
        let area = f.area();
        self.size = (area.width, area.height);
        let mut y = 0;
        let dim = TStyle::default().add_modifier(Modifier::DIM);
        // タブの一覧
        if self.tabs.len() > 1 {
            let speaking = self.speech.tab.and_then(|t| self.tab_index(t));
            let titles: Vec<String> = self
                .tabs
                .iter()
                .map(|t| match (&t.doc, &t.unloaded) {
                    (Some(d), _) => d.title.clone(),
                    (None, Some(title)) if !title.is_empty() => title.clone(),
                    _ => t!("読み込み中…").to_string(),
                })
                .collect();
            let bar = tab_bar(&titles, self.cur, area.width as usize, speaking);
            let mut x = 0;
            for (t, _, i) in &bar {
                let w = t.width() as u16;
                if let Some(i) = i {
                    self.hit.tabs.push((x, x + w, *i));
                }
                x += w;
            }
            let spans: Vec<TSpan> = bar.into_iter().map(|(t, s, _)| TSpan::styled(t, s)).collect();
            f.render_widget(Paragraph::new(TLine::from(spans)), Rect::new(0, y, area.width, 1));
            y += 1;
        }
        // 題名
        self.hit.title_y = y;
        let d = self.doc();
        let title = d.as_ref().map_or(t!("読み込み中").to_string(), |d| format!("{}  —  {}", d.title, d.url));
        let title = render::first_line(&title, area.width.saturating_sub(2) as usize);
        f.render_widget(
            Paragraph::new(TSpan::styled(title, TStyle::default().add_modifier(Modifier::BOLD))),
            Rect::new(1, y, area.width.saturating_sub(2), 1),
        );
        // 本文
        let body = self.body_rect();
        let max = self.max_scroll_y();
        self.scroll_y = self.scroll_y.min(max);
        let lines: Arc<Vec<Line>> = match &self.visual {
            Some(v) => Arc::new(v.render()),
            None => self.shown_lines(),
        };
        if d.is_some() && lines.is_empty() {
            f.render_widget(
                Paragraph::new(TSpan::styled(t!("(表示できる本文がありません。a で全体表示に切り替えられます)"), dim)),
                body,
            );
        }
        // マウスを乗せたリンクは反転させる (状態行にリンク先を出す)
        self.hover = self.hovered();
        let hover = self.hover.map(|(o, _)| o);
        let visible: Vec<TLine> = lines
            .iter()
            .skip(self.scroll_y)
            .take(body.height as usize)
            .map(|l| {
                TLine::from(
                    l.segs
                        .iter()
                        .map(|s| {
                            let st = tstyle(&s.style);
                            let st = if s.occ.is_some() && s.occ == hover { st.add_modifier(Modifier::REVERSED) } else { st };
                            TSpan::styled(s.text.clone(), st)
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        let page = Rect::new(body.x, body.y, self.wide_width() as u16, body.height);
        f.render_widget(Paragraph::new(visible).scroll((0, self.scroll_x as u16)), page);
        self.draw_images(f, body);
        // 選んだ文の訳文の枠。選んだ所の下に入ればその下、入らなければ上に出す
        if let Some(p) = &self.popup {
            let w = (self.page_width() as u16).min(area.width.saturating_sub(4)).max(10);
            let rows = wrap_cells(&p.text, w.saturating_sub(4) as usize);
            let h = (rows.len() as u16 + 2).min(body.height);
            let below = (p.lines.1 + 1).saturating_sub(self.scroll_y) as u16 + body.y;
            let above = (p.lines.0 as isize - self.scroll_y as isize + body.y as isize - h as isize).max(body.y as isize) as u16;
            let y = if below + h <= body.y + body.height { below } else { above };
            let r = Rect::new(body.x, y, w, h);
            let text: Vec<TLine> = rows.into_iter().map(|l| TLine::from(TSpan::styled(l, fg(render::TRANSLATION_COLOR)))).collect();
            f.render_widget(ratatui::widgets::Clear, r);
            f.render_widget(
                Paragraph::new(text).block(
                    ratatui::widgets::Block::bordered()
                        .title(format!(" {} ", t!("訳文")))
                        .padding(ratatui::widgets::Padding::horizontal(1))
                        .border_style(fg(render::TRANSLATION_COLOR)),
                ),
                r,
            );
        }
        // 状態行
        let status_y = body.y + body.height;
        let status = self.status_text();
        let line = format!(" {}", render::first_line(&status, area.width.saturating_sub(2) as usize));
        let line = format!("{line}{}", " ".repeat((area.width as usize).saturating_sub(line.width())));
        f.render_widget(
            Paragraph::new(TSpan::raw(line.clone())).style(TStyle::default().add_modifier(Modifier::REVERSED)),
            Rect::new(0, status_y, area.width, 1),
        );
        self.status_line = (status_y, line);
        self.cursor = None;
        // 候補と入力欄
        let sh = self.suggest_height();
        self.hit.suggest = (status_y + 1, sh);
        if sh > 0 {
            let rows = self.suggest_rows(area.width.saturating_sub(2) as usize);
            f.render_widget(Paragraph::new(rows), Rect::new(1, status_y + 1, area.width.saturating_sub(2), sh));
        }
        if let Some(p) = &self.prompt {
            let py = status_y + 1 + sh;
            let w = area.width.saturating_sub(2) as usize;
            let before: String = p.text[..p.cursor].iter().collect();
            // カーソルが欄からはみ出すときは、左側を隠す
            let skip = before.width().saturating_sub(w.saturating_sub(1));
            let (shown, cursor_x) = if p.text.is_empty() {
                let label = match (&self.form, p.mode) {
                    (Some(f), PromptMode::Form) if !f.label.is_empty() => f.label.as_str(),
                    _ => p.mode.label(),
                };
                let ph = format!("{label}{}", if p.new_tab { t!(" (新しいタブ)") } else { "" });
                (TLine::from(TSpan::styled(ph, dim)), 0)
            } else {
                let text: String = p.text.iter().collect();
                (TLine::from(skip_cells(&text, skip)), before.width() - skip)
            };
            f.render_widget(Paragraph::new(shown), Rect::new(1, py, area.width.saturating_sub(2), 1));
            // IME の変換中の文字と候補ウィンドウは端末の本物のカーソル位置に出るので、入力位置に置く
            f.set_cursor_position((1 + cursor_x as u16, py));
            self.cursor = Some((1 + cursor_x as u16, py));
            self.hit.prompt = Some((py, skip));
        }
    }

    /// 状態行が変わったら、その行を端末にまるごと書き直す。ratatui は前の画面との差分だけを書くが、全角文字が
    /// 1 マスずれると、端末が消した全角文字の残り半分 (反転した空白) を「前と同じ」として書かず、反転の背景が欠ける
    fn rewrite_status(&mut self) -> io::Result<()> {
        if self.status_line == self.status_written {
            return Ok(());
        }
        use crossterm::{cursor::MoveTo, queue, style::{Attribute, Print, SetAttribute}};
        let mut out = io::stdout();
        let (y, line) = &self.status_line;
        queue!(out, MoveTo(0, *y), SetAttribute(Attribute::Reset), SetAttribute(Attribute::Reverse), Print(line), SetAttribute(Attribute::Reset))?;
        if let Some((x, y)) = self.cursor {
            queue!(out, MoveTo(x, y))?;
        }
        io::Write::flush(&mut out)?;
        self.status_written = self.status_line.clone();
        Ok(())
    }

    fn status_text(&mut self) -> String {
        let d = self.doc();
        let mode = self.visual.as_ref().map(|v| format!("-- {} --", v.mode().name())).unwrap_or_default();
        let pending = if self.visual.is_some() {
            format!("{}{}", self.visual_count, self.visual_pending)
        } else {
            format!("{}{}", self.count, self.pending)
        };
        let pos = self.position();
        let note = d.as_ref().map(|d| d.note.clone()).unwrap_or_default();
        let hover = self.hover.and_then(|(_, l)| d.as_ref()?.links.get(l).map(|u| format!("→ {}", unquote(u))));
        let private = if fetch::is_private() { t!("プライベート (Tor)").to_string() } else { String::new() };
        let parts = match hover {
            // 聞いているときは、質問が切れないよう質問だけを出す
            _ if self.confirm.is_some() => vec![self.tab().status.clone()],
            Some(u) => vec![private, mode, pending, pos, u],
            None => vec![private, mode, pending, pos, note, self.tab().status.clone(), t!("? でヘルプ").into()],
        };
        parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" | ")
    }

    fn suggest_height(&self) -> u16 {
        match &self.prompt {
            Some(p) if p.mode.suggests() => {
                if p.suggestions.is_empty() {
                    u16::from(p.mode == PromptMode::Bookmark)
                } else {
                    p.suggestions.len().min(SUGGEST_MAX) as u16
                }
            }
            _ => 0,
        }
    }

    /// 入力中の文字に合う履歴・ブックマークを入力欄の上に出す (Vimium の Vomnibar と同じ)
    fn suggest_rows(&self, width: usize) -> Vec<TLine<'static>> {
        let Some(p) = &self.prompt else { return vec![] };
        if p.suggestions.is_empty() {
            return vec![TLine::from(TSpan::styled(
                t!("(ブックマークはまだありません。gb で追加できます)"),
                TStyle::default().add_modifier(Modifier::DIM),
            ))];
        }
        p.suggestions
            .iter()
            .enumerate()
            .map(|(n, item)| {
                // 1件1行に収め、はみ出す分は … で省く。URL はエンコードを戻して読めるようにする
                let mut parts: Vec<(String, TStyle)> =
                    vec![((if item.bookmark { "★ " } else { "  " }).into(), fg(HEADING_COLOR))];
                if item.kind == "search" {
                    parts.push((t!("検索: {target}", target = item.target), TStyle::default()));
                } else {
                    let title = if item.title.is_empty() { unquote(&item.target) } else { item.title.clone() };
                    parts.push((title, TStyle::default()));
                    parts.push((format!("  {}", unquote(&item.target)), fg(LINK_COLOR)));
                }
                let reverse = Some(n) == p.sel;
                let mut spans = Vec::new();
                let total: usize = parts.iter().map(|(t, _)| t.width()).sum();
                let mut room = width;
                for (t, s) in parts {
                    let t = if total > width && t.width() >= room { truncate(&t, room) } else { t };
                    room = room.saturating_sub(t.width());
                    let s = if reverse { s.add_modifier(Modifier::REVERSED) } else { s };
                    spans.push(TSpan::styled(t, s));
                    if room == 0 {
                        break;
                    }
                }
                TLine::from(spans)
            })
            .collect()
    }

    fn show_tab(&mut self) {
        self.overlay = None;
        self.reset_modes();
        self.scroll_y = self.tab().entry().map_or(0, |e| e.scroll);
        self.scroll_x = 0;
        // 設定を変えたら、取得済みの内容から本文を作り直す (ほかのタブは切り替えたときに作り直す)
        if std::mem::take(&mut self.settings_changed) {
            for (i, t) in self.tabs.iter_mut().enumerate() {
                if i != self.cur && t.doc.is_some() && t.unloaded.is_none() {
                    t.unloaded = t.doc.as_ref().map(|d| d.title.clone());
                }
            }
            let id = self.tab().id;
            self.load(id, false);
            return;
        }
        // 開き直したタブは、初めて表示するときに読み込む
        if self.tab_mut().unloaded.take().is_some() {
            let id = self.tab().id;
            self.load(id, false);
        }
    }

    // ---- 前回のタブ

    /// 開いているタブを保存する。保存できなくても閲覧は続けられるようにする
    fn save_session(&mut self) {
        if fetch::is_private() {
            return; // プライベートモードでは開いていたタブを残さない
        }
        let session = self.session();
        let path = session_path();
        let _ = (|| -> io::Result<()> {
            std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")))?;
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(&session).map_err(io::Error::other)?)?;
            std::fs::rename(&tmp, &path)
        })();
    }

    /// 保存するタブ
    fn session(&mut self) -> Session {
        if self.overlay.is_none() {
            self.remember_scroll();
        }
        let tabs = self
            .tabs
            .iter()
            .filter(|t| !t.history.is_empty())
            .map(|t| SavedTab {
                title: t.doc.as_ref().map(|d| d.title.clone()).or_else(|| t.unloaded.clone()).unwrap_or_default(),
                history: t
                    .history
                    .iter()
                    .map(|e| SavedEntry { kind: e.kind.name().into(), target: e.target.clone(), scroll: e.scroll, fields: e.fields.clone() })
                    .collect(),
                pos: t.pos,
            })
            .collect();
        Session { tabs, cur: self.cur }
    }

    /// 前回のタブを開き直す。開き直したら true。見ていたタブだけを読み込み、ほかは切り替えたときに読み込む
    fn restore_session(&mut self) -> bool {
        if fetch::is_private() {
            return false; // プライベートモードは新しいタブから始める
        }
        let Some(session) =
            std::fs::read(session_path()).ok().and_then(|b| serde_json::from_slice::<Session>(&b).ok())
        else {
            return false;
        };
        let tabs: Vec<Tab> = session
            .tabs
            .into_iter()
            .filter(|t| !t.history.is_empty())
            .map(|t| {
                let mut tab = Tab::new();
                tab.history = t
                    .history
                    .into_iter()
                    .map(|e| Entry { scroll: e.scroll, fields: e.fields, ..Entry::new(EntryKind::from_name(&e.kind), e.target) })
                    .collect();
                tab.pos = t.pos.filter(|p| *p < tab.history.len()).or(Some(tab.history.len() - 1));
                tab.unloaded = Some(t.title);
                tab
            })
            .collect();
        if tabs.is_empty() {
            return false;
        }
        self.cur = session.cur.min(tabs.len() - 1);
        self.tabs = tabs;
        self.show_tab();
        true
    }

    fn reset_modes(&mut self) {
        self.hints.clear();
        self.hint_typed.clear();
        self.find_q.clear();
        self.visual = None;
    }

    // ---- 読み込み
    /// URL なら開き、そうでなければ検索する
    fn go(&mut self, text: &str, new_tab: bool, background: bool) {
        let text = text.trim();
        if !text.is_empty() {
            let entry = if looks_like_url(text) {
                Entry::new(EntryKind::Url, to_url(text))
            } else {
                Entry::new(EntryKind::Search, text)
            };
            self.navigate(entry, new_tab, background);
        }
    }

    fn navigate(&mut self, entry: Entry, new_tab: bool, background: bool) {
        let idx = if new_tab {
            self.tabs.insert(self.cur + 1, Tab::new());
            if !background {
                self.switch_tab(self.cur as isize + 1);
            }
            self.cur + usize::from(background)
        } else {
            self.remember_scroll();
            let t = self.tab_mut();
            if let Some(p) = t.pos {
                t.history.truncate(p + 1);
            }
            self.cur
        };
        let t = &mut self.tabs[idx];
        t.history.push(entry);
        t.pos = Some(t.history.len() - 1);
        let id = t.id;
        self.load(id, false);
    }

    fn load(&mut self, tab: u64, reload: bool) {
        self.stop_speech(Some(tab)); // 読み上げているタブで別のページへ移ったら止める
        if tab == self.tab().id {
            self.overlay = None;
            self.reset_modes();
        }
        let Some(i) = self.tab_index(tab) else { return };
        let Some(entry) = self.tabs[i].entry().cloned() else { return };
        let (loader, tx, full) = (self.loader.clone(), self.tx.clone(), self.full);
        std::thread::spawn(move || {
            let say = |m: String| {
                let _ = tx.send(Msg::Status(tab, m));
            };
            let doc = loader.fetch_doc(&entry, reload, full, &say);
            let _ = tx.send(Msg::Loaded { tab, entry: entry.id, doc });
            // 本文の抽出で一時的に使ったメモリを OS に返す
            crate::fetch::release_memory();
        });
    }

    fn on_msg(&mut self, m: Msg) {
        match m {
            Msg::Status(tab, s) => self.say(tab, s),
            Msg::Notice(s) => self.say_here(s),
            Msg::Loaded { tab, entry, doc } => {
                self.loaded(tab, entry, doc);
                fetch::release_memory();
            }
            Msg::Image { url, result } => self.image_loaded(url, result),
            Msg::SpeechStarted(tab, lang) => {
                self.speech.lang = Some(lang);
                self.say(tab, t!("読み上げ中 (S で停止)"));
            }
            Msg::SpeechBlock { tab, doc, blk } => self.follow_speech(tab, doc, blk),
            Msg::Translated { tab, source, items } => self.translated(tab, source, items),
            Msg::TranslateFailed { tab, source, error } => {
                if let Some(i) = self.tab_index(tab)
                    && self.tabs[i].translation.as_ref().is_some_and(|t| Arc::as_ptr(&t.source) as usize == source)
                {
                    self.say(tab, t!("翻訳できませんでした: {e}", e = error));
                }
            }
            Msg::SelectionTranslated { tab, doc, result } => {
                // 訳している間に別のページへ移ったり、選び直したりしたら出さない
                let here = tab == self.tab().id && self.doc().is_some_and(|d| Arc::as_ptr(&d) as usize == doc);
                match (here, result, self.visual.as_ref()) {
                    (true, Ok(text), Some(v)) => {
                        let ((l0, _), (l1, _)) = v.span();
                        self.popup = Some(Popup { text, lines: (l0, l1) });
                        self.say(tab, t!("訳文を出しました (Esc で閉じる)"));
                    }
                    (_, Err(e), _) => self.say(tab, t!("翻訳できませんでした: {e}", e = e)),
                    _ => {}
                }
            }
            Msg::SpeechDone(tab) => {
                if self.speech.tab == Some(tab) {
                    self.speech.tab = None;
                }
                self.say(tab, t!("読み上げが終わりました"));
            }
        }
    }

    /// 読み込んだ結果を表示する
    fn loaded(&mut self, tab: u64, entry: u64, doc: Arc<Document>) {
        let Some(i) = self.tab_index(tab) else { return };
        // 読み込み中に別のページへ移った
        let Some(e) = self.tabs[i].entry().filter(|e| e.id == entry).cloned() else { return };
        self.tabs[i].doc = Some(doc.clone());
        self.tabs[i].unloaded = None;
        self.fetch_images(&doc);
        if let Some(t) = self.tabs[i].translation.take() {
            t.cancel.store(true, Ordering::Relaxed); // 別のページになったので翻訳をやめる
        }
        self.tabs[i].status.clear();
        if e.kind != EntryKind::Blank
            && doc.note != t!("エラー")
            && !doc.note.starts_with("HTTP 4")
            && !doc.note.starts_with("HTTP 5")
            && !fetch::is_private()
        {
            self.store.visit(e.kind.name(), &e.target, &doc.title); // プライベートモードでは履歴に残さない
        }
        if i == self.cur && self.overlay.is_none() {
            self.reset_modes();
            self.scroll_y = 0;
            self.scroll_to(e.scroll, false);
            let fragment = urldefrag(&e.target).1;
            if !fragment.is_empty() && e.scroll == 0 {
                self.jump_to_anchor(&fragment);
            }
        }
        // 端末ごと閉じられても前回のタブを開き直せるよう、ページを開くたびに保存する
        self.save_session();
    }

    /// #見出し の位置へ移動する
    fn jump_to_anchor(&mut self, fragment: &str) {
        let want = unquote(fragment);
        let blk = self
            .doc()
            .and_then(|d| d.blocks.iter().position(|b| b.anchors.iter().any(|a| unquote(a) == want)));
        let line = blk.and_then(|b| self.block_lines().get(&b).copied());
        match line {
            Some(l) => self.scroll_to(l, true),
            None => self.say_here(t!("#{want} が見つかりません (本文抽出で除かれた可能性があります。a で全体表示)", want = want)),
        }
    }

    fn history_move(&mut self, d: isize) {
        let Some(p) = self.tab().pos else { return };
        let j = p as isize + d;
        if j >= 0 && (j as usize) < self.tab().history.len() {
            self.remember_scroll();
            self.tab_mut().pos = Some(j as usize);
            let id = self.tab().id;
            self.load(id, false);
        }
    }

    /// 表示の仕方 (本文/全体) を変えたときに、先頭から表示し直す
    fn reload_view(&mut self) {
        if let Some(e) = self.tab_mut().entry_mut() {
            e.scroll = 0;
            let id = self.tab().id;
            self.load(id, false);
        }
    }

    fn same_page(&self, url: &str) -> bool {
        let cur = self.tab().doc.as_ref().map(|d| d.url.clone()).unwrap_or_default();
        unquote(&urldefrag(url).0) == unquote(&urldefrag(&cur).0)
    }

    fn toggle_bookmark(&mut self) {
        let (Some(e), Some(d)) = (self.tab().entry().cloned(), self.tab().doc.clone()) else { return };
        if e.kind == EntryKind::Blank {
            return;
        }
        let added = self.store.toggle_bookmark(e.kind.name(), &e.target, &d.title);
        self.say_here(if added { t!("ブックマークに追加しました") } else { t!("ブックマークから外しました") });
    }

    fn current_url(&self) -> Option<String> {
        let e = self.tab().entry()?;
        (e.kind == EntryKind::Url).then(|| self.tab().doc.as_ref().map(|d| d.url.clone())).flatten()
    }

    fn go_up(&mut self, root: bool) {
        if let Some(url) = self.current_url() {
            self.navigate(Entry::new(EntryKind::Url, parent_url(&url, root)), false, false);
        }
    }

    // ---- タブ
    fn switch_tab(&mut self, i: isize) {
        let i = i.rem_euclid(self.tabs.len() as isize) as usize;
        if i == self.cur {
            return;
        }
        self.remember_scroll();
        self.prev_tab = Some(self.tab().id);
        self.cur = i;
        self.show_tab();
    }

    fn close_tab(&mut self) {
        self.remember_scroll();
        let id = self.tab().id;
        self.stop_speech(Some(id));
        let t = self.tabs.remove(self.cur);
        self.closed.push(t);
        if self.tabs.is_empty() {
            // 最後のタブを閉じたら終了する (ブラウザで最後のタブを閉じたときと同じ)
            self.quit = true;
            return;
        }
        self.cur = self.cur.min(self.tabs.len() - 1);
        self.show_tab();
    }

    fn restore_tab(&mut self) {
        let Some(t) = self.closed.pop() else {
            self.say_here(t!("戻せるタブはありません"));
            return;
        };
        self.tabs.insert(self.cur + 1, t);
        self.switch_tab(self.cur as isize + 1);
    }

    fn duplicate_tab(&mut self) {
        let t = self.tab();
        let mut new = Tab::new();
        new.history = t.history.iter().map(Entry::copy).collect();
        new.pos = t.pos;
        new.doc = t.doc.clone();
        self.tabs.insert(self.cur + 1, new);
        self.switch_tab(self.cur as isize + 1);
    }

    fn move_tab(&mut self, d: isize) {
        let j = self.cur as isize + d;
        if j >= 0 && (j as usize) < self.tabs.len() {
            self.tabs.swap(self.cur, j as usize);
            self.cur = j as usize;
        }
    }

    fn previous_tab(&mut self) {
        if let Some(i) = self.prev_tab.and_then(|id| self.tab_index(id)) {
            self.switch_tab(i as isize);
        }
    }

    // ---- マウス
    /// クリック。リンクを開く (中ボタンか Ctrl+クリックは新しいタブ)、タブを切り替える (中ボタンは閉じる)、
    /// 題名 (URL) を編集する、入力欄の候補を選ぶ・カーソルを移す。ほかの操作はキーで行う
    fn on_click(&mut self, button: MouseButton, x: u16, y: u16, mods: crossterm::event::KeyModifiers) {
        use crossterm::event::KeyModifiers;
        let middle = button == MouseButton::Middle;
        self.press = None;
        self.dragging = false;
        if self.popup.take().is_some() {
            return;
        }
        if button == MouseButton::Right {
            return;
        }
        if !self.hints.is_empty() {
            self.hints.clear();
            self.hint_typed.clear();
            self.say_here("");
            return;
        }
        if self.prompt.is_some() {
            return self.click_prompt(x, y);
        }
        if self.tabs.len() > 1 && y == 0 {
            let Some(&(_, _, i)) = self.hit.tabs.iter().find(|(a, b, _)| (*a..*b).contains(&x)) else { return };
            if !middle {
                self.switch_tab(i as isize);
            } else if i == self.cur {
                self.close_tab();
            } else {
                // 見ているタブはそのままで、ほかのタブを閉じる
                self.remember_scroll();
                let keep = self.tab().id;
                self.stop_speech(Some(self.tabs[i].id));
                let t = self.tabs.remove(i);
                self.closed.push(t);
                self.cur = self.tab_index(keep).unwrap_or(0);
            }
            return;
        }
        if y == self.hit.title_y {
            let u = self.current_url().unwrap_or_default();
            return self.open_prompt(PromptMode::Open, &u, middle);
        }
        if self.visual.is_some() {
            // 選んでいるときにクリックしたら、選択を解く
            self.exit_visual("");
        }
        let body = self.body_rect();
        if !(body.y..body.y + body.height).contains(&y) || x < body.x || self.overlay.is_some() && button != MouseButton::Left {
            return;
        }
        if middle || mods.contains(KeyModifiers::CONTROL) {
            let at = self.body_pos(x, y);
            if let Some(l) = self.link_at(at) {
                self.open_link(l, HintAction::Tab);
            }
            return;
        }
        // 左ボタンは、離したときに動かしていなければリンクを開き、ドラッグしたら文字を選ぶ
        self.press = Some(self.body_pos(x, y));
    }

    /// 本文の画面の位置 → (行, 文字の位置)。本文の外なら、いちばん近い本文の端
    fn body_pos(&mut self, x: u16, y: u16) -> (usize, usize) {
        let body = self.body_rect();
        let y = y.clamp(body.y, (body.y + body.height).saturating_sub(1));
        let line = self.scroll_y + (y - body.y) as usize;
        let col = self.scroll_x + x.saturating_sub(body.x) as usize;
        let lines = self.layout();
        let Some(l) = lines.get(line) else { return (line, 0) };
        let mut w = 0;
        for (i, c) in l.text().chars().enumerate() {
            w += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            if col < w {
                return (line, i);
            }
        }
        (line, l.text().chars().count())
    }

    /// (行, 文字の位置) にあるリンク (Document.links のインデックス)
    fn link_at(&mut self, at: (usize, usize)) -> Option<usize> {
        self.occ_at(at).map(|(_, l)| l)
    }

    /// (行, 文字の位置) にあるリンクの (出現番号, Document.links のインデックス)
    fn occ_at(&mut self, (line, ch): (usize, usize)) -> Option<(usize, usize)> {
        let lines = self.layout();
        let mut n = 0;
        for s in &lines.get(line)?.segs {
            n += s.text.chars().count();
            if ch < n {
                return s.occ.zip(s.link);
            }
        }
        None
    }

    /// マウスの下にあるリンク。本文の上で、入力欄・選択・ヒントがないときだけ
    fn hovered(&mut self) -> Option<(usize, usize)> {
        let (x, y) = self.mouse?;
        let body = self.body_rect();
        if !(body.y..body.y + body.height).contains(&y) || x < body.x || self.prompt.is_some() || self.visual.is_some() || !self.hints.is_empty() {
            return None;
        }
        let at = self.body_pos(x, y);
        self.occ_at(at)
    }

    /// 左ボタンでドラッグしている。押した位置から今の位置までを選ぶ。本文の上下の外へ出たらスクロールする
    fn on_drag(&mut self, x: u16, y: u16) {
        let Some(anchor) = self.press else { return };
        let body = self.body_rect();
        if y < body.y {
            self.scroll_by(-1);
        } else if y >= body.y + body.height {
            self.scroll_by(1);
        }
        let cur = self.body_pos(x, y);
        if !self.dragging {
            if cur == anchor || self.overlay.is_some() || self.doc().is_none_or(|d| d.blocks.is_empty()) {
                return;
            }
            self.dragging = true;
            self.hints.clear();
            self.find_q.clear();
            let lines = self.layout().to_vec();
            self.visual = Some(Visual::new(lines, anchor.0, Mode::Char));
        }
        if let Some(v) = &mut self.visual {
            v.select(anchor, cur);
        }
        self.tab_mut().status = t!("選択中: 離すとコピー").into();
    }

    /// 左ボタンを離した。ドラッグしていたら選んだ文字をコピーし、選択は残す (S で読み上げ、Esc で解除)。
    /// 動かしていなければ、押した所のリンクを開く
    fn on_release(&mut self) {
        let Some(at) = self.press.take() else { return };
        if std::mem::take(&mut self.dragging) {
            let Some(v) = &self.visual else { return };
            let text = v.selected_text();
            self.yank(&text);
            let n = text.chars().count();
            self.say_here(t!("{n} 文字をコピーしました (S で読み上げ、E で翻訳、Esc で解除)", n = n));
        } else if let Some(l) = self.link_at(at) {
            self.open_link(l, HintAction::Open);
        }
    }

    /// 入力欄を開いているときのクリック。候補なら開き、入力欄ならカーソルを移す。ほかの所なら入力欄を閉じる
    fn click_prompt(&mut self, x: u16, y: u16) {
        let (sy, sh) = self.hit.suggest;
        if (sy..sy + sh).contains(&y) {
            let i = (y - sy) as usize;
            if let Some(p) = &mut self.prompt
                && i < p.suggestions.len()
            {
                p.sel = Some(i);
                self.prompt_submit();
            }
            return;
        }
        if let Some((py, skip)) = self.hit.prompt
            && y == py
        {
            let Some(p) = &mut self.prompt else { return };
            // クリックした文字の前にカーソルを置く (全角文字の右半分なら後ろ)
            let col = skip + x.saturating_sub(1) as usize;
            let mut w = 0;
            p.cursor = p.text.len();
            for (i, c) in p.text.iter().enumerate() {
                let cw = unicode_width::UnicodeWidthChar::width(*c).unwrap_or(0);
                if col < w + cw {
                    p.cursor = if cw > 1 && col > w { i + 1 } else { i };
                    break;
                }
                w += cw;
            }
            return;
        }
        if self.prompt.as_ref().is_some_and(|p| p.mode == PromptMode::Find) {
            self.find_q.clear();
        }
        self.close_prompt();
    }

    // ---- ヒント (f / F / yf)
    fn start_hints(&mut self, action: HintAction) {
        // ヘルプ・設定などの特別なページでも使う (設定画面の項目はリンクで切り替える)
        if self.doc().is_none() {
            return;
        }
        let (y0, h) = (self.scroll_y, self.height());
        let mut order: Vec<usize> = Vec::new();
        for line in self.layout().iter().skip(y0).take(h) {
            for (o, _) in line.occs() {
                if !order.contains(&o) {
                    order.push(o);
                }
            }
        }
        if order.is_empty() {
            self.say_here(t!("画面内にリンクがありません"));
            return;
        }
        self.hints = hint_labels(order.len()).into_iter().zip(order).collect();
        self.hint_typed.clear();
        self.hint_action = action;
        self.say_here(action.label());
    }

    fn hint_key(&mut self, key: &str, ch: Option<char>) {
        if key == "escape" {
            self.hints.clear();
            self.say_here("");
            return;
        }
        if key == "backspace" {
            self.hint_typed.pop();
        } else if let Some(c) = ch.map(|c| c.to_ascii_lowercase()).filter(|c| HINT_CHARS.contains(*c)) {
            let typed = format!("{}{c}", self.hint_typed);
            if !self.hints.iter().any(|(lab, _)| lab.starts_with(&typed)) {
                return;
            }
            self.hint_typed = typed.clone();
            if let Some(&(_, occ)) = self.hints.iter().find(|(lab, _)| *lab == typed) {
                self.layout();
                let link = self.occ_links.get(&occ).copied();
                let action = self.hint_action;
                self.hints.clear();
                self.hint_typed.clear();
                if let Some(l) = link {
                    self.open_link(l, action);
                }
            }
        }
    }

    fn open_link(&mut self, i: usize, action: HintAction) {
        let Some(url) = self.doc().and_then(|d| d.links.get(i).cloned()) else { return };
        if action == HintAction::Yank {
            self.yank(&url);
            return;
        }
        self.open_target(&url, action == HintAction::Tab);
        if action == HintAction::Tab {
            self.say_here(t!("新しいタブで開きました"));
        }
    }

    /// リンク先を開く。検索の「次の検索結果」と、同じページ内のリンク (目次・脚注など) は特別に扱う
    fn open_target(&mut self, url: &str, new_tab: bool) {
        if let Some(name) = url.strip_prefix(crate::settings::SETTING_SCHEME) {
            return self.toggle_setting(name);
        }
        if let Some(i) = url.strip_prefix("yomu-form:").and_then(|i| i.parse::<usize>().ok()) {
            let f = self.form_source.as_ref().and_then(|d| d.forms.get(i).cloned());
            self.show_tab();
            if let Some(f) = f {
                self.open_form(f);
            }
            return;
        }
        if let Some(rest) = url.strip_prefix(crate::history::HISTORY_SCHEME) {
            if let Some(r) = rest.strip_prefix("clear/").and_then(crate::history::ClearRange::from_name) {
                self.confirm = Some(Confirm::ClearHistory(r));
                return self.say_here(t!("{what}を消しますか? (y/n)", what = r.label()));
            }
            if let Some(q) = rest.strip_prefix("search:") {
                return self.navigate(Entry::new(EntryKind::Search, q), new_tab, new_tab);
            }
            return;
        }
        if url.starts_with(PAGE_SCHEME) {
            let fields = search::page_fields(url);
            let q = search::field(&fields, "q").unwrap_or_default().to_string();
            if let Some(g) = search::field(&fields, GOTO) {
                self.open_search_page(&q, g.parse().unwrap_or(1), new_tab);
            } else {
                let mut e = Entry::new(EntryKind::Search, q);
                e.fields = Some(fields);
                self.navigate(e, new_tab, new_tab);
            }
        } else if !new_tab && !urldefrag(url).1.is_empty() && self.same_page(url) {
            self.jump_to_anchor(&urldefrag(url).1);
        } else {
            self.navigate(Entry::new(EntryKind::Url, url), new_tab, new_tab);
        }
    }

    /// 「前の検索結果」: タブの履歴から、同じ検索語の page ページ目を探して開き直す
    fn open_search_page(&mut self, query: &str, page: usize, new_tab: bool) {
        let page_of = |e: &Entry| {
            e.fields.as_ref().and_then(|f| search::field(f, PAGE)).and_then(|p| p.parse().ok()).unwrap_or(1usize)
        };
        let mut found = self
            .tab()
            .history
            .iter()
            .rev()
            .find(|e| e.kind == EntryKind::Search && e.target == query && page_of(e) == page)
            .cloned();
        if found.is_none() && page == 1 {
            found = Some(Entry::new(EntryKind::Search, query));
        }
        match found {
            Some(e) => {
                let mut e = e.copy();
                e.scroll = 0;
                self.navigate(e, new_tab, new_tab);
            }
            None => self.say_here(t!("{page} ページ目が履歴にありません (H で戻れます)", page = page)),
        }
    }

    /// [[ / ]]: 「前へ」「次へ」らしいリンクを探して開く (後ろにあるものを優先)
    fn follow_rel(&mut self, pattern: &Regex) {
        if let Some(d) = self.doc() {
            for b in d.blocks.iter().rev() {
                for s in &b.spans {
                    if let Some(l) = s.link
                        && pattern.is_match(&s.text)
                    {
                        let url = d.links[l].clone();
                        self.open_target(&url, false);
                        return;
                    }
                }
            }
        }
        self.say_here(t!("該当するリンクがありません"));
    }

    // ---- ページ内検索 (/ n N)
    fn find_jump(&mut self, q: &str, step: isize, include_current: bool) {
        let ql = q.to_lowercase();
        let hits: Vec<usize> = if q.is_empty() {
            vec![]
        } else {
            self.layout().iter().enumerate().filter(|(_, l)| l.text().to_lowercase().contains(&ql)).map(|(i, _)| i).collect()
        };
        if hits.is_empty() {
            self.say_here(if q.is_empty() { String::new() } else { t!("見つかりません: {q}", q = q) });
            return;
        }
        let h = self.height();
        let here = self.scroll_y + h / 3;
        let target = if step > 0 {
            hits.iter().copied().find(|&x| x > here || (include_current && x == here)).unwrap_or(hits[0])
        } else {
            hits.iter().rev().copied().find(|&x| x < here).unwrap_or(*hits.last().unwrap())
        };
        self.scroll_to(target.saturating_sub(h / 3), true);
        let n = hits.iter().position(|&x| x == target).unwrap() + 1;
        self.say_here(format!("/{q}  {n}/{}", hits.len()));
    }

    fn find_again(&mut self, step: isize) {
        self.find_q = self.last_find.clone();
        let q = self.last_find.clone();
        self.find_jump(&q, step, false);
    }

    // ---- 入力欄 (o / O / ge / gE / s / /)
    fn open_prompt(&mut self, mode: PromptMode, initial: &str, new_tab: bool) {
        let text: Vec<char> = initial.chars().collect();
        let cursor = text.len();
        self.prompt = Some(Prompt { mode, new_tab, text, cursor, suggestions: vec![], sel: None });
        self.update_suggestions();
        self.ime_restore();
    }

    fn close_prompt(&mut self) {
        self.prompt = None;
        self.ime_normal();
    }

    fn update_suggestions(&mut self) {
        let Some(p) = &mut self.prompt else { return };
        if !p.mode.suggests() {
            return;
        }
        p.suggestions = self.store.suggest(&p.value(), p.mode == PromptMode::Bookmark, 10);
        p.sel = None;
    }

    fn move_suggestion(&mut self, d: isize) {
        let Some(p) = &mut self.prompt else { return };
        if p.suggestions.is_empty() {
            return;
        }
        // 入力した文字 → 0 … n-1 → 入力した文字 と巡回する
        let n = p.suggestions.len() as isize + 1;
        let cur = p.sel.map_or(0, |s| s as isize + 1);
        let next = (cur + d).rem_euclid(n);
        p.sel = if next == 0 { None } else { Some(next as usize - 1) };
    }

    fn prompt_changed(&mut self) {
        let Some(p) = &self.prompt else { return };
        let (mode, value) = (p.mode, p.value());
        if mode.suggests() {
            self.update_suggestions();
        }
        if mode == PromptMode::Find {
            self.find_q = value.clone();
            if !value.is_empty() {
                self.find_jump(&value, 1, true);
            }
        }
    }

    fn prompt_submit(&mut self) {
        let Some(p) = self.prompt.take() else { return };
        let chosen = p.sel.and_then(|i| p.suggestions.get(i).cloned());
        self.close_prompt();
        let v = p.value().trim().to_string();
        if let Some(c) = chosen {
            self.navigate(Entry::new(EntryKind::from_name(&c.kind), c.target), p.new_tab, false);
        } else if p.mode == PromptMode::Form {
            // ページの入力欄に入れた文字で送る (GET)
            if let Some(f) = self.form.take().filter(|_| !v.is_empty()) {
                self.navigate(Entry::new(EntryKind::Url, f.url(&v)), p.new_tab, false);
            }
        } else if p.mode == PromptMode::Find {
            self.last_find = v;
        } else if !v.is_empty() && p.mode == PromptMode::Search {
            self.navigate(Entry::new(EntryKind::Search, v), p.new_tab, false);
        } else if !v.is_empty() {
            self.go(&v, p.new_tab, false);
        }
    }

    fn prompt_key(&mut self, k: &KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        // Emacs 風のキー (Ctrl+N / P / A / E / D) は、同じ働きのキーに読み替える
        let code = match (k.code, ctrl) {
            (KeyCode::Char('n'), true) => KeyCode::Down,
            (KeyCode::Char('p'), true) => KeyCode::Up,
            (KeyCode::Char('a'), true) => KeyCode::Home,
            (KeyCode::Char('e'), true) => KeyCode::End,
            (KeyCode::Char('d'), true) => KeyCode::Delete,
            (c, _) => c,
        };
        match code {
            KeyCode::Esc => {
                if self.prompt.as_ref().is_some_and(|p| p.mode == PromptMode::Find) {
                    self.find_q.clear();
                }
                self.close_prompt();
                return;
            }
            KeyCode::Enter => return self.prompt_submit(),
            KeyCode::Down | KeyCode::Tab => return self.move_suggestion(1),
            KeyCode::Up | KeyCode::BackTab => return self.move_suggestion(-1),
            _ => {}
        }
        let Some(p) = &mut self.prompt else { return };
        let before = p.text.clone();
        match code {
            KeyCode::Left => p.cursor = p.cursor.saturating_sub(1),
            KeyCode::Right => p.cursor = (p.cursor + 1).min(p.text.len()),
            KeyCode::Home => p.cursor = 0,
            KeyCode::End => p.cursor = p.text.len(),
            KeyCode::Backspace => {
                if p.cursor > 0 {
                    p.cursor -= 1;
                    p.text.remove(p.cursor);
                }
            }
            KeyCode::Delete => {
                if p.cursor < p.text.len() {
                    p.text.remove(p.cursor);
                }
            }
            KeyCode::Char('u') if ctrl => {
                p.text.drain(..p.cursor);
                p.cursor = 0;
            }
            KeyCode::Char('k') if ctrl => p.text.truncate(p.cursor),
            KeyCode::Char('w') if ctrl => {
                // 前の語を消す
                let mut i = p.cursor;
                while i > 0 && p.text[i - 1] == ' ' {
                    i -= 1;
                }
                while i > 0 && p.text[i - 1] != ' ' {
                    i -= 1;
                }
                p.text.drain(i..p.cursor);
                p.cursor = i;
            }
            KeyCode::Char(c) if !ctrl => {
                p.text.insert(p.cursor, c);
                p.cursor += 1;
            }
            _ => {}
        }
        if p.text != before {
            self.prompt_changed();
        }
    }

    fn on_paste(&mut self, s: &str) {
        if let Some(p) = &mut self.prompt {
            for c in s.chars().filter(|c| *c != '\n' && *c != '\r') {
                p.text.insert(p.cursor, c);
                p.cursor += 1;
            }
            self.prompt_changed();
        }
    }

    // ---- マーク (m / `)
    fn set_mark(&mut self, ch: char) {
        let Some(e) = self.tab().entry().cloned() else { return };
        if !ch.is_alphabetic() {
            return;
        }
        if ch.is_uppercase() {
            let mut dest = e.copy();
            dest.scroll = 0;
            self.global_marks.insert(ch, (dest, self.scroll_y));
        } else {
            self.marks.insert((e.target.clone(), ch), self.scroll_y);
        }
        self.say_here(t!("マーク {ch} を記録しました", ch = ch));
    }

    fn jump_mark(&mut self, ch: char) {
        let e = self.tab().entry().cloned();
        if ch == '`' {
            if let Some(y) = self.tab().jump_from {
                let from = self.scroll_y;
                self.tab_mut().jump_from = Some(from);
                self.scroll_to(y, false);
            }
        } else if let Some((dest, y)) = self.global_marks.get(&ch).cloned() {
            if e.as_ref().is_some_and(|e| (e.kind, &e.target) == (dest.kind, &dest.target)) {
                self.scroll_to(y, true);
            } else {
                let mut d = dest.copy();
                d.scroll = y;
                self.navigate(d, false, false);
            }
        } else if let Some(y) = e.and_then(|e| self.marks.get(&(e.target, ch)).copied()) {
            self.scroll_to(y, true);
        } else {
            self.say_here(t!("マーク {ch} はありません", ch = ch));
        }
    }

    // ---- ビジュアルモード (v / V)
    fn start_visual(&mut self, mode: Mode) {
        if self.overlay.is_some() || self.doc().is_none_or(|d| d.blocks.is_empty()) {
            return;
        }
        self.hints.clear();
        self.find_q.clear();
        let lines = self.layout().to_vec();
        self.visual = Some(Visual::new(lines, self.scroll_y, mode));
        self.visual_count.clear();
        self.visual_pending.clear();
        self.draw_visual();
    }

    fn draw_visual(&mut self) {
        let Some(v) = &self.visual else { return };
        let help = match v.mode() {
            Mode::Caret => t!("キャレット: 移動して v で選択を開始 (V で行単位)、S でここから読み上げ、Esc で終了"),
            Mode::Char => t!("選択中: y でコピー、E で翻訳、o で反対の端へ、c でキャレットに戻る、Esc で終了"),
            Mode::Line => t!("行単位で選択中: y でコピー、E で翻訳、o で反対の端へ、Esc で終了"),
        };
        let line = v.cursor().0;
        self.tab_mut().status = help.into();
        let h = self.height();
        if line < self.scroll_y {
            self.scroll_to(line, false);
        } else if line >= self.scroll_y + h {
            self.scroll_to(line + 1 - h, false);
        }
    }

    fn exit_visual(&mut self, msg: &str) {
        self.visual = None;
        self.say_here(msg);
    }

    fn visual_key(&mut self, key: &str, ch: Option<char>) {
        if key == "escape" {
            return self.exit_visual("");
        }
        if let Some(c) = ch.filter(|c| c.is_ascii_digit() && (*c != '0' || !self.visual_count.is_empty())) {
            self.visual_count.push(c);
            return;
        }
        let n: usize = self.visual_count.parse().unwrap_or(1);
        self.visual_count.clear();
        let cmd = format!("{}{}", self.visual_pending, ch.map(String::from).unwrap_or_default());
        self.visual_pending.clear();
        let Some(v) = &mut self.visual else { return };
        match cmd.as_str() {
            "g" => self.visual_pending = "g".into(),
            "gg" | "h" | "j" | "k" | "l" | "w" | "b" | "e" | "0" | "^" | "$" | "G" | "{" | "}" => v.move_cursor(&cmd, n),
            "o" => v.swap(),
            "v" | "V" | "c" => {
                let mode = match cmd.as_str() {
                    "v" => Mode::Char,
                    "V" => Mode::Line,
                    _ => Mode::Caret,
                };
                if v.mode() == mode && cmd != "c" {
                    return self.exit_visual("");
                }
                v.set_mode(if v.mode() == Mode::Caret && cmd == "c" { Mode::Char } else { mode });
            }
            "y" => {
                let text = v.selected_text();
                self.exit_visual("");
                return self.yank(&text);
            }
            // ページの対訳 (E) と同じく、原文を残して訳文を下に出す。e は Vimium と同じく語の終わりへの移動
            "E" if v.mode() != Mode::Caret => {
                let texts: Vec<String> = v.selected_blocks().into_iter().map(|(_, t)| t).collect();
                return self.translate_selection(texts);
            }
            "S" => {
                // 選んでいれば選んだ文だけを、キャレットならカーソルの文から最後まで読み上げる
                if v.mode() == Mode::Caret {
                    let at = v.block_at(v.cursor());
                    self.exit_visual("");
                    if let Some((blk, rest)) = at {
                        self.stop_speech(None);
                        self.start_speech(blk, rest);
                    }
                } else {
                    let selected = v.selected_blocks();
                    self.exit_visual("");
                    self.stop_speech(None);
                    self.speak_selection(selected);
                }
                return;
            }
            _ => {}
        }
        self.draw_visual();
    }

    /// 選んだ文を訳して、選んだ所の近くに枠で出す (ページは書き換えない)
    fn translate_selection(&mut self, texts: Vec<String>) {
        if texts.is_empty() {
            return;
        }
        if fetch::is_private() {
            return self.say_here(private_no_translate());
        }
        if !crate::settings::get().translate_consent {
            return self.ask_translate(PendingTranslation::Selection(texts));
        }
        let Some(doc) = self.doc() else { return };
        let (tab, doc, tx) = (self.tab().id, Arc::as_ptr(&doc) as usize, self.tx.clone());
        self.say_here(t!("選んだ文を翻訳中…"));
        std::thread::spawn(move || {
            let result = translate::translate_selection(&fetch::client(), &texts);
            let _ = tx.send(Msg::SelectionTranslated { tab, doc, result });
        });
    }

    // ---- その他の操作
    fn yank(&mut self, s: &str) {
        let ok = std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut c| {
                c.stdin.take().unwrap().write_all(s.as_bytes())?;
                c.wait()
            })
            .is_ok_and(|st| st.success());
        if !ok {
            // OSC 52 に対応した端末ならこれでコピーされる
            let _ = write!(io::stdout(), "\x1b]52;c;{}\x07", base64(s.as_bytes()));
            let _ = io::stdout().flush();
        }
        self.say_here(t!("コピーしました: {s}", s = s));
    }

    fn paste_go(&mut self, new_tab: bool) {
        let text = clipboard_get();
        if !text.trim().is_empty() {
            self.go(text.trim(), new_tab, false);
        }
    }

    /// gi: ページの入力欄 (検索のフォームなど) に入力する。1 つならすぐ入力欄を開き、複数あれば一覧から選ぶ
    fn focus_input(&mut self) {
        let Some(d) = self.tab().doc.clone().filter(|_| self.overlay.is_none()) else { return };
        let gets: Vec<&crate::doc::Form> = d.forms.iter().filter(|f| f.get).collect();
        match (gets.len(), d.forms.len()) {
            (0, 0) => self.say_here(t!("このページには入力欄がありません")),
            (0, _) => self.say_here(t!("このページの入力欄は POST で送るフォームなので使えません (ログインなどは未対応)")),
            (1, 1) => {
                let f = gets[0].clone();
                self.open_form(f);
            }
            _ => {
                self.remember_scroll();
                self.reset_modes();
                self.overlay = Some(Arc::new(crate::pages::forms_page(&d.forms)));
                self.form_source = Some(d);
                self.scroll_y = 0;
            }
        }
    }

    fn open_form(&mut self, f: crate::doc::Form) {
        self.form = Some(f);
        self.open_prompt(PromptMode::Form, "", false);
    }

    /// gh: 履歴の画面を開く・閉じる
    fn toggle_history(&mut self) {
        if self.overlay.as_ref().is_some_and(|d| d.url == "about:history") {
            return self.show_tab();
        }
        if self.overlay.is_none() {
            self.remember_scroll();
        }
        self.reset_modes();
        self.overlay = Some(Arc::new(crate::history::history_page(&self.store.history.items)));
        self.scroll_y = 0;
    }

    /// 履歴を消して、履歴の画面を作り直す
    fn clear_history(&mut self, r: crate::history::ClearRange) {
        let n = self.store.history.remove_since(r.since());
        if self.overlay.as_ref().is_some_and(|d| d.url == "about:history") {
            self.overlay = Some(Arc::new(crate::history::history_page(&self.store.history.items)));
            self.scroll_y = 0;
        }
        self.say_here(t!("{what}を消しました ({n} 件)", what = r.label(), n = n));
    }

    /// gs: 設定画面を開く・閉じる
    fn toggle_settings(&mut self) {
        if self.overlay.as_ref().is_some_and(|d| d.url == "about:settings") {
            return self.show_tab();
        }
        if self.overlay.is_none() {
            self.remember_scroll();
        }
        self.reset_modes();
        self.overlay = Some(Arc::new(crate::settings::settings_page()));
        self.scroll_y = 0;
    }

    /// 設定画面の項目を切り替える
    fn toggle_setting(&mut self, name: &str) {
        if name == "private" {
            return self.toggle_private();
        }
        // 言語 (lang:コード。auto は自動): すぐ替え、開いているページは取得済みの内容から作り直す (ページの中の文言も替わるように)
        if let Some(code) = name.strip_prefix("lang:") {
            let lang = crate::settings::set_lang(crate::i18n::Lang::from_locale(code));
            crate::i18n::set_lang(lang);
            self.settings_changed = true;
            self.overlay = Some(Arc::new(crate::settings::settings_page()));
            return self.say_here(t!("言語を {lang} にしました", lang = lang.name()));
        }
        let Some(on) = crate::settings::toggle(name) else { return };
        match name {
            // Cookie: オンにしたら今の Cookie をすぐ残し、オフにしたら残したファイルを消す
            "cookies" if on && fetch::CookieJar::uses_file() => fetch::COOKIES.save(),
            "cookies" if on => {}
            "cookies" => fetch::CookieJar::forget_file(),
            _ => self.settings_changed = true,
        }
        self.overlay = Some(Arc::new(crate::settings::settings_page()));
        self.say_here(if on { t!("オンにしました") } else { t!("オフにしました") });
    }

    fn toggle_help(&mut self) {
        if self.overlay.is_some() {
            return self.show_tab();
        }
        self.remember_scroll();
        self.reset_modes();
        self.overlay = Some(Arc::new(help_page()));
        self.scroll_y = 0;
    }

    /// 今のページを普段のブラウザで開く (JavaScript がないと読めないページ用)
    fn open_in_browser(&mut self) {
        if fetch::is_private() {
            return self.say_here(t!("プライベートモードでは普段のブラウザで開けません (Tor を通らないため)"));
        }
        let Some(url) = self.current_url().filter(|u| u.starts_with("http")) else {
            return self.say_here(t!("ブラウザで開けるページではありません"));
        };
        match std::process::Command::new("open").arg(&url).status() {
            Ok(s) if s.success() => self.say_here(t!("ブラウザで開きました: {url}", url = url)),
            _ => self.say_here(t!("ブラウザで開けませんでした")),
        }
    }

    fn toggle_full(&mut self) {
        self.full = !self.full;
        self.reload_view();
    }

    // ---- 翻訳 (e / E)

    /// e は訳文に置き換え、E は原文の下に訳文を置く (対訳)。同じキーをもう一度押すと原文に戻す。
    /// 画面に見えている段落から先に訳す
    /// 初めて翻訳するときは、本文を Google に送ってよいかを y/n で聞く (y なら覚えておき、次からは聞かない)
    fn ask_translate(&mut self, pending: PendingTranslation) {
        self.confirm = Some(Confirm::Translate(pending));
        self.say_here(t!("翻訳すると、本文が Google 翻訳に送られます。翻訳しますか? (y/n。y なら次からは聞きません)"));
    }

    fn translate_page(&mut self, mode: translate::Mode) {
        if self.overlay.is_some() {
            return;
        }
        if fetch::is_private() {
            return self.say_here(private_no_translate());
        }
        if !crate::settings::get().translate_consent {
            return self.ask_translate(PendingTranslation::Page(mode));
        }
        let (i, tab) = (self.cur, self.tab().id);
        let top = self.top_block();
        if self.tabs[i].translation.is_none() {
            let Some(doc) = self.tabs[i].doc.clone() else { return };
            let targets = translate::targets(&doc);
            if targets.is_empty() {
                return self.say(tab, t!("訳す文章がありません"));
            }
            self.tabs[i].translation = Some(Translation {
                mode: None,
                index: (0..doc.blocks.len()).collect(),
                source: doc,
                targets,
                done: HashMap::new(),
                finished: HashSet::new(),
                cancel: Arc::new(AtomicBool::new(false)),
            });
        }
        let t = self.tabs[i].translation.as_mut().unwrap();
        let src_top = top.map(|(b, off)| (source_block(&t.index, b), off));
        match t.mode {
            Some(m) if m == mode => {
                t.cancel.store(true, Ordering::Relaxed);
                t.mode = None;
                t.index = (0..t.source.blocks.len()).collect();
                self.tabs[i].doc = Some(t.source.clone());
                self.keep_top(src_top);
                return self.say(tab, t!("原文に戻しました"));
            }
            Some(_) => t.mode = Some(mode),
            None => {
                t.mode = Some(mode);
                self.request_translation(i, src_top.map_or(0, |t| t.0));
            }
        }
        self.show_translation(i, src_top);
        let msg = self.translation_status(i);
        self.say(tab, msg);
    }

    /// 訳していない段落を、top の段落から先に (画面に見えているところから) 裏で訳す
    fn request_translation(&mut self, i: usize, top: usize) {
        let tab = self.tabs[i].id;
        let Some(t) = &mut self.tabs[i].translation else { return };
        let rest: Vec<usize> = t.targets.iter().copied().filter(|b| !t.finished.contains(b)).collect();
        if rest.is_empty() {
            return;
        }
        let order: Vec<usize> = rest.iter().copied().filter(|&b| b >= top).chain(rest.iter().copied().filter(|&b| b < top)).collect();
        let batches = translate::batches(&t.source, &order);
        t.cancel = Arc::new(AtomicBool::new(false));
        let (cancel, doc, tx) = (t.cancel.clone(), t.source.clone(), self.tx.clone());
        let source = Arc::as_ptr(&doc) as usize;
        std::thread::spawn(move || {
            let client = fetch::client();
            for batch in batches {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let texts: Vec<String> = batch.iter().map(|(_, t)| t.clone()).collect();
                match translate::translate(&client, &texts) {
                    Ok(res) => {
                        let items = batch
                            .iter()
                            .zip(res)
                            .map(|((b, _), (text, lang))| {
                                // Google が訳し先の言語と判定したものは原文のまま
                                let keep = lang == translate::target_lang() || text.trim().is_empty();
                                (*b, (!keep).then(|| translate::response_spans(&doc.blocks[*b], &text)))
                            })
                            .collect();
                        let _ = tx.send(Msg::Translated { tab, source, items });
                    }
                    Err(error) => {
                        let _ = tx.send(Msg::TranslateFailed { tab, source, error });
                        return;
                    }
                }
            }
        });
    }

    /// 訳文が届いた
    fn translated(&mut self, tab: u64, source: usize, items: Vec<(usize, Option<Vec<DocSpan>>)>) {
        let Some(i) = self.tab_index(tab) else { return };
        let top = if i == self.cur { self.top_block() } else { None };
        let Some(t) = self.tabs[i].translation.as_mut().filter(|t| Arc::as_ptr(&t.source) as usize == source) else { return };
        let src_top = top.map(|(b, off)| (source_block(&t.index, b), off));
        for (b, spans) in items {
            t.finished.insert(b);
            if let Some(spans) = spans {
                t.done.insert(b, spans);
            }
        }
        if t.mode.is_none() {
            return; // 原文に戻した後に届いたものは、覚えておくだけ
        }
        self.show_translation(i, src_top);
        let msg = self.translation_status(i);
        self.say(tab, msg);
    }

    fn translation_status(&self, i: usize) -> String {
        let Some(t) = &self.tabs[i].translation else { return String::new() };
        let key = if t.mode == Some(translate::Mode::Replace) { "e" } else { "E" };
        if t.finished.len() < t.targets.len() {
            t!("翻訳中 {done}/{total} (Google 翻訳。{key} で原文に戻す)", done = t.finished.len(), total = t.targets.len(), key = key)
        } else if t.done.is_empty() {
            t!("日本語のページなので翻訳しませんでした").into()
        } else {
            t!("翻訳しました (Google 翻訳。{key} で原文に戻す)", key = key)
        }
    }

    /// 原文に訳文を当てはめた文書を表示する。top (原文の段落の番号, その段落の中での行) が画面の一番上に来るようにする
    fn show_translation(&mut self, i: usize, top: Option<(usize, usize)>) {
        let Some(t) = &mut self.tabs[i].translation else { return };
        let Some(mode) = t.mode else { return };
        let (doc, index) = translate::apply(&t.source, &t.done, mode);
        let top = top.and_then(|(b, off)| Some((*index.get(b)?, off)));
        t.index = index;
        self.tabs[i].doc = Some(Arc::new(doc));
        if i == self.cur {
            // ヒントのラベルはリンクの出現順で付けているので、文書が変わったら出し直す
            self.hints.clear();
            self.hint_typed.clear();
            self.keep_top(top);
        }
    }

    /// 画面の一番上に見えているブロックと、その中での行
    fn top_block(&mut self) -> Option<(usize, usize)> {
        let y = self.scroll_y;
        self.block_lines().into_iter().filter(|(_, l)| *l <= y).max_by_key(|(_, l)| *l).map(|(b, l)| (b, y - l))
    }

    /// ブロック b の off 行目が画面の一番上に来るようにスクロールする
    fn keep_top(&mut self, top: Option<(usize, usize)>) {
        let Some((b, off)) = top else { return };
        if let Some(&l) = self.block_lines().get(&b) {
            self.scroll_to(l + off, false);
        }
    }

    // ---- 読み上げ (S)
    fn toggle_speech(&mut self) {
        if self.speech.speaking() {
            let speaking = self.speech.tab;
            self.stop_speech(None); // 別のタブが読み上げていても止める
            self.say_here(t!("読み上げを停止しました"));
            if let Some(t) = speaking.filter(|t| *t != self.tab().id) {
                self.say(t, t!("読み上げを停止しました"));
            }
            return;
        }
        let Some(d) = self.doc() else { return };
        if self.overlay.is_some() || d.blocks.is_empty() {
            return;
        }
        // 画面の一番上に見えているブロックから読む
        let y0 = self.scroll_y;
        let mut lines: Vec<(usize, usize)> = self.block_lines().into_iter().collect();
        lines.sort_by_key(|x| x.1);
        let start = lines.iter().find(|(_, l)| *l >= y0).map_or(0, |(b, _)| *b);
        self.start_speech(start, String::new());
    }

    /// start のブロックから読む。rest (ブロックのうちカーソルから後ろの文字列) があれば、その文から読む
    fn start_speech(&mut self, start: usize, rest: String) {
        let Some(doc) = self.doc() else { return };
        let tab = self.tab().id;
        let all = self.speech_chunks(&doc, 0);
        // ページごとに言語を1つに決める (読み始める位置によらないよう、ページ全体から)。読めない言語 (中国語など) なら読まない
        let page: Vec<String> = all.iter().map(|(_, t)| t.clone()).collect();
        let declared = if doc.lang.is_empty() { String::new() } else { format!(" ({}) ", doc.lang) };
        let detect = Detect { texts: page, declared: doc.lang.clone(), unsupported: t!("このページの言語{declared}は読み上げに対応していません", declared = declared) };
        let chunks: Vec<(usize, String)> = all.into_iter().filter(|(b, _)| *b >= start).collect();
        let chunks = if rest.is_empty() { chunks } else { speech::start_at(&chunks, start, &rest) };
        if chunks.is_empty() {
            return self.say(tab, t!("読み上げる文章がありません"));
        }
        self.speak(&doc, chunks, detect);
    }

    /// ビジュアルモードで選んだ文だけを読む。言語はページではなく、選んだ文だけから決める
    /// (日本語のページで「hello」だけを選べば英語で読む)
    fn speak_selection(&mut self, selected: Vec<(usize, String)>) {
        let Some(doc) = self.doc() else { return };
        let tab = self.tab().id;
        let chunks: Vec<(usize, String)> =
            selected.into_iter().flat_map(|(blk, text)| split_sentences(&text).into_iter().map(move |s| (blk, s))).collect();
        if chunks.is_empty() {
            return self.say(tab, t!("読み上げる文章がありません"));
        }
        let texts = chunks.iter().map(|(_, t)| t.clone()).collect();
        let detect = Detect { texts, declared: String::new(), unsupported: t!("選んだ文章の言語は読み上げに対応していません").into() };
        self.speak(&doc, chunks, detect);
    }

    /// (ブロック番号, 文) の列を、detect で決めた言語で読む
    fn speak(&mut self, doc: &Arc<Document>, chunks: Vec<(usize, String)>, detect: Detect) {
        // 初回は、読み上げに使うデータをダウンロードしてよいか先に聞く
        let loaded = self.speech.speaker.try_lock().is_ok_and(|g| g.is_some());
        if !loaded {
            let missing = yomu_tts::download::missing_bytes(&Resources::default());
            if missing > 0 && fetch::is_private() {
                return self.say_here(t!("プライベートモードでは読み上げのデータをダウンロードしません (普段のモードで一度ダウンロードしてください)"));
            }
            if missing > 0 {
                let mb = missing.div_ceil(1 << 20);
                self.confirm = Some(Confirm::Download((doc.clone(), chunks, detect)));
                return self.say_here(t!("読み上げのデータ (約 {mb}MB) をダウンロードしますか? (y/n)", mb = mb));
            }
        }
        self.start_speaking(doc, chunks, detect);
    }

    fn start_speaking(&mut self, doc: &Arc<Document>, chunks: Vec<(usize, String)>, detect: Detect) {
        let tab = self.tab().id;
        // 見出しや箇条書きの項目のような、文になっていないものは、読み上げの側で漢字を辞書の読みで読む
        let chunks: Vec<Chunk> =
            chunks.into_iter().map(|(blk, text)| Chunk { blk, label: doc.blocks.get(blk).is_some_and(|b| speech::is_label(b, &text)), text }).collect();
        // 読み上げの準備 (初回のダウンロードやモデルの読み込み) の間に、このタブで別のページへ移ったりタブを閉じたりしたら
        // 止められるよう、読み上げるタブは準備の前に決める
        self.speech.tab = Some(tab);
        self.speech.lang = None;
        self.speech.blk = None;
        let (speaker, tx) = (self.speech.speaker.clone(), self.tx.clone());
        let (preparing, cancel) = (self.speech.preparing.clone(), self.speech.cancel.clone());
        preparing.store(true, Ordering::Relaxed);
        cancel.store(false, Ordering::Relaxed);
        let doc_id = Arc::as_ptr(doc) as usize;
        std::thread::spawn(move || {
            let mut g = speaker.lock().unwrap();
            if g.is_none() {
                let say = |m: &str| {
                    let _ = tx.send(Msg::Status(tab, m.to_string()));
                };
                // 用意している間に S で止められたら、ダウンロードも止める (置き終えたファイルは次に使う)
                match Speaker::open(&Resources::default(), DEFAULT_VOICE, &say, &|| cancel.load(Ordering::Relaxed)) {
                    Ok(s) => *g = Some(s),
                    Err(e) => {
                        preparing.store(false, Ordering::Relaxed);
                        let m = if e == yomu_tts::download::CANCELLED {
                            t!("{e} (次に S を押すと続きから)", e = e)
                        } else {
                            t!("読み上げモデルを用意できませんでした: {e}", e = e)
                        };
                        let _ = tx.send(Msg::Status(tab, m));
                        return;
                    }
                }
            }
            preparing.store(false, Ordering::Relaxed);
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            // 言語の判定の統計データは初回の準備でダウンロードするので、判定はその後
            let Some(lang) = detect_lang(&detect.texts, &detect.declared) else {
                let _ = tx.send(Msg::Status(tab, detect.unsupported));
                return;
            };
            let s = g.as_mut().unwrap();
            let _ = tx.send(Msg::SpeechStarted(tab, lang));
            let (tx1, tx2) = (tx.clone(), tx.clone());
            s.start(
                chunks,
                lang,
                move |blk| {
                    let _ = tx1.send(Msg::SpeechBlock { tab, doc: doc_id, blk });
                },
                move || {
                    let _ = tx2.send(Msg::SpeechDone(tab));
                },
            );
        });
    }

    /// 読み上げる (画面上のブロック番号, 文) の列。全体表示のときも本文だけを読む
    fn speech_chunks(&self, doc: &Document, start: usize) -> Vec<(usize, String)> {
        let r = self
            .tab()
            .entry()
            .and_then(|e| self.loader.raw.lock().unwrap().get(&urldefrag(&e.target).0).cloned());
        match r {
            Some(r) if doc.full => {
                let body = crate::extract::extract(r.text(), &r.url, false);
                speech::chunks_for_view(&body.blocks, &doc.blocks, start)
            }
            _ => speech::chunks_from(&doc.blocks, start),
        }
    }

    /// 読んでいるブロックが画面から外れていたら、そこまでスクロールする。
    /// 別のタブを見ている間は状態行だけ更新し、戻ってきたら追いかけを再開する
    fn follow_speech(&mut self, tab: u64, doc: usize, blk: usize) {
        let Some(i) = self.tab_index(tab) else { return };
        let n = self.tabs[i].doc.as_ref().map_or(0, |d| d.blocks.len());
        let lang = self.speech.lang.map_or("", Lang::name);
        self.say(tab, t!("読み上げ中 ({lang}) {i}/{n} (S で停止)", lang = lang, i = blk + 1, n = n));
        if i != self.cur || self.doc().is_none_or(|d| Arc::as_ptr(&d) as usize != doc) {
            return;
        }
        let lines = self.block_lines();
        let Some(&line) = lines.get(&blk) else { return };
        let (y0, h) = (self.scroll_y, self.height());
        let visible = |n: Option<usize>| n.is_some_and(|n| y0 <= n && n + 2 < y0 + h);
        // 直前に読んでいた所が見えているときだけ追いかける (自分でスクロールして離れたら邪魔しない)
        let prev = self.speech.blk.and_then(|b| lines.get(&b).copied());
        if !visible(Some(line)) && (self.speech.blk.is_none() || visible(prev)) {
            self.scroll_to(line.saturating_sub(1), false);
        }
        self.speech.blk = Some(blk);
    }

    /// 読み上げを止める。tab を渡したときは、そのタブが読み上げているときだけ止める
    fn stop_speech(&mut self, tab: Option<u64>) {
        if self.speech.speaking() && (tab.is_none() || tab == self.speech.tab) {
            self.speech.stop();
        }
    }

    // ---- キー入力
    fn on_key(&mut self, k: KeyEvent) {
        if self.prompt.is_some() {
            // 入力欄の中のキーは入力欄で処理する
            return self.prompt_key(&k);
        }
        let (key, ch) = ime::read_key(&k);
        // コマンドは 1 文字。NFKC で複数の文字になったもの (㍻ など) はどのコマンドにも当たらない
        let ch = ch.and_then(|s| {
            let mut it = s.chars();
            let c = it.next()?;
            it.next().is_none().then_some(c)
        });
        // y/n で聞いていることへの返事。y なら実行し、それ以外なら取りやめる
        if let Some(c) = self.confirm.take() {
            let yes = ch == Some('y') || ch == Some('Y');
            return match c {
                Confirm::Download((doc, chunks, detect)) if yes => self.start_speaking(&doc, chunks, detect),
                Confirm::Download(_) => self.say_here(t!("ダウンロードを取りやめました")),
                Confirm::ClearHistory(r) if yes => self.clear_history(r),
                Confirm::ClearHistory(_) => self.say_here(t!("消すのを取りやめました")),
                Confirm::Translate(p) if yes => {
                    crate::settings::agree_translate();
                    match p {
                        PendingTranslation::Page(mode) => self.translate_page(mode),
                        PendingTranslation::Selection(texts) => self.translate_selection(texts),
                    }
                }
                Confirm::Translate(_) => self.say_here(t!("翻訳を取りやめました")),
            };
        }
        // 訳文の枠は、どのキーでも閉じる (Esc は閉じるだけ)
        if self.popup.take().is_some() && key == "escape" {
            self.ime_normal();
            return self.say_here("");
        }
        if key == "escape" {
            // Vimium と同じく Esc はいつでも操作モードに戻る。IME も英数にする
            // (変換中の文字があれば最初の Esc は IME が取り消しに使うので、2回目の Esc でここに来る)
            self.ime_normal();
        }
        if !self.hints.is_empty() {
            self.hint_key(&key, ch);
        } else if self.visual.is_some() {
            self.visual_key(&key, ch);
        } else if key == "escape" {
            self.cancel();
        } else if !self.pending.is_empty() {
            let first = std::mem::take(&mut self.pending);
            self.run_sequence(&first, ch);
        } else if let Some(c) = ch.filter(|c| c.is_ascii_digit() && (*c != '0' || !self.count.is_empty())) {
            self.count.push(c);
        } else if let Some(c) = ch.filter(|c| PREFIX_KEYS.contains(*c)) {
            self.pending = c.to_string();
        } else {
            let n: usize = self.count.parse().unwrap_or(1);
            self.count.clear();
            let name = ch.map(String::from).unwrap_or(key);
            self.command(&name, n);
        }
    }

    /// 1打鍵のコマンド。n は回数 (5j の 5)
    fn command(&mut self, name: &str, n: usize) {
        let h = self.height() as isize;
        let n_i = n as isize;
        match name {
            "j" | "down" => self.scroll_by(SCROLL_STEP as isize * n_i),
            "k" | "up" => self.scroll_by(-(SCROLL_STEP as isize) * n_i),
            "h" => self.scroll_x = self.scroll_x.saturating_sub(SCROLL_STEP * n),
            "l" => self.scroll_x = (self.scroll_x + SCROLL_STEP * n).min(self.max_scroll_x()),
            "d" => self.scroll_by(h / 2 * n_i),
            "u" => self.scroll_by(-(h / 2) * n_i),
            "space" | "pagedown" => self.scroll_by((h - 2) * n_i),
            "shift+space" | "pageup" => self.scroll_by(-(h - 2) * n_i),
            "G" | "end" => {
                let max = self.max_scroll_y();
                self.scroll_to(max, true);
            }
            "home" => self.scroll_to(0, false),
            "f" => self.start_hints(HintAction::Open),
            "F" => self.start_hints(HintAction::Tab),
            "H" => self.history_move(-n_i),
            "L" => self.history_move(n_i),
            "r" => {
                if self.tab().entry().is_some() {
                    let id = self.tab().id;
                    self.load(id, true);
                }
            }
            "o" => self.open_prompt(PromptMode::Open, "", false),
            "O" => self.open_prompt(PromptMode::Open, "", true),
            "s" => self.open_prompt(PromptMode::Search, "", false),
            "/" => self.open_prompt(PromptMode::Find, "", false),
            "n" => self.find_again(1),
            "N" => self.find_again(-1),
            "p" => self.paste_go(false),
            "P" => self.paste_go(true),
            "t" => self.navigate(Entry::new(EntryKind::Blank, ""), true, false),
            "x" => {
                for _ in 0..n {
                    if self.quit {
                        break;
                    }
                    self.close_tab();
                }
            }
            "X" => self.restore_tab(),
            "J" => self.switch_tab(self.cur as isize - n_i),
            "K" => self.switch_tab(self.cur as isize + n_i),
            "^" => self.previous_tab(),
            // Vimium と同じく、v は選択せずにキャレット (カーソルだけ) から始め、もう一度 v で選択を始める
            "v" => self.start_visual(Mode::Caret),
            "V" => self.start_visual(Mode::Line),
            "b" => self.open_prompt(PromptMode::Bookmark, "", false),
            "B" => self.open_prompt(PromptMode::Bookmark, "", true),
            "?" => self.toggle_help(),
            "a" => self.toggle_full(),
            "S" => self.toggle_speech(),
            "e" => self.translate_page(translate::Mode::Replace),
            "E" => self.translate_page(translate::Mode::Bilingual),
            "q" => {
                self.stop_speech(None);
                self.quit = true;
            }
            _ => {}
        }
    }

    fn max_scroll_x(&mut self) -> usize {
        let w = self.wide_width();
        self.shown_lines().iter().map(|l| l.text().width()).max().unwrap_or(0).saturating_sub(w)
    }

    /// 2打鍵のコマンド (m・` の後の英字は別に扱う)
    fn run_sequence(&mut self, first: &str, second: Option<char>) {
        let n = self.count.parse::<isize>().unwrap_or(1);
        self.count.clear();
        let Some(second) = second else { return };
        if first == "m" {
            return self.set_mark(second);
        }
        if first == "`" {
            return self.jump_mark(second);
        }
        match format!("{first}{second}").as_str() {
            "gg" => self.scroll_to(0, true),
            "gt" => self.switch_tab(self.cur as isize + n),
            "gT" => self.switch_tab(self.cur as isize - n),
            "g0" => self.switch_tab(0),
            "g$" => self.switch_tab(self.tabs.len() as isize - 1),
            "gu" => self.go_up(false),
            "gU" => self.go_up(true),
            "gb" => self.toggle_bookmark(),
            "gx" => self.open_in_browser(),
            "gs" => self.toggle_settings(),
            "gh" => self.toggle_history(),
            "gi" => self.focus_input(),
            "gp" => self.toggle_private(),
            "ge" => {
                let u = self.current_url().unwrap_or_default();
                self.open_prompt(PromptMode::Open, &u, false);
            }
            "gE" => {
                let u = self.current_url().unwrap_or_default();
                self.open_prompt(PromptMode::Open, &u, true);
            }
            "yy" => {
                if let Some(u) = self.current_url() {
                    self.yank(&u);
                }
            }
            "yf" => self.start_hints(HintAction::Yank),
            "yt" => self.duplicate_tab(),
            "[[" => self.follow_rel(&PREV_RE),
            "]]" => self.follow_rel(&NEXT_RE),
            "<<" => self.move_tab(-1),
            ">>" => self.move_tab(1),
            _ => {}
        }
    }

    /// Esc: 入力途中のコマンド・ヘルプ・検索の強調を取り消す
    fn cancel(&mut self) {
        self.pending.clear();
        self.count.clear();
        if self.overlay.is_some() {
            self.show_tab();
        } else {
            self.find_q.clear();
        }
    }
}

fn tstyle(s: &render::Style) -> TStyle {
    let mut t = TStyle::default();
    if let Some(c) = s.fg {
        t = t.fg(color(c));
    }
    if let Some(c) = s.bg {
        t = t.bg(color(c));
    }
    let mut m = Modifier::empty();
    for (on, flag) in [
        (s.bold, Modifier::BOLD),
        (s.italic, Modifier::ITALIC),
        (s.underline, Modifier::UNDERLINED),
        (s.strike, Modifier::CROSSED_OUT),
        (s.dim, Modifier::DIM),
        (s.reverse, Modifier::REVERSED),
    ] {
        if on {
            m |= flag;
        }
    }
    t.add_modifier(m)
}

fn color(c: Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

fn fg(c: Rgb) -> TStyle {
    TStyle::default().fg(color(c))
}

/// 先頭から skip マス分を飛ばした文字列
fn skip_cells(s: &str, skip: usize) -> String {
    let mut w = 0;
    let mut out = String::new();
    for c in s.chars() {
        if w >= skip {
            out.push(c);
        }
        w += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
    }
    out
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// 表示している文書のブロックの番号 → 原文の段落の番号 (対訳では原文の後に訳文のブロックがある)
fn source_block(index: &[usize], shown: usize) -> usize {
    index.iter().rposition(|&c| c <= shown).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_text_wraps_by_cells() {
        assert_eq!(wrap_cells("あいうえおかきくけこ", 8), ["あいうえ", "おかきく", "けこ"]);
        assert_eq!(wrap_cells("the quick brown fox", 10), ["the quick", "brown fox"]);
        assert_eq!(wrap_cells("一\n\n二", 8), ["一", "", "二"]);
    }

    #[test]
    fn closing_the_last_tab_quits_and_saves_no_tabs() {
        let mut app = App::new(None, false, false);
        app.tabs[0].history.push(Entry::new(EntryKind::Url, "https://example.com/"));
        app.tabs[0].pos = Some(0);
        app.close_tab();
        assert!(app.quit && app.tabs.is_empty());
        assert!(app.session().tabs.is_empty()); // タブがなくても落ちない
    }

    #[test]
    fn hint_labels_match_vimium() {
        assert_eq!(hint_labels(5), ["a", "d", "f", "j", "s"]);
        let labels = hint_labels(34);
        let set: std::collections::HashSet<_> = labels.iter().collect();
        assert_eq!(labels.len(), 34);
        assert_eq!(set.len(), 34);
        // どのラベルもほかのラベルの前方一致にならない
        assert!(!labels.iter().any(|a| labels.iter().any(|b| a != b && b.starts_with(a.as_str()))));
        assert!(labels.iter().any(|x| x.len() == 2));
    }

    #[test]
    fn tab_bar_marks_speaking_tab() {
        let titles: Vec<String> = ["将棋 - Wikipedia", "新しいタブ", "Python"].iter().map(|s| s.to_string()).collect();
        let bar: String = tab_bar(&titles, 1, 80, Some(0)).into_iter().map(|(t, _, _)| t).collect();
        assert_eq!(bar[" ".len()..].split('│').collect::<Vec<_>>(), [" 1 🔊 将棋 - Wikipedia ", " 2 新しいタブ ", " 3 Python "]);
        // 入りきらないときは今のタブの周りだけ出し、外にあるタブを ‹ › で示す
        let titles: Vec<String> = (0..10).map(|i| format!("とても長いページの題名 {i}")).collect();
        let bar: String = tab_bar(&titles, 5, 60, None).into_iter().map(|(t, _, _)| t).collect();
        assert!(bar.width() <= 60 && bar.starts_with('‹') && bar.ends_with('›') && bar.contains(" 6 "));
    }

    fn press(app: &mut App, keys: &str) {
        use crossterm::event::{KeyCode, KeyModifiers};
        for c in keys.chars() {
            app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    #[test]
    fn closing_last_tab_exits() {
        let mut app = App::new(None, false, false);
        press(&mut app, "x");
        assert!(app.tabs.is_empty() && app.quit);
    }

    #[test]
    fn closing_tabs_with_count() {
        let mut app = App::new(None, false, false);
        press(&mut app, "tt2x"); // 3 つのタブから 2 つ閉じる
        assert_eq!(app.tabs.len(), 1);
        assert!(!app.quit);
    }

    #[test]
    fn base64_encodes() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64("あ".as_bytes()), "44GC");
    }

}
