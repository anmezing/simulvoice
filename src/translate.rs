use anyhow::{anyhow, Result};
use serde_json::json;

fn glossary_block(g: &str) -> String {
    if g.trim().is_empty() {
        String::new()
    } else {
        format!("\nTerminology (follow strictly):\n{}", g.trim())
    }
}

pub trait Translator: Send {
    fn translate(&mut self, text: &str) -> Result<String>;
}

pub struct OpenAiTranslator {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub target_lang: String,
    pub glossary: String,
}

impl Translator for OpenAiTranslator {
    fn translate(&mut self, text: &str) -> Result<String> {
        let body = json!({
            "model": self.model,
            "temperature": 0.2,
            "messages": [
                {"role": "system", "content": format!(
                    "You are a real-time interpreter. Translate the user's speech into {}. \
                     Output only the translation, no quotes, no explanations. \
                     If the text is already in the target language, return it unchanged.{}",
                    self.target_lang,
                    glossary_block(&self.glossary))},
                {"role": "user", "content": text}
            ]
        });
        let resp = crate::http::agent().post(&format!("{}/chat/completions", self.base_url))
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set("Content-Type", "application/json")
            .send_json(body);
        match resp {
            Ok(r) => {
                let v: serde_json::Value = r.into_json()?;
                Ok(v["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .to_string())
            }
            Err(ureq::Error::Status(code, r)) => Err(anyhow!(
                "translate HTTP {code}: {}",
                r.into_string().unwrap_or_default()
            )),
            Err(e) => Err(anyhow!("translate 请求失败: {e}")),
        }
    }
}

pub struct GeminiTranslator {
    pub api_key: String,
    pub model: String,
    pub target_lang: String,
    pub glossary: String,
}

impl Translator for GeminiTranslator {
    fn translate(&mut self, text: &str) -> Result<String> {
        let prompt = format!(
            "You are a real-time interpreter. Translate into {}. Output only the translation.{}\n{}",
            self.target_lang, glossary_block(&self.glossary), text
        );
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.model
        );
        let body = json!({"contents":[{"parts":[{"text": prompt}]}]});
        let resp = crate::http::agent().post(&url)
            .set("x-goog-api-key", &self.api_key)
            .set("Content-Type", "application/json")
            .send_json(body);
        match resp {
            Ok(r) => {
                let v: serde_json::Value = r.into_json()?;
                Ok(v["candidates"][0]["content"]["parts"][0]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .to_string())
            }
            Err(ureq::Error::Status(code, r)) => Err(anyhow!(
                "gemini HTTP {code}: {}",
                r.into_string().unwrap_or_default()
            )),
            Err(e) => Err(anyhow!("gemini 请求失败: {e}")),
        }
    }
}

/// Local translation through a local Ollama server (e.g. `ollama pull qwen2.5:3b`).
pub struct OllamaTranslator {
    pub endpoint: String,
    pub model: String,
    pub target_lang: String,
    pub glossary: String,
}

impl Translator for OllamaTranslator {
    fn translate(&mut self, text: &str) -> Result<String> {
        let body = json!({
            "model": self.model,
            "stream": false,
            "messages": [
                {"role": "system", "content": format!(
                    "You are a real-time interpreter. Translate into {}. \
                     Output only the translation, nothing else.{}",
                    self.target_lang, glossary_block(&self.glossary))},
                {"role": "user", "content": text}
            ]
        });
        let resp = crate::http::agent().post(&format!("{}/api/chat", self.endpoint.trim_end_matches('/')))
            .set("Content-Type", "application/json")
            .send_json(body);
        match resp {
            Ok(r) => {
                let v: serde_json::Value = r.into_json()?;
                Ok(v["message"]["content"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .to_string())
            }
            Err(ureq::Error::Status(code, r)) => Err(anyhow!(
                "ollama HTTP {code}: {}（是否已运行 ollama serve？）",
                r.into_string().unwrap_or_default()
            )),
            Err(e) => Err(anyhow!("ollama 连接失败: {e}（先运行 ollama serve）")),
        }
    }
}

pub struct PassThroughTranslator;

impl Translator for PassThroughTranslator {
    fn translate(&mut self, text: &str) -> Result<String> {
        Ok(text.to_string())
    }
}

pub fn build(cfg: &crate::config::TranslateConfig) -> Result<Box<dyn Translator>> {
    Ok(match cfg.provider.as_str() {
        "none" => Box::new(PassThroughTranslator),
        "gemini" => {
            let key = std::env::var(&cfg.api_key_env)
                .or_else(|_| std::env::var("GEMINI_API_KEY"))
                .map_err(|_| anyhow!("GEMINI_API_KEY 未设置"))?;
            Box::new(GeminiTranslator {
                api_key: key,
                model: if cfg.model.is_empty() {
                    "gemini-2.0-flash".into()
                } else {
                    cfg.model.clone()
                },
                target_lang: cfg.target.clone(),
                glossary: cfg.glossary.clone(),
            })
        }
        "qwen" => {
            let key = std::env::var(&cfg.api_key_env)
                .or_else(|_| std::env::var("DASHSCOPE_API_KEY"))
                .map_err(|_| anyhow!("DASHSCOPE_API_KEY 未设置"))?;
            Box::new(OpenAiTranslator {
                api_key: key,
                model: if cfg.model.is_empty() {
                    "qwen-plus".into()
                } else {
                    cfg.model.clone()
                },
                base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
                target_lang: cfg.target.clone(),
                glossary: cfg.glossary.clone(),
            })
        }
        "custom" => {
            let key = std::env::var(&cfg.api_key_env).unwrap_or_default();
            Box::new(OpenAiTranslator {
                api_key: key,
                model: cfg.model.clone(),
                base_url: cfg.endpoint.trim_end_matches("/").to_string(),
                target_lang: cfg.target.clone(),
                glossary: cfg.glossary.clone(),
            })
        }
        "ollama" => Box::new(OllamaTranslator {
            endpoint: cfg.endpoint.clone(),
            model: cfg.model.clone(),
            target_lang: cfg.target.clone(),
            glossary: cfg.glossary.clone(),
        }),
        _ => {
            let key = std::env::var(&cfg.api_key_env)
                .map_err(|_| anyhow!("环境变量 {} 未设置", cfg.api_key_env))?;
            Box::new(OpenAiTranslator {
                api_key: key,
                model: if cfg.model.is_empty() {
                    "gpt-4o-mini".into()
                } else {
                    cfg.model.clone()
                },
                base_url: std::env::var("OPENAI_BASE_URL")
                    .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
                target_lang: cfg.target.clone(),
                glossary: cfg.glossary.clone(),
            })
        }
    })
}









