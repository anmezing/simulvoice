use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    /// 会议的两门语言。识别语言、翻译方向、朗读音色都由它推出（见 apply_languages）
    pub languages: LanguagesConfig,
    pub stt: SttConfig,
    pub translate: TranslateConfig,
    pub tts: TtsConfig,
    pub audio: AudioConfig,
    pub dsp: DspConfig,
    pub ui: UiConfig,
    pub realtime: RealtimeConfig,
    /// 前台进程名 -> 配置文件（watch 模式）
    pub profiles: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct LanguagesConfig {
    /// 我说的语言（入向译文的目标、出向识别的语言）
    pub mine: String,
    /// 对方说的语言（入向识别的语言、出向译文的目标）
    pub theirs: String,
}

impl Default for LanguagesConfig {
    fn default() -> Self {
        Self {
            mine: "zh".into(),
            theirs: "en".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct SttConfig {
    /// local（按语言自动选 SenseVoice/Whisper）| sherpa(SenseVoice) | whisper | zipformer | vosk | soniox | openai | none
    pub provider: String,
    pub language: String,
    /// 出向（我 → 对方）识别语言
    pub language_reverse: String,
    /// 环境变量名（soniox/openai）
    pub api_key_env: String,
    /// openai 模型
    pub model: String,
    /// vosk 模型目录
    pub model_path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct TranslateConfig {
    /// local（Marian/NLLB 自动选）| marian | nllb | openai | gemini | ollama | qwen | custom | none
    pub provider: String,
    pub target: String,
    /// 出向翻译目标语言
    pub target_reverse: String,
    pub model: String,
    pub endpoint: String,
    pub api_key_env: String,
    /// 术语表（每行「原文=译文」或自由描述）
    pub glossary: String,
    /// 额外字幕语言（仅字幕，不朗读）
    pub extra_targets: Vec<String>,
    /// Marian model dir (empty = data/models/marian)
    pub marian_dir: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct TtsConfig {
    /// edge | sapi | openai | none
    pub provider: String,
    pub voice: String,
    /// 出向音色（我 → 对方）
    pub voice_reverse: String,
    pub model: String,
    pub api_key_env: String,
    pub rate: String,
    /// Piper/sherpa 本地 TTS 模型目录
    pub model_dir: String,
    /// ONNX 执行提供者（cpu；DirectML 需自定义 sherpa-onnx 构建）
    pub onnx_provider: String,
    /// 本地 TTS 线程数
    pub threads: u32,
    /// 出向译文用「我的音色」朗读：data/voices 下的音色名，空 = 关（用上面的内置音色）。
    /// 只作用于出向；对方语言不是中/英、模型没下载、音色缺失时自动退回内置音色
    pub clone_voice: String,
    /// 我的音色采样步数：0 = 自动（中文 3、英文 2），越大越细腻也越慢
    pub clone_steps: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct AudioConfig {
    /// loopback | pid | mic
    pub input: String,
    /// 回环采集的渲染设备名（空=默认输出设备）
    pub loopback_device: String,
    pub pid: u32,
    /// 会议软件的进程名（如 wemeetapp.exe）。PID 每次启动都会变，按名字重新找
    pub process: String,
    /// 入向输出设备名（子串匹配），空=系统默认
    pub output_device: String,
    /// 出向采集：麦克风设备名（空 = 默认麦克风）
    pub mic_device: String,
    /// 出向输出：虚拟麦克风设备名，如 "CABLE"
    pub remote_output_device: String,
    /// 双向同传：入向（对方→你）+ 出向（你→对方）
    pub duplex: bool,
    /// 检测到语音时自动停止当前译文（实验性，默认关闭）
    pub barge_in: bool,
    /// both | listen | speak
    pub mode: String,
    /// 静音阈值（i16 RMS），低于视为静音
    pub vad_rms: f64,
    /// 静音多久判定一句话结束
    pub silence_ms: u64,
    /// 单句最长时长
    pub max_utterance_ms: u64,
    /// 翻译/TTS 待处理队列上限（句）
    pub queue_limit: u32,
    /// auto | toggle | hold | hold_translate
    pub voice_mode: String,
    /// 手动模式按键（F1-F12、字母、空格、Pause 等）
    pub ptt_key: String,
    /// 手动模式是否需要 Ctrl
    pub ptt_ctrl: bool,
    /// 手动模式是否需要 Alt
    pub ptt_alt: bool,
    /// 原声直通到虚拟麦克风
    pub passthrough: bool,
    /// 原声直通增益
    pub passthrough_gain: f32,
    /// 朗读译文时暂停出向识别，避免外放被麦克风拾取形成回环
    pub echo_guard: bool,
    /// 出向译文同时回放给自己
    pub monitor_translation: bool,
    /// 监听设备（空=入向输出设备）
    pub monitor_device: String,
    pub cross_barge_in: bool,
    pub mute_inbound: bool,
    pub mute_outbound: bool,
    pub backlog_jump_secs: u32,
    /// 对方那一路听到"我的语言"（如中英会议里中文同事插话）时只显示原文，不翻译、不朗读
    pub skip_own_language: bool,
    /// 只听译文：翻译期间把会议软件的音量关到听不见（采集端放大还原，识别不受影响）
    pub translation_only: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct DspConfig {
    /// RNNoise 降噪（48 kHz 源有效）
    pub denoise: bool,
    /// silero | energy
    pub vad: String,
    /// Silero VAD ONNX 模型路径
    pub vad_model: String,
    /// 麦克风自动增益
    pub agc: bool,
    /// 朗读时压低其他应用音量
    pub duck: bool,
    /// AEC 回声消除（用输出设备回环作为参考）
    pub aec: bool,
    /// AEC 自适应滤波器长度（毫秒）
    pub aec_filter_ms: u32,
    /// ONNX 执行提供者（cpu；DirectML 需自定义 sherpa-onnx 构建）
    pub onnx_provider: String,
    /// Silero VAD 线程数
    pub vad_threads: u32,
    pub neural_aec: bool,
    pub neural_aec_delay_ms: u32,
    pub neural_aec_model: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct UiConfig {
    /// 显示双语字幕悬浮窗
    pub subtitles: bool,
    /// 双语转写导出路径（空=不导出）
    pub transcript_path: String,
    /// 录音目录（原声 WAV + 译文 MP3；空=数据目录/recordings）
    pub record_dir: String,
    /// 保存译文音频以便回放
    pub keep_translation: bool,
    /// 录制原始音频
    pub record_source: bool,
    /// 录音总大小上限（MB，0=不限）
    pub record_max_mb: u64,
    /// 最小化时收到托盘（任务栏不再占位，点托盘图标恢复）
    pub minimize_to_tray: bool,
    /// 启动时自动检查新版本（每天最多一次）
    pub check_updates: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            languages: LanguagesConfig::default(),
            stt: SttConfig::default(),
            translate: TranslateConfig::default(),
            tts: TtsConfig::default(),
            audio: AudioConfig::default(),
            dsp: DspConfig::default(),
            ui: UiConfig::default(),
            realtime: RealtimeConfig::default(),
            profiles: std::collections::BTreeMap::new(),
        }
    }
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            provider: "local".into(),
            language: "en".into(),
            language_reverse: "zh".into(),
            api_key_env: "SONIOX_API_KEY".into(),
            model: "gpt-4o-mini-transcribe".into(),
            model_path: default_models_dir().join("vosk").display().to_string(),
        }
    }
}

impl Default for TranslateConfig {
    fn default() -> Self {
        Self {
            provider: "local".into(),
            target: "Chinese".into(),
            target_reverse: "English".into(),
            model: "qwen2.5:3b".into(),
            endpoint: "http://127.0.0.1:11434".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            glossary: String::new(),
            marian_dir: String::new(),
            extra_targets: Vec::new(),
        }
    }
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            provider: "edge".into(),
            voice: "zh-CN-XiaoxiaoNeural".into(),
            voice_reverse: "en-US-AriaNeural".into(),
            model: "gpt-4o-mini-tts".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            rate: "+0%".into(),
            model_dir: default_models_dir().join("tts").display().to_string(),
            onnx_provider: "cpu".into(),
            threads: 2,
            clone_voice: String::new(),
            clone_steps: 0,
        }
    }
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input: "loopback".into(),
            loopback_device: String::new(),
            pid: 0,
            process: String::new(),
            output_device: String::new(),
            mic_device: String::new(),
            remote_output_device: String::new(),
            duplex: false,
            barge_in: false,
            cross_barge_in: false,
            mute_inbound: false,
            mute_outbound: false,
            backlog_jump_secs: 0,
            skip_own_language: true,
            translation_only: false,
            mode: "both".into(),
            vad_rms: 250.0,
            silence_ms: 450,
            max_utterance_ms: 8000,
            queue_limit: 32,
            voice_mode: "auto".into(),
            ptt_key: "F7".into(),
            ptt_ctrl: true,
            ptt_alt: true,
            passthrough: false,
            passthrough_gain: 1.0,
            echo_guard: false,
            monitor_translation: false,
            monitor_device: String::new(),
        }
    }
}

impl Default for DspConfig {
    fn default() -> Self {
        Self {
            denoise: true,
            vad: "silero".into(),
            agc: true,
            duck: true,
            aec: true,
            aec_filter_ms: 300,
            onnx_provider: "cpu".into(),
            vad_threads: 1,
            neural_aec: false,
            neural_aec_delay_ms: 0,
            neural_aec_model: String::new(),
            vad_model: default_models_dir()
                .join("silero_vad_v5.onnx")
                .display()
                .to_string(),
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            subtitles: false,
            transcript_path: String::new(),
            record_dir: String::new(),
            keep_translation: true,
            record_source: false,
            record_max_mb: 2048,
            minimize_to_tray: true,
            check_updates: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct RealtimeConfig {
    /// none | gemini-live
    pub provider: String,
    pub model: String,
    pub voice: String,
    pub voice_reverse: String,
    pub output_device: String,
    /// Palabra 出向用我的实时音色（云端从我说话中克隆，无需录音）
    pub voice_cloning: bool,
}

impl Default for RealtimeConfig {
    fn default() -> Self {
        Self {
            provider: "none".into(),
            model: "models/gemini-2.0-flash-live-001".into(),
            voice: "Puck".into(),
            voice_reverse: "Puck".into(),
            output_device: String::new(),
            voice_cloning: false,
        }
    }
}

pub const CONFIG_VERSION: u32 = 1;

pub fn default_models_dir() -> PathBuf {
    crate::paths::models_dir()
}

impl Config {
    pub fn load(path: Option<&str>) -> Result<Self> {
        let path = match path {
            Some(p) => std::path::PathBuf::from(p),
            None => crate::paths::default_config_path(),
        };
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("读取配置失败: {}", path.display()))?;
        let raw: toml::Value = toml::from_str(&text)?;
        let mut cfg: Config = toml::from_str(&text)?;
        if raw.get("languages").is_none() {
            cfg.languages = infer_languages(&cfg);
            migrate_engines(&mut cfg);
        }
        cfg.apply_languages();
        if cfg.version == 0 {
            cfg.version = CONFIG_VERSION;
        } else if cfg.version > CONFIG_VERSION {
            eprintln!(
                "[config] 配置版本 {} 高于程序支持的 {}，可能有未知字段",
                cfg.version, CONFIG_VERSION
            );
        }
        Ok(cfg)
    }

    /// 由 `[languages]` 推出两个方向的识别语言、翻译目标和朗读音色。
    ///
    /// 入向（对方 → 我）：识别 theirs，译成 mine，用 mine 的音色朗读；
    /// 出向（我 → 对方）：识别 mine，译成 theirs，用 theirs 的音色朗读。
    /// 用户自选的音色只要语种对得上就保留，换了语言才换成该语言的默认音色。
    pub fn apply_languages(&mut self) {
        let mine = crate::lang::find(&self.languages.mine)
            .or_else(|| crate::lang::find("zh"))
            .expect("语言表里必有 zh");
        let theirs = crate::lang::find(&self.languages.theirs)
            .or_else(|| crate::lang::find("en"))
            .expect("语言表里必有 en");
        self.languages.mine = mine.code.into();
        self.languages.theirs = theirs.code.into();
        self.stt.language = theirs.code.into();
        self.stt.language_reverse = mine.code.into();
        self.translate.target = mine.name.into();
        self.translate.target_reverse = theirs.name.into();
        if !voice_fits(&self.tts.voice, mine) {
            self.tts.voice = mine.edge_voice.into();
        }
        if !voice_fits(&self.tts.voice_reverse, theirs) {
            self.tts.voice_reverse = theirs.edge_voice.into();
        }
    }

    /// 本次运行实际会跑的翻译方向（源语言, 目标语言）。
    /// 入向：对方 → 我；出向：我 → 对方（双向同传或 mode=speak 时）。
    pub fn directions(&self) -> Vec<(&'static crate::lang::Lang, &'static crate::lang::Lang)> {
        let (mine, theirs) = (self.mine(), self.theirs());
        match self.audio.mode.as_str() {
            "speak" => vec![(mine, theirs)],
            "listen" => vec![(theirs, mine)],
            _ if self.audio.duplex => vec![(theirs, mine), (mine, theirs)],
            _ => vec![(theirs, mine)],
        }
    }

    pub fn mine(&self) -> &'static crate::lang::Lang {
        crate::lang::find(&self.languages.mine).unwrap_or(&crate::lang::LANGS[0])
    }

    pub fn theirs(&self) -> &'static crate::lang::Lang {
        crate::lang::find(&self.languages.theirs).unwrap_or(&crate::lang::LANGS[1])
    }

    pub fn write_template(path: &str) -> Result<()> {
        std::fs::write(path, TEMPLATE)?;
        Ok(())
    }

    pub fn save(&self, path: &str) -> Result<()> {
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// 旧版配置（没有 `[languages]`）里跑不起来或已被取代的引擎，换成本地引擎。
///
/// 旧模板默认是 Soniox + Ollama：没有密钥、没装 Ollama 的机器上双击启动就报错。
/// 只替换"肯定跑不起来"或"已有更好替代"的情况，用户真配好的在线引擎保持不变。
fn migrate_engines(cfg: &mut Config) {
    let key = |name: &str| std::env::var(name).is_ok();
    let mut changed = Vec::new();
    let stt_dead = match cfg.stt.provider.as_str() {
        "soniox" => !key("SONIOX_API_KEY"),
        "openai" => !key("OPENAI_API_KEY"),
        // Zipformer 中文会叠字（"上周周周周"），SenseVoice 全面更好
        "zipformer" | "sherpa" => crate::stt::sensevoice_ready(),
        _ => false,
    };
    if stt_dead {
        changed.push(format!("识别 {} → local", cfg.stt.provider));
        cfg.stt.provider = "local".into();
    }
    let tr_dead = match cfg.translate.provider.as_str() {
        "openai" => !key("OPENAI_API_KEY"),
        "gemini" => !key("GEMINI_API_KEY"),
        "qwen" => !key("DASHSCOPE_API_KEY"),
        "ollama" => !ollama_running(&cfg.translate.endpoint),
        "marian" => true,
        _ => false,
    };
    if tr_dead {
        changed.push(format!("翻译 {} → local", cfg.translate.provider));
        cfg.translate.provider = "local".into();
    }
    if !changed.is_empty() {
        eprintln!("[config] 旧版配置已升级：{}", changed.join("，"));
    }
}

fn ollama_running(endpoint: &str) -> bool {
    use std::net::ToSocketAddrs;
    let host = endpoint
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or("127.0.0.1:11434")
        .to_string();
    let host = if host.contains(':') { host } else { format!("{host}:11434") };
    host.to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
        .map(|a| std::net::TcpStream::connect_timeout(&a, std::time::Duration::from_millis(300)).is_ok())
        .unwrap_or(false)
}

/// 旧配置没有 `[languages]`：从原来分散的 target / language 字段推断，保持原行为。
fn infer_languages(cfg: &Config) -> LanguagesConfig {
    let code = |s: &str| crate::lang::find(s).map(|l| l.code.to_string());
    let mine = code(&cfg.translate.target)
        .or_else(|| code(&cfg.stt.language_reverse))
        .unwrap_or_else(|| "zh".into());
    let theirs = code(&cfg.translate.target_reverse)
        .or_else(|| code(&cfg.stt.language))
        .filter(|t| *t != mine)
        .unwrap_or_else(|| if mine == "en" { "zh".into() } else { "en".into() });
    LanguagesConfig { mine, theirs }
}

/// 音色是否适合该语言。Edge 音色按语种前缀判断（中文/粤语要区分 zh-CN 与 zh-HK）；
/// 非 Edge 音色（SAPI 名、OpenAI 的 alloy 等）无法判断，原样保留。
fn voice_fits(voice: &str, lang: &crate::lang::Lang) -> bool {
    if voice.trim().is_empty() {
        return false;
    }
    if !voice.contains("Neural") {
        return true;
    }
    let want = crate::lang::voice_locale(lang.edge_voice);
    let have = crate::lang::voice_locale(voice);
    if matches!(lang.code, "zh" | "yue") {
        have.eq_ignore_ascii_case(want)
    } else {
        have.split('-').next() == want.split('-').next()
    }
}

pub const TEMPLATE: &str = r#"# simulvoice 配置
version = 1

[languages]
# 两门语言决定一切：对方说 theirs → 识别 → 译成 mine 念给你听；
# 你说 mine → 识别 → 译成 theirs 送给对方（双向需 audio.duplex = true）。
# 可用：zh en ja ko yue fr de es ru pt it ar vi th id ms hi tr nl pl uk
mine = "zh"
theirs = "en"

[stt]
# local(本地, 按语言自动选 SenseVoice/Whisper, 推荐) | soniox(在线流式) | openai(在线) | vosk | none
provider = "local"
# language / language_reverse 由 [languages] 自动推出，无需手填
api_key_env = "SONIOX_API_KEY"
model = "gpt-4o-mini-transcribe"
model_path = ""                # 空则用 ~/simulvoice/models/vosk

[translate]
# local(本地, 中英用 Marian、其他语言用 NLLB, 推荐) | ollama | openai | gemini | qwen | custom | none
provider = "local"
# target / target_reverse 由 [languages] 自动推出
model = "qwen2.5:3b"
endpoint = "http://127.0.0.1:11434"   # ollama / custom 的地址
api_key_env = "OPENAI_API_KEY"
glossary = ""                  # 术语表，每行「原文=译文」
extra_targets = []             # 额外字幕语言，如 ["English","Japanese"]（不朗读）

[tts]
# edge(在线免费) | sapi(本地 Windows 语音) | openai(在线) | none
provider = "edge"
voice = "zh-CN-XiaoxiaoNeural"        # 入向音色（须是 mine 语言的音色，否则自动换默认）
voice_reverse = "en-US-AriaNeural"    # 出向音色（须是 theirs 语言的音色）
model = "gpt-4o-mini-tts"
api_key_env = "OPENAI_API_KEY"
rate = "+0%"
model_dir = ""                 # 本地 TTS 模型目录（空=数据目录/models/tts）
onnx_provider = "cpu"          # 本地 ONNX provider（DirectML 需自定义构建）
threads = 2                    # 本地 TTS 线程数
clone_voice = ""               # 出向用我的音色（simulvoice voice record <名字> 录制；空=关）
clone_steps = 0                # 我的音色采样步数（0=自动：中文 3、英文 2；越大越细腻越慢）

[audio]
input = "loopback"             # loopback | pid | mic
loopback_device = ""           # 回环采集设备名（空=默认输出）
pid = 0
output_device = ""             # 你听到的译文（建议耳机，且不同于采集设备）
mic_device = ""                # 你的麦克风（空=默认）
remote_output_device = ""      # 对方听到的译文，如 "CABLE"
duplex = false                 # true = 双向同传
mode = "both"                  # both | listen | speak
barge_in = false               # 自动打断（实验性，默认关闭）
cross_barge_in = false         # 跨方向打断：对方说话也停另一方向译文
mute_inbound = false           # 静音入向译文
mute_outbound = false          # 静音出向译文
backlog_jump_secs = 0          # 积压超过 N 秒跳至最新（0=不跳，字幕保留）
skip_own_language = true       # 对方那路有人说我的语言时只显示原文（两门语言文字不同才生效）
translation_only = false       # 只听译文：翻译时把会议软件的声音关到听不见（需按进程收音）
vad_rms = 250
silence_ms = 450
max_utterance_ms = 8000
queue_limit = 32               # 待翻译/TTS 队列上限（句）
voice_mode = "auto"            # auto | toggle(按一下开始/再按结束) | hold(按住) | hold_translate(按住翻译，空闲原声直通)
ptt_key = "F7"                 # 手动模式按键
ptt_ctrl = true                # 需要 Ctrl
ptt_alt = true                 # 需要 Alt
passthrough = false            # 原声直通到虚拟麦克风
passthrough_gain = 1.0         # 直通增益（0.1-4.0）
echo_guard = false             # 朗读译文时暂停出向识别（外放防回环）
monitor_translation = false    # 译文同时回放给自己
monitor_device = ""            # 监听设备（空=用入向输出设备）

[dsp]
denoise = true                 # RNNoise 降噪（48k 源）
vad = "silero"                 # silero | energy
agc = true                     # 麦克风自动增益
duck = true                    # 朗读时压低其他应用音量
aec = true                     # 回声消除（用输出设备回环作参考）
aec_filter_ms = 300            # AEC 自适应滤波器长度（毫秒）
onnx_provider = "cpu"          # 本地 ONNX provider（DirectML 需自定义构建）
vad_threads = 1                # Silero VAD 线程数
neural_aec = false             # 神经 AEC（DEC baseline ONNX，替代 Speex）
neural_aec_delay_ms = 0        # 回环参考相对麦克风的延迟（毫秒）
neural_aec_model = ""          # 模型路径（空=数据目录/models/aec/dec-baseline.onnx）

[ui]
subtitles = false              # 双语字幕悬浮窗
transcript_path = ""           # 双语转写导出文件（空=不导出）
record_dir = ""               # 录音目录（空=数据目录/recordings）
keep_translation = true        # 保存译文音频，F6 可回放
record_source = false          # 录制原始音频 WAV
record_max_mb = 2048          # 录音总大小上限（MB，0=不限）
minimize_to_tray = true        # 最小化时收到托盘
check_updates = true           # 启动时自动检查新版本（每天最多一次）

[realtime]
provider = "none"              # none | gemini-live（端到端语音翻译）
model = "models/gemini-3.8-live"
voice = "Puck"
voice_reverse = "Puck"
output_device = ""
voice_cloning = false          # Palabra 出向用我的实时音色

[profiles]
# 前台进程 → 配置文件（simulvoice watch 使用）
# "Teams.exe" = "profiles/teams.toml"
# "game.exe"  = "profiles/game.toml"
"#;



















/// 应用场景预设。
///
/// 预设只描述「用途」（双向同传 / 听译 / 记录），引擎则按这台机器上真的能跑什么来选：
/// 先看密钥，再看已下载的本地模型。否则预设会写出一份启动即失败、而且看不出原因的配置。
pub fn apply_preset(cfg: &mut Config, name: &str) -> anyhow::Result<String> {
    match name {
        "teams" | "game" | "record" => {}
        _ => anyhow::bail!("未知预设: {name}"),
    }
    let mut notes = Vec::new();
    pick_stt_engine(cfg, &mut notes);
    pick_translate_engine(cfg, &mut notes);
    pick_tts_engine(cfg, &mut notes);
    // 语言沿用用户已选的 [languages]，预设只改用途
    cfg.apply_languages();
    match name {
        "teams" => {
            cfg.audio.duplex = true;
            cfg.audio.remote_output_device = String::new();
            cfg.audio.output_device = String::new();
            cfg.ui.subtitles = true;
        }
        "game" => {
            cfg.audio.mode = "listen".into();
            cfg.ui.subtitles = true;
        }
        "record" => {
            cfg.audio.mode = "listen".into();
            cfg.tts.provider = "none".into();
            cfg.ui.subtitles = true;
            cfg.ui.transcript_path = "transcript.txt".into();
        }
        _ => unreachable!(),
    }
    Ok(notes.join("；"))
}

fn has_file_named(dir: &std::path::Path, prefix: &str, ext: &str) -> bool {
    std::fs::read_dir(dir)
        .map(|it| {
            it.filter_map(|e| e.ok()).any(|e| {
                let p = e.path();
                p.extension().and_then(|x| x.to_str()) == Some(ext)
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with(prefix))
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn pick_stt_engine(cfg: &mut Config, notes: &mut Vec<String>) {
    let models = default_models_dir();
    let vosk = models.join("vosk");
    if std::env::var("SONIOX_API_KEY").is_ok() {
        cfg.stt.provider = "soniox".into();
        cfg.stt.api_key_env = "SONIOX_API_KEY".into();
        cfg.stt.model_path = String::new();
        return;
    }
    // 本地首选 SenseVoice（中英日韩粤，带标点、准确率远高于 Zipformer），其余语言 Whisper
    if crate::stt::sensevoice_ready() || crate::stt::whisper_ready() {
        cfg.stt.provider = "local".into();
        cfg.stt.model_path = String::new();
        notes.push("识别=本地(SenseVoice/Whisper)".into());
        return;
    }
    if has_file_named(&models.join("zipformer"), "encoder", "onnx") {
        cfg.stt.provider = "zipformer".into();
        cfg.stt.model_path = models.join("zipformer").display().to_string();
        notes.push("识别=Zipformer(本地，建议改下载 SenseVoice)".into());
        return;
    }
    if vosk.is_dir() && has_file_named(&vosk, "", "mdl") {
        cfg.stt.provider = "vosk".into();
        cfg.stt.model_path = vosk.display().to_string();
        notes.push("识别=Vosk(本地)".into());
        return;
    }
    cfg.stt.provider = "soniox".into();
    cfg.stt.api_key_env = "SONIOX_API_KEY".into();
    notes.push("识别=Soniox（缺 SONIOX_API_KEY，也可先 models sensevoice 下载本地模型）".into());
}

fn pick_translate_engine(cfg: &mut Config, notes: &mut Vec<String>) {
    let dirs = crate::nmt::ModelDirs::new("");
    let local_ok = crate::nmt::locate(&dirs, cfg.theirs(), cfg.mine(), None).is_ok()
        && crate::nmt::locate(&dirs, cfg.mine(), cfg.theirs(), None).is_ok();
    // 本地优先：会议里每句都要翻，在线接口的 503/超时会直接变成"没声音"；
    // 想要大模型的译文质量，可在设置里手动切到 OpenAI/Gemini。
    if local_ok {
        cfg.translate.provider = "local".into();
        cfg.translate.model = String::new();
        notes.push("翻译=本地(Marian/NLLB)".into());
        return;
    }
    if std::env::var("OPENAI_API_KEY").is_ok() {
        cfg.translate.provider = "openai".into();
        cfg.translate.api_key_env = "OPENAI_API_KEY".into();
        cfg.translate.model = "gpt-4o-mini".into();
        notes.push("翻译=OpenAI".into());
        return;
    }
    if std::env::var("GEMINI_API_KEY").is_ok() {
        cfg.translate.provider = "gemini".into();
        cfg.translate.api_key_env = "GEMINI_API_KEY".into();
        cfg.translate.model = "gemini-flash-latest".into();
        notes.push("翻译=Gemini".into());
        return;
    }
    cfg.translate.provider = "local".into();
    notes.push("翻译=本地（先运行 simulvoice models nllb 下载模型）".into());
}

fn pick_tts_engine(cfg: &mut Config, _notes: &mut Vec<String>) {
    cfg.tts.provider = "edge".into();
    cfg.tts.api_key_env = "OPENAI_API_KEY".into();
}






















