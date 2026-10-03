//! njd_set_digit.c: 数字の読み
use super::rules::*;
use super::{Link, Njd, atoi, strtopcmp, val};

fn get_digit(njd: &mut Njd, n: usize, convert: bool) -> i32 {
    let node = &mut njd.nodes[n];
    if node.string() == "*" {
        return -1;
    }
    if node.pos_group1() == NJD_SET_DIGIT_KAZU
        && let Some(r) = NJD_SET_DIGIT_RULE_NUMERAL_LIST1.chunks(3).find(|r| r[0] == node.string()) {
            if convert {
                node.string = val(r[2]);
                node.orig = val(r[2]);
            }
            return atoi(r[1]);
        }
    -1
}

fn is_period(s: &str) -> bool {
    s == NJD_SET_DIGIT_TEN1 || s == NJD_SET_DIGIT_TEN2
}

fn is_comma(s: &str) -> bool {
    s == NJD_SET_DIGIT_COMMA
}

fn is_haihun(s: &str) -> bool {
    [NJD_SET_DIGIT_HAIHUN1, NJD_SET_DIGIT_HAIHUN2, NJD_SET_DIGIT_HAIHUN3, NJD_SET_DIGIT_HAIHUN4, NJD_SET_DIGIT_HAIHUN5]
        .contains(&s)
}

fn get_digit_sequence_score(njd: &Njd, start: usize, end: usize) -> i32 {
    let nd = &njd.nodes;
    let mut score = 0;
    if let Some(p) = nd[start].prev {
        let (g1, g2, s) = (nd[p].pos_group1(), nd[p].pos_group2(), nd[p].string());
        if g1 == NJD_SET_DIGIT_SUUSETSUZOKU {
            score += 2;
        }
        if g2 == NJD_SET_DIGIT_JOSUUSHI || g1 == NJD_SET_DIGIT_FUKUSHIKANOU {
            score += 1;
        }
        let pp_is_kazu = nd[p].prev.is_some_and(|pp| nd[pp].pos_group1() == NJD_SET_DIGIT_KAZU);
        if is_period(s) {
            if pp_is_kazu {
                score -= 5;
            }
        } else if is_haihun(s) {
            score -= 2;
        } else if s == NJD_SET_DIGIT_KAKKO1 {
            if pp_is_kazu {
                score -= 2;
            }
        } else if s == NJD_SET_DIGIT_KAKKO2 || s == NJD_SET_DIGIT_BANGOU {
            score -= 2;
        }
        if let Some(pp) = nd[p].prev
            && nd[pp].string() == NJD_SET_DIGIT_BANGOU {
                score -= 2;
            }
    }
    if let Some(x) = nd[end].next {
        let (g1, g2, s) = (nd[x].pos_group1(), nd[x].pos_group2(), nd[x].string());
        if g2 == NJD_SET_DIGIT_JOSUUSHI || g1 == NJD_SET_DIGIT_FUKUSHIKANOU {
            score += 2;
        }
        if is_haihun(s) || s == NJD_SET_DIGIT_KAKKO1 {
            score -= 2;
        } else if s == NJD_SET_DIGIT_KAKKO2 {
            if nd[x].next.is_some_and(|xx| nd[xx].pos_group1() == NJD_SET_DIGIT_KAZU) {
                score -= 2;
            }
        } else if s == NJD_SET_DIGIT_BANGOU {
            score -= 2;
        } else if is_period(s) {
            score += 4;
        }
    }
    score
}

fn convert_for_non_numerical_reading(njd: &mut Njd, start: usize, end: usize) {
    let mut size = 0;
    let mut n = Some(start);
    while n != njd.nodes[end].next {
        size += 1;
        n = njd.nodes[n.unwrap()].next;
    }
    if size <= 1 {
        return;
    }
    let mut size = 0;
    let mut n = Some(start);
    while n != njd.nodes[end].next {
        let i = n.unwrap();
        let node = &mut njd.nodes[i];
        let s = node.string();
        if s == NJD_SET_DIGIT_ZERO1 || s == NJD_SET_DIGIT_ZERO2 {
            node.pron = val(NJD_SET_DIGIT_ZERO_AFTER_DP);
            node.set_mora_size(2);
        } else if s == NJD_SET_DIGIT_TWO {
            node.pron = val(NJD_SET_DIGIT_TWO_AFTER_DP);
            node.set_mora_size(2);
        } else if s == NJD_SET_DIGIT_FIVE {
            node.pron = val(NJD_SET_DIGIT_FIVE_AFTER_DP);
            node.set_mora_size(2);
        }
        node.chain_rule = None;
        if size % 2 == 0 {
            node.chain_flag = 0;
        } else {
            node.chain_flag = 1;
            let p = node.prev.unwrap();
            njd.nodes[p].set_acc(3);
        }
        size += 1;
        n = njd.nodes[i].next;
    }
}

/// node の後ろに feature のノードを足し、足したノード (の末尾) を返す
fn append_after(njd: &mut Njd, node: usize, feature: &str) -> usize {
    let nn = njd.new_node();
    njd.load(nn, feature);
    match njd.nodes[node].next {
        None => {
            njd.nodes[node].next = Some(nn);
            njd.nodes[nn].prev = Some(node);
            nn
        }
        next => njd.insert(node, next, nn),
    }
}

fn clear_digit(njd: &mut Njd, n: usize) {
    let node = &mut njd.nodes[n];
    node.pron = None;
    node.set_acc(0);
    node.set_mora_size(0);
}

fn convert_for_numerical_reading(njd: &mut Njd, start: usize, end: usize) {
    let mut size = 0i32;
    let mut n = Some(start);
    while n != njd.nodes[end].next {
        size += 1;
        n = njd.nodes[n.unwrap()].next;
    }
    if size <= 1 {
        return;
    }
    let mut index = size % 4;
    if index == 0 {
        index = 4;
    }
    let mut place = if size > index { (size - index) / 4 } else { 0 };
    index -= 1;
    if place > 17 {
        return;
    }
    let mut have = false;
    let mut n = Some(start);
    while n != njd.nodes[end].next {
        let mut node = n.unwrap();
        let digit = get_digit(njd, node, false);
        if index == 0 {
            if digit == 0 {
                clear_digit(njd, node);
            } else {
                have = true;
            }
            if have {
                if place > 0 {
                    node = append_after(njd, node, NJD_SET_DIGIT_RULE_NUMERAL_LIST3[place as usize]);
                }
                have = false;
            }
            place -= 1;
        } else if digit <= 0 {
            clear_digit(njd, node);
        } else if digit == 1 {
            njd.load(node, NJD_SET_DIGIT_RULE_NUMERAL_LIST2[index as usize]);
            have = true;
        } else {
            node = append_after(njd, node, NJD_SET_DIGIT_RULE_NUMERAL_LIST2[index as usize]);
            have = true;
        }
        index -= 1;
        if index < 0 {
            index = 3;
        }
        n = njd.nodes[node].next;
    }
}

fn fix_tail(njd: &mut Njd) {
    while let Some(x) = njd.tail.and_then(|t| njd.nodes[t].next) {
        njd.tail = Some(x);
    }
}

fn convert_digit_sequence(njd: &mut Njd, s: Link, e: Link) {
    let (Some(s), Some(e)) = (s, e) else { return };
    if is_comma(njd.nodes[s].string()) || is_period(njd.nodes[s].string()) {
        if s != e {
            let nx = njd.nodes[s].next;
            convert_digit_sequence(njd, nx, Some(e));
        }
        return;
    }
    // 小数点の前の最後の数字
    let mut fdbp = s;
    while let Some(x) = njd.nodes[fdbp].next {
        if fdbp == e || is_period(njd.nodes[x].string()) {
            break;
        }
        fdbp = x;
    }
    while let Some(p) = njd.nodes[fdbp].prev {
        if fdbp == s || !is_comma(njd.nodes[fdbp].string()) {
            break;
        }
        fdbp = p;
    }
    // 1: 数として読む 0: 不明 -1: 数として読まない
    let mut numerical_reading = 1;
    let mut num_comma = 0;
    let mut first_comma_before_period: Option<usize> = None;
    let mut rindex = 0;
    let mut node = fdbp;
    loop {
        if is_comma(njd.nodes[node].string()) {
            first_comma_before_period = Some(node);
            num_comma += 1;
            if numerical_reading == 1 && rindex % 4 != 3 {
                numerical_reading = 0;
            }
        } else if numerical_reading == 1 && rindex % 4 == 3 {
            numerical_reading = 0;
        }
        if node == s {
            break;
        }
        node = njd.nodes[node].prev.unwrap();
        rindex += 1;
    }
    // 0 始まり
    if s != fdbp && get_digit(njd, s, false) == 0 {
        numerical_reading = -1;
    }
    if numerical_reading == 1 && num_comma == 0 {
        numerical_reading = 0;
    }
    if numerical_reading == 1 {
        if num_comma > 0 {
            let mut n = Some(s);
            while let Some(i) = n {
                if i == fdbp {
                    break;
                }
                n = if is_comma(njd.nodes[i].string()) { njd.remove(i) } else { njd.nodes[i].next };
            }
        }
        convert_for_numerical_reading(njd, s, fdbp);
        fix_tail(njd);
        if fdbp != e {
            let nx = njd.nodes[fdbp].next;
            convert_digit_sequence(njd, nx, Some(e));
        }
    } else {
        let final_digit = match first_comma_before_period {
            None => fdbp,
            Some(c) => njd.nodes[c].prev.unwrap(),
        };
        if numerical_reading == 0 {
            numerical_reading = if get_digit_sequence_score(njd, s, final_digit) >= 0 { 1 } else { -1 };
        }
        if numerical_reading == 1 {
            convert_for_numerical_reading(njd, s, final_digit);
            fix_tail(njd);
        } else {
            convert_for_non_numerical_reading(njd, s, final_digit);
        }
        if final_digit != e {
            let nx = njd.nodes[final_digit].next;
            convert_digit_sequence(njd, nx, Some(e));
        }
    }
}

fn in_class(list: &[&str], s: &str) -> bool {
    s != "*" && list.contains(&s)
}

fn convert_digit_pron(njd: &mut Njd, list: &[&str], n: usize) {
    let node = &mut njd.nodes[n];
    if node.string() == "*" {
        return;
    }
    if let Some(r) = list.chunks(4).find(|r| r[0] == node.string()) {
        node.pron = val(r[1]);
        node.set_acc(atoi(r[2]));
        node.set_mora_size(atoi(r[3]));
    }
}

fn convert_numerative_pron(njd: &mut Njd, list: &[&str], n1: usize, n2: usize) {
    let s = njd.nodes[n1].string();
    if s == "*" {
        return;
    }
    let ty = list.chunks(2).find(|r| r[0] == s).map_or(0, |r| atoi(r[1]));
    let table = match ty {
        1 => NJD_SET_DIGIT_RULE_VOICED_SOUND_SYMBOL_LIST,
        2 => NJD_SET_DIGIT_RULE_SEMIVOICED_SOUND_SYMBOL_LIST,
        _ => return,
    };
    for r in table.chunks(2) {
        let pron = njd.nodes[n2].pron().to_string();
        let j = strtopcmp(pron.as_bytes(), r[0]);
        if j >= 0 {
            njd.nodes[n2].pron = val(&format!("{}{}", r[1], &pron[j as usize..]));
            break;
        }
    }
}

pub(crate) fn set_digit(njd: &mut Njd) {
    let (mut s, mut e): (Link, Link) = (None, None);
    let mut find = false;
    let mut n = njd.head;
    while let Some(i) = n {
        if !find && njd.nodes[i].pos_group1() == NJD_SET_DIGIT_KAZU {
            find = true;
        }
        let is_digit = get_digit(njd, i, true) >= 0 || {
            let node = &njd.nodes[i];
            node.pos_group1() == NJD_SET_DIGIT_KAZU && (is_period(node.string()) || is_comma(node.string()))
        };
        if is_digit {
            if s.is_none() {
                s = Some(i);
            }
            if Some(i) == njd.tail {
                e = Some(i);
            }
        } else if s.is_some() {
            e = njd.nodes[i].prev;
        }
        if s.is_some() && e.is_some() {
            convert_digit_sequence(njd, s, e);
            s = None;
            e = None;
        }
        n = njd.nodes[i].next;
    }
    if !find {
        return;
    }
    njd.remove_silent();
    let Some(head) = njd.head else { return };

    // 小数点
    let mut n = njd.nodes[head].next;
    while let Some(i) = n {
        let Some(nx) = njd.nodes[i].next else { break };
        let p = njd.nodes[i].prev.unwrap();
        let (node, prev, next) = (&njd.nodes[i], &njd.nodes[p], &njd.nodes[nx]);
        if node.string() != "*"
            && prev.string() != "*"
            && is_period(node.string())
            && prev.pos_group1() == NJD_SET_DIGIT_KAZU
            && next.pos_group1() == NJD_SET_DIGIT_KAZU
        {
            njd.load(i, NJD_SET_DIGIT_TEN_FEATURE);
            njd.nodes[i].chain_flag = 1;
            let prev = &mut njd.nodes[p];
            let ps = prev.string();
            if ps == NJD_SET_DIGIT_ZERO1 || ps == NJD_SET_DIGIT_ZERO2 {
                prev.pron = val(NJD_SET_DIGIT_ZERO_BEFORE_DP);
                prev.set_mora_size(2);
            } else if ps == NJD_SET_DIGIT_TWO {
                prev.pron = val(NJD_SET_DIGIT_TWO_BEFORE_DP);
                prev.set_mora_size(2);
            } else if ps == NJD_SET_DIGIT_FIVE {
                prev.pron = val(NJD_SET_DIGIT_FIVE_BEFORE_DP);
                prev.set_mora_size(2);
            } else if ps == NJD_SET_DIGIT_SIX {
                prev.set_acc(1);
            }
            // 小数点以下の数字を飛ばす
            let mut m = njd.nodes[i].next;
            while let Some(x) = m {
                if njd.nodes[x].pos() != NJD_SET_DIGIT_MEISHI {
                    break;
                }
                m = njd.nodes[x].next;
            }
            n = m.and_then(|x| njd.nodes[x].next);
        } else {
            n = njd.nodes[i].next;
        }
    }

    // 数字 + 助数詞
    let mut n = njd.nodes[njd.head.unwrap()].next;
    while let Some(i) = n {
        let p = njd.nodes[i].prev.unwrap();
        if njd.nodes[p].pos_group1() == NJD_SET_DIGIT_KAZU
            && (njd.nodes[i].pos_group2() == NJD_SET_DIGIT_JOSUUSHI
                || njd.nodes[i].pos_group1() == NJD_SET_DIGIT_FUKUSHIKANOU)
        {
            let s = njd.nodes[i].string().to_string();
            let t1: &[(&[&str], &[&str])] = &[
                (NJD_SET_DIGIT_RULE_NUMERATIVE_CLASS1B, NJD_SET_DIGIT_RULE_CONV_TABLE1B),
                (NJD_SET_DIGIT_RULE_NUMERATIVE_CLASS1C1, NJD_SET_DIGIT_RULE_CONV_TABLE1C1),
                (NJD_SET_DIGIT_RULE_NUMERATIVE_CLASS1C2, NJD_SET_DIGIT_RULE_CONV_TABLE1C2),
                (NJD_SET_DIGIT_RULE_NUMERATIVE_CLASS1D, NJD_SET_DIGIT_RULE_CONV_TABLE1D),
                (NJD_SET_DIGIT_RULE_NUMERATIVE_CLASS1E, NJD_SET_DIGIT_RULE_CONV_TABLE1E),
                (NJD_SET_DIGIT_RULE_NUMERATIVE_CLASS1F, NJD_SET_DIGIT_RULE_CONV_TABLE1F),
                (NJD_SET_DIGIT_RULE_NUMERATIVE_CLASS1G, NJD_SET_DIGIT_RULE_CONV_TABLE1G),
                (NJD_SET_DIGIT_RULE_NUMERATIVE_CLASS1H, NJD_SET_DIGIT_RULE_CONV_TABLE1H),
                (NJD_SET_DIGIT_RULE_NUMERATIVE_CLASS1I, NJD_SET_DIGIT_RULE_CONV_TABLE1I),
                (NJD_SET_DIGIT_RULE_NUMERATIVE_CLASS1J, NJD_SET_DIGIT_RULE_CONV_TABLE1J),
                (NJD_SET_DIGIT_RULE_NUMERATIVE_CLASS1K, NJD_SET_DIGIT_RULE_CONV_TABLE1K),
            ];
            if let Some((_, table)) = t1.iter().find(|(class, _)| in_class(class, &s)) {
                convert_digit_pron(njd, table, p);
            }
            let t2: &[(&[&str], &[&str])] = &[
                (NJD_SET_DIGIT_RULE_NUMERATIVE_CLASS2B, NJD_SET_DIGIT_RULE_CONV_TABLE2B),
                (NJD_SET_DIGIT_RULE_NUMERATIVE_CLASS2C, NJD_SET_DIGIT_RULE_CONV_TABLE2C),
                (NJD_SET_DIGIT_RULE_NUMERATIVE_CLASS2D, NJD_SET_DIGIT_RULE_CONV_TABLE2D),
                (NJD_SET_DIGIT_RULE_NUMERATIVE_CLASS2E, NJD_SET_DIGIT_RULE_CONV_TABLE2E),
                (NJD_SET_DIGIT_RULE_NUMERATIVE_CLASS2F, NJD_SET_DIGIT_RULE_CONV_TABLE2F),
            ];
            if let Some((_, table)) = t2.iter().find(|(class, _)| in_class(class, &s)) {
                convert_numerative_pron(njd, table, p, i);
            }
            njd.nodes[p].chain_flag = 0;
            njd.nodes[i].chain_flag = 1;
        }
        n = njd.nodes[i].next;
    }

    // 数字の連続
    let mut n = njd.nodes[njd.head.unwrap()].next;
    while let Some(i) = n {
        let p = njd.nodes[i].prev.unwrap();
        if njd.nodes[p].pos_group1() == NJD_SET_DIGIT_KAZU {
            if njd.nodes[i].pos_group1() == NJD_SET_DIGIT_KAZU {
                let (ps, s) = (njd.nodes[p].string().to_string(), njd.nodes[i].string().to_string());
                let mut found = false;
                if NJD_SET_DIGIT_RULE_NUMERAL_LIST4.contains(&ps.as_str())
                    && NJD_SET_DIGIT_RULE_NUMERAL_LIST5.contains(&s.as_str()) {
                        njd.nodes[p].chain_flag = 0;
                        njd.nodes[i].chain_flag = 1;
                        found = true;
                    }
                if !found
                    && NJD_SET_DIGIT_RULE_NUMERAL_LIST5.contains(&ps.as_str())
                    && NJD_SET_DIGIT_RULE_NUMERAL_LIST4.contains(&s.as_str())
                {
                    njd.nodes[i].chain_flag = 0;
                }
            }
            let s = njd.nodes[i].string().to_string();
            if in_class(NJD_SET_DIGIT_RULE_NUMERAL_LIST8, &s) {
                convert_digit_pron(njd, NJD_SET_DIGIT_RULE_NUMERAL_LIST9, p);
            }
            if in_class(NJD_SET_DIGIT_RULE_NUMERAL_LIST10, &s) {
                convert_digit_pron(njd, NJD_SET_DIGIT_RULE_NUMERAL_LIST11, p);
            }
            if in_class(NJD_SET_DIGIT_RULE_NUMERAL_LIST6, &s) {
                convert_numerative_pron(njd, NJD_SET_DIGIT_RULE_NUMERAL_LIST7, p, i);
            }
        }
        n = njd.nodes[i].next;
    }

    // 人数・日付
    let mut n = njd.head;
    while let Some(i) = n {
        let cond = njd.nodes[i].next.is_some_and(|nx| {
            let (node, next) = (&njd.nodes[i], &njd.nodes[nx]);
            next.string() != "*"
                && node.pos_group1() == NJD_SET_DIGIT_KAZU
                && node.prev.is_none_or(|p| {
                    njd.nodes[p].pos() == NJD_SET_DIGIT_KIGOU || njd.nodes[p].pos_group1() != NJD_SET_DIGIT_KAZU
                })
                && (next.pos_group2() == NJD_SET_DIGIT_JOSUUSHI || next.pos_group1() == NJD_SET_DIGIT_FUKUSHIKANOU)
        });
        if cond {
            let nx = njd.nodes[i].next.unwrap();
            let (ns, nr) = (njd.nodes[nx].string().to_string(), njd.nodes[nx].read().to_string());
            if NJD_SET_DIGIT_RULE_NUMERATIVE_CLASS3.chunks(2).any(|r| r[0] == ns && r[1] == nr) {
                let s = njd.nodes[i].string().to_string();
                if let Some(r) = NJD_SET_DIGIT_RULE_CONV_TABLE3.chunks(4).find(|r| r[0] == s) {
                    let node = &mut njd.nodes[i];
                    node.read = val(r[1]);
                    node.pron = val(r[1]);
                    node.set_acc(atoi(r[2]));
                    node.set_mora_size(atoi(r[3]));
                }
            }
            if njd.nodes[nx].string() == NJD_SET_DIGIT_NIN {
                let s = njd.nodes[i].string().to_string();
                if let Some(r) = NJD_SET_DIGIT_RULE_CONV_TABLE4.chunks(2).find(|r| r[0] == s) {
                    njd.load(i, r[1]);
                    njd.nodes[nx].pron = None;
                }
            }
            // 次のノードは load で変わりうるので取り直す
            let nx = njd.nodes[i].next.unwrap();
            if njd.nodes[nx].string() == NJD_SET_DIGIT_NICHI && njd.nodes[i].string() != "*" {
                let tsuitachi = njd.nodes[i].prev.is_some_and(|p| njd.nodes[p].string().contains(NJD_SET_DIGIT_GATSU))
                    && njd.nodes[i].string() == NJD_SET_DIGIT_ONE;
                if tsuitachi {
                    njd.load(i, NJD_SET_DIGIT_TSUITACHI);
                    njd.nodes[nx].pron = None;
                } else {
                    let s = njd.nodes[i].string().to_string();
                    if let Some(r) = NJD_SET_DIGIT_RULE_CONV_TABLE5.chunks(2).find(|r| r[0] == s) {
                        njd.load(i, r[1]);
                        njd.nodes[nx].pron = None;
                    }
                }
            } else if njd.nodes[nx].string() == NJD_SET_DIGIT_NICHIKAN {
                let s = njd.nodes[i].string().to_string();
                if let Some(r) = NJD_SET_DIGIT_RULE_CONV_TABLE6.chunks(2).find(|r| r[0] == s) {
                    njd.load(i, r[1]);
                    njd.nodes[nx].pron = None;
                }
            }
        }
        n = njd.nodes[i].next;
    }

    // 十四日・二十日・二十四日
    let mut n = njd.head;
    while let Some(i) = n {
        let p_ok = njd.nodes[i].prev.is_none_or(|p| njd.nodes[p].pos_group1() != NJD_SET_DIGIT_KAZU);
        if let (true, Some(n1)) = (p_ok, njd.nodes[i].next)
            && let Some(n2) = njd.nodes[n1].next {
                let (s0, s1, s2) = (
                    njd.nodes[i].string().to_string(),
                    njd.nodes[n1].string().to_string(),
                    njd.nodes[n2].string().to_string(),
                );
                if s0 == NJD_SET_DIGIT_TEN && s1 == NJD_SET_DIGIT_FOUR {
                    let f = if s2 == NJD_SET_DIGIT_NICHI {
                        Some(NJD_SET_DIGIT_JUYOKKA)
                    } else if s2 == NJD_SET_DIGIT_NICHIKAN {
                        Some(NJD_SET_DIGIT_JUYOKKAKAN)
                    } else {
                        None
                    };
                    if let Some(f) = f {
                        njd.load(i, f);
                        njd.nodes[n1].pron = None;
                        njd.nodes[n2].pron = None;
                    }
                } else if s0 == NJD_SET_DIGIT_TWO && s1 == NJD_SET_DIGIT_TEN {
                    if s2 == NJD_SET_DIGIT_NICHI || s2 == NJD_SET_DIGIT_NICHIKAN {
                        let f = if s2 == NJD_SET_DIGIT_NICHI { NJD_SET_DITIT_HATSUKA } else { NJD_SET_DIGIT_HATSUKAKAN };
                        njd.load(i, f);
                        njd.nodes[n1].pron = None;
                        njd.nodes[n2].pron = None;
                    } else if s2 == NJD_SET_DIGIT_FOUR
                        && let Some(n3) = njd.nodes[n2].next {
                            let s3 = njd.nodes[n3].string().to_string();
                            let f = if s3 == NJD_SET_DIGIT_NICHI {
                                Some(NJD_SET_DITIT_YOKKA)
                            } else if s3 == NJD_SET_DIGIT_NICHIKAN {
                                Some(NJD_SET_DIGIT_YOKKAKAN)
                            } else {
                                None
                            };
                            if let Some(f) = f {
                                njd.load(i, NJD_SET_DIGIT_NIJU);
                                njd.load(n1, f);
                                njd.nodes[n2].pron = None;
                                njd.nodes[n3].pron = None;
                            }
                        }
                }
            }
        n = njd.nodes[i].next;
    }
    njd.remove_silent();
}
