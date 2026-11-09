**简体中文** | [English](README.en.md)

# SimulVoice（同声）

Windows 上的会议同声传译工具。收听会议软件的声音，实时识别、翻译并念给你听；开启双向翻译后，你说的话也会被翻译成对方的语言送进会议。默认全部在本机运行，不需要账号和密钥。

## 功能

- **双向同传**：对方说英语你听中文，你说中文对方听英语；21 种语言任选两门
- **全本地可离线**：识别用 SenseVoice（中英日韩粤）/ Whisper（其他语言），翻译用 Marian / NLLB-200，按语言自动选模型；也可接 Soniox、OpenAI、Gemini、Qwen、Ollama 等在线服务
- **只收会议软件的声音**：可单独收听腾讯会议、Teams、Zoom、钉钉、飞书或浏览器，其他软件的声音不受影响
- **同事说你的语言时不翻译**：中英混合的会议里，中文同事插话只显示原话，不会被当成英文乱译
- **朗读**：Edge（免费在线）、Piper（离线）、Windows SAPI、OpenAI
- **我的音色**（可选）：录一段自己的声音，对方听到的译文用你的音色念（本地克隆，中/英文）
- **端到端实时引擎**：Gemini Live、Palabra.ai、火山引擎
- **音频处理**：降噪、Silero VAD、回声消除、自动增益、朗读时压低其他声音
- **使用体验**：最小化到托盘、全局热键、双语字幕、录音与回放、双语转写导出、检查更新、崩溃现场记录

## 下载安装

在 [Releases](https://github.com/anmezing/simulvoice/releases) 下载：

- **安装版** `SimulVoiceSetup-<版本>.exe`：默认装到 `%LOCALAPPDATA%\Programs\SimulVoice`，可改位置；创建开始菜单快捷方式（桌面快捷方式可选）
- **便携版** `simulvoice-<版本>-portable.zip`：解压后运行 `simulvoice.exe`，数据放在 exe 旁边的 `data` 文件夹，整个文件夹可以拷走

覆盖安装会保留设置和已下载的模型。卸载时会问是否同时删除数据（默认保留）。

## 快速开始

双击 `simulvoice.exe`，三步：

1. 选「对方说」和「我说」的语言；
2. 选「只听翻译」或「双向翻译」；
3. 在「收听来源」里选会议软件（打开时已在运行的会议软件会自动选中），点「开始翻译」。

第一次开始时自动下载所需模型（中英约 0.7 GB，只需一次）。

**双向翻译**要让对方听到你的译音，需要安装免费的虚拟麦克风 [VB-CABLE](https://vb-audio.com/Cable/)，然后在会议软件里把麦克风选成 `CABLE Output`。没装也能用，你的话只显示字幕。建议戴耳机。

**网页会议**（Google Meet、Teams 网页版等）：收听来源选 Chrome / Edge / Firefox，会收到整个浏览器的声音，其他标签页放的声音请先静音。

## 收听来源

| 选项 | 收到的声音 | 适合 |
|---|---|---|
| 会议软件（推荐） | 只有这个软件的声音 | 绝大多数会议 |
| 浏览器 | 整个浏览器的声音 | 网页会议 |
| 整个电脑的声音 | 电脑放出的所有声音 | 上面都不行时；需戴耳机，否则译文会被再次收进来 |

## 托盘与热键

- 点最小化会收到右下角托盘，翻译不中断；点托盘图标恢复窗口，右键可静音或退出（设置里可关闭）
- `Ctrl+Alt+F8`：停止当前正在念的译文
- `Ctrl+Alt+F9`：静音 / 恢复朗读
- `Ctrl+Alt+F10`：停止翻译
- `Ctrl+Alt+F6`：回放最近一句译文
- `Ctrl+Alt+F7`：手动说话模式下的按键（`[audio] voice_mode`、`ptt_key` 可改）

## 数据目录

模型、设置、密钥、录音、日志都在数据目录：安装版默认 `<安装目录>\data`，便携版默认 `<exe 目录>\data`。

- 在「高级设置 → 数据」里可以打开或更改；命令行 `simulvoice data-dir E:\SimulVoiceData --move`
- 环境变量 `SIMULVOICE_DATA_DIR` 优先级最高
- 日志：`simulvoice.log`；程序崩溃时的现场文件：`crashdumps\`（最多保留 5 份，反馈问题时请附上）

## 本地模型

界面会按所选语言自动下载，也可以手动下载：

```powershell
simulvoice models sensevoice       # 识别：中英日韩粤（230 MB）
simulvoice models whisper          # 识别：其他语言（约 380 MB）
simulvoice models marian           # 翻译：中英两个方向（每个方向约 250 MB）
simulvoice models marian ja-en     # 翻译：任意 opus-mt 语言对
simulvoice models nllb             # 翻译：200 种语言兜底（约 0.9 GB，较慢）
simulvoice models piper de         # 离线朗读，按语言
simulvoice models silero           # Silero VAD
simulvoice models zipvoice         # 我的音色：克隆模型（中/英，约 170 MB）
```

翻译路线：有直达的 Marian 语言对就直达（约 0.2–0.6 秒/句）；都能和英语互译时经英语中转（约 0.6–0.9 秒）；其余用 NLLB（2–5 秒/句）。更看重译文质量可在「高级设置」里改用 Gemini / OpenAI。

## 我的音色

双向翻译时，对方默认听到的是内置音色。想让对方听到你自己的声音：

1. 高级设置 → 我的音色：起个名字，点「录一段我的声音」，照提示念一段（约 10 秒），点「念完了，保存」；
2. 「对方听到」选这个音色。首次使用会下载克隆模型（约 170 MB）。

- **照提示念对方的语言**：对方说英语就念英文那段。用中文录音去说英文，语速会被拉慢、吐字不清
- 只能说中文和英文；对方是其他语言、模型没下载或合成失败时，自动退回内置音色
- 全本地合成。每句比 Edge 多 1–3 秒才开始出声（12 线程 CPU 实测：英文合成耗时约为音频长度的 0.65 倍，中文约 0.5 倍），不会越积越多
- 能还原音色和整体说话风格，但不会逐句跟着你当时的情绪变化
- 命令行：`simulvoice voice record|import|test|list|delete`；`[tts] clone_steps` 调采样步数（0=自动，越大越细腻越慢）
- 用 Palabra 实时引擎时，高级设置里勾选「对方听到我自己的音色」即可，云端从你说话中实时克隆，不用录音

## 命令行

```powershell
simulvoice settings         # 图形界面（双击 exe 也是它）
simulvoice settings --start # 打开即开始翻译
simulvoice run              # 按数据目录里的 simulvoice.toml 运行（无界面）
simulvoice preset teams     # 预设：双向同传
simulvoice doctor --online  # 自检设备、密钥、模型，并校验在线密钥
simulvoice transcribe <音频> [--inbound]           # 识别一个音频文件
simulvoice translate "文本" --from zh --to en --bench # 翻译测速
simulvoice watch            # 按前台进程自动切换配置（[profiles]）
```

## 配置

完整字段见 `simulvoice config-init` 生成的模板。语言只需写两门，识别、翻译方向和音色都由它推出：

```toml
[languages]
mine = "zh"      # 我说
theirs = "en"    # 对方说

[audio]
duplex = true                    # 双向同传
remote_output_device = "CABLE"   # 对方听到的译文
skip_own_language = true         # 对方那路有人说我的语言时只显示原话

[ui]
minimize_to_tray = true
check_updates = true             # 打开时检查新版本（每天最多一次）
```

在线服务的密钥放在数据目录的 `keys.toml` 或环境变量里，可用 `simulvoice keys encrypt` 以 DPAPI 加密。

## 从源码构建

要求：Windows 10/11、Rust stable（MSVC）、CMake、LLVM/libclang（回声消除依赖的 speexdsp 需要）。

```powershell
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
cargo build --release      # target\release\simulvoice.exe，同目录需要 onnxruntime_marian.dll
.\package.ps1              # 便携版 zip + 安装包（需要 Inno Setup 6）到 dist\
```

`package.ps1 -DirectML` 生成 DirectML 加速版（需自行编译带 DirectML 的 sherpa-onnx，并设置 `SHERPA_ONNX_LIB_DIR`）。

## 发布新版本

1. 改 `Cargo.toml` 的 `version`，以及 `simulvoice.iss` 里的版本号；
2. 运行 `.\package.ps1`；
3. 在 GitHub 新建 Release，标签写 `v<版本>`，上传 `dist\SimulVoiceSetup-<版本>.exe` 和 `dist\simulvoice-<版本>-portable.zip`。

程序的「检查更新」读的就是这里的最新正式版。`dist\symbols\<版本>\` 里的 exe 和 pdb 不要上传，自己留着，用来分析用户发回的崩溃文件。

## 已知限制

- 只支持 Windows；按进程收听需要 Windows 10 2004 及以上
- 两个人同时说话时声音会混在一起，识别效果明显下降
- 两门语言文字相同（如英语和法语、普通话和粤语）时，无法区分同事是否在说你的语言
- 语音持续且朗读跟不上时，队列满会丢弃句子（`[audio] queue_limit`，默认 32 句）
- 安装包未做代码签名，首次运行可能被 SmartScreen 提示

## 许可与授权

SimulVoice 采用 [PolyForm Noncommercial License 1.0.0](LICENSE)：

- **免费**：个人使用（学习、研究、个人项目、爱好），以及学校、公益组织、政府机构等非商业组织使用
- **需要商业授权**：公司或团队在工作中使用、集成到商业产品或服务里、对外提供收费的翻译服务、再分发用于商业目的等任何商业用途

商业授权请在 [GitHub](https://github.com/anmezing) 联系作者，或在仓库提交一个标题为「商业授权」的 Issue。

在本次许可变更之前公开的代码（提交 `fbcb39b` 及更早）仍按当时的 MIT 许可提供。

### 第三方模型的许可

程序会按需下载的模型有各自的许可，与 SimulVoice 本身的授权无关，商业使用时需要分别确认：

| 模型 / 服务 | 用途 | 许可 |
|---|---|---|
| NLLB-200 | 小语种翻译兜底 | CC-BY-NC 4.0，**不允许商用** |
| Marian（opus-mt） | 翻译 | CC-BY 4.0，需注明出处 |
| Whisper | 识别其他语言 | MIT |
| SenseVoice | 识别中英日韩粤 | 见 FunASR 的模型许可 |
| Silero VAD | 语音检测 | MIT |
| Piper 音色 | 离线朗读 | 每个音色不同，见各音色说明 |
| ZipVoice + Vocos | 我的音色（克隆） | 代码 Apache-2.0 / MIT；ZipVoice 模型用 Emilia 数据集训练（CC-BY-NC 4.0），商用前请自行确认 |
| Edge 朗读 | 在线朗读 | 微软未公开的接口，不建议商业产品依赖 |
| VB-CABLE | 虚拟麦克风 | 见 VB-Audio 官网的使用条款 |
