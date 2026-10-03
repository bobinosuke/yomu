# yomu

English | [日本語](README.ja.md) | [한국어](README.ko.md)

**yomu** (**Y**uto's **O**reore **M**inimal **U**ser-interface browser) is a lightweight terminal web browser written in Rust, made for reading.
It pulls the main text out of a page and shows it with its headings, lists, tables and code intact, without the navigation, ads and footers around it.
You move around with Vimium-style keys, can have the page read aloud by a local text-to-speech model, and can translate it while keeping the original in view.

<img src="docs/images/main-text.png" width="760" alt="yomu showing the Wikipedia article Text-based web browser, with a label on every link">

<sub>The page shown is [Text-based web browser](https://en.wikipedia.org/wiki/Text-based_web_browser) on English Wikipedia, licensed under [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/).</sub>

## Features

- yomu extracts the main content of each page and lays it out in the terminal. Press `a` to see the whole page instead. PDFs are shown as text, and other files are saved to `~/Downloads`.
- The keys follow [Vimium](https://github.com/philc/vimium): `j`/`k` to scroll, `f` to follow links, `o` to open, `H`/`L` to go back and forward. The mouse works too.
- `S` reads the page aloud in any of 31 languages with [Supertonic 3](https://huggingface.co/Supertone/supertonic-3), a high-quality TTS model that runs entirely on your machine. The text of the page is never sent anywhere.
- `E` puts a translation under each original paragraph, in the style of [Immersive Translate](https://immersivetranslate.com/), and `e` replaces the original instead. Translation uses Google Translate, so the text is sent to Google.
- Private mode (experimental) sends all traffic through Tor and keeps no history, tabs or cookies. Start yomu with `--private`, or switch in the settings (`gs`).
- The interface is available in English, Japanese and Korean. It follows your system language, and you can change it in the settings.

yomu does not run JavaScript. Pages that build their content with JavaScript (social media, web apps) cannot be read; press `gx` to open such a page in your usual browser.

## Installation

yomu is developed and tested on macOS. Prebuilt binaries are available for Apple Silicon Macs.

### Homebrew

```sh
brew install bobinosuke/tap/yomu
```

After that, run `yomu` from any terminal.

### Install script

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/bobinosuke/yomu/releases/latest/download/yomu-installer.sh | sh
```

This places `yomu` in `~/.local/bin` and adds it to your `PATH` if needed.

### Cargo

If you have an Intel Mac, or prefer to build it yourself, install yomu with Cargo. This needs Rust 1.92 or later ([rustup](https://rustup.rs/) is the easiest way to get it).

```sh
cargo install --git https://github.com/bobinosuke/yomu yomu
```

Cargo puts `yomu` in `~/.cargo/bin`. If your shell cannot find `yomu` afterwards, add `. "$HOME/.cargo/env"` to `~/.zshenv` (or your shell's startup file).

### Building from source

```sh
git clone https://github.com/bobinosuke/yomu
cd yomu
cargo build --release
./target/release/yomu
```

The build downloads a prebuilt ONNX Runtime (used for text-to-speech), so it needs a network connection the first time. Linux may work, but it is not tested yet.

## Usage

```sh
yomu                       # start with an empty tab (o to open, ? for help)
yomu https://example.com   # open a URL
yomu rust ownership        # anything that is not a URL is searched on DuckDuckGo
yomu --dump URL            # print the page as Markdown (useful in pipes)
yomu --private             # private mode (Tor)
```

Press `?` inside yomu to see every key. The most common ones:

| Key | Action |
|---|---|
| `j` `k` / `h` `l` | Scroll down, up / left, right |
| `d` `u` / `gg` `G` | Half a page down, up / top, bottom |
| `f` / `F` | Show labels on links and open one / open it in a new tab |
| `o` / `O` | Open a URL or search, with suggestions from history and bookmarks / in a new tab |
| `H` `L` | Back / forward |
| `[[` `]]` | Follow the "previous" / "next" link |
| `/` `n` `N` | Find in page / next / previous match |
| `v` `V` | Visual mode (select text by character / by line), `y` to copy |
| `t` `x` `X` | New tab / close tab / reopen closed tab |
| `J` `K` | Previous / next tab |
| `yy` / `yf` | Copy the page URL / a link URL |
| `r` / `Esc` | Reload / cancel |

And the keys that are specific to yomu:

| Key | Action |
|---|---|
| `s` | Search |
| `a` | Switch between the extracted text and the whole page |
| `S` | Read aloud from the top of the screen / stop |
| `e` / `E` | Translate the page (replace / show both) |
| `gi` | Type into an input field on the page (such as a search box) and submit it |
| `gb` | Add or remove a bookmark |
| `gh` | History (you can also clear the last hour, today, or everything) |
| `gs` | Settings |
| `gp` | Switch private mode on or off |
| `gx` | Open the page in your usual browser |
| `q` | Quit |

You can also click links (middle-click or Ctrl+click for a new tab), click tabs to switch, scroll with the wheel, and drag to select text.

### Settings

`gs` opens the settings. Choose an item with its `f` label or a click.

- **Language**: the interface language (Auto, 日本語, English, 한국어). It also decides the target language of translation and the region of search. Auto follows `LC_ALL`, `LC_MESSAGES`, `LANG` and then the macOS language setting.
- **Save cookies to a file**: remember cookie consent and logins between runs.
- **Remove ad and tracking links**: strip tracking parameters such as `utm_*` from links and drop links to ad and tracking services.
- **Show images**: draw images inside the terminal (Kitty, Ghostty, iTerm2, WezTerm and others; other terminals get a rough block-character rendering).
- **Private mode (Tor)**: turning it on restarts yomu in private mode, and turning it off returns to normal mode (the same as `gp`).

History, bookmarks and your open tabs are saved under `~/.local/share/yomu/`, and tabs are reopened the next time you start yomu.

### Reading aloud

The first time you press `S`, yomu asks before downloading what it needs (about 770 MB in total): the Supertonic 3 model, dictionaries for reading English words inside Japanese text, and language-detection data. Everything is stored in `~/.cache/yomu`.
yomu detects the language of the page and can read 31 languages, including English, Japanese and Korean. Select text in visual mode and press `S` to read just that part.

<img src="docs/images/read-aloud.png" width="760" alt="yomu reading a chapter of The Rust Programming Language aloud, with its progress in the status line">

### Translation

`E` shows the translation under each original paragraph, and `e` replaces the original. Press the same key again to go back to the original. You can also select text and press `E` to translate only the selection. Pages are translated into the interface language. The first time you translate, yomu asks whether it is OK to send the text to Google (`y`/`n`); after `y` it does not ask again.

<img src="docs/images/translation.png" width="760" alt="A Japanese Wikipedia article with an English translation under each paragraph">

<sub>The page shown is [Rust (プログラミング言語)](https://ja.wikipedia.org/wiki/Rust_(%E3%83%97%E3%83%AD%E3%82%B0%E3%83%A9%E3%83%9F%E3%83%B3%E3%82%B0%E8%A8%80%E8%AA%9E)) on Japanese Wikipedia, licensed under [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/).</sub>

### Private mode (experimental)

To use private mode, start yomu with `--private`, or switch at any time with "Private mode" in the settings (`gs`) or with `gp`. yomu restarts itself each time you switch, so that nothing is shared with normal mode.

- All traffic goes through Tor, using [Arti](https://gitlab.torproject.org/tpo/core/arti) built into yomu. DNS lookups happen at the Tor exit, and `.onion` sites work.
- Each site gets its own Tor circuit, like first-party isolation in Tor Browser.
- Only HTTPS pages are opened (`.onion` sites excepted).
- yomu presents itself as Tor Browser, matching its User-Agent, TLS and HTTP/2 fingerprints and headers.
- History, tabs and cookies are never written to disk. Translation and `gx` are disabled, because they would leave Tor.

This feature is experimental. It hides your IP address, but sites can tell from the way pages are loaded that you are using yomu, so it is not as anonymous as Tor Browser. If you need strong anonymity, use Tor Browser.

## Acknowledgements

yomu takes many ideas from these projects and services:

- [Vimium](https://github.com/philc/vimium): the key bindings
- [Immersive Translate](https://immersivetranslate.com/): bilingual translation that keeps the original
- [Translate Web Pages (TWP)](https://github.com/FilipePS/Traduzir-paginas-web): translating a page in place while keeping links and formatting
- [Tor Browser](https://www.torproject.org/): the behavior of private mode
- [curl_cffi](https://github.com/lexiforest/curl_cffi): the idea of matching a real browser's connection fingerprint
- [DuckDuckGo](https://duckduckgo.com/) for search and [Google Translate](https://translate.google.com/) for translation

### Open source software

| Project | Used for |
|---|---|
| [Supertonic 3](https://huggingface.co/Supertone/supertonic-3) | Text-to-speech model (downloaded on first use) |
| [ONNX Runtime](https://onnxruntime.ai/) via [ort](https://github.com/pykeio/ort) | Running the TTS model |
| [OpenJTalk](https://open-jtalk.sourceforge.net/) dictionary from [pyopenjtalk-plus](https://github.com/tsukumijima/pyopenjtalk-plus) | Japanese text analysis for TTS |
| [AivisSpeech Engine](https://github.com/Aivis-Project/AivisSpeech-Engine) dictionaries | Readings of words for Japanese TTS |
| [kanalizer](https://github.com/VOICEVOX/kanalizer) | Reading English words in Japanese text |
| [lingua-rs](https://github.com/pemistahl/lingua-rs) | Detecting the language of a page |
| [rs-trafilatura](https://github.com/Murrough-Foley/rs-trafilatura) | Main content extraction |
| [html5ever](https://github.com/servo/html5ever), [dom_query](https://github.com/niklak/dom_query), [scraper](https://github.com/rust-scraper/scraper) | HTML parsing |
| [ratatui](https://github.com/ratatui/ratatui), [ratatui-image](https://github.com/ratatui/ratatui-image), [crossterm](https://github.com/crossterm-rs/crossterm) | Terminal interface and images |
| [wreq](https://github.com/0x676e67/wreq), [wreq-util](https://github.com/0x676e67/wreq-util) | HTTP with browser fingerprints |
| [Arti](https://gitlab.torproject.org/tpo/core/arti) | Tor in private mode |
| [tokio](https://github.com/tokio-rs/tokio) | Async runtime |
| [encoding_rs](https://github.com/hsivonen/encoding_rs), [chardetng](https://github.com/hsivonen/chardetng) | Character encodings (Shift_JIS, EUC-JP and more) |
| [pdf-extract](https://github.com/jrmuizel/pdf-extract) | Text from PDFs |
| [image](https://github.com/image-rs/image) | Decoding images |
| [cpal](https://github.com/rustaudio/cpal) | Audio output |
| [mimalloc](https://github.com/purpleprotocol/mimalloc_rust) | Memory allocator |

## License

yomu is available under either the [MIT License](LICENSE-MIT) or the [Apache License 2.0](LICENSE-APACHE), at your option.

`vendor/lingua` is a modified copy of [lingua](https://github.com/pemistahl/lingua-rs) 1.8.0 and stays under the Apache License 2.0. The TTS model, dictionaries and language data are not included in yomu; they are downloaded from their original sources on first use and are covered by their own licenses (the Supertonic 3 model is under OpenRAIL-M).
