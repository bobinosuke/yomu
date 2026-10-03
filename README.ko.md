# yomu

[English](README.md) | [日本語](README.ja.md) | 한국어

**yomu**(**Y**uto's **O**reore **M**inimal **U**ser-interface browser)는 읽기에 특화된, Rust로 만든 가벼운 터미널 웹 브라우저입니다.
페이지에서 내비게이션, 광고, 푸터를 걷어 내고 본문만 뽑아서, 제목, 목록, 표, 코드의 구조를 그대로 살려 보여 줍니다.
Vimium과 같은 키로 조작할 수 있고, 내 컴퓨터에서 돌아가는 음성 합성으로 페이지를 읽어 주거나 원문을 남긴 채 번역할 수도 있습니다.

<img src="docs/images/main-text.png" width="760" alt="The Rust Programming Language의 한 장을 yomu로 본문만 표시한 화면">

## 특징

- 페이지의 본문을 뽑아 터미널에 읽기 좋게 보여 줍니다. `a`를 누르면 페이지 전체 보기로 바뀝니다. PDF는 텍스트를 뽑아 보여 주고, 표시할 수 없는 파일은 `~/Downloads`에 저장합니다.
- 키 조작은 [Vimium](https://github.com/philc/vimium)과 같습니다. `j` `k`로 스크롤하고, `f`로 링크를 열고, `o`로 URL을 열고, `H` `L`로 뒤로 가거나 앞으로 갑니다. 마우스로도 조작할 수 있습니다.
- `S`를 누르면 내 컴퓨터에서 돌아가는 고품질 음성 합성 모델 [Supertonic 3](https://huggingface.co/Supertone/supertonic-3)가 31개 언어로 페이지를 읽어 줍니다. 페이지의 글은 외부로 보내지 않습니다.
- `E`를 누르면 [Immersive Translate](https://immersivetranslate.com/)처럼 원문 단락 바로 아래에 번역문을 놓고, `e`를 누르면 원문을 번역문으로 바꿉니다. 번역에는 Google 번역을 쓰므로 본문이 Google로 전송됩니다.
- 프라이빗 모드(실험적)에서는 모든 통신을 Tor로 보내고, 기록, 탭, 쿠키를 남기지 않습니다. 시작할 때 `--private`를 붙이거나 설정 화면(`gs`)에서 전환합니다.
- 화면은 영어, 일본어, 한국어로 볼 수 있습니다. 화면 언어는 시스템 설정을 따르며, 설정 화면에서 바꿀 수 있습니다.

yomu는 페이지의 JavaScript를 실행하지 않습니다. SNS나 웹 앱처럼 JavaScript로 본문을 만드는 페이지는 읽을 수 없으니, `gx`로 평소 브라우저에서 열어 주세요.

## 설치

yomu는 macOS에서 개발하고 동작을 확인하고 있습니다. 빌드된 바이너리는 Apple Silicon Mac용입니다.

### Homebrew

```sh
brew install bobinosuke/tap/yomu
```

설치한 뒤에는 어느 터미널에서든 `yomu`로 실행할 수 있습니다.

### 설치 스크립트

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/bobinosuke/yomu/releases/latest/download/yomu-installer.sh | sh
```

`yomu`를 `~/.local/bin`에 두고, 필요하면 `PATH`에 추가합니다.

### Cargo

Intel Mac을 쓰거나 직접 빌드하고 싶다면 Cargo로 설치할 수 있습니다. Rust 1.92 이상이 필요합니다([rustup](https://rustup.rs/)으로 설치하는 것이 가장 간단합니다).

```sh
cargo install --git https://github.com/bobinosuke/yomu yomu
```

Cargo는 `yomu`를 `~/.cargo/bin`에 둡니다. 설치한 뒤 `yomu`를 찾을 수 없다면 `~/.zshenv`(또는 사용하는 셸의 설정 파일)에 `. "$HOME/.cargo/env"`를 추가해 주세요.

### 소스에서 빌드하기

```sh
git clone https://github.com/bobinosuke/yomu
cd yomu
cargo build --release
./target/release/yomu
```

빌드하는 동안 읽어 주기에 쓰는 ONNX Runtime의 빌드된 바이너리를 내려받으므로, 처음 빌드할 때는 네트워크 연결이 필요합니다. Linux에서도 동작할 수 있지만 아직 확인하지 않았습니다.

## 사용법

```sh
yomu                       # 빈 탭으로 시작 (o로 열기, ?로 도움말)
yomu https://example.com   # URL 열기
yomu rust 소유권            # URL이 아니면 DuckDuckGo에서 검색
yomu --dump URL            # 페이지를 Markdown으로 출력 (파이프용)
yomu --private             # 프라이빗 모드 (Tor)
```

yomu 안에서 `?`를 누르면 모든 키 조작이 나옵니다. 자주 쓰는 키는 다음과 같습니다.

| 키 | 동작 |
|---|---|
| `j` `k` / `h` `l` | 아래와 위 / 왼쪽과 오른쪽으로 스크롤 |
| `d` `u` / `gg` `G` | 반 페이지 아래와 위 / 맨 위와 맨 아래 |
| `f` / `F` | 링크에 라벨을 띄워서 열기 / 새 탭에서 열기 |
| `o` / `O` | URL 열기와 검색(기록과 북마크에서 후보 표시) / 새 탭에서 |
| `H` `L` | 뒤로 / 앞으로 |
| `[[` `]]` | "이전" / "다음" 링크 열기 |
| `/` `n` `N` | 페이지 내 검색 / 다음과 이전 일치 |
| `v` `V` | 비주얼 모드(글자 단위나 줄 단위로 선택). `y`로 복사 |
| `t` `x` `X` | 새 탭 / 탭 닫기 / 닫은 탭 되살리기 |
| `J` `K` | 왼쪽 / 오른쪽 탭 |
| `yy` / `yf` | 페이지 URL / 링크 URL 복사 |
| `r` / `Esc` | 새로고침 / 취소 |

yomu만의 키도 있습니다.

| 키 | 동작 |
|---|---|
| `s` | 검색 |
| `a` | 본문 추출과 페이지 전체 보기 전환 |
| `S` | 화면 위치부터 읽어 주기 / 정지 |
| `e` / `E` | 페이지 번역(바꾸기 / 대역) |
| `gi` | 페이지의 입력란(검색창 등)에 입력해서 보내기 |
| `gb` | 북마크 추가 / 삭제 |
| `gh` | 기록(최근 1시간, 어제와 오늘, 모든 기록을 지울 수 있음) |
| `gs` | 설정 |
| `gp` | 프라이빗 모드 전환 / 복귀 |
| `gx` | 평소 브라우저로 열기 |
| `q` | 종료 |

마우스로는 링크 클릭(가운데 버튼이나 Ctrl+클릭은 새 탭), 탭 전환, 휠 스크롤, 드래그로 글자 선택을 할 수 있습니다.

### 설정

`gs`로 설정 화면을 열고, `f` 라벨이나 클릭으로 항목을 고릅니다.

- **언어 (Language)**: 화면 언어(자동, 日本語, English, 한국어). 번역 대상 언어와 검색 지역도 이 언어에 맞춥니다. 자동이면 `LC_ALL`, `LC_MESSAGES`, `LANG`, macOS 언어 설정 순으로 보고 정합니다.
- **쿠키를 파일에 저장**: 쿠키 동의와 로그인을 다음에 시작할 때도 기억합니다.
- **광고·추적 링크 제거**: 링크에서 `utm_*` 같은 추적용 값을 빼고, 광고와 추적 서비스로 가는 링크를 없앱니다.
- **이미지 표시**: 터미널 안에 이미지를 그립니다(Kitty, Ghostty, iTerm2, WezTerm 등. 그 밖의 터미널에서는 문자 블록으로 거칠게 그립니다).
- **프라이빗 모드 (Tor)**: 켜면 yomu를 프라이빗 모드로 다시 시작하고, 끄면 일반 모드로 돌아갑니다(`gp`와 같습니다).

기록, 북마크, 열어 둔 탭은 `~/.local/share/yomu/`에 저장되며, 다음에 시작할 때 탭을 다시 엽니다.

### 읽어 주기

처음 `S`를 누르면 필요한 데이터(모두 약 770MB)를 내려받아도 되는지 묻습니다. 내려받는 것은 Supertonic 3 모델, 일본어 문장 속 영어 단어를 읽기 위한 사전, 언어 판별용 데이터이며, 모두 `~/.cache/yomu`에 저장됩니다.
페이지의 언어를 자동으로 판별해 한국어, 영어, 일본어를 비롯한 31개 언어로 읽어 줍니다. 비주얼 모드에서 문장을 골라 `S`를 누르면 고른 부분만 읽습니다.

<img src="docs/images/read-aloud.png" width="760" alt="The Rust Programming Language의 한 장을 읽어 주는 화면. 상태 표시줄에 진행 상황이 나온다">

### 번역

`E`는 원문 단락 아래에 번역문을 놓고, `e`는 원문을 번역문으로 바꿉니다. 같은 키를 한 번 더 누르면 원문으로 돌아갑니다. 문장을 골라 `E`를 누르면 고른 부분만 번역할 수 있습니다. 번역 대상 언어는 화면 언어입니다. 처음 번역할 때는 본문을 Google로 보내도 되는지 `y` / `n`으로 묻습니다(`y`를 고르면 다음부터 묻지 않습니다).

<img src="docs/images/translation.png" width="760" alt="일본어 위키백과 문서의 단락마다 아래에 영어 번역문을 놓은 화면">

<sub>화면의 페이지는 일본어 위키백과의 「[Rust (プログラミング言語)](https://ja.wikipedia.org/wiki/Rust_(%E3%83%97%E3%83%AD%E3%82%B0%E3%83%A9%E3%83%9F%E3%83%B3%E3%82%B0%E8%A8%80%E8%AA%9E))」입니다([CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)).</sub>

### 프라이빗 모드(실험적)

프라이빗 모드는 시작할 때 `--private`를 붙이거나, 쓰는 도중에 설정 화면(`gs`)의 "프라이빗 모드"를 고르거나 `gp`를 누르면 전환됩니다. 일반 모드와 아무것도 공유하지 않도록, 전환할 때마다 yomu가 다시 시작됩니다.

- 모든 통신을 yomu에 내장한 [Arti](https://gitlab.torproject.org/tpo/core/arti)로 Tor에 보냅니다. 이름 확인(DNS)도 Tor 출구에서 하며, `.onion` 사이트도 열 수 있습니다.
- 사이트마다 다른 Tor 회선을 씁니다(Tor Browser의 first-party isolation과 같은 방식입니다).
- HTTPS 페이지만 엽니다(`.onion`은 예외).
- User-Agent, TLS와 HTTP/2 지문, 헤더를 Tor Browser에 맞춰서 보냅니다.
- 기록, 탭, 쿠키는 디스크에 쓰지 않습니다. 번역과 `gx`는 Tor를 거치지 않으므로 쓸 수 없습니다.

이 기능은 실험적입니다. IP 주소는 숨길 수 있지만, 페이지를 불러오는 방식의 차이로 yomu를 쓴다는 것이 드러나므로 Tor Browser만큼 익명성이 높지는 않습니다. 강한 익명성이 필요하다면 Tor Browser를 쓰세요.

## 감사의 말

yomu는 다음 프로젝트와 서비스에서 많은 아이디어를 얻었습니다.

- [Vimium](https://github.com/philc/vimium): 키 조작
- [Immersive Translate](https://immersivetranslate.com/): 원문을 남기는 대역 번역
- [Translate Web Pages (TWP)](https://github.com/FilipePS/Traduzir-paginas-web): 링크와 서식을 살린 채 페이지를 번역하는 방법
- [Tor Browser](https://www.torproject.org/): 프라이빗 모드의 동작
- [curl_cffi](https://github.com/lexiforest/curl_cffi): 실제 브라우저와 같은 연결 지문으로 통신하는 아이디어
- 검색에는 [DuckDuckGo](https://duckduckgo.com/), 번역에는 [Google 번역](https://translate.google.com/)을 씁니다

### 사용하는 오픈 소스 소프트웨어

| 프로젝트 | 용도 |
|---|---|
| [Supertonic 3](https://huggingface.co/Supertone/supertonic-3) | 읽어 주기 모델(처음 쓸 때 내려받음) |
| [ONNX Runtime](https://onnxruntime.ai/)([ort](https://github.com/pykeio/ort)를 통해) | 읽어 주기 모델 실행 |
| [OpenJTalk](https://open-jtalk.sourceforge.net/) 사전([pyopenjtalk-plus](https://github.com/tsukumijima/pyopenjtalk-plus)에서) | 읽어 주기를 위한 일본어 분석 |
| [AivisSpeech Engine](https://github.com/Aivis-Project/AivisSpeech-Engine) 사전 | 일본어 읽어 주기의 단어 읽기 |
| [kanalizer](https://github.com/VOICEVOX/kanalizer) | 일본어 문장 속 영어 단어 읽기 |
| [lingua-rs](https://github.com/pemistahl/lingua-rs) | 페이지 언어 판별 |
| [rs-trafilatura](https://github.com/Murrough-Foley/rs-trafilatura) | 본문 추출 |
| [html5ever](https://github.com/servo/html5ever), [dom_query](https://github.com/niklak/dom_query), [scraper](https://github.com/rust-scraper/scraper) | HTML 분석 |
| [ratatui](https://github.com/ratatui/ratatui), [ratatui-image](https://github.com/ratatui/ratatui-image), [crossterm](https://github.com/crossterm-rs/crossterm) | 터미널 화면과 이미지 표시 |
| [wreq](https://github.com/0x676e67/wreq), [wreq-util](https://github.com/0x676e67/wreq-util) | 브라우저와 같은 지문으로 HTTP 통신 |
| [Arti](https://gitlab.torproject.org/tpo/core/arti) | 프라이빗 모드의 Tor |
| [tokio](https://github.com/tokio-rs/tokio) | 비동기 처리 |
| [encoding_rs](https://github.com/hsivonen/encoding_rs), [chardetng](https://github.com/hsivonen/chardetng) | 문자 인코딩(Shift_JIS, EUC-JP 등) |
| [pdf-extract](https://github.com/jrmuizel/pdf-extract) | PDF에서 텍스트 추출 |
| [image](https://github.com/image-rs/image) | 이미지 읽기 |
| [cpal](https://github.com/rustaudio/cpal) | 소리 출력 |
| [mimalloc](https://github.com/purpleprotocol/mimalloc_rust) | 메모리 관리 |

## 라이선스

yomu는 [MIT License](LICENSE-MIT)와 [Apache License 2.0](LICENSE-APACHE) 중 원하는 쪽을 골라 쓸 수 있습니다.

`vendor/lingua`는 [lingua](https://github.com/pemistahl/lingua-rs) 1.8.0을 고친 것으로, Apache License 2.0을 그대로 따릅니다. 읽어 주기 모델, 사전, 언어 데이터는 yomu에 들어 있지 않으며, 처음 쓸 때 각 배포처에서 내려받습니다. 이것들은 각자의 라이선스를 따릅니다(Supertonic 3 모델은 OpenRAIL-M입니다).
