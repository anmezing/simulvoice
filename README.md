# SimulVoice（同声）

Windows 上的实时语音同传工具：捕获系统或指定进程的声音（会议、游戏、播放器），做语音识别 → 翻译 → 语音合成，按你配置的路线输出；支持双向同传、纯本地离线、纯在线和混合模式。

## 功能

- **采集**：WASAPI 系统回环、指定进程（Windows 10 2004+）、麦克风，双向并行
- **识别**：本地 SenseVoice（中英日韩粤）/ Whisper（其他语言），按语言自动选；在线 Soniox、OpenAI
- **翻译**：本地 Marian（单语言对，快）/ NLLB-200（任意语言兜底），自动选路线；在线 OpenAI、Gemini、Qwen、Ollama、OpenAI 兼容端点
- **语言**：只选「对方说 / 我说」两门语言（21 种），两个方向的识别、翻译、音色自动推出
- **TTS**：Edge（免费）、OpenAI、Windows SAPI、Piper（离线）
- **端到端实时引擎**：Gemini Live、Palabra.ai、火山引擎 Doubao（语音/文本直出）
- **音频处理**：RNNoise 降噪、Silero VAD、AGC、抖动缓冲、16/24/32-bit 与 float 格式、朗读时压低其他应用音量
- **交互**：双语置顶字幕、多语言额外字幕、托盘图标、全局热键、barge-in 打断
- **工程**：录音回放、双语转写导出、场景预设、自检、单实例、断线重连、DPAPI 密钥加密

## 构建

要求：Windows 10/11、Rust stable（MSVC 工具链）。
AEC 依赖静态编译 speexdsp：另需 CMake 与 LLVM/libclang。构建前先设好环境变量（本机 LLVM 在 `D:\Program Files\LLVM`）：

```powershell
$env:LIBCLANG_PATH = "D:\Program Files\LLVM\bin"
cargo build --release
# 产物: target\release\simulvoice.exe（同目录需有 onnxruntime_marian.dll，本地翻译用）
```

或生成便携包：

```powershell
.\package.ps1   # dist\simulvoice-<version>.zip
```

## 安装

- **安装版**：运行 `SimulVoiceSetup-*.exe`（安装向导支持中文/英文），默认安装到 `C:\Users\<你>\AppData\Local\Programs\SimulVoice`，可修改安装位置
  - 自动创建开始菜单快捷方式（可选桌面快捷方式）
  - 卸载：Windows 设置 → 应用 → SimulVoice，或开始菜单里的「Uninstall SimulVoice」
  - 数据默认在 `<安装目录>\data`，随安装位置变化；卸载会保留数据，需手动删除 `data` 目录
- **便携版**：解压 zip 直接运行 `simulvoice.exe`，数据默认在 `<exe 目录>\data`（相对标记，整体搬走数据跟随）
## 快速开始

双击 `simulvoice.exe`，三步：

1. 选「对方说」和「我说」的语言；
2. 选「只听翻译」或「双向翻译」；
3. 选会议软件（腾讯会议、Teams、Zoom 等开着时会自动选中），点「开始翻译」。

第一次开始时会自动下载所需模型（中英约 0.7 GB，只需一次）。译文字幕实时显示在窗口里，日志写在数据目录的 `simulvoice.log`。
双向翻译要让对方听到你的译音，需安装免费的 [VB-CABLE](https://vb-audio.com/Cable/)，并在会议软件里把麦克风选成 `CABLE Output`；没装时你的话只显示字幕。

命令行等价：

```powershell
simulvoice settings         # 图形界面
simulvoice settings --start # 打开即开始翻译（可做成桌面快捷方式）
simulvoice preset teams     # 双向同传（Teams / Zoom）
simulvoice preset game      # 游戏听译（只显示字幕）
simulvoice preset record    # 会议记录（字幕 + 转写文件）
simulvoice doctor --online  # 自检设备/密钥/模型/路由，并校验 Key 有效性
simulvoice run              # 使用数据目录里的 simulvoice.toml
```

## 数据目录

- **安装版**：默认 `<安装目录>\data`，与安装位置一致
- **便携版**：默认 `<exe 所在目录>\data`，标记为相对路径，整体搬动目录后数据仍跟随
- 查看/修改：`simulvoice data-dir`、`simulvoice data-dir E:\SimulVoiceData --move`，或在设置面板里点击「更改数据目录…」（立即生效）
- 环境变量 `SIMULVOICE_DATA_DIR` 优先级最高
- 模型、密钥、配置、录音都在该目录

## 三种组合

| 路线 | 配置 |
|---|---|
| 全本地（推荐） | `stt=local` + `translate=local` + `tts=edge`（在线朗读）或 `sherpa`（离线） |
| 全在线 | `stt=soniox` + `translate=openai` + `tts=edge` |
| 混合 | 本地识别 + 在线翻译 + 本地 TTS |

端到端引擎只需一行：

```toml
[realtime]
provider = "gemini-live"   # 或 palabra / volcengine
```

## 本地模型

图形界面会按所选语言自动下载；命令行也可手动下载：

```powershell
simulvoice models sensevoice       # 识别：中英日韩粤（230 MB，准确、带标点）
simulvoice models whisper          # 识别：其他语言（Whisper-small int8，约 380 MB）
simulvoice models marian           # 翻译：中英两个方向（每个方向约 250 MB）
simulvoice models marian ja-en     # 翻译：任意 opus-mt 语言对
simulvoice models nllb             # 翻译：200 种语言兜底（约 0.9 GB，较慢）
simulvoice models piper de         # 离线朗读，按语言（zh en fr de es ru pt it ar vi tr pl）
simulvoice models silero           # Silero VAD
simulvoice models aec              # 神经 AEC 模型（可选）
```

本地翻译路线：有直达的 Marian 语言对就直达（约 0.2–0.6 秒/句）；两边都能和英语互译时经英语中转（约 0.6–0.9 秒）；
其余（如 英→日/韩、粤语）用 NLLB（2–5 秒/句）。追求译文质量可在「高级设置」里改用 Gemini / OpenAI。

自测：`simulvoice translate "文本" --from zh --to en --bench`、`simulvoice transcribe 音频文件`。

## 双向同传（Teams / Zoom / 游戏）

```toml
[audio]
duplex = true
input = "loopback"          # 或 "pid" 只抓会议/游戏进程
mic_device = ""             # 你的麦克风
output_device = "耳机"       # 你听到的译文
remote_output_device = "CABLE"  # 对方听到的译文（虚拟声卡）
```

- 会议软件：麦克风选 `CABLE Output`，扬声器保持被采集的设备
- 译文播放设备必须 ≠ 被采集设备，程序会拒绝相同设备以防自激
- 3.5mm 耳机同时说/听：`input="pid"` + `pid=<会议进程>` 即可避免自激

## 热键 / 托盘

- `Ctrl+Alt+F7`：开始/结束（toggle）或按住（hold），取决于 `[audio] voice_mode`
  - 按键可改：`ptt_key`、`ptt_ctrl`、`ptt_alt`
- `Ctrl+Alt+F8`：停止当前译文（手动打断）
- `Ctrl+Alt+F9`：静音/恢复译文朗读
- `Ctrl+Alt+F10`：停止
- `Ctrl+Alt+F6`：回放最近一次保存的译文
- 托盘图标：双击静音，右键菜单「静音」「退出」

## 按前台进程切换配置

```toml
[profiles]
"Teams.exe" = "profiles/teams.toml"
"game.exe"  = "profiles/game.toml"
```

```powershell
simulvoice watch --config simulvoice.toml
```

前台进程变化时自动结束当前子进程并用对应配置重启；未匹配的进程回到基础配置。按 F10 或子进程正常退出会停止监视。

## 录音与回放

- `[ui] keep_translation = true` 默认保存译文 MP3 到数据目录 `recordings/`，`F6` 回放
- `record_source = true` 才额外录制原始 WAV；`record_max_mb` 限制总大小
- 外放且双向同传时开启 `[audio] echo_guard = true`：朗读译文期间暂停出向识别，避免回环

## 密钥

支持环境变量或本地 `keys.toml`（不提交到仓库）：

```powershell
simulvoice keys encrypt --config keys.toml   # DPAPI 加密（当前用户）
simulvoice keys test                         # 加解密自检
```

## 配置

完整字段见 `simulvoice config-init` 生成的模板。主要分节：

```toml
version = 1
[stt]        provider, language, language_reverse, api_key_env, model, model_path
[translate]  provider, target, target_reverse, extra_targets, marian_dir, model, endpoint, api_key_env, glossary
[tts]        provider, voice, voice_reverse, model, api_key_env, rate, model_dir
[audio]      input, loopback_device, pid, duplex, voice_mode, ptt_key, ptt_ctrl, ptt_alt, barge_in, echo_guard, passthrough, passthrough_gain, mic_device, output_device, remote_output_device, monitor_translation, monitor_device, vad_rms, silence_ms, queue_limit
[dsp]        denoise, vad, agc, duck, aec, aec_filter_ms
[ui]         subtitles, transcript_path, record_dir, keep_translation, record_source, record_max_mb
[realtime]   provider, model, voice, voice_reverse, output_device
[profiles]   前台进程名 -> 配置文件（watch 模式）
```

## 架构

```
WASAPI / 进程回环 / 麦克风
  → 格式解析 → 降混 → (48k) 降噪 → 重采样 16k → VAD
      → STT（Vosk / Soniox / OpenAI）→ 翻译（OpenAI / Gemini / Qwen / Ollama）
          → TTS（Edge / SAPI / Piper）→ 指定输出设备
  或
  → 实时引擎（Gemini Live / Palabra / 火山）→ 译文语音/文本
```

源码：`src/capture.rs`、`dsp.rs`、`vad.rs`、`silero_vad.rs`、`stt.rs`、`translate.rs`、`tts.rs`、`edge_tts.rs`、`gemini_live.rs`、`palabra.rs`、`volc.rs`、`sink.rs`、`ui.rs`、`tray.rs`、`hotkeys.rs`、`recorder.rs`、`keys.rs`、`config.rs`、`main.rs`。

## 已知限制

- 仅 Windows
- AEC 默认开启 `[dsp] aec = true`：用输出设备回环作为参考（Speex），外放时抑制译文回声；不需要可关闭\n- 可选神经 AEC：`[dsp] neural_aec = true`（先 models aec），用 DEC baseline ONNX 替代 Speex，失败自动回退
- 实时引擎双向已支持（`duplex=true` 时各开一条会话）；额外字幕与 `echo_guard` 已覆盖实时链路（barge-in 仍只作用于经典链路）
- 语音持续且 TTS 明显跟不上时，队列满会丢弃句子；上限可配 `[audio] queue_limit`（默认 32 句）
- 卸载保留 `<安装目录>\data`，避免误删模型与录音；需要时手动删除
- 未实现：模型量化/DirectML；代码签名需自行申请证书（可用 SignPath 免费 OSS 计划）
## 本地模型加速

- 默认 CPU：`[tts] onnx_provider`、`[dsp] onnx_provider` 均为 `cpu`。
- DirectML 不在官方预编译包里：需自行用源码构建 sherpa-onnx（`-DSHERPA_ONNX_ENABLE_DIRECTML=ON -DBUILD_SHARED_LIBS=ON -DSHERPA_ONNX_USE_STATIC_CRT=OFF`），设置 `SHERPA_ONNX_LIB_DIR`，并随程序分发 onnxruntime/DirectML DLL，然后把 provider 改成 `directml`。
- 当前本地 ONNX 链路已覆盖 SenseVoice / Whisper ASR、Marian / NLLB 翻译、Silero VAD、Piper TTS；`[stt] provider = "local"` 按语言自动选识别模型。
- DirectML 版：源码编译 sherpa-onnx shared（`SHERPA_ONNX_ENABLE_DIRECTML=ON`）后运行 `package.ps1 -DirectML`，生成 `*-directml-portable.zip`（含 DirectML/onnxruntime DLL），解压后把 `[dsp]/[tts] onnx_provider` 设为 `directml`；缺 DLL 或驱动不支持时 sherpa 自动回退 CPU。
- 量化模型可直接使用 int8 文件。

## 代码签名

免费途径：

- **SignPath Foundation**：公开仓库 + OSI 许可证的 OSS 项目可免费获得 OV 级签名，但新项目可能因仓库历史不足被拒；可等仓库有历史/使用量后再申请。
- **Microsoft Store（MSIX）**：走商店分发时由 Microsoft 免费重签，无需自备证书；需要做 MSIX 打包并上架。
- **Certum Open Source**：约 €25/年，不是免费，但比普通 OV 便宜；安装证书后可用 `sign.ps1 -Thumbprint <SHA1>` 走证书容器/云签名。
- **Azure Artifact Signing（原 Trusted Signing）**：约 $9.99/月，不免费。
- 自签名证书免费，但公众机器不信任，只适合内网/测试。

已有证书时：

```powershell
.\sign.ps1 -CertPath C:\path\cert.pfx -Password <password>
# 或证书已装在 CurrentUser\My：
.\sign.ps1 -Thumbprint <SHA1>
```
使用 Windows SDK 的 signtool 对 exe 与安装包做 SHA-256 时间戳签名。

## License

MIT



























