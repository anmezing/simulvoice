//! 本地神经机器翻译（ONNX encoder-decoder）：Marian（opus-mt，一对语言一个模型）
//! 与 NLLB-200（一个模型覆盖 200 种语言、任意方向）共用同一套推理代码。
//!
//! 两种解码方式：
//! - Marian：不带缓存的 int8 decoder，整批重算；默认贪心（束宽可用 SIMULVOICE_NMT_BEAMS
//!   调）。opus-mt 按单句训练，多句输入会整句丢失，所以上层先切句再逐句翻译
//!   （见 NmtTranslator::translate）。切句后贪心与 4 束质量相当，速度快 2~3 倍。
//! - NLLB：词表 25 万，每步输出层很重，只做贪心 + KV 缓存。

use anyhow::{anyhow, Result};
use ort::session::{Session, SessionInputValue};
use ort::value::{DynValue, Tensor};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tokenizers::Tokenizer;

use crate::lang::Lang;

/// 生成长度硬上限（token）
const MAX_NEW: usize = 256;
/// Marian 默认束宽（1 = 贪心）。实测切句后贪心与 4 束译文几乎一致，延迟低一半以上。
const BEAMS: usize = 1;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Family {
    Marian,
    Nllb,
}

pub struct Seq2Seq {
    family: Family,
    encoder: Session,
    decoder: Session,
    /// decoder 是否是带 KV 缓存的 merged 版本
    cached: bool,
    tokenizer: Tokenizer,
    d_model: usize,
    heads: usize,
    eos: i64,
    decoder_start: i64,
    /// 永不生成的 token（Marian 的 pad）
    banned: Vec<i64>,
    /// merged decoder 的 past_key_values.* 输入名（按模型声明顺序）
    past_names: Vec<String>,
}

// ort::Session 内部是裸指针；同一时刻只在一个线程里用（外层有 Mutex）。
unsafe impl Send for Seq2Seq {}

fn cfg_usize(v: &serde_json::Value, key: &str) -> Option<usize> {
    v.get(key).and_then(|x| x.as_u64()).map(|x| x as usize)
}

fn first_existing(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    names.iter().map(|n| dir.join(n)).find(|p| p.exists())
}

fn debug() -> bool {
    std::env::var("SIMULVOICE_NMT_DEBUG").is_ok()
}

impl Seq2Seq {
    pub fn load(dir: &Path, family: Family, threads: usize) -> Result<Self> {
        let config: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("config.json"))
                .map_err(|e| anyhow!("读取 {} 失败: {e}", dir.join("config.json").display()))?,
        )?;
        let encoder_path = first_existing(
            dir,
            &["encoder_model_int8.onnx", "encoder_model_quantized.onnx", "encoder_model.onnx"],
        )
        .ok_or_else(|| anyhow!("翻译模型缺 encoder: {}", dir.display()))?;
        let merged = first_existing(
            dir,
            &[
                "decoder_model_merged_quantized.onnx",
                "decoder_model_merged_int8.onnx",
                "decoder_model_merged.onnx",
            ],
        );
        // Marian 首选不带缓存的 decoder_model_int8：它的输出层保留 fp32，实测比全量化的
        // merged 版每 token 快 2~3 倍（量化矩阵乘在这类 CPU 上很慢），且束搜索整批跑更简单。
        let plain = first_existing(
            dir,
            &["decoder_model_int8.onnx", "decoder_model.onnx", "decoder_model_quantized.onnx"],
        );
        let (decoder_path, cached) = match (family, plain, merged) {
            (Family::Marian, Some(p), _) => (p, false),
            (_, _, Some(m)) => (m, true),
            (_, Some(p), None) => (p, false),
            (_, None, None) => return Err(anyhow!("翻译模型缺 decoder: {}", dir.display())),
        };

        let env = ort_env()?;
        let threads = threads.clamp(1, 16);
        let build = |path: &Path| -> Result<Session> {
            Session::builder(env)?
                .with_optimization_level(ort::session::builder::GraphOptimizationLevel::All)
                .map_err(|e| anyhow!("ORT 优化级别设置失败: {e}"))?
                .with_intra_threads(threads)
                .map_err(|e| anyhow!("ORT 线程设置失败: {e}"))?
                .commit_from_file(path)
                .map_err(|e| anyhow!("加载 {} 失败: {e}", path.display()))
        };
        let encoder = build(&encoder_path)?;
        let decoder = build(&decoder_path)?;

        let past_names: Vec<String> = decoder
            .inputs()
            .iter()
            .map(|i| i.name().to_string())
            .filter(|n| n.starts_with("past_key_values."))
            .collect();
        let cached = cached && !past_names.is_empty();
        if debug() {
            eprintln!("[nmt] {} decoder={} cached={cached}", dir.display(), decoder_path.display());
        }

        let tokenizer = load_tokenizer(dir)?;
        let d_model = cfg_usize(&config, "d_model").unwrap_or(512);
        let heads = cfg_usize(&config, "decoder_attention_heads").unwrap_or(8);
        let eos = cfg_usize(&config, "eos_token_id").unwrap_or(0) as i64;
        let decoder_start = cfg_usize(&config, "decoder_start_token_id").unwrap_or(eos as usize) as i64;
        let mut banned = Vec::new();
        if family == Family::Marian {
            // HF 的 opus-mt 配置 bad_words_ids=[[pad]]：pad 一旦被选中就会无限重复
            if let Some(pad) = cfg_usize(&config, "pad_token_id") {
                banned.push(pad as i64);
            }
        }
        Ok(Self {
            family,
            encoder,
            decoder,
            cached,
            tokenizer,
            d_model,
            heads,
            eos,
            decoder_start,
            banned,
            past_names,
        })
    }

    fn token_id(&self, token: &str) -> Result<i64> {
        self.tokenizer
            .token_to_id(token)
            .map(|i| i as i64)
            .ok_or_else(|| anyhow!("翻译模型不认识语言标记 {token}"))
    }

    /// 翻译一句。`src`/`tgt` 只对 NLLB 有意义；Marian 的方向由模型本身决定。
    pub fn translate(&mut self, text: &str, src: &Lang, tgt: &Lang) -> Result<String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(String::new());
        }
        let (input_ids, prefix) = match self.family {
            Family::Marian => {
                let enc = self
                    .tokenizer
                    .encode(text, true)
                    .map_err(|e| anyhow!("tokenize 失败: {e}"))?;
                let ids: Vec<i64> = enc.get_ids().iter().map(|&i| i as i64).collect();
                (ids, vec![self.decoder_start])
            }
            Family::Nllb => {
                let enc = self
                    .tokenizer
                    .encode(text, false)
                    .map_err(|e| anyhow!("tokenize 失败: {e}"))?;
                let mut ids = vec![self.token_id(src.nllb)?];
                ids.extend(enc.get_ids().iter().map(|&i| i as i64));
                ids.push(self.eos);
                // NLLB：decoder 以 </s> 起步，第二个 token 强制为目标语言标记
                (ids, vec![self.decoder_start, self.token_id(tgt.nllb)?])
            }
        };
        if input_ids.is_empty() {
            return Ok(String::new());
        }
        let n = input_ids.len();
        let mask = vec![1i64; n];

        let enc_out = self.encoder.run(ort::inputs![
            "input_ids" => Tensor::from_array(([1usize, n], input_ids.into_boxed_slice()))?,
            "attention_mask" => Tensor::from_array(([1usize, n], mask.into_boxed_slice()))?
        ])?;
        let (_, hidden) = enc_out["last_hidden_state"].try_extract_tensor::<f32>()?;
        let hidden = hidden.to_vec();
        drop(enc_out);
        if hidden.len() != n * self.d_model {
            return Err(anyhow!(
                "encoder 输出维度不符（{} != {}×{}），config.json 与模型不匹配",
                hidden.len(),
                n,
                self.d_model
            ));
        }

        let max_new = MAX_NEW.min(n * 3 + 16);
        let t_dec = std::time::Instant::now();
        let generated = if self.cached {
            self.decode_cached(&hidden, n, &prefix, max_new)?
        } else {
            let beams = if self.family == Family::Marian {
                std::env::var("SIMULVOICE_NMT_BEAMS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(BEAMS)
            } else {
                1
            };
            self.decode_beam(&hidden, n, &prefix, max_new, beams)?
        };
        if debug() {
            eprintln!(
                "[nmt] {} 个输入 token，生成 {} 个，解码 {} ms",
                n,
                generated.len(),
                t_dec.elapsed().as_millis()
            );
        }
        let out: Vec<u32> = generated
            .iter()
            .filter(|&&id| id != self.eos && !self.banned.contains(&id))
            .map(|&id| id as u32)
            .collect();
        let text = self
            .tokenizer
            .decode(&out, true)
            .map_err(|e| anyhow!("detokenize 失败: {e}"))?;
        Ok(text.trim().to_string())
    }

    /// 贪心 + KV 缓存（NLLB）
    fn decode_cached(&mut self, hidden: &[f32], n: usize, prefix: &[i64], max_new: usize) -> Result<Vec<i64>> {
        let head_dim = self.d_model / self.heads;
        let banned = self.banned.clone();
        let hidden_t = Tensor::from_array(([1usize, n, self.d_model], hidden.to_vec().into_boxed_slice()))?;
        let mask_t = Tensor::from_array(([1usize, n], vec![1i64; n].into_boxed_slice()))?;
        // 第一步没有缓存：各层 past 是长度 0 的占位张量，use_cache_branch=false
        let mut past: HashMap<String, DynValue> = HashMap::new();
        for name in &self.past_names {
            let empty = Tensor::from_array((
                [1usize, self.heads, 0, head_dim],
                Vec::<f32>::new().into_boxed_slice(),
            ))?;
            past.insert(name.clone(), empty.into_dyn());
        }
        let mut out: Vec<i64> = Vec::new();
        let mut step_ids: Vec<i64> = prefix.to_vec();
        let mut first = true;
        for _ in 0..max_new {
            let seq = step_ids.len();
            let mut inputs: Vec<(Cow<str>, SessionInputValue)> = Vec::with_capacity(self.past_names.len() + 4);
            inputs.push((
                "input_ids".into(),
                Tensor::from_array(([1usize, seq], step_ids.clone().into_boxed_slice()))?.into(),
            ));
            inputs.push(("encoder_attention_mask".into(), (&mask_t).into()));
            inputs.push(("encoder_hidden_states".into(), (&hidden_t).into()));
            inputs.push((
                "use_cache_branch".into(),
                Tensor::from_array(([1usize], vec![!first].into_boxed_slice()))?.into(),
            ));
            for name in &self.past_names {
                inputs.push((name.as_str().into(), (&past[name]).into()));
            }
            let mut outputs = self.decoder.run(inputs)?;
            let next = {
                let (shape, logits) = outputs["logits"].try_extract_tensor::<f32>()?;
                let vocab = *shape.last().unwrap_or(&0) as usize;
                if vocab == 0 || logits.len() < vocab {
                    return Err(anyhow!("decoder logits 形状异常"));
                }
                pick(&logits[logits.len() - vocab..], &banned)
            };
            for name in &self.past_names {
                let present = name.replacen("past_key_values", "present", 1);
                if let Some(v) = outputs.remove(&present) {
                    // 缓存分支里 encoder 的 present 只是占位张量（形如 [0,8,1,64]，某一维为 0），
                    // 必须保留第一步算好的那份，否则下一步 cross-attention 拿到空缓存
                    let empty = v
                        .dtype()
                        .tensor_shape()
                        .map(|s| s.iter().any(|&d| d == 0))
                        .unwrap_or(true);
                    if !empty {
                        past.insert(name.clone(), v);
                    }
                }
            }
            drop(outputs);
            first = false;
            if next == self.eos {
                break;
            }
            out.push(next);
            if let Some(keep) = loop_cut(&out) {
                out.truncate(keep);
                break;
            }
            step_ids = vec![next];
        }
        Ok(out)
    }

    /// 束搜索（beams=1 即贪心），不带缓存：所有活着的束拼成一批，每步整段重算。
    /// 评分按 HF 的做法：累计对数概率 / 生成长度（length_penalty=1）。
    fn decode_beam(
        &mut self,
        hidden: &[f32],
        n: usize,
        prefix: &[i64],
        max_new: usize,
        beams: usize,
    ) -> Result<Vec<i64>> {
        let beams = beams.max(1);
        let plen = prefix.len();
        // (token 序列, 累计对数概率)
        let mut alive: Vec<(Vec<i64>, f32)> = vec![(prefix.to_vec(), 0.0)];
        // (token 序列, 归一化分数)
        let mut done: Vec<(Vec<i64>, f32)> = Vec::new();
        for step in 0..max_new {
            let b = alive.len();
            let seq = alive[0].0.len();
            let mut ids = Vec::with_capacity(b * seq);
            for (t, _) in &alive {
                ids.extend_from_slice(t);
            }
            let mut hid = Vec::with_capacity(b * hidden.len());
            for _ in 0..b {
                hid.extend_from_slice(hidden);
            }
            let outputs = self.decoder.run(ort::inputs![
                "input_ids" => Tensor::from_array(([b, seq], ids.into_boxed_slice()))?,
                "encoder_attention_mask" => Tensor::from_array(([b, n], vec![1i64; b * n].into_boxed_slice()))?,
                "encoder_hidden_states" => Tensor::from_array(([b, n, self.d_model], hid.into_boxed_slice()))?
            ])?;
            let (shape, logits) = outputs["logits"].try_extract_tensor::<f32>()?;
            let vocab = *shape.last().unwrap_or(&0) as usize;
            if vocab == 0 || logits.len() < b * seq * vocab {
                return Err(anyhow!("decoder logits 形状异常"));
            }
            // 每束取前 2×beams 个候选（HF 同样做法：留出被 EOS 占掉的名额）
            let mut cands: Vec<(f32, usize, i64)> = Vec::with_capacity(b * beams * 2);
            for (i, (_, score)) in alive.iter().enumerate() {
                let row = &logits[(i * seq + seq - 1) * vocab..(i * seq + seq) * vocab];
                let max = row.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                let lse = max + row.iter().map(|&v| (v - max).exp()).sum::<f32>().ln();
                for (tok, v) in top_k(row, beams * 2, &self.banned) {
                    cands.push((score + v - lse, i, tok));
                }
            }
            drop(outputs);
            cands.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            let gen_len = (seq + 1 - plen) as f32;
            let mut next: Vec<(Vec<i64>, f32)> = Vec::with_capacity(beams);
            for (rank, (score, i, tok)) in cands.into_iter().enumerate() {
                if tok == self.eos {
                    // 只有排进前 beams 名的 EOS 才算完成（HF: beam_token_rank < num_beams）
                    if rank < beams {
                        done.push((alive[i].0.clone(), score / gen_len));
                    }
                } else {
                    let mut t = alive[i].0.clone();
                    t.push(tok);
                    next.push((t, score));
                }
                if next.len() == beams {
                    break;
                }
            }
            if next.is_empty() {
                break;
            }
            // 复读的束直接截断收尾
            next.retain(|(t, s)| match loop_cut(&t[plen..]) {
                Some(keep) => {
                    done.push((t[..plen + keep].to_vec(), *s / (keep.max(1) as f32)));
                    false
                }
                None => true,
            });
            alive = next;
            if alive.is_empty() {
                break;
            }
            // 已完成的束够数，且活着的最好那束（按当前长度归一）已不可能超过最差的完成束
            if done.len() >= beams {
                let worst = done
                    .iter()
                    .map(|d| d.1)
                    .fold(f32::INFINITY, f32::min);
                let best_alive = alive[0].1 / gen_len;
                if best_alive < worst {
                    break;
                }
            }
            let _ = step;
        }
        let best = done
            .into_iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|d| d.0)
            .or_else(|| alive.into_iter().next().map(|a| a.0))
            .unwrap_or_default();
        Ok(best.get(plen..).map(|s| s.to_vec()).unwrap_or_default())
    }
}

/// 取分数最高的 k 个 token（跳过禁用的），按分数降序。
fn top_k(row: &[f32], k: usize, banned: &[i64]) -> Vec<(i64, f32)> {
    let mut best: Vec<(i64, f32)> = Vec::with_capacity(k + 1);
    for (i, &v) in row.iter().enumerate() {
        if best.len() == k && v <= best[k - 1].1 {
            continue;
        }
        if banned.contains(&(i as i64)) {
            continue;
        }
        let pos = best.iter().position(|&(_, b)| v > b).unwrap_or(best.len());
        best.insert(pos, (i as i64, v));
        if best.len() > k {
            best.pop();
        }
    }
    best
}

/// 贪心取最大 logit，跳过禁用 token。
fn pick(logits: &[f32], banned: &[i64]) -> i64 {
    top_k(logits, 1, banned).first().map(|t| t.0).unwrap_or(0)
}

/// 贪心解码偶尔陷入复读（"的的的的"、同一短语反复）。检测到尾部同一片段连续
/// 重复时返回应保留的长度（保留一份）。单 token 要连续 4 次才算，避免误伤"哈哈哈"。
fn loop_cut(ids: &[i64]) -> Option<usize> {
    let len = ids.len();
    for k in 1..=8usize {
        let reps = if k <= 2 { 4 } else { 3 };
        if len < k * reps {
            continue;
        }
        let tail = &ids[len - k..];
        if (2..=reps).all(|r| &ids[len - r * k..len - (r - 1) * k] == tail) {
            return Some(len - (reps - 1) * k);
        }
    }
    None
}

fn load_tokenizer(dir: &Path) -> Result<Tokenizer> {
    let raw_path = dir.join("tokenizer.json");
    let patched = dir.join("tokenizer_fixed.json");
    if !patched.exists() {
        let raw = std::fs::read_to_string(&raw_path)
            .map_err(|e| anyhow!("读取 {} 失败: {e}", raw_path.display()))?;
        let mut v: serde_json::Value = serde_json::from_str(&raw)?;
        // 部分 Xenova 导出的 Marian tokenizer.json 带 precompiled_charsmap=null，tokenizers 解析不了
        if v.get("normalizer")
            .and_then(|n| n.get("precompiled_charsmap"))
            .map(|m| m.is_null())
            .unwrap_or(false)
        {
            v["normalizer"] = serde_json::Value::Null;
        }
        // 先写临时文件再改名：写到一半被中断不会留下坏缓存
        let tmp = dir.join("tokenizer_fixed.json.tmp");
        std::fs::write(&tmp, serde_json::to_string(&v)?)?;
        std::fs::rename(&tmp, &patched)?;
    }
    Tokenizer::from_file(&patched).map_err(|e| anyhow!("加载 tokenizer 失败: {e}"))
}

// ───────────────────────────── 模型选择与共享 ─────────────────────────────

/// 本地翻译模型的位置。`marian_override` 是旧配置 `translate.marian_dir`（含 en-zh/zh-en 子目录的目录）。
pub struct ModelDirs {
    pub marian: PathBuf,
    pub nllb: PathBuf,
}

impl ModelDirs {
    pub fn new(marian_override: &str) -> Self {
        let models = crate::config::default_models_dir();
        Self {
            marian: if marian_override.trim().is_empty() {
                models.join("marian")
            } else {
                PathBuf::from(marian_override.trim())
            },
            nllb: models.join("nllb"),
        }
    }

    pub fn marian_pair(&self, src: &Lang, tgt: &Lang) -> PathBuf {
        self.marian.join(format!("{}-{}", src.code, tgt.code))
    }
}

pub fn ready(dir: &Path) -> bool {
    dir.join("config.json").exists() && dir.join("tokenizer.json").exists()
}

/// 翻译路线中的一段
#[derive(Clone, Copy)]
pub struct Leg {
    pub src: &'static Lang,
    pub tgt: &'static Lang,
    pub family: Family,
}

impl Leg {
    pub fn dir(&self, dirs: &ModelDirs) -> PathBuf {
        match self.family {
            Family::Marian => dirs.marian_pair(self.src, self.tgt),
            Family::Nllb => dirs.nllb.clone(),
        }
    }

    /// HuggingFace 上 Xenova 转好的 ONNX 仓库名（仅 Marian）
    pub fn repo(&self) -> String {
        format!("Xenova/opus-mt-{}-{}", self.src.code, self.tgt.code)
    }
}

fn english() -> &'static Lang {
    crate::lang::find("en").expect("语言表里必有 en")
}

/// Marian 有没有 src→tgt 这一对（已下载，或 HuggingFace 上有现成的）
fn marian_has(dirs: &ModelDirs, src: &'static Lang, tgt: &'static Lang) -> bool {
    if ready(&dirs.marian_pair(src, tgt)) {
        return true;
    }
    match (src.code, tgt.code) {
        (_, "en") => src.marian_to_en,
        ("en", _) => tgt.marian_from_en,
        _ => false,
    }
}

/// 规划翻译路线：
/// 1. 有直达的 Marian（小而快，约 0.6 秒/句）就直达；
/// 2. 两边都能和英语互译，就经英语中转（两段 Marian，约 1.2 秒）；
/// 3. 否则用 NLLB 一步到位（覆盖全部语言，但慢、准确度一般）。
///
/// `only` 限定家族（旧配置 provider="marian" / "nllb"）。
pub fn plan(dirs: &ModelDirs, src: &'static Lang, tgt: &'static Lang, only: Option<Family>) -> Vec<Leg> {
    if src.code == tgt.code {
        return Vec::new();
    }
    let nllb = vec![Leg { src, tgt, family: Family::Nllb }];
    if only == Some(Family::Nllb) {
        return nllb;
    }
    if marian_has(dirs, src, tgt) {
        return vec![Leg { src, tgt, family: Family::Marian }];
    }
    let en = english();
    if src.code != "en" && tgt.code != "en" && marian_has(dirs, src, en) && marian_has(dirs, en, tgt) {
        return vec![
            Leg { src, tgt: en, family: Family::Marian },
            Leg { src: en, tgt, family: Family::Marian },
        ];
    }
    if only == Some(Family::Marian) {
        // 旧配置强制 Marian：仍返回直达段，加载时报出缺哪个模型
        return vec![Leg { src, tgt, family: Family::Marian }];
    }
    nllb
}

/// 路线上还没下载的模型
pub fn missing(dirs: &ModelDirs, legs: &[Leg]) -> Vec<Leg> {
    legs.iter().filter(|l| !ready(&l.dir(dirs))).copied().collect()
}

/// 检查一个方向能否本地翻译；不能时说明缺什么。
pub fn locate(dirs: &ModelDirs, src: &'static Lang, tgt: &'static Lang, only: Option<Family>) -> Result<Vec<Leg>> {
    let legs = plan(dirs, src, tgt, only);
    let miss = missing(dirs, &legs);
    if let Some(l) = miss.first() {
        return Err(match l.family {
            Family::Marian => anyhow!(
                "本地翻译 {}→{} 缺模型 {}（在设置窗口点「开始翻译」会自动下载）",
                l.src.label,
                l.tgt.label,
                l.repo()
            ),
            Family::Nllb => anyhow!(
                "本地翻译 {}→{} 需要 NLLB 模型（simulvoice models nllb，约 0.9 GB）",
                src.label,
                tgt.label
            ),
        });
    }
    Ok(legs)
}

type Shared = Arc<Mutex<Seq2Seq>>;

/// 同一个模型目录只加载一次：双向同传时两个方向共用一份 NLLB（约 1 GB 内存），
/// 中转路线的英语段也可能被两个方向共用。
fn shared(dir: &Path, family: Family) -> Result<Shared> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Shared>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    // 加载很慢（NLLB 数秒），不能持锁加载，否则另一方向的不同模型也被卡住；
    // 两个线程恰好同时加载同一模型的代价只是多读一次文件，后到的那份被丢弃。
    if let Some(m) = cache.lock().map_err(|_| anyhow!("模型缓存锁损坏"))?.get(dir) {
        return Ok(m.clone());
    }
    let threads = std::thread::available_parallelism()
        .map(|n| n.get() / 2)
        .unwrap_or(4)
        .max(2);
    let model = Arc::new(Mutex::new(Seq2Seq::load(dir, family, threads)?));
    let mut guard = cache.lock().map_err(|_| anyhow!("模型缓存锁损坏"))?;
    Ok(guard.entry(dir.to_path_buf()).or_insert(model).clone())
}

pub struct NmtTranslator {
    legs: Vec<(Leg, Shared)>,
}

impl NmtTranslator {
    pub fn new(dirs: &ModelDirs, src: &'static Lang, tgt: &'static Lang, only: Option<Family>) -> Result<Self> {
        let legs = locate(dirs, src, tgt, only)?;
        let mut loaded = Vec::with_capacity(legs.len());
        for leg in legs {
            loaded.push((leg, shared(&leg.dir(dirs), leg.family)?));
        }
        Ok(Self { legs: loaded })
    }

    pub fn describe(&self) -> String {
        self.legs
            .iter()
            .map(|(l, _)| {
                format!(
                    "{} {}→{}",
                    match l.family {
                        Family::Marian => "Marian",
                        Family::Nllb => "NLLB",
                    },
                    l.src.label,
                    l.tgt.label
                )
            })
            .collect::<Vec<_>>()
            .join(" + ")
    }
}

impl NmtTranslator {
    fn translate_sentence(&mut self, text: &str) -> Result<String> {
        let mut cur = text.to_string();
        for (leg, model) in &self.legs {
            let mut m = model
                .lock()
                .map_err(|_| anyhow!("翻译模型锁损坏（上次翻译崩溃）"))?;
            cur = m.translate(&cur, leg.src, leg.tgt)?;
            if cur.is_empty() {
                break;
            }
        }
        Ok(cur)
    }
}

impl crate::translate::Translator for NmtTranslator {
    /// 逐句翻译再拼接：opus-mt 是按单句训练的，一次喂两句常把其中一句整句丢掉
    /// （"Can you share the slides? I missed the first part." → 只剩前半句）。
    fn translate(&mut self, text: &str) -> Result<String> {
        let Some(tgt) = self.legs.last().map(|(l, _)| l.tgt) else {
            return Ok(text.to_string());
        };
        let joiner = if matches!(tgt.code, "zh" | "yue" | "ja" | "th") { "" } else { " " };
        let mut out: Vec<String> = Vec::new();
        for sentence in split_sentences(text) {
            let t = dedupe_clauses(&self.translate_sentence(&sentence)?);
            if !t.is_empty() {
                out.push(if joiner.is_empty() { t } else { capitalize(&t) });
            }
        }
        Ok(out.join(joiner))
    }
}

/// 去掉译文里重复的分句并去掉句尾多余逗号。
/// opus-mt 对很短的寒暄会打转："Morning, everyone." → "早上好,大家好,早安,大家好,早安"。
pub fn dedupe_clauses(text: &str) -> String {
    let is_sep = |c: char| matches!(c, ',' | '，' | '、' | ';' | '；');
    let mut out = String::new();
    let mut seen: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut flush = |cur: &mut String, sep: Option<char>, out: &mut String| {
        let key: String = cur
            .chars()
            .filter(|c| !c.is_whitespace() && !matches!(c, '。' | '.' | '!' | '?' | '！' | '？'))
            .collect::<String>()
            .to_lowercase();
        if !key.is_empty() && !seen.contains(&key) {
            seen.push(key);
            out.push_str(cur);
            if let Some(s) = sep {
                out.push(s);
            }
        }
        cur.clear();
    };
    for c in text.chars() {
        if is_sep(c) {
            flush(&mut cur, Some(c), &mut out);
        } else {
            cur.push(c);
        }
    }
    flush(&mut cur, None, &mut out);
    out.trim()
        .trim_end_matches(|c: char| is_sep(c) || c.is_whitespace())
        .to_string()
}

/// 拼音文字的句首大写（逐句翻译时模型常输出 "can you hear me?"）
fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) if f.is_lowercase() => f.to_uppercase().collect::<String>() + c.as_str(),
        _ => s.to_string(),
    }
}

/// 按句末标点切句，标点留在句尾。中日文句号/问号/感叹号直接断句；
/// 英文的 . ? ! 后面要跟空白或结尾才断（避免把 3.5、Q3.2、e.g. 中间切开）。
pub fn split_sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        cur.push(c);
        let next = chars.get(i + 1).copied();
        let closing = |n: char| matches!(n, '"' | '\'' | '”' | '’' | '」' | '』' | ')' | '）');
        let still_punct = next
            .map(|n| matches!(n, '.' | '?' | '!' | '。' | '？' | '！' | '…') || closing(n))
            .unwrap_or(false);
        let end = match c {
            '。' | '？' | '！' => !still_punct,
            '.' | '?' | '!' => !still_punct && next.map(|n| n.is_whitespace()).unwrap_or(true),
            '"' | '\'' | '”' | '’' | '」' | '』' | ')' | '）' => {
                // 引号/括号紧跟在句末标点后，一起收尾
                i > 0
                    && matches!(chars[i - 1], '.' | '?' | '!' | '。' | '？' | '！')
                    && !still_punct
            }
            _ => false,
        };
        if end {
            let t = cur.trim().to_string();
            if !t.is_empty() {
                out.push(t);
            }
            cur.clear();
        }
    }
    let t = cur.trim().to_string();
    if !t.is_empty() {
        out.push(t);
    }
    out
}

pub(crate) fn ort_env() -> Result<&'static ort::environment::Environment> {
    static ENV: OnceLock<ort::environment::Environment> = OnceLock::new();
    if let Some(env) = ENV.get() {
        return Ok(env);
    }
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let mut candidates = Vec::new();
    if let Ok(p) = std::env::var("ORT_DYLIB_PATH") {
        candidates.push(PathBuf::from(p));
    }
    candidates.push(exe.join("onnxruntime_marian.dll"));
    candidates.push(exe.join("onnxruntime.dll"));
    candidates.push(PathBuf::from("onnxruntime_marian.dll"));
    let dll = candidates
        .iter()
        .find(|p| p.exists())
        .cloned()
        .ok_or_else(|| anyhow!("找不到 onnxruntime_marian.dll（应与 simulvoice.exe 放在同一目录）"))?;
    let env = ort::init_from(&dll)
        .map_err(|e| anyhow!("加载 ORT 失败: {e}"))?
        .build()
        .map_err(|e| anyhow!("初始化 ORT 失败: {e}"))?;
    let _ = ENV.set(env);
    Ok(ENV.get().expect("env just set"))
}

#[cfg(test)]
mod tests {
    use super::loop_cut;

    #[test]
    fn splits_sentences() {
        use super::split_sentences as sp;
        assert_eq!(
            sp("Can you share the slides? I missed the first part."),
            vec!["Can you share the slides?", "I missed the first part."]
        );
        assert_eq!(sp("大家能听到吗？我先介绍一下。好"), vec!["大家能听到吗？", "我先介绍一下。", "好"]);
        assert_eq!(sp("Version 3.5 ships in Q3."), vec!["Version 3.5 ships in Q3."]);
        assert_eq!(sp("真的吗？！太好了"), vec!["真的吗？！", "太好了"]);
        assert_eq!(sp("He said \"go.\" Then left."), vec!["He said \"go.\"", "Then left."]);
        assert!(sp("   ").is_empty());
    }

    #[test]
    fn dedupes_repeated_clauses() {
        use super::dedupe_clauses as d;
        assert_eq!(d("早上好,大家好,早安,大家好,早安"), "早上好,大家好,早安");
        assert_eq!(d("大家早安,"), "大家早安");
        assert_eq!(d("我同意，不过我们需要先确认一下预算。"), "我同意，不过我们需要先确认一下预算。");
        assert_eq!(d("Yes, yes, I agree."), "Yes, I agree.");
    }

    #[test]
    fn top_k_sorted_and_skips_banned() {
        let row = [0.1, 5.0, 3.0, 9.0, 4.0];
        assert_eq!(super::top_k(&row, 2, &[3]), vec![(1, 5.0), (4, 4.0)]);
        assert_eq!(super::pick(&row, &[]), 3);
    }

    #[test]
    fn loop_cut_detects_phrase_repeat() {
        let ids = [5, 1, 2, 3, 1, 2, 3, 1, 2, 3];
        assert_eq!(loop_cut(&ids), Some(4));
    }

    #[test]
    fn loop_cut_tolerates_short_repeats() {
        assert_eq!(loop_cut(&[7, 7, 7]), None);
        assert_eq!(loop_cut(&[1, 2, 3, 4, 5]), None);
        assert_eq!(loop_cut(&[9, 7, 7, 7, 7]), Some(2));
    }
}
