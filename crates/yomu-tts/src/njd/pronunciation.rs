//! njd_set_pronunciation.c
use super::rules::*;
use super::{Njd, add, atoi, strtopcmp, val};

pub(crate) fn set_pronunciation(njd: &mut Njd) {
    let list = NJD_SET_PRONUNCIATION_LIST;
    let mut n = njd.head;
    while let Some(i) = n {
        let node = &mut njd.nodes[i];
        if node.mora_size == 0 {
            node.read = None;
            node.pron = None;
            // かなだけの語はフィラーとして読みを付ける
            let s = node.string().to_string();
            let b = s.as_bytes();
            let mut pos = 0;
            while pos < b.len() {
                let found = list.chunks(3).find(|r| strtopcmp(&b[pos..], r[0]) > 0);
                if let Some(r) = found {
                    pos += r[0].len();
                    add(&mut node.read, r[1]);
                    add(&mut node.pron, r[1]);
                    node.add_mora_size(atoi(r[2]));
                } else {
                    pos += 1;
                }
            }
            if node.mora_size != 0 {
                node.pos = val(NJD_SET_PRONUNCIATION_FILLER);
                node.pos_group1 = None;
                node.pos_group2 = None;
                node.pos_group3 = None;
            }
            if node.orig() == "*" {
                node.orig = val(&s);
            }
            // 既知の記号
            if node.pron() == "*"
                && let Some(r) = NJD_SET_PRONUNCIATION_SYMBOL_LIST.chunks(2).find(|r| node.string() == r[0]) {
                    node.read = val(r[1]);
                    node.pron = val(r[1]);
                }
            // それ以外は読点にする
            if node.pron() == "*" {
                node.read = val(NJD_SET_PRONUNCIATION_TOUTEN);
                node.pron = val(NJD_SET_PRONUNCIATION_TOUTEN);
                node.pos = val(NJD_SET_PRONUNCIATION_KIGOU);
                node.pos_group1 = val(NJD_SET_PRONUNCIATION_TOUTEN_POS_GROUP1);
                node.pos_group2 = val("*");
                node.pos_group3 = val("*");
                node.ctype = val("*");
                node.cform = val("*");
            }
        }
        n = njd.nodes[i].next;
    }
    njd.remove_silent();

    // 続くかなのフィラーをつなげる
    let mut head_of_seq: Option<usize> = None;
    let mut n = njd.head;
    while let Some(i) = n {
        if njd.nodes[i].pos() == NJD_SET_PRONUNCIATION_FILLER {
            let s = njd.nodes[i].string().to_string();
            if list.chunks(3).any(|r| s == r[0]) {
                match head_of_seq {
                    None => head_of_seq = Some(i),
                    Some(h) => {
                        let o = njd.nodes[i].clone();
                        let hn = &mut njd.nodes[h];
                        add(&mut hn.string, o.string());
                        add(&mut hn.orig, o.orig());
                        add(&mut hn.read, o.read());
                        add(&mut hn.pron, o.pron());
                        hn.add_mora_size(o.mora_size);
                        njd.nodes[i].pron = None;
                    }
                }
            } else {
                head_of_seq = None;
            }
        } else {
            head_of_seq = None;
        }
        n = njd.nodes[i].next;
    }
    njd.remove_silent();

    let mut n = njd.head;
    while let Some(i) = n {
        if let Some(nx) = njd.nodes[i].next {
            let (cur, next) = (&njd.nodes[i], &njd.nodes[nx]);
            if next.pron() == NJD_SET_PRONUNCIATION_U
                && next.pos() == NJD_SET_PRONUNCIATION_JODOUSHI
                && (cur.pos() == NJD_SET_PRONUNCIATION_DOUSHI || cur.pos() == NJD_SET_PRONUNCIATION_JODOUSHI)
                && cur.mora_size > 0
            {
                njd.nodes[nx].pron = val(NJD_SET_PRONUNCIATION_CHOUON);
            }
            let (cur, next) = (&njd.nodes[i], &njd.nodes[nx]);
            if cur.pos() == NJD_SET_PRONUNCIATION_JODOUSHI
                && (next.string() == NJD_SET_PRONUNCIATION_QUESTION
                    || next.string() == NJD_SET_PRONUNCIATION_EXCLAMATION)
            {
                if cur.string() == NJD_SET_PRONUNCIATION_DESU_STR {
                    njd.nodes[i].pron = val(NJD_SET_PRONUNCIATION_DESU_PRON);
                } else if cur.string() == NJD_SET_PRONUNCIATION_MASU_STR {
                    njd.nodes[i].pron = val(NJD_SET_PRONUNCIATION_MASU_PRON);
                }
            }
        }
        n = njd.nodes[i].next;
    }
}
