//! 読み上げ。Python 版 (yomu-python) の Speaker と同じく、合成と再生を別スレッドにし、
//! 次の文を合成しながら今の文を再生する。
//! 合成は Supertonic 3。日本語も英語も同じ声で読む。日本語の文の中の英単語は、先にカタカナにしてから渡す
//! (日本語の声で英語の発音が混ざらないように)。
//! 再生は 1 本の出力ストリームを開いたまま流し込む (文ごとに開き直すと、Bluetooth の出力先で文の境目が途切れる)。
//! 出力先のサンプルレートがモデルの出力 (44.1kHz) と違うときは、線形補間で変換する。
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::frontend::Frontend;
use crate::katakana::{count_moras, text_for_tts};
use crate::resources::Resources;
use crate::sentences::Lang;
use crate::supertonic::Supertonic;

const LATENCY: f32 = 0.4; // 出力側にためておく長さ (秒)
const WRITE_SECS: f32 = 0.1; // 一度に書き込む長さ (秒)。停止の反応をよくするため細かく書く
pub const SENTENCE_GAP: f32 = 0.25; // 同じ段落の文と文の間の無音 (秒)
const BLOCK_GAP: f32 = 0.45; // 段落・見出しが変わるところの無音 (秒)
const QUEUE: usize = 3; // 合成済みで再生を待つ文の数
/// 日本語の見出しや箇条書きの項目 (label。漢字を辞書の読みのカタカナにして渡す) は、合成する長さをこの倍率で伸ばす。
/// モデルはカタカナの文を短く見積もり、そのままでは早口になって母音が抜けたように (無声化したように) 聞こえる。
/// また、長さが足りないと途中で切れることがある (参考文献 → サンコウっ)
const LABEL_STRETCH: f32 = 1.3;

/// 日本語の文を合成する長さの下限 (秒)。モデルの見積もりがこれより短ければ、ここまで伸ばす。
/// 短い語を「・」で並べた文 (荷物・場所・手順、山・海・水・米・雨など) は短く見積もられ、区切りの間に時間を取られて
/// 残りが早口になり、語を読み飛ばしたり母音が抜けたりする。
/// - 拍の部分: F5 の声で約 300 文のモデルの見積もりに当てはめた 0.27 + 拍の数 × 0.154 (秒) の 0.95 倍
/// - 区切り (文の中の句読点) の部分: 1 つ 0.3 秒。モデルは区切りに 0.15〜0.3 秒の間を取る。
///   聞き比べで良かった長さから逆算すると、上の 2 つの例とも区切り 1 つあたり 0.3 秒ほど要った
///
/// 普通の文は約 6 割が伸ばされず、伸ばされる文も 9 割は 1.09 倍まで
fn min_secs_ja(moras: usize, pauses: usize) -> f32 {
    0.95 * (0.27 + 0.154 * moras as f32) + 0.3 * pauses as f32
}

/// 読み上げる文
pub struct Chunk {
    pub blk: usize, // ブロック番号 (読んでいるブロックが変わったら知らせるのに使う)
    pub text: String,
    pub label: bool, // 見出しや箇条書きの項目のような、文になっていない短いもの (日本語は漢字を辞書の読みで読む。katakana.rs)
}

/// 合成に使うもの。合成のスレッドと共有する
struct Engines {
    frontend: Frontend,
    tts: Supertonic,
}

impl Engines {
    /// 1 文を合成する。読むものがなければ None
    fn synthesize(&self, text: &str, lang: Lang, label: bool, speed: f32) -> Result<Option<Vec<f32>>, String> {
        let t = text_for_tts(&self.frontend, text, lang, label);
        if !t.chars().any(char::is_alphanumeric) {
            return Ok(None);
        }
        let speed = if lang == Lang::JA && label { speed / LABEL_STRETCH } else { speed };
        let min_secs = if lang == Lang::JA {
            let (moras, pauses) = count_moras(&self.frontend, text);
            min_secs_ja(moras, pauses)
        } else {
            0.0
        };
        self.tts.synthesize(&t, lang, speed, min_secs).map(Some)
    }
}

pub struct Speaker {
    engines: Arc<Engines>,
    pub speed: f32,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Speaker {
    /// 足りない辞書・モデルがあれば先にダウンロードする (say に進み具合を知らせる。cancelled が true を返したら
    /// ダウンロードを止めて Err(download::CANCELLED))。voice は声 (F1〜F5、M1〜M5)
    pub fn open(res: &Resources, voice: &str, say: &dyn Fn(&str), cancelled: &dyn Fn() -> bool) -> Result<Self, String> {
        crate::download::ensure(res, say, cancelled)?;
        say("読み上げモデルを読み込み中");
        let engines = Engines { frontend: Frontend::open(res)?, tts: Supertonic::open(res, voice)? };
        Ok(Self { engines: Arc::new(engines), speed: 1.0, stop: Arc::new(AtomicBool::new(false)), threads: Vec::new() })
    }

    pub fn sample_rate(&self) -> u32 {
        self.engines.tts.sample_rate()
    }

    /// 読み上げのモデルに渡す文 (日本語は一部をカタカナにしたもの)
    pub fn text_for(&self, text: &str, lang: Lang, label: bool) -> String {
        text_for_tts(&self.engines.frontend, text, lang, label)
    }

    /// 1 文を合成する。読むものがなければ None
    pub fn synthesize(&self, text: &str, lang: Lang, label: bool) -> Result<Option<Vec<f32>>, String> {
        self.engines.synthesize(text, lang, label, self.speed)
    }

    pub fn speaking(&self) -> bool {
        self.threads.iter().any(|t| !t.is_finished())
    }

    /// 読んでいるブロックが変わるたびに on_block、読み終えたら on_done を呼ぶ
    pub fn start(
        &mut self,
        chunks: Vec<Chunk>,
        lang: Lang,
        on_block: impl Fn(usize) + Send + 'static,
        on_done: impl FnOnce() + Send + 'static,
    ) {
        self.stop();
        let stop = Arc::new(AtomicBool::new(false));
        self.stop = stop.clone();
        let (tx, rx) = sync_channel::<(usize, Vec<f32>)>(QUEUE);
        let (engines, speed, rate) = (self.engines.clone(), self.speed, self.sample_rate());
        let stop_p = stop.clone();
        let produce = std::thread::spawn(move || {
            for c in chunks {
                if stop_p.load(Ordering::Relaxed) {
                    break;
                }
                match engines.synthesize(&c.text, lang, c.label, speed) {
                    Ok(None) => {}
                    Ok(Some(a)) => {
                        if !send(&tx, (c.blk, a), &stop_p) {
                            break;
                        }
                    }
                    Err(e) => eprintln!("合成できない: {e}"),
                }
            }
        });
        let play = std::thread::spawn(move || {
            if let Err(e) = play(rx, rate, &stop, on_block) {
                eprintln!("再生できない: {e}");
            }
            if !stop.load(Ordering::Relaxed) {
                on_done();
            }
        });
        self.threads = vec![produce, play];
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.wait();
    }

    /// 読み終えるまで待つ
    pub fn wait(&mut self) {
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for Speaker {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 止めるまで送り続ける。止めたら false
fn send(tx: &SyncSender<(usize, Vec<f32>)>, mut item: (usize, Vec<f32>), stop: &AtomicBool) -> bool {
    loop {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        match tx.try_send(item) {
            Ok(()) => return true,
            Err(std::sync::mpsc::TrySendError::Full(back)) => {
                item = back;
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return false,
        }
    }
}

/// 出力側のバッファ。再生のコールバックがモデルの出力のサンプルを取り出し、出力先のサンプルレートに変換する
struct Buffer {
    samples: VecDeque<f32>,
    pos: f64, // 次に出す位置 (samples の先頭からの、モデルの出力のサンプル単位)
}

fn play(rx: Receiver<(usize, Vec<f32>)>, rate: u32, stop: &AtomicBool, on_block: impl Fn(usize)) -> Result<(), String> {
    let device = cpal::default_host().default_output_device().ok_or("出力先がない")?;
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    let channels = config.channels() as usize;
    let out_rate = config.sample_rate().0 as f64;
    let step = rate as f64 / out_rate;
    let buf = Arc::new(Mutex::new(Buffer { samples: VecDeque::new(), pos: 0.0 }));
    let cb = buf.clone();
    let stream = device
        .build_output_stream(
            &config.config(),
            move |data: &mut [f32], _| {
                let mut b = cb.lock().unwrap();
                for frame in data.chunks_mut(channels) {
                    let i = b.pos as usize;
                    let v = if i + 1 < b.samples.len() {
                        let t = (b.pos - i as f64) as f32;
                        b.samples[i] * (1.0 - t) + b.samples[i + 1] * t
                    } else if i < b.samples.len() {
                        b.samples[i]
                    } else {
                        0.0
                    };
                    if i < b.samples.len() {
                        b.pos += step;
                    }
                    frame.fill(v);
                }
                let used = (b.pos as usize).min(b.samples.len());
                b.samples.drain(..used);
                b.pos -= used as f64;
            },
            |e| eprintln!("再生のエラー: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;

    let max_buffered = (rate as f32 * LATENCY) as usize;
    let silence = |secs: f32| std::iter::repeat_n(0.0f32, (rate as f32 * secs) as usize);
    let mut last = None;
    'outer: while !stop.load(Ordering::Relaxed) {
        let (blk, samples) = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(item) => item,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        // 合成した音声は前後の無音を切り取ってあるので、ここで間を置く
        let gap = match last {
            None => 0.0,
            Some(b) if b == blk => SENTENCE_GAP,
            Some(_) => BLOCK_GAP,
        };
        if last != Some(blk) {
            on_block(blk);
            last = Some(blk);
        }
        let data: Vec<f32> = silence(gap).chain(samples).collect();
        for part in data.chunks((rate as f32 * WRITE_SECS) as usize) {
            // 出力側にたまっている分が LATENCY を超えないように待ってから書く
            loop {
                if stop.load(Ordering::Relaxed) {
                    break 'outer;
                }
                if buf.lock().unwrap().samples.len() < max_buffered {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            buf.lock().unwrap().samples.extend(part);
        }
    }
    if stop.load(Ordering::Relaxed) {
        return Ok(()); // 止めるときは残りを捨てる
    }
    // 読み終えたときは最後まで鳴らす
    while !buf.lock().unwrap().samples.is_empty() && !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(100));
    Ok(())
}
