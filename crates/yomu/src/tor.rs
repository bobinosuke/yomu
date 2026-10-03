//! プライベートモードの通信 (Tor)。Arti を組み込み、Tor Browser と同じく SOCKS の窓口を経由してすべての通信を Tor に通す。
//!
//! - 窓口は 127.0.0.1 の空いているポートに立て、yomu の通信だけが使う
//! - 名前の解決も Tor の出口で行う (socks5h)。自分の回線から DNS の問い合わせが出ない
//! - SOCKS のユーザー名ごとに別の回線を使う。yomu はユーザー名にページのサイト (first party) を入れるので、
//!   サイトごとに経路が分かれる (Tor Browser の first-party isolation と同じ)
//! - .onion のサイトにもつなぐ
//! - Tor の作業用データ (ネットワークの一覧・入口のリレー) は ~/.cache/yomu/arti に残す (閲覧の情報は入らない。Tor Browser と同じ)
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::Duration;

use arti_client::config::{BoolOrAuto, CfgPath};
use arti_client::{ErrorKind, HasKind, IsolationToken, StreamPrefs, TorClient, TorClientConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

type Tor = TorClient<tor_rtcompat::PreferredRuntime>;

/// 窓口のポート (つながったら Ok、つなげなければ Err)
static PROXY: OnceLock<Result<u16, String>> = OnceLock::new();
static STARTED: OnceLock<()> = OnceLock::new();
/// 宛先 (ホスト:ポート) ごとの、Tor でつなげなかった理由。SOCKS の返事では細かい理由を伝えられないので、
/// ここに残して fetch.rs が取り出し、エラーに出す
static FAILURES: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);
/// 窓口の準備ができるのを待つ上限
const WAIT: Duration = Duration::from_secs(120);

fn data_dir() -> std::path::PathBuf {
    std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default().join(".cache").join("yomu").join("arti")
}

/// Tor につなぎ始める (裏で。何度呼んでも 1 回だけ)。say には進み具合を知らせる
pub fn start(say: impl Fn(String) + Send + 'static) {
    if STARTED.set(()).is_err() {
        return;
    }
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("Tor の実行環境を作れない");
        rt.block_on(async move {
            say(t!("Tor に接続中…").into());
            match connect().await {
                Ok((tor, listener)) => {
                    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
                    let _ = PROXY.set(Ok(port));
                    say(t!("Tor に接続しました").into());
                    serve(tor, listener).await;
                }
                Err(e) => {
                    let _ = PROXY.set(Err(e.clone()));
                    say(t!("Tor に接続できませんでした: {e}", e = e));
                }
            }
        });
    });
}

async fn connect() -> Result<(Arc<Tor>, TcpListener), String> {
    let dir = data_dir();
    let mut b = TorClientConfig::builder();
    b.storage().state_dir(CfgPath::new_literal(dir.join("state"))).cache_dir(CfgPath::new_literal(dir.join("cache")));
    let cfg = b.build().map_err(|e| e.to_string())?;
    let tor = TorClient::create_bootstrapped(cfg).await.map_err(|e| e.to_string())?;
    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| e.to_string())?;
    Ok((tor, listener))
}

/// 窓口のプロキシの URL。first_party ごとに別の回線になる。Tor につながるまで待つ (WAIT まで)
pub fn proxy_for(first_party: &str) -> Result<String, String> {
    let t = std::time::Instant::now();
    loop {
        match PROXY.get() {
            Some(Ok(port)) => {
                // ユーザー名に使えない文字を避けて、サイトの名前をそのまま使う
                let user: String = first_party.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-')).collect();
                let user = if user.is_empty() { "none".to_string() } else { user };
                return Ok(format!("socks5h://{user}:yomu@127.0.0.1:{port}"));
            }
            Some(Err(e)) => return Err(t!("Tor に接続できませんでした: {e}", e = e)),
            None if STARTED.get().is_none() => return Err(t!("Tor を始めていません").into()),
            None if t.elapsed() > WAIT => return Err(t!("Tor への接続が終わりません").into()),
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// host:port に Tor でつなげなかった理由 (あれば。取り出すと消える)
pub fn take_failure(host: &str, port: u16) -> Option<String> {
    FAILURES.lock().unwrap().remove(&format!("{}:{port}", host.to_ascii_lowercase()))
}

/// つなげなかった理由と、SOCKS で返す番号 (RFC 1928)
fn describe(e: &arti_client::Error) -> (u8, String) {
    use ErrorKind::*;
    let (code, why) = match e.kind() {
        RemoteNetworkTimeout | ExitTimeout => (6, t!("出口のリレーが接続先から時間内に返事をもらえない (サイトが Tor からの接続を断っているか、混んでいる)")),
        RemoteConnectionRefused => (5, t!("接続先が接続を断った")),
        ExitPolicyRejected => (2, t!("Tor の出口がこの宛先 (ポート) への接続を許していない")),
        InvalidStreamTarget | OnionServiceAddressInvalid => (4, t!("宛先のアドレスが正しくない")),
        ForbiddenStreamTarget => (2, t!("Tor では使えない宛先 (手元のネットワークのアドレスなど)")),
        RemoteHostNotFound => (4, t!("出口のリレーで名前が見つからない (サイトのアドレスが違う)")),
        RemoteHostResolutionFailed | RemoteNetworkFailed => (4, t!("出口のリレーから接続先に届かない")),
        OnionServiceNotFound => (4, t!("この .onion の情報が見つからない (止まっているか、アドレスが違う)")),
        OnionServiceNotRunning => (4, t!("この .onion は止まっているらしい (情報はあるが、応答がない)")),
        OnionServiceConnectionFailed | OnionServiceProtocolViolation => (4, t!("この .onion につなげない (止まっているか、Tor の回線が不安定)")),
        OnionServiceMissingClientAuth | OnionServiceWrongClientAuth => (4, t!("この .onion は認証が必要")),
        TorNetworkTimeout | CircuitCollapse | TorAccessFailed | NoPath | NoExit => (1, t!("Tor の回線を作れない (手元の回線が不安定かもしれない)")),
        _ => (1, ""),
    };
    let why = if why.is_empty() { e.to_string() } else { format!("{why} [{e}]") };
    (code, why)
}

/// SOCKS5 の窓口。1 本ずつ受けて Tor の回線につなぐ
async fn serve(tor: Arc<Tor>, listener: TcpListener) {
    let groups: Arc<Mutex<HashMap<String, IsolationToken>>> = Arc::default();
    loop {
        let Ok((sock, _)) = listener.accept().await else { continue };
        let (tor, groups) = (tor.clone(), groups.clone());
        tokio::spawn(async move {
            let _ = handle(tor, groups, sock).await;
        });
    }
}

/// SOCKS5 (RFC 1928) の CONNECT と、ユーザー名とパスワードの認証 (RFC 1929) だけを扱う
async fn handle(tor: Arc<Tor>, groups: Arc<Mutex<HashMap<String, IsolationToken>>>, mut s: TcpStream) -> std::io::Result<()> {
    let bad = || std::io::Error::other("SOCKS の要求が読めない");
    // あいさつ: 認証はユーザー名とパスワード (2) だけを受ける
    let mut head = [0u8; 2];
    s.read_exact(&mut head).await?;
    let mut methods = vec![0u8; head[1] as usize];
    s.read_exact(&mut methods).await?;
    if head[0] != 5 || !methods.contains(&2) {
        s.write_all(&[5, 0xff]).await?;
        return Err(bad());
    }
    s.write_all(&[5, 2]).await?;
    let mut ver = [0u8; 2];
    s.read_exact(&mut ver).await?;
    let mut user = vec![0u8; ver[1] as usize];
    s.read_exact(&mut user).await?;
    let mut plen = [0u8; 1];
    s.read_exact(&mut plen).await?;
    let mut pass = vec![0u8; plen[0] as usize];
    s.read_exact(&mut pass).await?;
    s.write_all(&[1, 0]).await?;
    // 要求: CONNECT だけ。宛先は名前のまま受け取り、Tor の出口で解決する
    let mut req = [0u8; 4];
    s.read_exact(&mut req).await?;
    if req[0] != 5 || req[1] != 1 {
        s.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        return Err(bad());
    }
    let host = match req[3] {
        1 => {
            let mut a = [0u8; 4];
            s.read_exact(&mut a).await?;
            std::net::Ipv4Addr::from(a).to_string()
        }
        3 => {
            let mut n = [0u8; 1];
            s.read_exact(&mut n).await?;
            let mut name = vec![0u8; n[0] as usize];
            s.read_exact(&mut name).await?;
            String::from_utf8(name).map_err(|_| bad())?
        }
        4 => {
            let mut a = [0u8; 16];
            s.read_exact(&mut a).await?;
            std::net::Ipv6Addr::from(a).to_string()
        }
        _ => return Err(bad()),
    };
    let mut port = [0u8; 2];
    s.read_exact(&mut port).await?;
    let port = u16::from_be_bytes(port);
    // ユーザー名 (サイト) ごとに別の回線
    let token = *groups.lock().unwrap().entry(String::from_utf8_lossy(&user).into_owned()).or_insert_with(IsolationToken::new);
    let mut prefs = StreamPrefs::new();
    prefs.set_isolation(token).connect_to_onion_services(BoolOrAuto::Explicit(true));
    match tor.connect_with_prefs((host.as_str(), port), &prefs).await {
        Ok(mut remote) => {
            s.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            let _ = tokio::io::copy_bidirectional(&mut s, &mut remote).await;
            Ok(())
        }
        Err(e) => {
            let (code, why) = describe(&e);
            FAILURES.lock().unwrap().insert(format!("{}:{port}", host.to_ascii_lowercase()), why);
            s.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            Err(bad())
        }
    }
}
