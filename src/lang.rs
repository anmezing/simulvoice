//! 语言表：每种会议语言在各引擎里的代号，全项目只此一份。
//!
//! 配置里只存两门语言（`[languages] mine / theirs`），识别语言、翻译方向、
//! 朗读音色、本地模型目录都从这里推出来，避免六个字段各改各的、互相对不上。

pub struct Lang {
    /// 统一代码（配置文件里写的就是它）
    pub code: &'static str,
    /// 界面显示名
    pub label: &'static str,
    /// 英文名，给在线大模型翻译的提示词用
    pub name: &'static str,
    /// NLLB-200 语言代号
    pub nllb: &'static str,
    /// SenseVoice 是否支持（zh/en/ja/ko/yue，准确率最高、最快）
    pub sensevoice: bool,
    /// Whisper 语言代号
    pub whisper: &'static str,
    /// Edge 在线朗读默认音色
    pub edge_voice: &'static str,
    /// 本地 Piper 音色包（sherpa-onnx tts-models 里的名字，空 = 没有本地音色）
    pub piper: &'static str,
    /// HuggingFace 上有 Xenova/opus-mt-<code>-en（本语言 → 英语的 Marian 模型）
    pub marian_to_en: bool,
    /// HuggingFace 上有 Xenova/opus-mt-en-<code>（英语 → 本语言）
    pub marian_from_en: bool,
}

pub const LANGS: &[Lang] = &[
    Lang { code: "zh", label: "中文", name: "Chinese", nllb: "zho_Hans", sensevoice: true, whisper: "zh", edge_voice: "zh-CN-XiaoxiaoNeural", piper: "zh_CN-huayan-medium", marian_to_en: true, marian_from_en: true },
    Lang { code: "en", label: "英语", name: "English", nllb: "eng_Latn", sensevoice: true, whisper: "en", edge_voice: "en-US-AriaNeural", piper: "en_US-lessac-medium", marian_to_en: false, marian_from_en: false },
    Lang { code: "ja", label: "日语", name: "Japanese", nllb: "jpn_Jpan", sensevoice: true, whisper: "ja", edge_voice: "ja-JP-NanamiNeural", piper: "", marian_to_en: true, marian_from_en: false },
    Lang { code: "ko", label: "韩语", name: "Korean", nllb: "kor_Hang", sensevoice: true, whisper: "ko", edge_voice: "ko-KR-SunHiNeural", piper: "", marian_to_en: true, marian_from_en: false },
    Lang { code: "yue", label: "粤语", name: "Cantonese", nllb: "yue_Hant", sensevoice: true, whisper: "zh", edge_voice: "zh-HK-HiuMaanNeural", piper: "", marian_to_en: false, marian_from_en: false },
    Lang { code: "fr", label: "法语", name: "French", nllb: "fra_Latn", sensevoice: false, whisper: "fr", edge_voice: "fr-FR-DeniseNeural", piper: "fr_FR-siwis-medium", marian_to_en: true, marian_from_en: true },
    Lang { code: "de", label: "德语", name: "German", nllb: "deu_Latn", sensevoice: false, whisper: "de", edge_voice: "de-DE-KatjaNeural", piper: "de_DE-thorsten-medium", marian_to_en: true, marian_from_en: true },
    Lang { code: "es", label: "西班牙语", name: "Spanish", nllb: "spa_Latn", sensevoice: false, whisper: "es", edge_voice: "es-ES-ElviraNeural", piper: "es_ES-davefx-medium", marian_to_en: true, marian_from_en: true },
    Lang { code: "ru", label: "俄语", name: "Russian", nllb: "rus_Cyrl", sensevoice: false, whisper: "ru", edge_voice: "ru-RU-SvetlanaNeural", piper: "ru_RU-irina-medium", marian_to_en: true, marian_from_en: true },
    Lang { code: "pt", label: "葡萄牙语", name: "Portuguese", nllb: "por_Latn", sensevoice: false, whisper: "pt", edge_voice: "pt-BR-FranciscaNeural", piper: "pt_BR-faber-medium", marian_to_en: false, marian_from_en: false },
    Lang { code: "it", label: "意大利语", name: "Italian", nllb: "ita_Latn", sensevoice: false, whisper: "it", edge_voice: "it-IT-ElsaNeural", piper: "it_IT-riccardo-x_low", marian_to_en: true, marian_from_en: true },
    Lang { code: "ar", label: "阿拉伯语", name: "Arabic", nllb: "arb_Arab", sensevoice: false, whisper: "ar", edge_voice: "ar-SA-ZariyahNeural", piper: "ar_JO-kareem-medium", marian_to_en: true, marian_from_en: true },
    Lang { code: "vi", label: "越南语", name: "Vietnamese", nllb: "vie_Latn", sensevoice: false, whisper: "vi", edge_voice: "vi-VN-HoaiMyNeural", piper: "vi_VN-vais1000-medium", marian_to_en: true, marian_from_en: true },
    Lang { code: "th", label: "泰语", name: "Thai", nllb: "tha_Thai", sensevoice: false, whisper: "th", edge_voice: "th-TH-PremwadeeNeural", piper: "", marian_to_en: true, marian_from_en: false },
    Lang { code: "id", label: "印尼语", name: "Indonesian", nllb: "ind_Latn", sensevoice: false, whisper: "id", edge_voice: "id-ID-GadisNeural", piper: "", marian_to_en: true, marian_from_en: true },
    Lang { code: "ms", label: "马来语", name: "Malay", nllb: "zsm_Latn", sensevoice: false, whisper: "ms", edge_voice: "ms-MY-YasminNeural", piper: "", marian_to_en: false, marian_from_en: false },
    Lang { code: "hi", label: "印地语", name: "Hindi", nllb: "hin_Deva", sensevoice: false, whisper: "hi", edge_voice: "hi-IN-SwaraNeural", piper: "", marian_to_en: true, marian_from_en: true },
    Lang { code: "tr", label: "土耳其语", name: "Turkish", nllb: "tur_Latn", sensevoice: false, whisper: "tr", edge_voice: "tr-TR-EmelNeural", piper: "tr_TR-dfki-medium", marian_to_en: true, marian_from_en: false },
    Lang { code: "nl", label: "荷兰语", name: "Dutch", nllb: "nld_Latn", sensevoice: false, whisper: "nl", edge_voice: "nl-NL-ColetteNeural", piper: "", marian_to_en: true, marian_from_en: true },
    Lang { code: "pl", label: "波兰语", name: "Polish", nllb: "pol_Latn", sensevoice: false, whisper: "pl", edge_voice: "pl-PL-ZofiaNeural", piper: "pl_PL-darkman-medium", marian_to_en: true, marian_from_en: false },
    Lang { code: "uk", label: "乌克兰语", name: "Ukrainian", nllb: "ukr_Cyrl", sensevoice: false, whisper: "uk", edge_voice: "uk-UA-PolinaNeural", piper: "", marian_to_en: true, marian_from_en: true },
];

/// 按代码 / 英文名 / 中文名查语言，大小写不敏感。兼容旧配置里的 "Chinese"、"cn"、"中文"。
pub fn find(s: &str) -> Option<&'static Lang> {
    let t = s.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("auto") {
        return None;
    }
    let t = match t.to_ascii_lowercase().as_str() {
        "cn" | "zh-cn" | "zh_cn" | "mandarin" => "zh".to_string(),
        "zh-hk" | "zh_hk" => "yue".to_string(),
        other => other.to_string(),
    };
    LANGS.iter().find(|l| {
        l.code == t
            || l.name.eq_ignore_ascii_case(&t)
            || l.label == t
            || l.label.trim_end_matches('语') == t
    })
}

/// 查不到时报错，提示可用代码。
pub fn get(s: &str) -> anyhow::Result<&'static Lang> {
    find(s).ok_or_else(|| {
        let codes: Vec<_> = LANGS.iter().map(|l| l.code).collect();
        anyhow::anyhow!("不支持的语言: {s}（可用: {}）", codes.join(" "))
    })
}

/// 音色的语种前缀（"zh-CN-XiaoxiaoNeural" → "zh-CN"）
pub fn voice_locale(voice: &str) -> &str {
    let mut parts = voice.splitn(3, '-');
    match (parts.next(), parts.next()) {
        (Some(a), Some(b)) => &voice[..a.len() + 1 + b.len()],
        _ => voice,
    }
}

/// 该语言常用的 Edge 音色（设置面板的下拉用）
pub fn edge_voices(code: &str) -> Vec<&'static str> {
    let extra: &[&str] = match code {
        "zh" => &["zh-CN-XiaoxiaoNeural", "zh-CN-YunxiNeural", "zh-CN-XiaoyiNeural", "zh-CN-YunjianNeural"],
        "en" => &["en-US-AriaNeural", "en-US-GuyNeural", "en-US-JennyNeural", "en-GB-SoniaNeural"],
        "ja" => &["ja-JP-NanamiNeural", "ja-JP-KeitaNeural"],
        "ko" => &["ko-KR-SunHiNeural", "ko-KR-InJoonNeural"],
        "fr" => &["fr-FR-DeniseNeural", "fr-FR-HenriNeural"],
        "de" => &["de-DE-KatjaNeural", "de-DE-ConradNeural"],
        "es" => &["es-ES-ElviraNeural", "es-ES-AlvaroNeural", "es-MX-DaliaNeural"],
        _ => &[],
    };
    let mut out: Vec<&'static str> = extra.to_vec();
    if let Some(l) = find(code) {
        if !out.contains(&l.edge_voice) {
            out.insert(0, l.edge_voice);
        }
    }
    out
}

// ───────────────────────────── 文字体系：判断一句话是不是"我的语言" ─────────────────────────────

/// 书写体系。两门会议语言体系不同（中/英、日/英、中/俄……）时，看识别结果用的是哪种文字
/// 就能可靠地分出是谁的语言；体系相同（英/法、中/粤）时分不出，功能自动关闭。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Script {
    Han,
    Japanese,
    Hangul,
    Latin,
    Cyrillic,
    Arabic,
    Thai,
    Devanagari,
}

pub fn script(l: &Lang) -> Script {
    match l.code {
        "zh" | "yue" => Script::Han,
        "ja" => Script::Japanese,
        "ko" => Script::Hangul,
        "ru" | "uk" => Script::Cyrillic,
        "ar" => Script::Arabic,
        "th" => Script::Thai,
        "hi" => Script::Devanagari,
        _ => Script::Latin,
    }
}

/// 两门语言能否靠文字分开
pub fn distinguishable(a: &Lang, b: &Lang) -> bool {
    script(a) != script(b)
}

#[derive(Clone, Copy, PartialEq)]
enum Ch {
    Han,
    Kana,
    Hangul,
    Latin,
    Cyrillic,
    Arabic,
    Thai,
    Devanagari,
}

fn char_class(c: char) -> Option<Ch> {
    let u = c as u32;
    Some(match u {
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => Ch::Kana,
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF | 0x20000..=0x2FFFF => Ch::Han,
        0xAC00..=0xD7AF | 0x1100..=0x11FF | 0x3130..=0x318F => Ch::Hangul,
        0x0400..=0x052F => Ch::Cyrillic,
        0x0600..=0x06FF | 0x0750..=0x077F | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => Ch::Arabic,
        0x0E00..=0x0E7F => Ch::Thai,
        0x0900..=0x097F => Ch::Devanagari,
        _ if c.is_alphabetic() && u < 0x0250 => Ch::Latin,
        0x1E00..=0x1EFF => Ch::Latin, // 越南语带声调字母
        _ => return None,
    })
}

/// 一句话主要用的是哪种文字。汉字、假名按字计（一个字约等于一个词），
/// 拼音文字按词计——这样"这个 API 的 latency 太高了"仍算中文。
/// 混得太杂（主导文字不到 70%）或字太少时返回 None：拿不准就当没判出来。
pub fn text_script(text: &str) -> Option<Script> {
    let (mut han, mut kana) = (0usize, 0usize);
    let mut words = [0usize; 6]; // Hangul Latin Cyrillic Arabic Thai Devanagari
    let mut prev: Option<Ch> = None;
    for c in text.chars() {
        let cls = char_class(c);
        match cls {
            Some(Ch::Han) => han += 1,
            Some(Ch::Kana) => kana += 1,
            Some(k) if prev != Some(k) => {
                let i = match k {
                    Ch::Hangul => 0,
                    Ch::Latin => 1,
                    Ch::Cyrillic => 2,
                    Ch::Arabic => 3,
                    Ch::Thai => 4,
                    _ => 5,
                };
                words[i] += 1;
            }
            _ => {}
        }
        // 撇号、连字符不打断单词（don't、e-mail）
        if cls.is_some() || !(c == '\'' || c == '’' || c == '-') {
            prev = cls;
        }
    }
    // 日语句子里假名通常占三成以上；中文里偶尔混个"の"不算
    let japanese = kana > 0 && kana * 10 >= han + kana;
    let cjk = han + kana;
    let total = cjk + words.iter().sum::<usize>();
    if total < 2 {
        return None;
    }
    let mut best = (
        if japanese { Script::Japanese } else { Script::Han },
        if japanese { cjk } else { han },
    );
    for (i, &n) in words.iter().enumerate() {
        if n > best.1 {
            best = (
                [
                    Script::Hangul,
                    Script::Latin,
                    Script::Cyrillic,
                    Script::Arabic,
                    Script::Thai,
                    Script::Devanagari,
                ][i],
                n,
            );
        }
    }
    (best.1 * 10 >= total * 7).then_some(best.0)
}

/// 对方那一路识别出的这句话是不是"我的语言"（例如中英会议里中文同事插话）。
/// 只在两门语言文字不同、且这句话明确是我的文字时返回 true；其余一律照常翻译。
pub fn is_own_language(text: &str, mine: &Lang, theirs: &Lang) -> bool {
    distinguishable(mine, theirs) && text_script(text) == Some(script(mine))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_of_text() {
        assert_eq!(text_script("我们下周再讨论这个问题。"), Some(Script::Han));
        assert_eq!(text_script("这个 API 的 latency 太高了"), Some(Script::Han));
        assert_eq!(text_script("Let's move on to the next item."), Some(Script::Latin));
        assert_eq!(text_script("I don't think e-mail works."), Some(Script::Latin));
        assert_eq!(text_script("今日はいい天気ですね"), Some(Script::Japanese));
        assert_eq!(text_script("東京の会議"), Some(Script::Japanese));
        assert_eq!(text_script("감사합니다 여러분"), Some(Script::Hangul));
        assert_eq!(text_script("Добрый день, коллеги"), Some(Script::Cyrillic));
        assert_eq!(text_script("Chúng ta bắt đầu nhé"), Some(Script::Latin));
        // 太短或混得太杂：判不出
        assert_eq!(text_script("嗯"), None);
        assert_eq!(text_script("OK"), None);
        assert_eq!(text_script("Let's ask 王总"), None);
        assert_eq!(text_script("123, 456."), None);
    }

    #[test]
    fn own_language_only_when_scripts_differ() {
        let (zh, en, fr, yue, ja) = (
            find("zh").unwrap(),
            find("en").unwrap(),
            find("fr").unwrap(),
            find("yue").unwrap(),
            find("ja").unwrap(),
        );
        assert!(is_own_language("我觉得可以", zh, en));
        assert!(!is_own_language("I think so, yes", zh, en));
        assert!(!is_own_language("Je pense que oui", en, fr));
        assert!(!is_own_language("我觉得可以", zh, yue));
        assert!(is_own_language("我觉得可以", zh, ja));
        assert!(!is_own_language("そう思います", zh, ja));
        assert!(is_own_language("Yes, I agree with that", en, zh));
    }

    #[test]
    fn find_accepts_legacy_names() {
        assert_eq!(find("Chinese").unwrap().code, "zh");
        assert_eq!(find("cn").unwrap().code, "zh");
        assert_eq!(find("中文").unwrap().code, "zh");
        assert_eq!(find("日语").unwrap().code, "ja");
        assert_eq!(find("日").unwrap().code, "ja");
        assert_eq!(find("English").unwrap().code, "en");
        assert_eq!(find("Cantonese").unwrap().code, "yue");
        assert!(find("auto").is_none());
        assert!(find("klingon").is_none());
    }

    #[test]
    fn voice_locale_prefix() {
        assert_eq!(voice_locale("zh-CN-XiaoxiaoNeural"), "zh-CN");
        assert_eq!(voice_locale("Puck"), "Puck");
    }

    #[test]
    fn codes_unique() {
        for (i, a) in LANGS.iter().enumerate() {
            for b in &LANGS[i + 1..] {
                assert_ne!(a.code, b.code);
            }
        }
    }
}
