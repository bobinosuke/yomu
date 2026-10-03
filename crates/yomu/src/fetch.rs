//! HTTP 取得と文字コード判定。日本語サイトに多い Shift_JIS / EUC-JP も読めるようにする。
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::Duration;

use encoding_rs::Encoding;
use regex::bytes::Regex;

pub const UA: &str = "Mozilla/5.0 yomu/0.1";
static META_CHARSET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i-u)<meta[^>]+charset=["']?([\w-]+)"#).unwrap());

/// 通信は wreq (非同期) を、この実行環境で待って使う。yomu の通信はどれも結果を待つので、呼び出し側は同期のまま
static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("通信の実行環境を作れない")
});

/// Cookie。起動している間は覚えておく。設定 (save_cookies) がオンなら、期限つきのものを終了するときにファイルに残す
pub static COOKIES: LazyLock<Arc<CookieJar>> = LazyLock::new(Default::default);

/// wreq の Cookie 置き場に、受け取った Set-Cookie の記録を足したもの。wreq の置き場からは、Domain のない Cookie が
/// どのサイトのものかを読み出せないので、ファイルに残すために受け取ったときの URL と一緒に記録する
#[derive(Default)]
pub struct CookieJar {
    jar: wreq::cookie::Jar,
    seen: std::sync::Mutex<std::collections::BTreeMap<CookieKey, (String, String)>>, // → (受け取った URL, 期限を絶対時刻にした Set-Cookie)
}

/// 記録する Cookie の見分け方 (サイトのホスト, 名前, Domain, Path)
type CookieKey = (String, String, String, String);

impl wreq::cookie::CookieStore for CookieJar {
    fn set_cookies(&self, headers: &mut dyn Iterator<Item = &wreq::header::HeaderValue>, uri: &wreq::Uri) {
        let headers: Vec<wreq::header::HeaderValue> = headers.cloned().collect();
        let mut seen = self.seen.lock().unwrap();
        for h in &headers {
            if let Some(c) = h.to_str().ok().and_then(|s| cookie::Cookie::parse(s.to_string()).ok()) {
                let key = (uri.host().unwrap_or("").to_string(), c.name().to_string(), c.domain().unwrap_or("").to_string(), c.path().unwrap_or("").to_string());
                match persistent(c) {
                    Some(c) => seen.insert(key, (uri.to_string(), c.to_string())),
                    None => seen.remove(&key),
                };
            }
        }
        self.jar.set_cookies(&mut headers.iter(), uri);
    }
    fn cookies(&self, uri: &wreq::Uri, version: wreq::Version) -> wreq::cookie::Cookies {
        self.jar.cookies(uri, version)
    }
}

/// 期限つきの Cookie なら、Max-Age を絶対時刻の Expires に直したもの (ファイルから読み直したときに期限が延びないように)。
/// 期限のない Cookie (ブラウザを閉じたら消えるもの) と、もう切れているものは None
fn persistent(mut c: cookie::Cookie<'static>) -> Option<cookie::Cookie<'static>> {
    let now = cookie::time::OffsetDateTime::now_utc();
    if let Some(age) = c.max_age() {
        c.set_max_age(None);
        c.set_expires(now + age);
    }
    c.expires_datetime().filter(|t| *t > now)?;
    Some(c)
}

fn cookies_path() -> std::path::PathBuf {
    crate::store::data_dir().join("cookies.json")
}

impl CookieJar {
    /// ファイルに残した Cookie を読み込む (切れたものは除く)
    pub fn load(&self) {
        let Ok(s) = std::fs::read_to_string(cookies_path()) else { return };
        let entries: Vec<(String, String)> = serde_json::from_str(&s).unwrap_or_default();
        for (url, set_cookie) in entries {
            if let (Ok(uri), Ok(v)) = (url.parse::<wreq::Uri>(), wreq::header::HeaderValue::from_str(&set_cookie)) {
                wreq::cookie::CookieStore::set_cookies(self, &mut std::iter::once(&v), &uri);
            }
        }
    }

    /// 期限つきの Cookie をファイルに残す。ログインの情報も入るので、自分だけが読める権限で書く
    pub fn save(&self) {
        let now = cookie::time::OffsetDateTime::now_utc();
        let entries: Vec<(String, String)> = self
            .seen
            .lock()
            .unwrap()
            .values()
            .filter(|(_, c)| cookie::Cookie::parse(c.as_str()).ok().and_then(|c| c.expires_datetime()).is_some_and(|t| t > now))
            .cloned()
            .collect();
        let path = cookies_path();
        let _ = (|| -> std::io::Result<()> {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::create_dir_all(path.parent().unwrap())?;
            let tmp = path.with_extension("tmp");
            let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
            std::io::Write::write_all(&mut f, &serde_json::to_vec_pretty(&entries).map_err(std::io::Error::other)?)?;
            std::fs::rename(&tmp, &path)
        })();
    }

    /// Cookie をファイルに残すか (設定がオンで、プライベートモードでないとき)
    pub fn uses_file() -> bool {
        crate::settings::get().save_cookies && !is_private()
    }

    /// 残した Cookie のファイルを消す (設定をオフにしたとき)
    pub fn forget_file() {
        let _ = std::fs::remove_file(cookies_path());
    }
}

/// プライベートモード (yomu --private)。すべての通信を Tor に通す (tor.rs)
static PRIVATE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// プライベートモードにする (起動したときに 1 回だけ)
pub fn set_private() {
    PRIVATE.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn is_private() -> bool {
    PRIVATE.load(std::sync::atomic::Ordering::Relaxed)
}

/// 普段のモードで名乗る言語 (画面の言語を先に、英語を次に。プライベートモードはいつも英語)
fn accept_language() -> &'static str {
    match crate::i18n::lang() {
        crate::i18n::Lang::Ja => "ja,en;q=0.8",
        crate::i18n::Lang::En => "en-US,en;q=0.9",
        crate::i18n::Lang::Ko => "ko-KR,ko;q=0.9,en;q=0.8",
    }
}

/// URL のホスト (小文字)
fn host_of(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or("");
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = if host.starts_with('[') { host.split(']').next().map(|h| format!("{h}]")).unwrap_or_default() } else { host.split(':').next().unwrap_or("").to_string() };
    host.to_ascii_lowercase()
}

fn is_onion(url: &str) -> bool {
    host_of(url).ends_with(".onion")
}

/// URL のポート (書いてなければ scheme の既定)
fn port_of(url: &str) -> u16 {
    let rest = url.split("://").nth(1).unwrap_or("");
    let netloc = rest.split(['/', '?', '#']).next().unwrap_or("");
    let netloc = netloc.rsplit('@').next().unwrap_or(netloc);
    let after_host = if netloc.starts_with('[') { netloc.split(']').nth(1).unwrap_or("") } else { netloc };
    match after_host.rsplit_once(':').map(|(_, p)| p.parse()) {
        Some(Ok(p)) => p,
        _ if url.to_ascii_lowercase().starts_with("http://") => 80,
        _ => 443,
    }
}

/// .onion の読み込みの上限。回線を 6 つのリレーでつなぎ、つなぐ前にサービスの情報も取るので時間がかかる
/// (Tor Browser には上限がない)
const ONION_TIMEOUT: Duration = Duration::from_secs(120);

/// プライベートモードで開いてよい URL か。HTTPS だけ (.onion は Tor の中で暗号化されるので http でもよい)。
/// Tor Browser の HTTPS-Only と同じ
fn private_allowed(url: &str) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://") || (lower.starts_with("http://") && is_onion(url)) {
        Ok(())
    } else {
        Err(t!("プライベートモードでは HTTPS のページだけを開きます: {url}", url = url))
    }
}

/// プライベートモードで、http の URL を https に読み替える (.onion はそのまま)
pub fn private_upgrade(url: &str) -> String {
    match url.strip_prefix("http://") {
        Some(rest) if is_private() && !is_onion(url) => format!("https://{rest}"),
        _ => url.to_string(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Flavor {
    /// ページを開く (指紋は Firefox、Cookie を使う)
    Page,
    /// 素の接続 (DuckDuckGo の検索。プライベートモードでは Page と同じく Tor Browser を名乗り、Cookie だけ使わない)
    Plain,
    /// プライベートモードで、ページの一部 (画像など) を取る。よそのサイトの Cookie で追跡されないよう Cookie を送らない
    Sub,
}

/// プライベートモードで名乗るブラウザ。Tor の利用者の多くと見分けがつかないよう、Tor Browser の安定版 (15。Firefox ESR 140)
/// に合わせる。Tor Browser 16 (ESR 153) が安定版になったら替える。Tor Browser は 14 から OS の種類だけは本当のものを名乗る
#[cfg(target_os = "macos")]
const TOR_BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:140.0) Gecko/20100101 Firefox/140.0";
#[cfg(target_os = "windows")]
const TOR_BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:140.0) Gecko/20100101 Firefox/140.0";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const TOR_BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0";

/// Firefox が送るヘッダーの順と、HTTP/1.1 での書き方 (送らないものは飛ばされる)。順番も書き方も指紋になるので合わせる。
/// HTTP/2 では小文字になり、Connection は wreq が取り除く (HTTP/2 では使わないので。Firefox も送らない)
const FIREFOX_HEADER_ORDER: &[&str] = &[
    "Host",
    "User-Agent",
    "Accept",
    "Accept-Language",
    "Accept-Encoding",
    "Content-Type",
    "Content-Length",
    "Origin",
    "Connection",
    "Referer",
    "Cookie",
    "Upgrade-Insecure-Requests",
    "Sec-Fetch-Dest",
    "Sec-Fetch-Mode",
    "Sec-Fetch-Site",
    "Sec-Fetch-User",
    "Priority",
    "TE",
];

/// プライベートモードの既定のヘッダー (URL を入れてページを開いたときのもの。画像は images.rs で画像のものに替える)。
/// te は TE: trailers を付けるか (Firefox は HTTP/2 のときだけ付ける)
fn firefox_headers(flavor: Flavor, te: bool) -> wreq::header::HeaderMap {
    let mut h = wreq::header::HeaderMap::new();
    let mut put = |k: &'static str, v: &'static str| h.insert(k, wreq::header::HeaderValue::from_static(v));
    put("user-agent", TOR_BROWSER_UA);
    put("accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8");
    // 言語は英語を名乗る (日本語の設定から人を絞り込まれないように)
    put("accept-language", "en-US,en;q=0.5");
    put("accept-encoding", "gzip, deflate, br, zstd");
    put("connection", "keep-alive");
    if flavor != Flavor::Sub {
        put("upgrade-insecure-requests", "1");
    }
    put("sec-fetch-dest", "document");
    put("sec-fetch-mode", "navigate");
    put("sec-fetch-site", "none");
    if flavor != Flavor::Sub {
        put("sec-fetch-user", "?1");
    }
    put("priority", "u=0, i");
    if te {
        put("te", "trailers");
    }
    h
}

/// プライベートモードで使う、サイト (first party) ごとのクライアント。サイトごとに Tor の回線が分かれる
static PRIVATE_CLIENTS: LazyLock<std::sync::Mutex<std::collections::HashMap<(Flavor, String), wreq::Client>>> = LazyLock::new(Default::default);

fn private_client(flavor: Flavor, first_party: &str) -> Result<wreq::Client, String> {
    if let Some(c) = PRIVATE_CLIENTS.lock().unwrap().get(&(flavor, first_party.to_string())) {
        return Ok(c.clone());
    }
    let proxy = wreq::Proxy::all(crate::tor::proxy_for(first_party)?).map_err(|e| e.to_string())?;
    let _guard = RUNTIME.enter();
    let c = private_builder(flavor, first_party).proxy(proxy).build().map_err(|e| e.to_string())?;
    PRIVATE_CLIENTS.lock().unwrap().insert((flavor, first_party.to_string()), c.clone());
    Ok(c)
}

/// プライベートモードのクライアントの設定 (Tor の窓口を除く)。first_party はこのクライアントの回線のサイト
fn private_builder(flavor: Flavor, first_party: &str) -> wreq::ClientBuilder {
    // 接続の挨拶 (TLS・HTTP/2) も、ヘッダーも Firefox (Tor Browser) と同じにする。wreq-util の Firefox 135〜151 は
    // 接続の挨拶が同じなので、ESR 140 にはそのうちの 139 を使う
    let mut order = wreq::header::OrigHeaderMap::new();
    for h in FIREFOX_HEADER_ORDER {
        order.insert(*h);
    }
    // http への転送は断る (.onion は除く)。ページを開くときによそのサイトへ転送されたら、ここで止め、転送先のサイトの
    // 回線で開き直す (RequestBuilder::send)。このサイトの回線のまま、転送先のサイトの Cookie を送らないように
    // (Tor Browser と同じ)。画像などはページの回線のまま転送をたどる
    let (page, first_party) = (flavor == Flavor::Page, first_party.to_string());
    let redirect = wreq::redirect::Policy::custom(move |a| {
        let next = a.uri.to_string();
        if a.previous.len() >= 20 {
            a.error(t!("転送が多すぎる"))
        } else if page && host_of(&next) != first_party {
            a.stop() // 開き直すときに、http でないかも確かめる
        } else if private_allowed(&next).is_err() {
            a.error(t!("プライベートモードでは http へ転送しない"))
        } else {
            a.follow()
        }
    });
    let b = wreq::Client::builder()
        .emulation(wreq_util::Emulation::Firefox139)
        .default_headers(firefox_headers(flavor, true))
        .orig_headers(order)
        .redirect(redirect)
        .timeout(Duration::from_secs(60));
    // 画像などは、よそのサイトの Cookie で追跡されないよう Cookie を送らない
    if flavor == Flavor::Page { b.cookie_provider(COOKIES.clone()) } else { b }
}

/// プライベートモードの要求。.onion は待つ時間を延ばす。平文 (http の .onion) はいつも HTTP/1.1 で、
/// Firefox は HTTP/1.1 では TE を送らないので外す
fn private_request(c: &wreq::Client, flavor: Flavor, method: wreq::Method, url: &str) -> wreq::RequestBuilder {
    let b = c.request(method, url);
    let b = if is_onion(url) { b.timeout(ONION_TIMEOUT) } else { b };
    if url.to_ascii_lowercase().starts_with("http://") { b.default_headers(false).headers(firefox_headers(flavor, false)) } else { b }
}

/// HTTP クライアント。接続の指紋はブラウザ (Firefox) と同じにし、User-Agent は yomu のまま。
/// プライベートモードでは、要求ごとにサイトの Tor の回線を使うクライアントに替え、名乗りも Tor Browser にする
#[derive(Clone)]
pub struct Client {
    inner: wreq::Client,
    flavor: Flavor,
}

impl Client {
    pub fn get(&self, url: &str) -> RequestBuilder {
        self.request(wreq::Method::GET, url)
    }
    pub fn post(&self, url: &str) -> RequestBuilder {
        self.request(wreq::Method::POST, url)
    }
    pub fn request(&self, method: wreq::Method, url: &str) -> RequestBuilder {
        self.request_for(method, url, &host_of(url))
    }
    /// ページの一部 (画像など) を取る。プライベートモードでは、そのページ (first_party) の回線を使う
    pub fn get_for(&self, url: &str, first_party_url: &str) -> RequestBuilder {
        let sub = Client { inner: self.inner.clone(), flavor: if self.flavor == Flavor::Page { Flavor::Sub } else { self.flavor } };
        sub.request_for(wreq::Method::GET, url, &host_of(first_party_url))
    }
    fn request_for(&self, method: wreq::Method, url: &str, first_party: &str) -> RequestBuilder {
        if !is_private() {
            let b = self.inner.request(method, url).header(wreq::header::ACCEPT_LANGUAGE, accept_language());
            return RequestBuilder { inner: Ok(b), target: None, page: false };
        }
        let c = private_allowed(url).and_then(|_| private_client(self.flavor, first_party));
        RequestBuilder {
            inner: c.map(|c| private_request(&c, self.flavor, method, url)),
            target: Some((host_of(url), port_of(url))),
            page: self.flavor == Flavor::Page,
        }
    }
}

/// 要求
pub struct RequestBuilder {
    inner: Result<wreq::RequestBuilder, String>,
    /// プライベートモードでの宛先 (ホスト, ポート)。Tor でつなげなかったときに理由を出すため
    target: Option<(String, u16)>,
    /// プライベートモードでページを開く要求か (よそのサイトへの転送を、転送先のサイトの回線でたどる)
    page: bool,
}

impl RequestBuilder {
    pub fn header(self, name: &str, value: &str) -> Self {
        Self { inner: self.inner.map(|b| b.header(name, value)), ..self }
    }
    pub fn body(self, body: impl Into<wreq::Body>) -> Self {
        Self { inner: self.inner.map(|b| b.body(body)), ..self }
    }
    pub fn form<T: serde::Serialize + ?Sized>(self, form: &T) -> Self {
        Self { inner: self.inner.map(|b| b.form(form)), ..self }
    }
    /// 送って、本文まで受け取る
    pub fn send(self) -> Result<HttpResponse, HttpError> {
        let mut r = send_once(self.inner, self.target)?;
        if !self.page {
            return Ok(r);
        }
        // よそのサイトへの転送 (private_builder で止めたもの) を、転送先のサイトの回線で開き直す
        for _ in 0..20 {
            let Some(next) = cross_site_location(&r) else { return Ok(r) };
            let req = private_allowed(&next)
                .map_err(|_| t!("プライベートモードでは http へ転送しない").to_string())
                .and_then(|_| private_client(Flavor::Page, &host_of(&next)))
                .map(|c| private_request(&c, Flavor::Page, wreq::Method::GET, &next));
            r = send_once(req, Some((host_of(&next), port_of(&next))))?;
        }
        Err(HttpError(t!("転送が多すぎる").into()))
    }
}

/// 転送の応答なら、転送先の URL
fn cross_site_location(r: &HttpResponse) -> Option<String> {
    let loc = r.headers.get(wreq::header::LOCATION)?.to_str().ok()?;
    matches!(r.status.as_u16(), 301 | 302 | 303 | 307 | 308).then(|| crate::urls::urljoin(&r.url, loc))
}

/// 1 回送って、本文まで受け取る
fn send_once(b: Result<wreq::RequestBuilder, String>, target: Option<(String, u16)>) -> Result<HttpResponse, HttpError> {
    let b = b.map_err(HttpError)?;
    let r = RUNTIME.block_on(async move {
        let r = b.send().await?;
        let (status, url, headers) = (r.status(), r.uri().to_string(), r.headers().clone());
        let body = r.bytes().await?.to_vec();
        Ok(HttpResponse { status, url, headers, body })
    });
    // Tor でつなげなかった (SOCKS の窓口が断った) なら、窓口が残した理由に替える。リダイレクトの先でつなげなかった
    // ときは理由が見つからないので、そのまま出す
    r.map_err(|e: HttpError| match target.filter(|_| e.0.contains("SOCKS")).and_then(|(h, p)| crate::tor::take_failure(&h, p)) {
        Some(why) => HttpError(t!("Tor でつなげませんでした: {why}", why = why)),
        None if is_private() && e.0.contains("operation timed out") => {
            HttpError(t!("Tor 越しで時間内に読み込めませんでした (Tor の回線が遅いか、サイトが応答しない): {e}", e = e))
        }
        None => e,
    })
}

/// 受け取った応答 (本文まで受け取ったもの)
pub struct HttpResponse {
    status: wreq::StatusCode,
    url: String,
    headers: wreq::header::HeaderMap,
    body: Vec<u8>,
}

impl HttpResponse {
    pub fn status(&self) -> wreq::StatusCode {
        self.status
    }
    /// リダイレクト後の最終 URL
    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn headers(&self) -> &wreq::header::HeaderMap {
        &self.headers
    }
    pub fn header(&self, name: &str) -> String {
        self.headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("").to_string()
    }
    pub fn bytes(self) -> Vec<u8> {
        self.body
    }
    pub fn text(self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// 解放したのに抱えたままのメモリを OS に返す。大きいページの本文の抽出は一時的に数十 MB を使い、
/// メモリの管理 (mimalloc) は解放した領域を次に使うためにしばらく抱えるので、読み込みが終わったらすぐ返させる
/// (読み上げのモデル (ONNX Runtime) は C++ で、macOS の malloc を使うので、そちらにも返させる)
pub fn release_memory() {
    unsafe { libmimalloc_sys::mi_collect(true) };
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
        }
        // zone に NULL を渡すと全部の zone、goal に 0 を渡すとできるだけ多く返す
        unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
    }
}

/// 取得の失敗 (接続できない・タイムアウトなど)
#[derive(Debug, Clone)]
pub struct HttpError(pub String);

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HttpError {}

impl From<wreq::Error> for HttpError {
    fn from(e: wreq::Error) -> Self {
        // wreq のエラーは原因を source に持つので、つないで出す
        let mut msg = e.to_string();
        let mut src = std::error::Error::source(&e);
        while let Some(s) = src {
            msg = format!("{msg}: {s}");
            src = s.source();
        }
        HttpError(msg)
    }
}

#[derive(Debug, Default)]
pub struct Response {
    pub url: String, // リダイレクト後の最終 URL
    pub status: u16,
    pub content_type: String,
    pub content: Vec<u8>,
    pub charset: Option<String>, // Content-Type で宣言された文字コード
    pub disposition: String,     // Content-Disposition (保存するときのファイル名に使う)
    text: OnceLock<String>,
}

impl Response {
    pub fn new(url: &str, status: u16, content_type: &str, content: Vec<u8>) -> Self {
        let charset = charset_param(content_type);
        Self { url: url.into(), status, content_type: content_type.into(), content, charset, ..Default::default() }
    }

    pub fn text(&self) -> &str {
        self.text.get_or_init(|| decode(&self.content, self.charset.as_deref()))
    }

    pub fn mime(&self) -> String {
        self.content_type.split(';').next().unwrap_or("").trim().to_lowercase()
    }
}

/// Content-Type の charset
fn charset_param(content_type: &str) -> Option<String> {
    content_type.split(';').skip(1).find_map(|p| {
        let (k, v) = p.split_once('=')?;
        (k.trim().eq_ignore_ascii_case("charset")).then(|| v.trim().trim_matches(['"', '\'']).to_string())
    })
}

pub fn client() -> Client {
    build(builder().cookie_provider(COOKIES.clone()), Flavor::Page)
}

/// 接続の指紋をブラウザに似せないクライアント。DuckDuckGo の検索 (html.duckduckgo.com) は JavaScript を使わない
/// クライアント向けの窓口で、ブラウザの指紋で送ると逆に確認画面を返すので、検索にはこちらを使う
pub fn plain_client() -> Client {
    build(plain_builder(), Flavor::Plain)
}

fn builder() -> wreq::ClientBuilder {
    plain_builder().emulation(wreq_util::Emulation::Firefox139).user_agent(UA)
}

fn plain_builder() -> wreq::ClientBuilder {
    wreq::Client::builder()
        .user_agent(UA)
        .redirect(wreq::redirect::Policy::limited(20))
        .timeout(Duration::from_secs(20))
}

fn build(b: wreq::ClientBuilder, flavor: Flavor) -> Client {
    // 実行環境の中で作る (接続の管理に実行環境が要る)
    let _guard = RUNTIME.enter();
    Client { inner: b.build().expect("HTTP クライアントを作れない"), flavor }
}

/// ヘッダ → <meta charset> → 推定 の順で文字コードを決める
pub fn decode(body: &[u8], header_charset: Option<&str>) -> String {
    let mut candidates: Vec<String> = header_charset.into_iter().map(String::from).collect();
    if let Some(m) = META_CHARSET.captures(&body[..body.len().min(4096)]) {
        candidates.push(String::from_utf8_lossy(&m[1]).into_owned());
    }
    for label in candidates.iter().filter(|c| !c.is_empty()) {
        // Shift_JIS と宣言していても機種依存文字を含むページが多いので cp932 で読む
        // (encoding_rs の Shift_JIS は WHATWG の定義で、cp932 と同じく機種依存文字を読める)
        let Some(enc) = encoding_for(label) else { continue };
        if let Some(s) = enc.decode_without_bom_handling_and_without_replacement(body) {
            return s.into_owned();
        }
    }
    let mut det = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Allow);
    det.feed(body, true);
    let enc = det.guess(None, chardetng::Utf8Detection::Allow);
    enc.decode_without_bom_handling(body).0.into_owned()
}

/// 文字コードの名前から。WHATWG の名前のほか、Python が受け付ける書き方 (cp932、euc_jp など) も読む
fn encoding_for(label: &str) -> Option<&'static Encoding> {
    let label = label.trim().to_ascii_lowercase();
    if matches!(label.as_str(), "cp932" | "ms_kanji" | "mskanji") {
        return Some(encoding_rs::SHIFT_JIS);
    }
    Encoding::for_label(label.as_bytes()).or_else(|| Encoding::for_label(label.replace('_', "-").as_bytes()))
}

/// 404 などのエラーでも Err にせず、サイトが返したエラーページの本文ごと返す
pub fn fetch(c: &Client, url: &str) -> Result<Response, HttpError> {
    let url = &private_upgrade(url);
    let r = c.get(url).send()?;
    let content_type = r.header("content-type");
    let disposition = r.header("content-disposition");
    let final_url = r.url().to_string();
    let status = r.status().as_u16();
    let content = r.bytes();
    let mut resp = Response::new(&final_url, status, &content_type, content);
    resp.disposition = disposition;
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_declared_and_guessed_charsets() {
        let (sjis, _, _) = encoding_rs::SHIFT_JIS.encode("将棋①");
        assert_eq!(decode(&sjis, Some("shift_jis")), "将棋①");
        let html = [b"<meta charset=\"euc-jp\">".as_slice(), &encoding_rs::EUC_JP.encode("日本語").0].concat();
        assert!(decode(&html, None).ends_with("日本語"));
        assert_eq!(decode("ただの UTF-8".as_bytes(), None), "ただの UTF-8");
        assert_eq!(charset_param("text/html; charset=\"UTF-8\"").as_deref(), Some("UTF-8"));
    }

    #[test]
    fn port_of_reads_explicit_and_default_ports() {
        assert_eq!(port_of("https://example.com/a"), 443);
        assert_eq!(port_of("http://abc.onion/"), 80);
        assert_eq!(port_of("https://user:pw@example.com:8443/a?b=c:d"), 8443);
        assert_eq!(port_of("http://[::1]:8080/"), 8080);
        assert_eq!(port_of("https://[::1]/"), 443);
    }

    /// プライベートモードの HTTP/1.1 の要求を、Firefox と同じ書き方・順番で送る (Tor を通さず、手元で受けて確かめる)
    #[test]
    fn private_http1_request_looks_like_firefox() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 4096];
            let n = s.read(&mut buf).unwrap();
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n");
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        let c = {
            let _guard = RUNTIME.enter();
            private_builder(Flavor::Page, "127.0.0.1").build().unwrap()
        };
        let b = private_request(&c, Flavor::Page, wreq::Method::GET, &url);
        let _ = RUNTIME.block_on(b.send());
        let req = server.join().unwrap();
        let names: Vec<&str> = req.lines().skip(1).filter_map(|l| l.split_once(':').map(|(k, _)| k)).collect();
        assert_eq!(
            names,
            [
                "Host",
                "User-Agent",
                "Accept",
                "Accept-Language",
                "Accept-Encoding",
                "Connection",
                "Upgrade-Insecure-Requests",
                "Sec-Fetch-Dest",
                "Sec-Fetch-Mode",
                "Sec-Fetch-Site",
                "Sec-Fetch-User",
                "Priority"
            ],
            "{req}"
        );
        assert!(req.contains(&format!("User-Agent: {TOR_BROWSER_UA}\r\n")), "{req}");
        assert!(req.contains("Connection: keep-alive\r\n"), "{req}");
    }

    /// ページを開くときによそのサイトへ転送されたら、たどらずに止める (転送先のサイトの回線で開き直すため)
    #[test]
    fn private_page_stops_at_cross_site_redirect() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = s.read(&mut buf).unwrap();
            let res = format!("HTTP/1.1 302 Found\r\nlocation: http://localhost:{port}/c\r\ncontent-length: 0\r\n\r\n");
            let _ = s.write_all(res.as_bytes());
        });
        let c = {
            let _guard = RUNTIME.enter();
            private_builder(Flavor::Page, "127.0.0.1").build().unwrap()
        };
        let b = private_request(&c, Flavor::Page, wreq::Method::GET, &format!("http://127.0.0.1:{port}/a"));
        let r = send_once(Ok(b), None).unwrap();
        server.join().unwrap();
        assert_eq!(r.status().as_u16(), 302);
        assert_eq!(cross_site_location(&r), Some(format!("http://localhost:{port}/c")));
    }
}
