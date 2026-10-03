//! 履歴の画面 (gh)。日付ごとに並べ、1 時間以内・昨日と今日・すべての履歴を消せる。
use chrono::{Local, TimeZone};

use crate::doc::{Block, Document, Kind, Span};
use crate::store::Item;

/// 履歴の画面のリンク (yomu-history:…)。消す操作と、検索の履歴を開くのに使う
pub const HISTORY_SCHEME: &str = "yomu-history:";

/// 消す範囲
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClearRange {
    Hour,
    Today,
    All,
}

impl ClearRange {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "hour" => Some(Self::Hour),
            "today" => Some(Self::Today),
            "all" => Some(Self::All),
            _ => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Hour => "hour",
            Self::Today => "today",
            Self::All => "all",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Hour => t!("1 時間以内の履歴"),
            Self::Today => t!("昨日と今日の履歴"),
            Self::All => t!("すべての履歴"),
        }
    }
    /// この時刻 (UNIX 時刻の秒) 以降に開いたものを消す
    pub fn since(self) -> f64 {
        let now = Local::now();
        match self {
            Self::Hour => now.timestamp() as f64 - 3600.0,
            Self::Today => {
                let yesterday = now.date_naive() - chrono::Days::new(1);
                let start = yesterday.and_hms_opt(0, 0, 0).and_then(|t| Local.from_local_datetime(&t).earliest());
                start.map_or(f64::MIN, |t| t.timestamp() as f64)
            }
            Self::All => f64::MIN,
        }
    }
}

/// 日付の見出し (今日・昨日・それより前は日付)
fn day_label(time: f64) -> String {
    let Some(t) = Local.timestamp_opt(time as i64, 0).single().filter(|_| time > 0.0) else { return t!("日時不明").into() };
    let today = Local::now().date_naive();
    match (today - t.date_naive()).num_days() {
        0 => t!("今日").into(),
        1 => t!("昨日").into(),
        _ => t.format(t!("%Y年%-m月%-d日")).to_string(),
    }
}

/// 履歴の画面。items は新しい順
pub fn history_page(items: &[Item]) -> Document {
    let mut blocks = vec![Block::heading(t!("履歴"), 1)];
    let mut links: Vec<String> = Vec::new();
    let link = |links: &mut Vec<String>, url: String| {
        links.push(url);
        Some(links.len() - 1)
    };
    // 消す操作
    let mut ops = vec![Span::new(t!("消す: "))];
    for (i, r) in [ClearRange::Hour, ClearRange::Today, ClearRange::All].into_iter().enumerate() {
        if i > 0 {
            ops.push(Span::new("  "));
        }
        let l = link(&mut links, format!("{HISTORY_SCHEME}clear/{}", r.name()));
        ops.push(Span { bold: true, link: l, ..Span::new(format!("[{}]", r.label())) });
    }
    blocks.push(Block::new(Kind::P, ops));
    if items.is_empty() {
        blocks.push(Block::para(t!("履歴はありません。")));
    }
    let mut day = String::new();
    for it in items {
        let d = day_label(it.time);
        if d != day {
            blocks.push(Block::heading(d.clone(), 3));
            day = d;
        }
        let time = Local.timestamp_opt(it.time as i64, 0).single().filter(|_| it.time > 0.0).map(|t| t.format("%H:%M").to_string()).unwrap_or_default();
        let (title, url) = if it.kind == "search" {
            (t!("検索: {target}", target = it.target), format!("{HISTORY_SCHEME}search:{}", it.target))
        } else {
            (if it.title.is_empty() { it.target.clone() } else { it.title.clone() }, it.target.clone())
        };
        let l = link(&mut links, url);
        let mut spans = vec![Span { minor: true, ..Span::new(format!("{time:>5}  ")) }, Span { link: l, ..Span::new(title) }];
        if it.kind != "search" {
            spans.push(Span { minor: true, ..Span::new(format!("  {}", crate::urls::unquote(&it.target))) });
        }
        blocks.push(Block::new(Kind::Li, spans));
    }
    Document { url: "about:history".into(), title: t!("履歴").into(), blocks, links, note: t!("履歴 (Esc で閉じる)").into(), ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_ordered() {
        let (h, t, a) = (ClearRange::Hour.since(), ClearRange::Today.since(), ClearRange::All.since());
        let now = Local::now().timestamp() as f64;
        assert!(a < t && t < h && h < now);
        assert!(now - t >= 86400.0 && now - t < 2.0 * 86400.0 + 3600.0, "昨日の 0 時から");
    }

    #[test]
    fn page_lists_items_by_day_with_clear_links() {
        let now = Local::now().timestamp() as f64;
        let item = |kind: &str, target: &str, time: f64| Item { kind: kind.into(), target: target.into(), title: String::new(), time, bookmark: false };
        let d = history_page(&[item("url", "https://a.example/", now), item("search", "将棋", now - 86400.0)]);
        assert_eq!(&d.links[..3], ["yomu-history:clear/hour", "yomu-history:clear/today", "yomu-history:clear/all"]);
        assert!(d.links.contains(&"yomu-history:search:将棋".to_string()));
        let text: Vec<String> = d.blocks.iter().map(|b| b.text()).collect();
        assert!(text.contains(&"今日".to_string()) && text.contains(&"昨日".to_string()));
    }
}
