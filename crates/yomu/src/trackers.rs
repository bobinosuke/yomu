//! 広告・追跡のリンクを消す (設定の block_trackers)。
//!
//! - リンクの URL から追跡用の値 (utm_* や fbclid など) を除く (ClearURLs の共通の規則と同じ考え方)
//! - 広告・追跡・アフィリエイトのサービスへのリンクは、リンクを外す。段落や項目がそういうリンクだけでできていれば
//!   (広告の「おすすめ記事」の一覧など)、その段落ごと除く
use crate::doc::{Block, Document, Kind, Span};

/// 段落の spans と、表のセルの spans のすべて
fn all_spans(b: &mut Block) -> impl Iterator<Item = &mut Span> {
    let rows = b.rows.iter_mut().flatten().flatten().flatten();
    b.spans.iter_mut().chain(rows)
}

/// 追跡用の値の名前 (前方一致は末尾が *)
const TRACKING_PARAMS: &[&str] = &[
    "utm_*", "fbclid", "gclid", "gclsrc", "dclid", "gbraid", "wbraid", "msclkid", "yclid", "twclid", "igshid", "mc_cid", "mc_eid",
    "_hsenc", "_hsmi", "mkt_tok", "oly_anon_id", "oly_enc_id", "vero_id", "_openstat", "ttclid", "li_fat_id", "s_cid", "wickedid",
];

/// 広告・追跡・アフィリエイトのサービスのドメイン (サブドメインも含む)
const AD_DOMAINS: &[&str] = &[
    // 広告の配信・計測
    "doubleclick.net", "googleadservices.com", "googlesyndication.com", "adservice.google.com", "adservice.google.co.jp",
    "amazon-adsystem.com", "adnxs.com", "criteo.com", "criteo.net", "rubiconproject.com", "pubmatic.com", "openx.net",
    "smartadserver.com", "advertising.com", "adform.net", "yieldmo.com", "teads.tv", "media.net", "zedo.com", "ad.yieldmanager.com",
    "taboola.com", "outbrain.com", "revcontent.com", "mgid.com", "zergnet.com", "popin.cc", "logly.co.jp", "uzou.jp",
    "i-mobile.co.jp", "microad.jp", "ad-stir.com", "adingo.jp", "fluct.jp", "impact-ad.jp", "yads.yahoo.co.jp", "ads.yahoo.com",
    // アフィリエイト
    "a8.net", "af.moshimo.com", "valuecommerce.com", "valuecommerce.ne.jp", "accesstrade.net", "afi-b.com", "felmat.net",
    "hb.afl.rakuten.co.jp", "linksynergy.com", "click.linksynergy.com", "rentracks.jp", "xmax.jp", "affiliate-b.com",
    "awin1.com", "shareasale.com", "commission-junction.com", "anrdoezrs.net", "jdoqocy.com", "tkqlhce.com", "dpbolvw.net",
];

fn is_tracking_param(name: &str) -> bool {
    TRACKING_PARAMS.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == *p,
    })
}

/// URL から追跡用の値を除く
pub fn clean_url(url: &str) -> String {
    let (base, frag) = match url.split_once('#') {
        Some((b, f)) => (b, Some(f)),
        None => (url, None),
    };
    let Some((path, query)) = base.split_once('?') else { return url.to_string() };
    let kept: Vec<&str> = query.split('&').filter(|kv| !kv.is_empty() && !is_tracking_param(kv.split('=').next().unwrap_or(""))).collect();
    let mut out = path.to_string();
    if !kept.is_empty() {
        out.push('?');
        out.push_str(&kept.join("&"));
    }
    if let Some(f) = frag {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// 広告・追跡・アフィリエイトのサービスへのリンクか
pub fn is_ad(url: &str) -> bool {
    let host = url.split("://").nth(1).and_then(|r| r.split(['/', '?', '#']).next()).unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host).split(':').next().unwrap_or("").to_ascii_lowercase();
    AD_DOMAINS.iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// 文書から広告・追跡のリンクを除く。除いたリンクの数を返す
pub fn clean(doc: &mut Document) -> usize {
    let ad: Vec<bool> = doc.links.iter().map(|u| is_ad(u)).collect();
    for u in doc.links.iter_mut() {
        *u = clean_url(u);
    }
    let mut removed = 0;
    // 中身が広告のリンク (と画像の代替テキスト) だけの段落・項目・引用は段落ごと除く
    doc.blocks.retain(|b| {
        let only_ads = matches!(b.kind, Kind::P | Kind::Li | Kind::Quote)
            && b.spans.iter().any(|s| s.link.is_some_and(|l| ad[l]))
            && b.spans.iter().all(|s| s.link.is_some_and(|l| ad[l]) || s.text.trim().is_empty());
        if only_ads {
            removed += 1;
        }
        !only_ads
    });
    for b in doc.blocks.iter_mut() {
        for s in all_spans(b) {
            if s.link.is_some_and(|l| ad[l]) {
                s.link = None;
                removed += 1;
            }
        }
    }
    // 広告のリンクは文書のリンクの一覧からも除き、番号を詰める (--dump の末尾の一覧に出さないように)
    let mut new_index = vec![None; doc.links.len()];
    let mut links = Vec::new();
    for (i, u) in std::mem::take(&mut doc.links).into_iter().enumerate() {
        if !ad[i] {
            new_index[i] = Some(links.len());
            links.push(u);
        }
    }
    doc.links = links;
    for b in doc.blocks.iter_mut() {
        for s in all_spans(b) {
            s.link = s.link.and_then(|l| new_index[l]);
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracking_params_are_removed() {
        assert_eq!(clean_url("https://ex.com/a?utm_source=x&id=3&fbclid=y#top"), "https://ex.com/a?id=3#top");
        assert_eq!(clean_url("https://ex.com/a?utm_medium=x"), "https://ex.com/a");
        assert_eq!(clean_url("https://ex.com/a?q=1"), "https://ex.com/a?q=1");
    }

    #[test]
    fn ad_links_and_ad_only_blocks_are_removed() {
        assert!(is_ad("https://px.a8.net/svt/ejp?a8mat=1") && is_ad("https://www.taboola.com/x") && !is_ad("https://example.com/a8.net"));
        let link = |text: &str, l: usize| Span { link: Some(l), ..Span::new(text) };
        let mut doc = Document {
            links: vec!["https://px.a8.net/x".into(), "https://example.com/?utm_source=a".into()],
            blocks: vec![
                Block::new(Kind::Li, vec![link("楽天で見る", 0)]),
                Block::new(Kind::P, vec![Span::new("本文 "), link("広告", 0), Span::new(" と "), link("記事", 1)]),
                Block { kind: Kind::Table, rows: Some(vec![vec![vec![link("表の広告", 0)], vec![link("表の記事", 1)]]]), ..Default::default() },
            ],
            ..Default::default()
        };
        assert_eq!(clean(&mut doc), 3);
        assert_eq!(doc.blocks.len(), 2);
        let row = &doc.blocks[1].rows.as_ref().unwrap()[0];
        assert_eq!((row[0][0].link, row[1][0].link), (None, Some(0)));
        assert_eq!(doc.blocks[0].spans[1].link, None);
        assert_eq!(doc.links, ["https://example.com/"]);
        assert_eq!(doc.blocks[0].spans[3].link, Some(0));
    }
}
