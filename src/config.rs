use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
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
pub struct SttConfig {
    /// soniox | openai | vosk | none
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
    /// openai | gemini | ollama | qwen | custom | none
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
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct AudioConfig {
    /// loopback | pid | mic
    pub input: String,
    /// 回环采集的渲染设备名（空=默认输出设备）
    pub loopback_device: String,
    pub pid: u32,
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
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
            provider: "soniox".into(),
            language: "auto".into(),
            language_reverse: "auto".into(),
            api_key_env: "SONIOX_API_KEY".into(),
            model: "gpt-4o-mini-transcribe".into(),
            model_path: default_models_dir().join("vosk").display().to_string(),
        }
    }
}

impl Default for TranslateConfig {
    fn default() -> Self {
        Self {
            provider: "ollama".into(),
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
        }
    }
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input: "loopback".into(),
            loopback_device: String::new(),
            pid: 0,
            output_device: String::new(),
            mic_device: String::new(),
            remote_output_device: String::new(),
            duplex: false,
            barge_in: false,
            cross_barge_in: false,
            mute_inbound: false,
            mute_outbound: false,
            backlog_jump_secs: 0,
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
}

impl Default for RealtimeConfig {
    fn default() -> Self {
        Self {
            provider: "none".into(),
            model: "models/gemini-2.0-flash-live-001".into(),
            voice: "Puck".into(),
            voice_reverse: "Puck".into(),
            output_device: String::new(),
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
        let mut cfg: Config = toml::from_str(&text)?;
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

    pub fn write_template(path: &str) -> Result<()> {
        std::fs::write(path, TEMPLATE)?;
        Ok(())
    }

    pub fn save(&self, path: &str) -> Result<()> {
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

pub const TEMPLATE: &str = r#"# simulvoice 配置
version = 1

[stt]
# soniox(在线流式) | openai(在线) | vosk(本地, 需先 models vosk) | none
provider = "soniox"
language = "auto"              # 入向(对方→你)识别语言
language_reverse = "auto"      # 出向(你→对方)识别语言
api_key_env = "SONIOX_API_KEY"
model = "gpt-4o-mini-transcribe"
model_path = ""                # 空则用 ~/simulvoice/models/vosk

[translate]
# ollama(本地) | openai | gemini | qwen | custom | none
provider = "ollama"
target = "Chinese"             # 入向(对方→你)
target_reverse = "English"     # 出向(你→对方)
model = "qwen2.5:3b"
endpoint = "http://127.0.0.1:11434"   # ollama / custom 的地址
api_key_env = "OPENAI_API_KEY"
glossary = ""                  # 术语表，每行「原文=译文」
extra_targets = []             # 额外字幕语言，如 ["English","Japanese"]（不朗读）

[tts]
# edge(在线免费) | sapi(本地 Windows 语音) | openai(在线) | none
provider = "edge"
voice = "zh-CN-XiaoxiaoNeural"        # 入向音色
voice_reverse = "en-US-AriaNeural"    # 出向音色
model = "gpt-4o-mini-tts"
api_key_env = "OPENAI_API_KEY"
rate = "+0%"
model_dir = ""                 # 本地 TTS 模型目录（空=数据目录/models/tts）
onnx_provider = "cpu"          # 本地 ONNX provider（DirectML 需自定义构建）
threads = 2                    # 本地 TTS 线程数

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

[realtime]
provider = "none"              # none | gemini-live（端到端语音翻译）
model = "models/gemini-3.8-live"
voice = "Puck"
voice_reverse = "Puck"
output_device = ""

[profiles]
# 前台进程 → 配置文件（simulvoice watch 使用）
# "Teams.exe" = "profiles/teams.toml"
# "game.exe"  = "profiles/game.toml"
"#;



















/// Applies a named scenario preset to `cfg`.
pub fn apply_preset(cfg: &mut Config, name: &str) -> anyhow::Result<()> {
    cfg.stt.api_key_env = "SONIOX_API_KEY".into();
    cfg.translate.provider = "openai".into();
    cfg.translate.api_key_env = "OPENAI_API_KEY".into();
    cfg.tts.provider = "edge".into();
    match name {
        "teams" => {
            cfg.audio.duplex = true;
            cfg.audio.remote_output_device = String::new();
            cfg.audio.output_device = String::new();
            cfg.stt.language = "en".into();
            cfg.stt.language_reverse = "zh".into();
            cfg.translate.target = "Chinese".into();
            cfg.translate.target_reverse = "English".into();
            cfg.tts.voice = "zh-CN-XiaoxiaoNeural".into();
            cfg.tts.voice_reverse = "en-US-AriaNeural".into();
            cfg.ui.subtitles = true;
        }
        "game" => {
            cfg.audio.mode = "listen".into();
            cfg.stt.language = "en".into();
            cfg.translate.target = "Chinese".into();
            cfg.tts.voice = "zh-CN-XiaoxiaoNeural".into();
            cfg.ui.subtitles = true;
        }
        "record" => {
            cfg.audio.mode = "listen".into();
            cfg.stt.language = "auto".into();
            cfg.translate.target = "Chinese".into();
            cfg.tts.provider = "none".into();
            cfg.ui.subtitles = true;
            cfg.ui.transcript_path = "transcript.txt".into();
        }
        _ => anyhow::bail!("未知预设: {name}"),
    }
    Ok(())
}






















