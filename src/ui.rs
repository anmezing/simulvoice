use eframe::egui;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default, Clone)]
pub struct SubtitleState {
    pub in_src: String,
    pub in_dst: String,
    pub out_src: String,
    pub out_dst: String,
    pub version: u64,
}

impl SubtitleState {
    pub fn set_in(&mut self, src: String, dst: String) {
        if !src.is_empty() && self.in_src != src {
            self.in_src = src;
            self.version += 1;
        }
        if !dst.is_empty() && self.in_dst != dst {
            self.in_dst = dst;
            self.version += 1;
        }
    }
    pub fn set_out(&mut self, src: String, dst: String) {
        if !src.is_empty() && self.out_src != src {
            self.out_src = src;
            self.version += 1;
        }
        if !dst.is_empty() && self.out_dst != dst {
            self.out_dst = dst;
            self.version += 1;
        }
    }
}

pub type Shared = Arc<Mutex<SubtitleState>>;

pub fn new_state() -> Shared {
    Arc::new(Mutex::new(SubtitleState::default()))
}

/// Must run on the main thread (winit requirement on Windows).
pub fn run(state: Shared, auto_close: Option<Duration>) -> anyhow::Result<()> {
    let shared = state.clone();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_always_on_top()
            .with_decorations(false)
            .with_transparent(true)
            .with_inner_size([780.0, 210.0]),
        ..Default::default()
    };
    eframe::run_native(
        "simulvoice subtitles",
        options,
        Box::new(move |cc| {
            install_cjk_font(&cc.egui_ctx);
            Ok(Box::new(SubtitleApp {
                state: shared,
                started: std::time::Instant::now(),
                auto_close,
                last_version: 0,
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("字幕窗口失败: {e}"))
}

fn install_cjk_font(ctx: &egui::Context) {
    for path in [
        "C:/Windows/Fonts/simhei.ttf",
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/simsun.ttc",
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("cjk".to_owned(), egui::FontData::from_owned(bytes).into());
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "cjk".to_owned());
            ctx.set_fonts(fonts);
            break;
        }
    }
}

struct SubtitleApp {
    state: Shared,
    started: std::time::Instant,
    auto_close: Option<Duration>,
    last_version: u64,
}

impl eframe::App for SubtitleApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let s = self.state.lock().unwrap().clone();
        if s.version != self.last_version {
            self.last_version = s.version;
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(400));
        }
        if let Some(limit) = self.auto_close {
            if self.started.elapsed() >= limit {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(egui::Color32::from_black_alpha(165))
                    .inner_margin(14.0),
            )
            .show(ctx, |ui| {
                if !s.in_src.is_empty() || !s.in_dst.is_empty() {
                    ui.label(
                        egui::RichText::new(format!("对方  {}", s.in_src))
                            .size(18.0)
                            .color(egui::Color32::from_gray(200)),
                    );
                    ui.label(
                        egui::RichText::new(&s.in_dst)
                            .size(24.0)
                            .strong()
                            .color(egui::Color32::WHITE),
                    );
                    ui.add_space(8.0);
                }
                if !s.out_src.is_empty() || !s.out_dst.is_empty() {
                    ui.label(
                        egui::RichText::new(format!("你    {}", s.out_src))
                            .size(18.0)
                            .color(egui::Color32::from_gray(200)),
                    );
                    ui.label(
                        egui::RichText::new(&s.out_dst)
                            .size(24.0)
                            .strong()
                            .color(egui::Color32::from_rgb(170, 220, 255)),
                    );
                }
            });
    }
}

pub fn run_settings(cfg: crate::config::Config, path: String) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([600.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "simulvoice settings",
        options,
        Box::new(move |cc| {
            install_cjk_font(&cc.egui_ctx);
            Ok(Box::new(SettingsApp {
                cfg,
                path,
                status: String::new(),
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("设置窗口失败: {e}"))
}

struct SettingsApp {
    cfg: crate::config::Config,
    path: String,
    status: String,
}

fn pick_folder() -> Option<String> {
    let script = "Add-Type -AssemblyName System.Windows.Forms; \
                  $d = New-Object System.Windows.Forms.FolderBrowserDialog; \
                  if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { Write-Output $d.SelectedPath }";
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-STA", "-Command", script])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn combo(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[&str]) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(value.clone())
        .show_ui(ui, |ui| {
            for o in options {
                ui.selectable_value(value, o.to_string(), *o);
            }
        });
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _f: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("simulvoice 设置");
            ui.add_space(8.0);
            egui::Grid::new("settings")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("STT 引擎");
                    combo(ui, "stt", &mut self.cfg.stt.provider, &["soniox", "openai", "vosk", "none"]);
                    ui.end_row();
                    ui.label("入向语言");
                    ui.text_edit_singleline(&mut self.cfg.stt.language);
                    ui.end_row();
                    ui.label("出向语言");
                    ui.text_edit_singleline(&mut self.cfg.stt.language_reverse);
                    ui.end_row();
                    ui.label("翻译引擎");
                    combo(ui, "tr", &mut self.cfg.translate.provider, &["openai", "gemini", "ollama", "qwen", "custom", "none"]);
                    ui.end_row();
                    ui.label("翻译目标(入向)");
                    ui.text_edit_singleline(&mut self.cfg.translate.target);
                    ui.end_row();
                    ui.label("翻译目标(出向)");
                    ui.text_edit_singleline(&mut self.cfg.translate.target_reverse);
                    ui.end_row();
                    ui.label("翻译模型/地址");
                    ui.text_edit_singleline(&mut self.cfg.translate.model);
                    ui.end_row();
                    ui.label("自定义端点");
                    ui.text_edit_singleline(&mut self.cfg.translate.endpoint);
                    ui.end_row();
                    ui.label("TTS 引擎");
                    combo(ui, "tts", &mut self.cfg.tts.provider, &["edge", "sapi", "sherpa", "openai", "none"]);
                    ui.end_row();
                    ui.label("入向音色");
                    ui.text_edit_singleline(&mut self.cfg.tts.voice);
                    ui.end_row();
                    ui.label("出向音色");
                    ui.text_edit_singleline(&mut self.cfg.tts.voice_reverse);
                    ui.end_row();
                    ui.label("采集");
                    combo(ui, "input", &mut self.cfg.audio.input, &["loopback", "pid", "mic"]);
                    ui.end_row();
                    ui.label("入向输出设备");
                    ui.text_edit_singleline(&mut self.cfg.audio.output_device);
                    ui.end_row();
                    ui.label("麦克风设备");
                    ui.text_edit_singleline(&mut self.cfg.audio.mic_device);
                    ui.end_row();
                    ui.label("出向输出(虚拟麦)");
                    ui.text_edit_singleline(&mut self.cfg.audio.remote_output_device);
                    ui.end_row();
                    ui.label("实时引擎");
                    combo(ui, "rt", &mut self.cfg.realtime.provider, &["none", "gemini-live", "palabra", "volcengine"]);
                    ui.end_row();
                });
            ui.add_space(6.0);
            ui.checkbox(&mut self.cfg.audio.duplex, "双向同传");
            ui.checkbox(&mut self.cfg.dsp.denoise, "RNNoise 降噪");
            ui.checkbox(&mut self.cfg.ui.subtitles, "双语字幕悬浮窗");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("保存").clicked() {
                    self.status = match self.cfg.save(&self.path) {
                        Ok(_) => format!("已保存: {}", self.path),
                        Err(e) => format!("保存失败: {e}"),
                    };
                }
                if ui.button("关闭").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.add_space(4.0);
            ui.label(&self.status);
        });
    }
}





