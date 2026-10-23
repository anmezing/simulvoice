# SimulVoice（同声）

Windows 上的实时语音同传工具：捕获系统或指定进程的声音（会议、游戏、播放器），做语音识别 → 翻译 → 语音合成，按你配置的路线输出；支持双向同传、纯本地离线、纯在线和混合模式。

## 功能

- **采集**：WASAPI 系统回环、指定进程（Windows 10 2004+）、麦克风，双向并行
- **识别**：Soniox（流式）、OpenAI、Vosk（离线，模型可下载）
- **翻译**：OpenAI、Gemini、Qwen、任意 OpenAI 兼容端点、Ollama（本地）
- **TTS**：Edge（免费）、OpenAI、Windows SAPI、Piper（离线）
- **端到端实时引擎**：Gemini Live、Palabra.ai、火山引擎 Doubao（语音/文本直出）
- **音频处理**：RNNoise 降噪、Silero VAD、AGC、抖动缓冲、16/24/32-bit 与 float 格式、朗读时压低其他应用音量
- **交互**：双语置顶字幕、多语言额外字幕、托盘图标、全局热键、barge-in 打断
- **工程**：录音回放、双语转写导出、场景预设、自检、单实例、断线重连、DPAPI 密钥加密

## 构建

要求：Windows 10/11、Rust stable（MSVC 工具链）。

```powershell
cargo build --release
# 产物: target\release\simulvoice.exe
```

或生成便携包：

```powershell
.\package.ps1   # dist\simulvoice-<version>.zip
```

## 安装

- **安装版**：运行 `SimulVoiceSetup-*.exe`（安装向导支持中文/英文），默认安装到 `C:\Users\<你>\AppData\Local\Programs\SimulVoice`，可修改安装位置
  - 自动创建开始菜单快捷方式（可选桌面快捷方式）
  - 卸载：Windows 设置 → 应用 → SimulVoice，或开始菜单里的「Uninstall SimulVoice」
  - 数据默认在 `<安装目录>\data`，随安装位置变化；卸载时一并删除
- **便携版**：解压 zip 直接运行 `simulvoice.exe`，数据默认在 `<exe 目录>\data`（相对标记，整体搬走数据跟随）
## 快速开始

双击 `simulvoice.exe`（不带参数）会直接打开设置面板：选场景预设、填密钥、下载模型、保存即可。

命令行等价：

```powershell
simulvoice settings         # 图形化设置（预设 / 密钥 / 模型下载 / 数据目录）
simulvoice preset teams     # 双向同传（Teams / Zoom）
simulvoice preset game      # 游戏听译（只显示字幕）
simulvoice preset record    # 会议记录（字幕 + 转写文件）
simulvoice doctor           # 自检设备/密钥/模型/路由
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
| 全本地 | `stt=vosk` + `translate=ollama` + `tts=sapi` 或 `sherpa` |
| 全在线 | `stt=soniox` + `translate=openai` + `tts=edge` |
| 混合 | 本地识别 + 在线翻译 + 本地 TTS |

端到端引擎只需一行：

```toml
[realtime]
provider = "gemini-live"   # 或 palabra / volcengine
```

## 本地模型

```powershell
simulvoice models vosk cn          # 中文离线识别（小模型）
simulvoice models vosk cn-big      # 大模型（精度更高）
simulvoice models silero           # Silero VAD
simulvoice models piper zh         # 离线神经 TTS（中文）
```

本地翻译：安装 Ollama 后 `ollama pull qwen2.5:3b`，配置 `translate.provider="ollama"`。

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

- `Ctrl+Alt+F8`：停止当前译文（手动打断）\n- `Ctrl+Alt+F9`：静音/恢复译文朗读
- `Ctrl+Alt+F10`：停止
- 托盘图标：双击静音，右键菜单「静音」「退出」

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
[translate]  provider, target, target_reverse, extra_targets, model, endpoint, api_key_env, glossary
[tts]        provider, voice, voice_reverse, model, api_key_env, rate, model_dir
[audio]      input, pid, duplex, mode, mic_device, output_device, remote_output_device, vad_rms, silence_ms
[dsp]        denoise, vad, agc, duck
[ui]         subtitles, transcript_path, record_dir
[realtime]   provider, model, voice, voice_reverse, output_device
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

- 仅 Windows（macOS 需要 CoreAudio/BlackHole 移植）
- 热键静音只影响经典链路与火山 TTS；Gemini/Palabra 自带音频播放不受 F9 控制
- 自动打断默认关闭（避免漏听对方译文），可在设置里开启或按 Ctrl+Alt+F8 手动停止；ducking 覆盖经典链路及 Gemini/Palabra/火山\n- 额外字幕语言仍只作用于经典链路（实时引擎固定单目标）
- 未实现：按前台进程切换配置、模型量化/DirectML、代码签名

## License

MIT










