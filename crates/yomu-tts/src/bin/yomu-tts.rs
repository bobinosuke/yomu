//! 読み上げを試すコマンド。
//!   yomu-tts [--lang ja] [--voice F5] [--speed 1.0] [--label] [--text] [--wav FILE] [文章...]
//! 文章を渡さなければ標準入力から読む。言語は指定しなければ文章から決める。
//! --text は読み上げのモデルに渡す文 (日本語は一部をカタカナにしたもの) を出す。--label は文を見出しや箇条書きの項目として読む。
use std::io::Read;

use yomu_tts::resources::Resources;
use yomu_tts::sentences::{Lang, detect_lang, split_sentences};
use yomu_tts::speaker::{Chunk, SENTENCE_GAP, Speaker};
use yomu_tts::supertonic::DEFAULT_VOICE;

const USAGE: &str = "yomu-tts [--lang ja|en|ko|fr|…] [--voice F1〜F5|M1〜M5] [--speed 1.0] [--label] [--text] [--wav FILE] [文章...]";

fn main() {
    if let Err(e) = run() {
        eprintln!("yomu-tts: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let (mut lang, mut voice, mut speed, mut show, mut label, mut wav, mut words) =
        (None, DEFAULT_VOICE.to_string(), 1.0f32, false, false, None, Vec::new());
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--lang" => {
                let code = args.next().unwrap_or_default();
                lang = Some(Lang::from_code(&code).ok_or(format!("読めない言語: {code}"))?);
            }
            "--voice" => voice = args.next().ok_or("--voice には声の名前を渡す")?,
            "--speed" => speed = args.next().and_then(|s| s.parse().ok()).ok_or("--speed には数を渡す")?,
            "--text" => show = true,
            "--label" => label = true,
            "--wav" => wav = Some(args.next().ok_or("--wav にはファイル名を渡す")?),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => words.push(a),
        }
    }
    let text = if words.is_empty() {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).map_err(|e| e.to_string())?;
        s
    } else {
        words.join(" ")
    };
    let sents = split_sentences(&text);
    // 言語の判定の統計データも Speaker::open でダウンロードするので、判定はその後
    let mut speaker = Speaker::open(&Resources::default(), &voice, &|m| eprint!("\r\x1b[K{m}"), &|| false)?;
    eprint!("\r\x1b[K");
    let lang = match lang.or_else(|| detect_lang(&sents, "")) {
        Some(l) => l,
        None => return Err("この文章の言語は読み上げに対応していない (--lang で指定できる)".into()),
    };
    speaker.speed = speed;
    if show {
        for s in &sents {
            println!("{}", speaker.text_for(s, lang, label));
        }
    }
    if let Some(path) = wav {
        return write_wav(&speaker, &sents, lang, label, &path);
    }
    if !show {
        let chunks = sents.into_iter().enumerate().map(|(blk, text)| Chunk { blk, text, label }).collect();
        speaker.start(chunks, lang, |_| {}, || {});
        speaker.wait();
    }
    Ok(())
}

/// 文ごとに合成し、再生のときの同じ段落の文の間と同じ無音を入れてつなぐ
fn write_wav(speaker: &Speaker, sents: &[String], lang: Lang, label: bool, path: &str) -> Result<(), String> {
    let rate = speaker.sample_rate();
    let spec = hound::WavSpec { channels: 1, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    let gap = (rate as f32 * SENTENCE_GAP) as usize;
    for s in sents {
        let Some(audio) = speaker.synthesize(s, lang, label)? else { continue };
        for v in audio.into_iter().chain(std::iter::repeat_n(0.0, gap)) {
            w.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16).map_err(|e| e.to_string())?;
        }
    }
    w.finalize().map_err(|e| e.to_string())
}
