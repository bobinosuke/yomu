//! Supertonic 3 での合成。日本語も英語も同じ声で読む。
//!
//! 公式のサンプル (https://github.com/supertone-oss-archive/supertonic の rust/src/helper.rs、MIT) を移植したもの。
//! 文字をそのまま (Unicode の番号で) モデルに渡し、読みも抑揚もモデルが決める。
//! 流れ: 長さの予測 (duration_predictor) → 文の符号化 (text_encoder) → ノイズから潜在表現を作る (vector_estimator を
//! TOTAL_STEP 回) → 波形 (vocoder)。
use std::path::Path;
use std::sync::Mutex;

use ort::session::Session;
use ort::value::Tensor;
use unicode_normalization::UnicodeNormalization;

use crate::resources::Resources;
use crate::sentences::{Lang, cut_long};

pub const DEFAULT_VOICE: &str = "F5";
const TOTAL_STEP: usize = 8; // ノイズを取り除く回数 (公式の既定)。減らすと速いが音が粗くなる
const BASE_SPEED: f32 = 1.05; // 公式の既定の速さ
const CHUNK_SILENCE: f32 = 0.25; // 長い文を分けて合成したときの間 (秒)
const SILENCE_DB: f32 = -40.0; // 一番大きい所よりこれだけ小さい所は無音とみなす (前後の無音を切り取るのに使う)
const SILENCE_MARGIN: f32 = 0.15; // 前後の無音を切り取るときに残す余白 (秒)
// 切り取った端を絞る長さ (秒)。モデルの出力には声のない所にも小さなノイズがあり、急に無音にすると途切れて聞こえる
const SILENCE_FADE: f32 = 0.02;

/// モデルの設定 (tts.json の一部)
struct Config {
    sample_rate: u32,
    base_chunk_size: usize,
    chunk_compress_factor: usize,
    latent_dim: usize,
}

/// 声の特徴 (voice_styles/*.json)
struct Style {
    ttl: (Vec<usize>, Vec<f32>),
    dp: (Vec<usize>, Vec<f32>),
}

struct Sessions {
    duration: Session,
    text_encoder: Session,
    vector_estimator: Session,
    vocoder: Session,
}

pub struct Supertonic {
    cfg: Config,
    indexer: Vec<i64>, // Unicode の番号 → 文字の番号
    style: Style,
    sessions: Mutex<Sessions>,
    seed: Mutex<u64>, // ノイズの乱数
}

fn err(e: impl std::fmt::Display) -> String {
    format!("Supertonic: {e}")
}

fn read_json(path: &Path) -> Result<serde_json::Value, String> {
    let s = std::fs::read_to_string(path).map_err(|e| err(format!("{}: {e}", path.display())))?;
    serde_json::from_str(&s).map_err(err)
}

impl Supertonic {
    pub fn open(res: &Resources, voice: &str) -> Result<Self, String> {
        let dir = res.supertonic();
        let onnx = dir.join("onnx");
        let tts = read_json(&onnx.join("tts.json"))?;
        let num = |v: &serde_json::Value| v.as_u64().map(|x| x as usize).ok_or_else(|| err("tts.json が読めない"));
        let cfg = Config {
            sample_rate: num(&tts["ae"]["sample_rate"])? as u32,
            base_chunk_size: num(&tts["ae"]["base_chunk_size"])?,
            chunk_compress_factor: num(&tts["ttl"]["chunk_compress_factor"])?,
            latent_dim: num(&tts["ttl"]["latent_dim"])?,
        };
        let indexer: Vec<i64> = serde_json::from_value(read_json(&onnx.join("unicode_indexer.json"))?).map_err(err)?;
        let style = load_style(&dir.join("voice_styles").join(format!("{voice}.json")))?;
        // 合成のスレッド数は CPU の半分に抑え、再生側へのデータ供給が遅れて音が欠けるのを防ぐ
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).div_ceil(2);
        let load = |name: &str| -> Result<Session, String> {
            let fail = |e: &dyn std::fmt::Display| err(format!("{name}: {e}"));
            Session::builder()
                .map_err(|e| fail(&e))?
                .with_intra_threads(threads)
                .map_err(|e| fail(&e))?
                .commit_from_file(onnx.join(name))
                .map_err(|e| fail(&e))
        };
        let sessions = Sessions {
            duration: load("duration_predictor.onnx")?,
            text_encoder: load("text_encoder.onnx")?,
            vector_estimator: load("vector_estimator.onnx")?,
            vocoder: load("vocoder.onnx")?,
        };
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        Ok(Self { cfg, indexer, style, sessions: Mutex::new(sessions), seed: Mutex::new(seed | 1) })
    }

    pub fn sample_rate(&self) -> u32 {
        self.cfg.sample_rate
    }

    /// 1 文を合成する。speed は読む速さの倍率。長い文は分けて合成してつなぐ。
    /// min_secs は合成する長さの下限 (速さ 1 のときの秒。モデルの見積もりが短すぎて早口になるのを防ぐ。0 なら見積もりのまま)。
    /// モデルは前後に 0.3〜0.7 秒の無音を付けて出すので切り取る (文の間の無音は再生する側で置く。
    /// 切り取らないと、短い見出しなどは声より無音のほうが長くなり、ぶつ切りに聞こえる)
    pub fn synthesize(&self, text: &str, lang: Lang, speed: f32, min_secs: f32) -> Result<Vec<f32>, String> {
        // 公式のサンプルと同じく、日本語と韓国語は 120 文字、ほかは 300 文字までを一度に合成する
        let max_len = if lang == Lang::JA || lang == Lang::KO { 120 } else { 300 };
        let total = text.chars().count().max(1) as f32;
        let mut out = Vec::new();
        for (i, chunk) in cut_long(text, max_len).iter().enumerate() {
            if i > 0 {
                out.extend(std::iter::repeat_n(0.0, (CHUNK_SILENCE * self.cfg.sample_rate as f32) as usize));
            }
            // 分けたときは、下限を文字の数で割り振る
            let min = min_secs * chunk.chars().count() as f32 / total;
            let wav = self.infer(&preprocess(chunk, lang.code()), speed * BASE_SPEED, min)?;
            out.extend(trim_silence(&wav, self.cfg.sample_rate));
        }
        Ok(out)
    }

    fn infer(&self, text: &str, speed: f32, min_secs: f32) -> Result<Vec<f32>, String> {
        let ids: Vec<i64> = text
            .chars()
            .map(|c| self.indexer.get(c as usize).copied().unwrap_or(-1))
            .collect();
        let n = ids.len();
        let tensor = |shape: &[usize], data: Vec<f32>| Tensor::from_array((shape.to_vec(), data)).map_err(err);
        let text_ids = Tensor::from_array(([1usize, n], ids)).map_err(err)?;
        let text_mask = tensor(&[1, 1, n], vec![1.0; n])?;
        let style_dp = tensor(&self.style.dp.0, self.style.dp.1.clone())?;
        let style_ttl = tensor(&self.style.ttl.0, self.style.ttl.1.clone())?;
        let mut s = self.sessions.lock().unwrap();

        // 長さ (秒) の予測
        let out = s
            .duration
            .run(ort::inputs! {"text_ids" => &text_ids, "style_dp" => &style_dp, "text_mask" => &text_mask})
            .map_err(err)?;
        let duration = out["duration"].try_extract_tensor::<f32>().map_err(err)?.1[0].max(min_secs) / speed;
        drop(out);

        let out = s
            .text_encoder
            .run(ort::inputs! {"text_ids" => &text_ids, "style_ttl" => &style_ttl, "text_mask" => &text_mask})
            .map_err(err)?;
        let (shape, data) = out["text_emb"].try_extract_tensor::<f32>().map_err(err)?;
        let text_emb = tensor(&shape.iter().map(|&d| d as usize).collect::<Vec<_>>(), data.to_vec())?;
        drop(out);

        // 長さに合わせたノイズから始めて、少しずつノイズを取り除く
        let wav_len = (duration * self.cfg.sample_rate as f32) as usize;
        let chunk = self.cfg.base_chunk_size * self.cfg.chunk_compress_factor;
        let latent_len = wav_len.div_ceil(chunk).max(1);
        let dim = self.cfg.latent_dim * self.cfg.chunk_compress_factor;
        let mut latent = self.normal_noise(dim * latent_len);
        let latent_mask = tensor(&[1, 1, latent_len], vec![1.0; latent_len])?;
        let total = tensor(&[1], vec![TOTAL_STEP as f32])?;
        for step in 0..TOTAL_STEP {
            let xt = tensor(&[1, dim, latent_len], latent)?;
            let current = tensor(&[1], vec![step as f32])?;
            let out = s
                .vector_estimator
                .run(ort::inputs! {
                    "noisy_latent" => &xt, "text_emb" => &text_emb, "style_ttl" => &style_ttl,
                    "latent_mask" => &latent_mask, "text_mask" => &text_mask,
                    "current_step" => &current, "total_step" => &total
                })
                .map_err(err)?;
            latent = out["denoised_latent"].try_extract_tensor::<f32>().map_err(err)?.1.to_vec();
        }

        let latent = tensor(&[1, dim, latent_len], latent)?;
        let out = s.vocoder.run(ort::inputs! {"latent" => &latent}).map_err(err)?;
        let wav = out["wav_tts"].try_extract_tensor::<f32>().map_err(err)?.1;
        Ok(wav[..wav_len.min(wav.len())].to_vec())
    }

    /// 標準正規分布の乱数を n 個 (xorshift と Box-Muller)
    fn normal_noise(&self, n: usize) -> Vec<f32> {
        let mut s = self.seed.lock().unwrap();
        let mut next = || {
            *s ^= *s << 13;
            *s ^= *s >> 7;
            *s ^= *s << 17;
            ((*s >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        };
        (0..n)
            .map(|_| {
                let (u1, u2) = (next(), next());
                ((-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()) as f32
            })
            .collect()
    }
}

/// 前後の無音を切り取る (声の前後に SILENCE_MARGIN だけ残し、端を SILENCE_FADE かけて絞る)。
/// 10ms ごとの音の大きさで判断する
pub fn trim_silence(wav: &[f32], rate: u32) -> Vec<f32> {
    let frame = (rate / 100) as usize;
    let rms: Vec<f32> = wav.chunks(frame).map(|c| (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt()).collect();
    let max = rms.iter().copied().fold(0.0, f32::max);
    let threshold = max * 10f32.powf(SILENCE_DB / 20.0);
    let (Some(first), Some(last)) = (rms.iter().position(|&r| r > threshold), rms.iter().rposition(|&r| r > threshold)) else {
        return Vec::new();
    };
    let margin = (SILENCE_MARGIN * rate as f32) as usize;
    let start = (first * frame).saturating_sub(margin);
    let end = ((last + 1) * frame + margin).min(wav.len());
    let mut out = wav[start..end].to_vec();
    let n = ((SILENCE_FADE * rate as f32) as usize).min(out.len() / 2);
    let len = out.len();
    for i in 0..n {
        let g = i as f32 / n as f32;
        out[i] *= g;
        out[len - 1 - i] *= g;
    }
    out
}

fn load_style(path: &Path) -> Result<Style, String> {
    let v = read_json(path)?;
    let part = |key: &str| -> Result<(Vec<usize>, Vec<f32>), String> {
        let dims: Vec<usize> = serde_json::from_value(v[key]["dims"].clone()).map_err(err)?;
        let data: Vec<Vec<Vec<f32>>> = serde_json::from_value(v[key]["data"].clone()).map_err(err)?;
        Ok((dims, data.into_iter().flatten().flatten().collect()))
    };
    Ok(Style { ttl: part("style_ttl")?, dp: part("style_dp")? })
}

/// 公式の前処理 (helper.py の _preprocess_text): NFKD、絵文字を消す、記号を置き換える、末尾に句点がなければ足す、
/// 言語のタグで囲む
pub fn preprocess(text: &str, lang: &str) -> String {
    let mut t: String = text.nfkd().filter(|&c| !is_emoji(c)).collect();
    for (from, to) in [
        ("–", "-"),
        ("‑", "-"),
        ("—", "-"),
        ("_", " "),
        ("\u{201C}", "\""),
        ("\u{201D}", "\""),
        ("\u{2018}", "'"),
        ("\u{2019}", "'"),
        ("´", "'"),
        ("`", "'"),
        ("[", " "),
        ("]", " "),
        ("|", " "),
        ("/", " "),
        ("#", " "),
        ("→", " "),
        ("←", " "),
    ] {
        t = t.replace(from, to);
    }
    t.retain(|c| !"♥☆♡©\\".contains(c));
    for (from, to) in [("@", " at "), ("e.g.,", "for example, "), ("i.e.,", "that is, ")] {
        t = t.replace(from, to);
    }
    for p in [",", ".", "!", "?", ";", ":", "'"] {
        t = t.replace(&format!(" {p}"), p);
    }
    for q in ["\"", "'", "`"] {
        let double = q.repeat(2);
        while t.contains(&double) {
            t = t.replace(&double, q);
        }
    }
    let mut t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if !t.ends_with(|c: char| ".!?;:,'\")]}…。」』】〉》›»".contains(c)) {
        t.push('.');
    }
    format!("<{lang}>{t}</{lang}>")
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32, 0x1F600..=0x1F64F | 0x1F300..=0x1F5FF | 0x1F680..=0x1F6FF | 0x1F700..=0x1F8FF
        | 0x1F900..=0x1FAFF | 0x2600..=0x26FF | 0x2700..=0x27BF | 0x1F1E6..=0x1F1FF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocess_like_official() {
        assert_eq!(preprocess("こんにちは", "ja"), "<ja>こんにちは.</ja>");
        assert_eq!(preprocess("Hello , world!", "en"), "<en>Hello, world!</en>");
        assert_eq!(preprocess("a — b", "en"), "<en>a - b.</en>");
    }

    #[test]
    fn silence_is_trimmed_with_margin() {
        let rate = 1000;
        let mut wav = vec![0.0; 500];
        wav.extend(std::iter::repeat_n(0.5, 200));
        wav.extend(vec![0.0; 300]);
        let t = trim_silence(&wav, rate);
        assert_eq!(t.len(), 200 + 2 * 150); // 声の前後に 0.15 秒ずつ
        assert_eq!((t[0], t[t.len() - 1]), (0.0, 0.0)); // 端は絞る
        assert!(trim_silence(&[0.0; 100], rate).is_empty());
    }
}
