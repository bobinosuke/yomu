//! pyopenjtalk-plus の apply_original_rule_before_chaining (openjtalk.pyx)。アクセント句をまとめる前の独自の規則。
use super::NjdFeature;

fn drop_last_chars(s: &str, n: usize) -> String {
    let mut c: Vec<char> = s.chars().collect();
    c.truncate(c.len().saturating_sub(n));
    c.into_iter().collect()
}

pub(crate) fn apply(f: &mut [NjdFeature]) {
    if f.is_empty() {
        return;
    }
    let len = f.len();
    for i in 0..len - 1 {
        // 名詞の後ろの「不足」はブソク
        if f[i].pos == "名詞" && f[i + 1].string == "不足" && f[i + 1].pron == "フソク" {
            f[i + 1].read = "ブソク".into();
            f[i + 1].pron = "ブソク".into();
        }

        // 「数値 + 分 + の + 数値」の分はブン
        let is_fraction_denominator = i + 2 < len && f[i + 1].string == "の" && f[i + 2].pos_group1 == "数";
        if is_fraction_denominator && f[i].string.ends_with('分') {
            if f[i].pron.ends_with("フン") || f[i].pron.ends_with("プン") {
                f[i].read = drop_last_chars(&f[i].read, 2) + "ブン";
                f[i].pron = drop_last_chars(&f[i].pron, 2) + "ブン";
            } else if f[i].pron.ends_with('ブ') {
                f[i].read.push('ン');
                f[i].pron.push('ン');
            }
        }

        if i > 0
            && i + 2 < len
            && f[i].string == "分"
            && f[i - 1].pos_group1 == "数"
            && f[i + 1].string == "の"
            && f[i + 2].pos_group1 == "数"
        {
            f[i].read = "ブン".into();
            f[i].pron = "ブン".into();
        }

        // 2文字以上続く「〇」は伏字なのでマル
        if f[i].string == "〇" && f[i + 1].string == "〇" {
            for k in [i, i + 1] {
                f[k].pos_group1 = "一般".into();
                f[k].read = "マル".into();
                f[k].pron = "マル".into();
                f[k].acc = 1;
                f[k].mora_size = 2;
            }
        }

        // 接尾辞「球」: 送り仮名を持つ和語の後だけダマ
        if f[i + 1].string == "球"
            && f[i + 1].pos == "名詞"
            && f[i + 1].pos_group1 == "接尾"
            && f[i + 1].pron == "キュー"
            && f[i].string.chars().any(|c| ('一'..='鿿').contains(&c))
            && f[i].string.chars().any(|c| ('ぁ'..='ゖ').contains(&c))
        {
            let n = &mut f[i + 1];
            n.read = "ダマ".into();
            n.pron = "ダマ".into();
            n.acc = 1;
            n.mora_size = 2;
            n.chain_rule = "C4".into();
        }

        // サ変動詞(スル)の前にサ変接続や名詞が来たら一つのアクセント句に
        if (["サ変接続", "格助詞", "接続助詞"].contains(&f[i].pos_group1.as_str())
            || (f[i].pos == "名詞" && f[i].pos_group1 == "一般")
            || f[i].pos == "副詞")
            && f[i + 1].ctype == "サ変・スル"
        {
            f[i + 1].chain_flag = 1;
        }
        // ご遠慮、ご配慮のような接頭語
        if ["お", "御", "ご"].contains(&f[i].string.as_str()) && f[i].chain_rule == "P1" {
            if f[i + 1].acc == 0 || f[i + 1].acc == f[i + 1].mora_size {
                f[i + 1].chain_rule = "C4".into();
                f[i + 1].acc = 0;
            } else {
                f[i + 1].chain_rule = "C1".into();
            }
        }
        // 動詞(自立)の連続は後ろの動詞のアクセント核
        if f[i].pos == "動詞" && f[i + 1].pos == "動詞" {
            f[i + 1].chain_rule = if f[i + 1].acc != 0 { "C1" } else { "C4" }.into();
        }
        // 連用形のアクセント核
        if ["連用形", "連用タ接続", "連用ゴザイ接続", "連用テ接続"].contains(&f[i].cform.as_str())
            && f[i].acc == f[i].mora_size
            && f[i].mora_size > 1
        {
            f[i].acc -= 1;
        }
        // 「らる、られる」＋「た」
        if ["れる", "られる", "せる", "させる", "ちゃう"].contains(&f[i].orig.as_str()) && f[i + 1].string == "た" {
            f[i + 1].chain_rule = "F2@1".into();
        }
        // 形容詞＋「なる、する」
        if f[i].pos == "形容詞" && ["なる", "する"].contains(&f[i + 1].orig.as_str()) {
            f[i + 1].chain_flag = 1;
        }
    }
}
