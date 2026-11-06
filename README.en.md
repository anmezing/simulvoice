[简体中文](README.md) | **English**

# SimulVoice

Simultaneous interpretation for meetings on Windows. SimulVoice listens to your meeting app, transcribes and translates what is said in real time, and reads the translation to you. In two-way mode it also translates what you say and sends it into the meeting. Everything runs locally by default: no account, no API key.

## Features

- **Two-way interpretation**: they speak English and you hear Chinese; you speak Chinese and they hear English. Pick any two of 21 languages
- **Fully local, works offline**: speech recognition with SenseVoice (Chinese, English, Japanese, Korean, Cantonese) or Whisper (other languages); translation with Marian or NLLB-200, chosen per language automatically. Online services (Soniox, OpenAI, Gemini, Qwen, Ollama) are optional
- **Listens to the meeting app only**: capture just Tencent Meeting, Teams, Zoom, DingTalk, Feishu or a browser, so other apps don't get in
- **No translation when a colleague speaks your language**: in a mixed meeting, a remark in your own language is shown as-is instead of being mis-recognized as the other language
- **Speech output**: Edge (free, online), Piper (offline), Windows SAPI, OpenAI
- **End-to-end realtime engines**: Gemini Live, Palabra.ai, Volcengine
- **Audio processing**: noise suppression, Silero VAD, echo cancellation, automatic gain, ducking other audio while speaking
- **Everyday use**: minimize to tray, global hotkeys, bilingual subtitles, recording and playback, transcript export, update check, crash dumps

## Download

Get it from [Releases](https://github.com/anmezing/simulvoice/releases):

- **Installer** `SimulVoiceSetup-<version>.exe`: installs to `%LOCALAPPDATA%\Programs\SimulVoice` by default (changeable), adds a Start menu shortcut and optionally a desktop shortcut
- **Portable** `simulvoice-<version>-portable.zip`: unzip and run `simulvoice.exe`. Data lives in the `data` folder next to the exe, so the whole folder can be moved

Installing over an existing copy keeps your settings and downloaded models. The uninstaller asks whether to delete your data too (default: keep).

## Quick start

The user interface is currently in Chinese; Chinese labels are given in parentheses.

Double-click `simulvoice.exe`, then:

1. Choose the language they speak (对方说) and the language you speak (我说);
2. Choose listen only (只听翻译) or two-way (双向翻译);
3. Under audio source (收听来源), pick your meeting app (one that is already running is selected for you), then click Start (开始翻译).

The first start downloads the models it needs (about 0.7 GB for Chinese/English, once).

For **two-way mode**, other participants can hear your translated voice only through a virtual microphone: install the free [VB-CABLE](https://vb-audio.com/Cable/) and select `CABLE Output` as the microphone in your meeting app. Without it, what you say is shown as subtitles only. Headphones are recommended.

**Browser meetings** (Google Meet, Teams on the web, …): choose Chrome / Edge / Firefox as the audio source. It captures the whole browser, so mute other tabs that play sound.

## Audio source

| Option | What is captured | Use it for |
|---|---|---|
| Meeting app (recommended) | Only that app | Most meetings |
| Browser | The whole browser | Web meetings |
| Whole computer | Everything the PC plays | When nothing else works; use headphones, or the translation is captured again |

## Tray and hotkeys

- Minimizing hides the window to the notification area; translation keeps running. Click the tray icon to restore, right-click to mute or quit (can be turned off in settings)
- `Ctrl+Alt+F8`: stop the translation being read right now
- `Ctrl+Alt+F9`: mute / unmute speech output
- `Ctrl+Alt+F10`: stop translating
- `Ctrl+Alt+F6`: replay the last translation
- `Ctrl+Alt+F7`: push-to-talk key in manual voice modes (`[audio] voice_mode`, `ptt_key`)

## Data folder

Models, settings, keys, recordings and logs all live in the data folder: `<install dir>\data` for the installer, `<exe dir>\data` for the portable build.

- Open or change it under Advanced settings → Data (高级设置 → 数据), or run `simulvoice data-dir E:\SimulVoiceData --move`
- The `SIMULVOICE_DATA_DIR` environment variable takes precedence
- Log: `simulvoice.log`. Crash dumps: `crashdumps\` (newest 5 kept; please attach one when reporting a crash)

## Local models

The app downloads what the chosen languages need. To download manually:

```powershell
simulvoice models sensevoice       # ASR: zh/en/ja/ko/yue (230 MB)
simulvoice models whisper          # ASR: other languages (~380 MB)
simulvoice models marian           # MT: Chinese <-> English (~250 MB per direction)
simulvoice models marian ja-en     # MT: any opus-mt language pair
simulvoice models nllb             # MT: 200-language fallback (~0.9 GB, slower)
simulvoice models piper de         # offline voices, per language
simulvoice models silero           # Silero VAD
```

Translation route: a direct Marian pair when one exists (about 0.2–0.6 s per sentence); a pivot through English when both sides pair with English (about 0.6–0.9 s); NLLB otherwise (2–5 s). For higher quality, switch to Gemini or OpenAI in advanced settings.

## Command line

```powershell
simulvoice settings         # the GUI (same as double-clicking the exe)
simulvoice settings --start # open and start translating right away
simulvoice run              # headless, using simulvoice.toml in the data folder
simulvoice preset teams     # preset: two-way interpretation
simulvoice doctor --online  # check devices, keys and models, and validate online keys
simulvoice transcribe <audio> [--inbound]              # transcribe an audio file
simulvoice translate "text" --from zh --to en --bench  # translation speed test
simulvoice watch            # switch config by foreground process ([profiles])
```

## Configuration

See the template written by `simulvoice config-init` for every field. Only two languages are configured; recognition languages, translation directions and voices are derived from them:

```toml
[languages]
mine = "zh"      # the language you speak
theirs = "en"    # the language they speak

[audio]
duplex = true                    # two-way interpretation
remote_output_device = "CABLE"   # where the other side hears your translation
skip_own_language = true         # show remarks in your own language untranslated

[ui]
minimize_to_tray = true
check_updates = true             # check for a new version at startup (at most daily)
```

API keys for online services go in `keys.toml` in the data folder or in environment variables; `simulvoice keys encrypt` protects them with DPAPI.

## Building from source

Requirements: Windows 10/11, Rust stable (MSVC), CMake, LLVM/libclang (needed by speexdsp for echo cancellation).

```powershell
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
cargo build --release      # target\release\simulvoice.exe; needs onnxruntime_marian.dll beside it
.\package.ps1              # portable zip + installer (needs Inno Setup 6) into dist\
```

`package.ps1 -DirectML` builds the DirectML-accelerated variant (requires a sherpa-onnx build with DirectML and `SHERPA_ONNX_LIB_DIR`).

## Releasing

1. Bump `version` in `Cargo.toml` and the version in `simulvoice.iss`;
2. Run `.\package.ps1`;
3. Create a GitHub Release tagged `v<version>` and upload `dist\SimulVoiceSetup-<version>.exe` and `dist\simulvoice-<version>-portable.zip`.

The in-app update check reads the latest non-prerelease release. Do not upload `dist\symbols\<version>\` (exe + pdb): keep it to read crash dumps that users send back.

## Known limitations

- Windows only; per-app capture needs Windows 10 2004 or later
- When two people talk at once, their voices are mixed and recognition degrades noticeably
- If both languages use the same script (e.g. English and French, Mandarin and Cantonese), a colleague speaking your language cannot be told apart
- When speech runs on and speech output can't keep up, sentences are dropped once the queue is full (`[audio] queue_limit`, default 32)
- Builds are not code-signed, so SmartScreen may warn on first run

## License

SimulVoice is licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE):

- **Free**: personal use (study, research, personal projects, hobbies) and use by noncommercial organizations such as schools, charities and government institutions
- **Commercial license required**: use at work by a company or team, integration into a commercial product or service, offering paid interpretation services, redistribution for commercial purposes, or any other commercial use

For a commercial license, contact the author on [GitHub](https://github.com/anmezing) or open an issue titled "Commercial license".

Code published before this license change (commit `fbcb39b` and earlier) remains available under the MIT License it was released with.

### Third-party model licenses

Models downloaded on demand carry their own licenses, independent of SimulVoice's; check them separately for commercial use:

| Model / service | Used for | License |
|---|---|---|
| NLLB-200 | fallback translation for less common pairs | CC-BY-NC 4.0, **no commercial use** |
| Marian (opus-mt) | translation | CC-BY 4.0, attribution required |
| Whisper | recognition of other languages | MIT |
| SenseVoice | recognition of zh/en/ja/ko/yue | see FunASR's model license |
| Silero VAD | voice activity detection | MIT |
| Piper voices | offline speech | varies per voice; see each voice card |
| Edge read-aloud | online speech | undocumented Microsoft endpoint; not advisable for commercial products |
| VB-CABLE | virtual microphone | see VB-Audio's terms of use |
