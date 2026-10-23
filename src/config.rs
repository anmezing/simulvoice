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
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct UiConfig {
    /// 显示双语字幕悬浮窗
    pub subtitles: bool,
    /// 双语转写导出路径（空=不导出）
    pub transcript_path: String,
    /// 录音目录（原声 WAV + 译文 MP3；空=不录音）
    pub record_dir: String,
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
            mode: "both".into(),
            vad_rms: 250.0,
            silence_ms: 450,
            max_utterance_ms: 8000,
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
vad_rms = 250
silence_ms = 450
max_utterance_ms = 8000

[dsp]
denoise = true                 # RNNoise 降噪（48k 源）
vad = "silero"                 # silero | energy
agc = true                     # 麦克风自动增益
duck = true                    # 朗读时压低其他应用音量

[ui]
subtitles = false              # 双语字幕悬浮窗
transcript_path = ""           # 双语转写导出文件（空=不导出）
record_dir = ""               # 录音目录（原声 WAV + 译文 MP3）
record_max_mb = 2048          # 录音总大小上限（MB，0=不限）

[realtime]
provider = "none"              # none | gemini-live（端到端语音翻译）
model = "models/gemini-3.8-live"
voice = "Puck"
voice_reverse = "Puck"
output_device = ""
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
            cfg.audio.remote_output_device = "CABLE".into();
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







