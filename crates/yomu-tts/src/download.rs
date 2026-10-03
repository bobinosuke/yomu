//! 辞書・モデルの用意。初回にダウンロードして ~/.cache/yomu に置く。ダウンロードしたものは SHA-256 で確かめる。
//! 先に足りないものを一覧にして、全体の量で進み具合を知らせる。途中で止めても、置き終えたファイルは次に使う
//!
//! - Supertonic 3 (読み上げのモデル): Hugging Face の Supertone/supertonic-3 から、版を固定して取る
//! - OpenJTalk の辞書: pyopenjtalk-plus (Python 版と同じ版) の wheel から取り出す
//! - AivisSpeech の辞書: Python 版と同じ場所・同じ置き方なので、Python 版がダウンロード済みならそれを使う
//! - 言語の判定 (lingua) の統計データ: crates.io の言語ごとのパッケージ (lingua-*-language-model 1.3.0) から取り出す
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::resources::Resources;

const AIVIS_REPO: &str = "Aivis-Project/AivisSpeech-Engine";
const AIVIS_DICT_PATH: &str = "resources/dictionaries/";

/// OpenJTalk の辞書 (pyopenjtalk-plus 0.4.1.post9 が同梱するもの)。辞書はどの OS の wheel でも同じ
const OPENJTALK_WHEEL: &str = "https://files.pythonhosted.org/packages/49/46/f509711876c6997842586bd7a55f8803dc0d37d8aa2ab65485d36b844507/pyopenjtalk_plus-0.4.1.post9-cp312-cp312-macosx_10_13_universal2.whl";
const OPENJTALK_WHEEL_SHA256: &str = "3a0c9e3d7a61e72be10c6e2dc37449037e72b2b6c7a97633eb03e393836936a2";
const OPENJTALK_WHEEL_SIZE: u64 = 25_656_701;
const OPENJTALK_FILES: [&str; 5] = ["sys.dic", "unk.dic", "char.bin", "matrix.bin", "COPYING"];

/// Supertonic 3 のファイル (Supertone/supertonic-3 の 2026-05-18 の版)
const SUPERTONIC_URL: &str = "https://huggingface.co/Supertone/supertonic-3/resolve/3cadd1ee6394adea1bd021217a0e650ede09a323/";
/// (ファイル, SHA-256, 大きさ)
const SUPERTONIC_FILES: &[(&str, &str, u64)] = &[
    ("LICENSE", "0d944a9110fed9a9602d60e0423a272903e7bd21ab060490774efc77c2275e9f", 15007),
    ("onnx/tts.json", "42078d3aef1cd43ab43021f3c54f47d2d75ceb4e75f627f118890128b06a0d09", 8253),
    ("onnx/unicode_indexer.json", "9bf7346e43883a81f8645c81224f786d43c5b57f3641f6e7671a7d6c493cb24f", 277676),
    ("onnx/duration_predictor.onnx", "c3eb91414d5ff8a7a239b7fe9e34e7e2bf8a8140d8375ffb14718b1c639325db", 3700147),
    ("onnx/text_encoder.onnx", "c7befd5ea8c3119769e8a6c1486c4edc6a3bc8365c67621c881bbb774b9902ff", 36416150),
    ("onnx/vector_estimator.onnx", "883ac868ea0275ef0e991524dc64f16b3c0376efd7c320af6b53f5b780d7c61c", 256534781),
    ("onnx/vocoder.onnx", "085de76dd8e8d5836d6ca66826601f615939218f90e519f70ee8a36ed2a4c4ba", 101424195),
    ("voice_styles/F1.json", "bbdec6ee00231c2c742ad05483df5334cab3b52fda3ba38e6a07059c4563dbc2", 292046),
    ("voice_styles/F2.json", "7c722c6a72707b1a77f035d67f0d1351ba187738e06f7683e8c72b1df3477fc6", 292423),
    ("voice_styles/F3.json", "12f6ef2573baa2defa1128069cb59f203e3ab67c92af77b42df8a0e3a2f7c6ab", 290794),
    ("voice_styles/F4.json", "c2fa764c1225a76dfc3e2c73e8aa4f70d9ee48793860eb34c295fff01c2e032b", 291808),
    ("voice_styles/F5.json", "45966e73316415626cf41a7d1c6f3b4c70dbc1ba2bee5c1978ef0ce33244fc8d", 291479),
    ("voice_styles/M1.json", "e35604687f5d23694b8e91593a93eec0e4eca6c0b02bb8ed69139ab2ea6b0a5b", 291748),
    ("voice_styles/M2.json", "b76cbf62bac707c710cf0ae5aba5e31eea1a6339a9734bfae33ab98499534a50", 292055),
    ("voice_styles/M3.json", "ea1ac35ccb91b0d7ecad533a2fbd0eec10c91513d8951e3b25fbba99954e159b", 290198),
    ("voice_styles/M4.json", "ca8eefad4fcd989c9379032ff3e50738adc547eeb5e221b82593a6d7b3bac303", 291522),
    ("voice_styles/M5.json", "dd22b92740314321f8ae11c5e87f8dd60d060f15dd3a632b5adf77f471f77af2", 291469),
];

/// lingua の言語ごとの統計データのパッケージ (lingua の機能の名前, パッケージの SHA-256, 大きさ)。どれも 1.3.0。
/// 判定の候補にする言語 (sentences.rs の DETECTOR) と同じ
const LINGUA_VERSION: &str = "1.3.0";
const LINGUA_MODELS: &[(&str, &str, u64)] = &[
    ("arabic", "5bcc254ff44209c2a50dea58a644f9c257a0aa5cd7e1739fd9c373fe4c7456cf", 5213203),
    ("bulgarian", "2f4679441ff2d94b21a3d76293ffd5310a012ea62247b7b2066939e2728cbfc0", 3020538),
    ("chinese", "21ca7fa9f7671d684c82c168725f380fc873f14d6f4e8c82f0da681bcc0048d1", 131712),
    ("croatian", "d86b1346b98397e7cf87e8f3fe544fce6514918e6bde253a014eea7ca03c31d8", 3389508),
    ("czech", "348f06e4f90e1c2bc597ea3caf52abb9d3dd09ba227fa7cf4f2dc723a9810e98", 5256761),
    ("danish", "363953413132601a06fe0cae1fffb5d519d0cfb378049cc78b3893edd03a194f", 3283961),
    ("dutch", "fe0da523f4726c05ed557b5a309315c1c161a5ec9ca6036c6c1c799ce492b698", 3238585),
    ("english", "97102de08b134a49f1cce05a1b6f5bf08ef21fe858074ae2b794e7892c43dd4b", 2589587),
    ("estonian", "7095f107a6e89147a9066db6a3f9e1b0a4706323d59a4bb58fa8218d74c1fde8", 3343172),
    ("finnish", "f5b3c44089812704148c89dbe77fb34122c2f6e8182bbf0f72ea69c92e948e9e", 3239332),
    ("french", "45508227e42c9cc5eb202e17c4c40e38ea7b9be9421faeb3ab0fb7ac37d9c681", 2558887),
    ("german", "f584da803e8c135ea22dd3ed321a6b3e2ff3465559606be0a597924ecd465cb2", 3463282),
    ("greek", "a5da9a688ceb41963f3555526a2e4431e0e5208a3320e56b72b1de998ba6b1c7", 4190204),
    ("hindi", "dca88af8dede1a30fcd79318b9c22f3662785fd87ed5b39881d19770651bf720", 325532),
    ("hungarian", "0d7072b2cdf43438214e82b53c51ad85a16d657e5c07a5a77bef78a4cd39423c", 6232488),
    ("indonesian", "fe1f2e05145c3fb4172c01c4563f729938c85f77742948444523dfcda2c3e80d", 2079520),
    ("italian", "83a87385ee42f6a0306df81066d4f77c225a337137fc15978a4b319c6bcf4bfe", 2190758),
    ("japanese", "df0938f75de3ae5dcdc925d823ed409854ca14f6a653782b9a1ad5d899462fbe", 81955),
    ("korean", "aa87f6c43ff894fc75159c021480d2fdf96882bf5bd235f8916ceb6b7caae561", 105036),
    ("latvian", "f1b2f30766e9e1ce10d960ce3eb2da6514fd0ee18c8d617a566b03126140e656", 3838919),
    ("lithuanian", "4e4a2a1994e48841c1d2d63099fbd68476615bf0d833fa14a3654f7043e38fc7", 3972760),
    ("persian", "ddf084e05d33ed66d64461e6397b571fdf74fe93c13c4ac4c84a633e33d2d07b", 3309288),
    ("polish", "72eb03e7940b3178c152138a16976e374ed0a3ddc80dc6f0b56984ab1189cb67", 3761854),
    ("portuguese", "432eda7984456055033ffa168037be9afe0f5c9ecd891bf5f27435247d496b47", 2297653),
    ("romanian", "1051aa082754acfc52110456e29fbbf6d2c648da9a4753a861c34f19325cfab8", 3092594),
    ("russian", "4fc0850578299531b50192be2c1de1af651f1289784253645e737fe456a6a138", 3259320),
    ("slovak", "529a596f3d90b2051b3ead90e864f6ad9e14b43716c3f252268e9e6e43ab28de", 4487233),
    ("slovene", "db52978574611533d873b727281d1c1d0ab2b031331654c1729723b682bc76a1", 3102006),
    ("spanish", "56395a8d96c892130a9efb433c5f042977fb75ae5fb6e0058c8814a947459137", 2505036),
    ("swedish", "77c965c5b11d6e1e619a98e393d16f73a0754cb05b5b067b537fd25e9b74bec0", 3552321),
    ("thai", "2582eeabf02e39680856c3f88d3b93fcdb351009407d38a4735326c8b0a33bce", 2741493),
    ("turkish", "2a2ab6b46c596ebd7f58e0db7a3d732c42a2fa48b77094a0737e920a5c47dca0", 3324609),
    ("ukrainian", "b86175a68d53b3b1e3aaf2e2fa61d31e8f03df7e0fcb51d7e1dd62fce86cb393", 3458990),
    ("urdu", "1d308fdae7c8caa05ffe455e8a2e568e5f018cb2742f588a810ecf306b890622", 3452493),
    ("vietnamese", "45ef9a92dce65c6e9fea7f2343e51de8b5b9cd764fa8ae3592c4ba3edd992d4a", 1326125),
];
/// 言語ごとに置く統計データのファイル
const LINGUA_FILES: [&str; 3] = ["ngrams.fst", "unique-ngrams.fst", "mostcommon-ngrams.fst"];

/// 止められたときの Err
pub const CANCELLED: &str = "ダウンロードを中断しました";

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent("yomu")
        .connect_timeout(Duration::from_secs(30))
        .timeout(None)
        .build()
        .map_err(|e| e.to_string())
}

/// AivisSpeech の辞書の合計の目安 (一覧を GitHub から取るまで正確な大きさがわからないので、2026 年 10 月の大きさ)
const AIVIS_APPROX_SIZE: u64 = 238_541_245;

/// まだダウンロードしていないものの合計 (バイト。通信せずに求める目安)。0 ならダウンロードはいらない
pub fn missing_bytes(res: &Resources) -> u64 {
    let mut n: u64 = SUPERTONIC_FILES.iter().filter(|(p, _, _)| !res.supertonic().join(p).exists()).map(|(_, _, s)| s).sum();
    if !res.openjtalk_dict().exists() {
        n += OPENJTALK_WHEEL_SIZE;
    }
    if !res.cache.join("aivis-dict").join(".complete").exists() {
        n += AIVIS_APPROX_SIZE;
    }
    for (name, _, size) in LINGUA_MODELS {
        let code = name.parse::<lingua::Language>().map(|l| l.iso_code_639_1().to_string()).unwrap_or_default();
        if !res.lingua().join(code).exists() {
            n += size;
        }
    }
    n
}

/// ダウンロードしたものの置き方
enum Then {
    /// そのまま置く
    Keep,
    /// pyopenjtalk-plus の wheel から OpenJTalk の辞書を取り出す
    OpenJtalk,
    /// lingua のパッケージから、この言語のコードのディレクトリへ統計データを取り出す
    Lingua(String),
}

/// ダウンロードするもの 1 つ
struct Item {
    url: String,
    file: PathBuf,
    sha256: Option<&'static str>,
    size: u64,
    then: Then,
}

/// 足りないものをダウンロードする。say には進み具合を知らせる。cancelled が true を返したら止めて Err(CANCELLED)
pub fn ensure(res: &Resources, say: &dyn Fn(&str), cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    let client = client()?;
    let aivis_dir = res.cache.join("aivis-dict");
    let mut items = Vec::new();
    for (path, sha, size) in SUPERTONIC_FILES {
        let file = res.supertonic().join(path);
        if !file.exists() {
            items.push(Item { url: format!("{SUPERTONIC_URL}{path}"), file, sha256: Some(sha), size: *size, then: Then::Keep });
        }
    }
    let work = res.cache.join(".download");
    if !res.openjtalk_dict().exists() {
        let file = work.join("pyopenjtalk_plus.whl");
        items.push(Item { url: OPENJTALK_WHEEL.into(), file, sha256: Some(OPENJTALK_WHEEL_SHA256), size: OPENJTALK_WHEEL_SIZE, then: Then::OpenJtalk });
    }
    let aivis = !aivis_dir.join(".complete").exists();
    if aivis {
        say("読み上げの準備: ダウンロードするものを確かめています");
        items.extend(aivis_dicts(&client, &aivis_dir)?);
    }
    for (name, sha, size) in LINGUA_MODELS {
        let code = name.parse::<lingua::Language>().map_err(|e| format!("{name}: {e}"))?.iso_code_639_1().to_string();
        if !res.lingua().join(&code).exists() {
            let krate = format!("lingua-{name}-language-model");
            let url = format!("https://static.crates.io/crates/{krate}/{krate}-{LINGUA_VERSION}.crate");
            items.push(Item { url, file: work.join(format!("{krate}.crate")), sha256: Some(sha), size: *size, then: Then::Lingua(code) });
        }
    }
    let total: u64 = items.iter().map(|i| i.size).sum();
    let mb = |n: u64| n.div_ceil(1 << 20);
    let mut done = 0;
    for item in &items {
        let last = std::cell::Cell::new(u64::MAX);
        download(&client, &item.url, &item.file, item.sha256, cancelled, &|n| {
            // 全体の量で知らせる (1MB 進むごと)
            let now = (done + n).min(total);
            if mb(now) != last.get() {
                last.set(mb(now));
                say(&format!("読み上げの準備 (初回のみ): ダウンロード中 {}/{}MB {}%", mb(now), mb(total), now * 100 / total.max(1)));
            }
        })?;
        done += item.size;
        match &item.then {
            Then::Keep => {}
            Then::OpenJtalk => openjtalk_dict(&item.file, res)?,
            Then::Lingua(code) => lingua_model(&item.file, &res.lingua().join(code))?,
        }
    }
    if aivis {
        File::create(aivis_dir.join(".complete")).map_err(|e| e.to_string())?;
    }
    let _ = fs::remove_dir_all(&work);
    Ok(())
}

/// pyopenjtalk-plus の wheel から OpenJTalk の辞書を取り出す。全部そろってから置き場所に移すので、途中で止まっても半端なものが残らない
fn openjtalk_dict(wheel: &Path, res: &Resources) -> Result<(), String> {
    let fail = |e: io::Error| format!("OpenJTalk の辞書を展開できない: {e}");
    let out = wheel.with_extension("dict");
    let _ = fs::remove_dir_all(&out);
    fs::create_dir_all(&out).map_err(fail)?;
    let mut z = zip::ZipArchive::new(File::open(wheel).map_err(fail)?).map_err(|e| fail(e.into()))?;
    for name in OPENJTALK_FILES {
        let mut entry = z.by_name(&format!("pyopenjtalk/dictionary/{name}")).map_err(|e| fail(e.into()))?;
        io::copy(&mut entry, &mut File::create(out.join(name)).map_err(fail)?).map_err(fail)?;
    }
    fs::rename(&out, res.openjtalk_dict()).map_err(fail)?;
    let _ = fs::remove_file(wheel);
    Ok(())
}

/// lingua のパッケージ (.tar.gz) から models/ の下の統計データを取り出して dest に置く。全部そろってから置き場所に移す
fn lingua_model(krate: &Path, dest: &Path) -> Result<(), String> {
    let fail = |e: io::Error| format!("言語の判定のデータを展開できない: {e}");
    let out = krate.with_extension("models");
    let _ = fs::remove_dir_all(&out);
    fs::create_dir_all(&out).map_err(fail)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(File::open(krate).map_err(fail)?));
    for entry in tar.entries().map_err(fail)? {
        let mut entry = entry.map_err(fail)?;
        let path = entry.path().map_err(fail)?.into_owned();
        if let Some(f) = LINGUA_FILES.iter().find(|f| path.ends_with(format!("models/{f}"))) {
            io::copy(&mut entry, &mut File::create(out.join(f)).map_err(fail)?).map_err(fail)?;
        }
    }
    if let Some(f) = LINGUA_FILES.iter().find(|f| !out.join(f).exists()) {
        return Err(fail(io::Error::other(format!("{} に models/{f} がない", krate.display()))));
    }
    fs::create_dir_all(dest.parent().unwrap()).map_err(fail)?;
    fs::rename(&out, dest).map_err(fail)?;
    let _ = fs::remove_file(krate);
    Ok(())
}

/// url を dest にダウンロードする。sha256 があれば確かめる。途中で止まったり壊れていたりしても半端なファイルが
/// 残らないよう、.part に書いて確かめてから置き換える。progress にはこのファイルで受け取ったバイト数を知らせる
fn download(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
    sha256: Option<&str>,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64),
) -> Result<(), String> {
    let fail = |e: &dyn std::fmt::Display| format!("ダウンロードできない: {url}: {e}");
    fs::create_dir_all(dest.parent().unwrap()).map_err(|e| fail(&e))?;
    let mut r = client.get(url).send().and_then(|r| r.error_for_status()).map_err(|e| fail(&e))?;
    let tmp = dest.with_extension("part");
    let mut f = File::create(&tmp).map_err(|e| fail(&e))?;
    let mut done = 0u64;
    let mut buf = vec![0; 1 << 16];
    loop {
        if cancelled() {
            drop(f);
            let _ = fs::remove_file(&tmp);
            return Err(CANCELLED.into());
        }
        let n = r.read(&mut buf).map_err(|e| fail(&e))?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n]).map_err(|e| fail(&e))?;
        done += n as u64;
        progress(done);
    }
    drop(f);
    if let Some(want) = sha256 {
        let got = file_sha256(&tmp).map_err(|e| fail(&e))?;
        if got != want {
            let _ = fs::remove_file(&tmp);
            return Err(fail(&format!("ダウンロードが壊れている (SHA-256 が {got})")));
        }
    }
    fs::rename(&tmp, dest).map_err(|e| fail(&e))
}

fn file_sha256(path: &Path) -> io::Result<String> {
    let mut h = Sha256::new();
    io::copy(&mut File::open(path)?, &mut h)?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// AivisSpeech Engine の内蔵辞書 (コンパイル済みの MeCab ユーザー辞書) のうち、まだないもの。辞書は同梱せず、使う人の手元で
/// AivisSpeech Engine のリポジトリから直接ダウンロードする (辞書ごとの元データのライセンスはリポジトリに明記されていない)
fn aivis_dicts(client: &reqwest::blocking::Client, dir: &Path) -> Result<Vec<Item>, String> {
    let url = format!("https://api.github.com/repos/{AIVIS_REPO}/git/trees/master?recursive=1");
    let tree: serde_json::Value = client
        .get(&url)
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json())
        .map_err(|e| format!("AivisSpeech の辞書の一覧を取得できない: {e}"))?;
    Ok(tree["tree"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| Some((t["path"].as_str()?.strip_prefix(AIVIS_DICT_PATH)?, t["size"].as_u64().unwrap_or(0))))
        .filter(|(name, _)| name.ends_with(".dic") && !dir.join(name).exists())
        .map(|(name, size)| Item {
            url: format!("https://raw.githubusercontent.com/{AIVIS_REPO}/master/{AIVIS_DICT_PATH}{name}"),
            file: dir.join(name),
            sha256: None,
            size,
            then: Then::Keep,
        })
        .collect())
}
