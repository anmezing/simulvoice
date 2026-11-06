use eframe::egui;
use egui::{Color32, RichText, Stroke};
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ───────────────────────────── 字幕状态（流水线写，窗口读） ─────────────────────────────

/// 一条已完成的翻译
#[derive(Clone, PartialEq)]
pub struct HistoryLine {
    /// true = 我说的（出向），false = 对方说的（入向）
    pub outbound: bool,
    pub src: String,
    pub dst: String,
    /// 对方那一路有人说了我的语言：只有原文，没翻译
    pub own: bool,
}

/// 窗口里保留的历史条数上限
const HISTORY_MAX: usize = 200;

#[derive(Default, Clone)]
pub struct SubtitleState {
    pub in_src: String,
    pub in_dst: String,
    pub out_src: String,
    pub out_dst: String,
    pub in_extras: Vec<(String, String)>,
    pub out_extras: Vec<(String, String)>,
    pub history: VecDeque<HistoryLine>,
    /// 运行中的提示（如翻译失败），显示在窗口里
    pub notice: String,
    pub version: u64,
}

impl SubtitleState {
    pub fn set_in(&mut self, src: String, dst: String) {
        if !src.is_empty() && self.in_src != src {
            self.in_src = src;
            self.in_dst.clear();
            self.version += 1;
        }
        if !dst.is_empty() && self.in_dst != dst {
            self.in_dst = dst;
            self.version += 1;
        }
    }

    pub fn set_extras(&mut self, outbound: bool, extras: Vec<(String, String)>) {
        if outbound {
            if self.out_extras != extras {
                self.out_extras = extras;
                self.version += 1;
            }
        } else if self.in_extras != extras {
            self.in_extras = extras;
            self.version += 1;
        }
    }

    pub fn set_out(&mut self, src: String, dst: String) {
        if !src.is_empty() && self.out_src != src {
            self.out_src = src;
            self.out_dst.clear();
            self.version += 1;
        }
        if !dst.is_empty() && self.out_dst != dst {
            self.out_dst = dst;
            self.version += 1;
        }
    }

    pub fn push_history(&mut self, outbound: bool, src: String, dst: String) {
        self.push_line(HistoryLine { outbound, src, dst, own: false });
    }

    /// 对方那一路说的是我的语言，原样记下
    pub fn push_own(&mut self, text: String) {
        self.push_line(HistoryLine { outbound: false, src: text.clone(), dst: text, own: true });
    }

    fn push_line(&mut self, line: HistoryLine) {
        self.history.push_back(line);
        while self.history.len() > HISTORY_MAX {
            self.history.pop_front();
        }
        self.notice.clear();
        self.version += 1;
    }

    pub fn set_notice(&mut self, msg: String) {
        if self.notice != msg {
            self.notice = msg;
            self.version += 1;
        }
    }
}

pub type Shared = Arc<Mutex<SubtitleState>>;

pub fn new_state() -> Shared {
    Arc::new(Mutex::new(SubtitleState::default()))
}

// ───────────────────────────── 命令行模式的悬浮字幕窗 ─────────────────────────────

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
    let mut fonts = egui::FontDefinitions::default();
    // 微软雅黑优先：笔画清晰，界面观感远好于黑体/宋体
    let regular = [
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/simhei.ttf",
        "C:/Windows/Fonts/simsun.ttc",
    ]
    .iter()
    .find_map(|p| std::fs::read(p).ok());
    if let Some(bytes) = regular {
        fonts.font_data.insert("cjk".to_owned(), egui::FontData::from_owned(bytes).into());
        fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "cjk".to_owned());
        fonts.families.entry(egui::FontFamily::Monospace).or_default().push("cjk".to_owned());
    }
    let mut bold = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    if let Ok(bytes) = std::fs::read("C:/Windows/Fonts/msyhbd.ttc") {
        fonts.font_data.insert("cjk-bold".to_owned(), egui::FontData::from_owned(bytes).into());
        bold.insert(0, "cjk-bold".to_owned());
    }
    fonts.families.insert(egui::FontFamily::Name(BOLD.into()), bold);
    ctx.set_fonts(fonts);
}

/// 粗体字族名（install_cjk_font 一定会注册它）
const BOLD: &str = "bold";

fn bold(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).size(size).family(egui::FontFamily::Name(BOLD.into()))
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
                    .fill(Color32::from_black_alpha(165))
                    .inner_margin(14.0),
            )
            .show(ctx, |ui| {
                let backlog = crate::TTS_BACKLOG.load(Ordering::Relaxed);
                if backlog > 0 {
                    ui.label(
                        RichText::new(format!("积压约 {} 秒（{} 句）", backlog * 3, backlog))
                            .size(13.0)
                            .color(Color32::from_rgb(240, 180, 80)),
                    );
                }
                if !s.in_src.is_empty() || !s.in_dst.is_empty() {
                    ui.label(RichText::new(format!("对方  {}", s.in_src)).size(18.0).color(Color32::from_gray(200)));
                    ui.label(RichText::new(&s.in_dst).size(24.0).strong().color(Color32::WHITE));
                    for (lang, text) in &s.in_extras {
                        ui.label(RichText::new(format!("[{lang}] {text}")).size(19.0).color(Color32::from_gray(190)));
                    }
                    ui.add_space(8.0);
                }
                if !s.out_src.is_empty() || !s.out_dst.is_empty() {
                    ui.label(RichText::new(format!("你    {}", s.out_src)).size(18.0).color(Color32::from_gray(200)));
                    ui.label(
                        RichText::new(&s.out_dst)
                            .size(24.0)
                            .strong()
                            .color(Color32::from_rgb(170, 220, 255)),
                    );
                    for (lang, text) in &s.out_extras {
                        ui.label(RichText::new(format!("[{lang}] {text}")).size(19.0).color(Color32::from_gray(190)));
                    }
                }
            });
    }
}

// ───────────────────────────── 主窗口 ─────────────────────────────

// 设计 token 参照 bencho.dev 的 tokens.css：暖调中性灰 + 墨色分级，彩色只表示状态
// （ok = 在听，ember = 出错）。界面里不直接写颜色值，都从这里取。
const INK: Color32 = Color32::from_rgb(0x17, 0x18, 0x1a);
/// 墨色的次级：ink-3 用于说明文字，ink-5 用于占位/最弱提示
const INK_3: Color32 = Color32::from_rgb(0x5c, 0x5b, 0x56);
const INK_5: Color32 = Color32::from_rgb(0x85, 0x84, 0x7e);
const DANGER: Color32 = Color32::from_rgb(0xb8, 0x43, 0x1c);
const DANGER_SOFT: Color32 = Color32::from_rgb(0xfb, 0xf0, 0xeb);
const OK_GREEN: Color32 = Color32::from_rgb(0x15, 0x80, 0x3d);
const BG: Color32 = Color32::WHITE;
/// card / surface：白底上的分区块（不加边框）
const TILE: Color32 = Color32::from_rgb(0xf7, 0xf7, 0xf6);
/// surface-2 / surface-3：轨道、悬停
const TRACK: Color32 = Color32::from_rgb(0xec, 0xeb, 0xe7);
const CHIP: Color32 = Color32::from_rgb(0xf1, 0xf1, 0xf0);
/// fill-slab：分区上的白色控件
const CARD: Color32 = Color32::WHITE;
/// pane-edge：rgba(ink, 0.08) 的细线，代替实线边框
const HAIRLINE: Color32 = Color32::from_rgba_premultiplied(2, 2, 2, 20);
const TEXT: Color32 = INK;
const MUTED: Color32 = INK_3;
const WARN_BG: Color32 = Color32::from_rgb(0xfb, 0xf5, 0xe9);
const WARN_FG: Color32 = Color32::from_rgb(0x8a, 0x5a, 0x10);
const TILE_R: f32 = 18.0;
/// 展开/切换动画：0.42 秒 ease-out（「它是打开的，不是跳出来的」）
const MOTION: f32 = 0.42;
/// 高级设置里表单标签列的宽度（各张表统一，下拉框左边才对齐）
const LABEL_W: f32 = 124.0;

/// 常见会议软件的进程名 → 显示名，列表里优先展示。
/// 第三列 = 首次打开时可自动选中：只限专门的会议软件。微信、QQ、浏览器平时也常开着，
/// 自动选它们会让真正开会用的软件（如腾讯会议）的声音收不到。
const MEETING_APPS: &[(&str, &str, bool)] = &[
    ("wemeetapp.exe", "腾讯会议", true),
    ("ms-teams.exe", "Microsoft Teams", true),
    ("teams.exe", "Microsoft Teams", true),
    ("zoom.exe", "Zoom", true),
    ("dingtalk.exe", "钉钉", true),
    ("feishu.exe", "飞书", true),
    ("lark.exe", "Lark", true),
    ("webex.exe", "Webex", true),
    ("ciscocollabhost.exe", "Webex", true),
    ("skype.exe", "Skype", true),
    ("weixin.exe", "微信", false),
    ("wechat.exe", "微信", false),
    ("qq.exe", "QQ", false),
    ("chrome.exe", "Chrome（网页会议）", false),
    ("msedge.exe", "Edge（网页会议）", false),
    ("firefox.exe", "Firefox（网页会议）", false),
];

fn meeting_label(exe: &str) -> Option<&'static str> {
    MEETING_APPS
        .iter()
        .find(|(n, _, _)| n.eq_ignore_ascii_case(exe))
        .map(|(_, l, _)| *l)
}

fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::light();
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.extreme_bg_color = CARD;
    v.faint_bg_color = TILE;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = TRACK;
    v.selection.stroke = Stroke::new(1.0, INK);
    v.hyperlink_color = INK;
    // 输入控件都是胶囊形（高 28 → 圆角 14）
    let r = egui::Rounding::same(14.0);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.rounding = r;
        w.expansion = 0.0;
    }
    // 灰块上放白色胶囊，只留一条很淡的描边勾出轮廓
    let edge = HAIRLINE;
    v.widgets.inactive.weak_bg_fill = CARD;
    v.widgets.inactive.bg_fill = CARD;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, edge);
    v.widgets.inactive.fg_stroke = Stroke::new(1.5, TEXT);
    v.widgets.hovered.weak_bg_fill = CHIP;
    v.widgets.hovered.bg_fill = CHIP;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_black_alpha(40));
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, TEXT);
    v.widgets.active.weak_bg_fill = TRACK;
    v.widgets.active.bg_fill = TRACK;
    v.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_black_alpha(70));
    v.widgets.active.fg_stroke = Stroke::new(1.5, TEXT);
    v.widgets.open.weak_bg_fill = CARD;
    v.widgets.open.bg_stroke = Stroke::new(1.0, Color32::from_black_alpha(70));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, HAIRLINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.window_rounding = egui::Rounding::same(14.0);
    v.menu_rounding = egui::Rounding::same(14.0);
    v.window_stroke = Stroke::new(1.0, Color32::from_black_alpha(14));
    v.popup_shadow = egui::epaint::Shadow {
        offset: egui::vec2(0.0, 8.0),
        blur: 24.0,
        spread: 0.0,
        color: Color32::from_black_alpha(28),
    };
    // 同一套浅色配色同时写进浅色/深色两套主题：egui 跟随系统主题时
    // 不论系统是哪种，用的都是这里的配色（否则下拉框会变回灰块）
    ctx.set_visuals_of(egui::Theme::Light, v.clone());
    ctx.set_visuals_of(egui::Theme::Dark, v.clone());

    let mut style = (*ctx.style()).clone();
    use egui::{FontFamily::Proportional, FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Heading, FontId::new(17.0, Proportional)),
        (TextStyle::Body, FontId::new(14.0, Proportional)),
        (TextStyle::Button, FontId::new(14.0, Proportional)),
        (TextStyle::Small, FontId::new(12.0, Proportional)),
        (TextStyle::Monospace, FontId::new(12.5, egui::FontFamily::Monospace)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 4.0);
    style.spacing.interact_size = egui::vec2(28.0, 28.0);
    style.spacing.combo_height = 300.0;
    style.spacing.icon_width = 15.0;
    style.spacing.menu_margin = egui::Margin::same(6.0);
    style.visuals = v;
    ctx.set_style_of(egui::Theme::Light, style.clone());
    ctx.set_style_of(egui::Theme::Dark, style);
}

/// 分区：白底上的浅灰大圆角块，不加边框；标题是深色粗体小字
fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::none()
        .fill(TILE)
        .rounding(TILE_R)
        .inner_margin(egui::Margin::same(16.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(bold(title, 13.0).color(TEXT));
            }
            add(ui)
        })
        .inner
}

fn hint_box(ui: &mut egui::Ui, text: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(WARN_BG)
        .rounding(12.0)
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(WARN_FG)).wrap());
            add(ui);
        });
}

/// 胶囊按钮（纯色、无描边）
fn pill_button(text: RichText, fill: Color32) -> egui::Button<'static> {
    egui::Button::new(text).fill(fill).stroke(Stroke::NONE).rounding(100.0)
}

/// 分段开关：灰色轨道里一块白色滑块，切换时滑块滑过去（不是瞬间跳）。返回被点击的下标。
fn segmented(ui: &mut egui::Ui, id: &str, selected: usize, options: &[&str]) -> Option<usize> {
    let n = options.len().max(1) as f32;
    let (track, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), egui::Sense::hover());
    let pad = 3.0;
    let seg_w = (track.width() - pad * 2.0) / n;
    let x = ui.ctx().animate_value_with_time(ui.id().with(id), selected as f32, MOTION * 0.6);
    let painter = ui.painter();
    painter.rect_filled(track, 100.0, TRACK);
    let thumb = egui::Rect::from_min_size(
        track.min + egui::vec2(pad + seg_w * x, pad),
        egui::vec2(seg_w, track.height() - pad * 2.0),
    );
    let shadow = egui::epaint::Shadow {
        offset: egui::vec2(0.0, 1.0),
        blur: 4.0,
        spread: 0.0,
        color: Color32::from_black_alpha(22),
    };
    painter.add(shadow.as_shape(thumb, 100.0));
    painter.rect_filled(thumb, 100.0, CARD);
    let mut clicked = None;
    for (i, label) in options.iter().enumerate() {
        let rect = egui::Rect::from_min_size(
            track.min + egui::vec2(pad + seg_w * i as f32, pad),
            egui::vec2(seg_w, track.height() - pad * 2.0),
        );
        let resp = ui.interact(rect, ui.id().with((id, i)), egui::Sense::click());
        let on = i == selected;
        // 选中项满墨色，其余用次级墨色，悬停时提亮
        let color = if on || resp.hovered() { TEXT } else { MUTED };
        let font = if on {
            egui::FontId::new(13.5, egui::FontFamily::Name(BOLD.into()))
        } else {
            egui::FontId::proportional(13.5)
        };
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, *label, font, color);
        if resp.hovered() && !on {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() && !on && ui.is_enabled() {
            clicked = Some(i);
        }
    }
    clicked
}

/// 小胶囊标签（状态、说话人）；dot 非空时前面画一个彩色圆点。
/// 整块自绘、按文字量尺寸：放在任何方向的布局里都不会被撑开或左右颠倒。
fn pill(ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32, dot: Option<Color32>) {
    let font = egui::FontId::new(11.5, egui::FontFamily::Name(BOLD.into()));
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, fg);
    let (pad_x, h) = (9.0, 20.0);
    let dot_w = if dot.is_some() { 11.0 } else { 0.0 };
    let size = egui::vec2(pad_x * 2.0 + dot_w + galley.size().x, h);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 100.0, bg);
    if let Some(c) = dot {
        painter.circle_filled(egui::pos2(rect.left() + pad_x + 3.0, rect.center().y), 3.0, c);
    }
    let pos = egui::pos2(rect.left() + pad_x + dot_w, rect.center().y - galley.size().y / 2.0);
    painter.galley(pos, galley, fg);
}

/// 下拉框：右侧用细的灰色折线箭头，代替默认的黑色实心三角（太重）
fn combo(id: impl std::hash::Hash) -> egui::ComboBox {
    egui::ComboBox::from_id_salt(id).icon(|ui, rect, visuals, is_open, _| {
        let c = rect.center();
        let (w, h) = (4.0, 2.2);
        let dy = if is_open { -1.0 } else { 1.0 };
        let pts = vec![c + egui::vec2(-w, -h * dy), c + egui::vec2(0.0, h * dy), c + egui::vec2(w, -h * dy)];
        let color = if visuals.fg_stroke.color == TEXT { MUTED } else { visuals.fg_stroke.color };
        ui.painter().add(egui::Shape::line(pts, Stroke::new(1.5, color)));
    })
}

fn lang_combo(ui: &mut egui::Ui, id: &str, value: &mut String, width: f32) -> bool {
    let current = crate::lang::find(value).map(|l| l.label).unwrap_or("请选择");
    let mut changed = false;
    combo(id)
        .width(width)
        .selected_text(current)
        .show_ui(ui, |ui| {
            for l in crate::lang::LANGS {
                if ui
                    .selectable_label(value == l.code, format!("{}  {}", l.label, l.name))
                    .clicked()
                {
                    *value = l.code.to_string();
                    changed = true;
                }
            }
        });
    changed
}

fn device_combo(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[String], empty: &str) {
    let current = value.clone();
    let label = if current.is_empty() { empty.to_string() } else { current.clone() };
    combo(id)
        .width(220.0)
        .selected_text(label)
        .show_ui(ui, |ui| {
            ui.selectable_value(value, String::new(), empty);
            for option in options {
                ui.selectable_value(value, option.clone(), option);
            }
            if !current.is_empty() && !options.contains(&current) {
                ui.selectable_value(value, current.clone(), current.clone());
            }
        });
}

fn option_combo(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[(&str, &str)]) {
    let label = options
        .iter()
        .find(|(k, _)| *k == value.as_str())
        .map(|(_, l)| l.to_string())
        .unwrap_or_else(|| value.clone());
    combo(id)
        .width(220.0)
        .selected_text(label)
        .show_ui(ui, |ui| {
            for (k, l) in options {
                ui.selectable_value(value, k.to_string(), *l);
            }
        });
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

/// VB-Audio 虚拟线缆的播放端（程序把译文播进去，会议软件把 CABLE Output 当麦克风）
fn cable_device(outputs: &[String]) -> Option<String> {
    outputs
        .iter()
        .find(|n| n.to_ascii_uppercase().contains("CABLE INPUT"))
        .or_else(|| outputs.iter().find(|n| n.to_ascii_uppercase().contains("CABLE")))
        .cloned()
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Idle,
    Preparing,
    Running,
    Stopping,
}

struct Session {
    phase: Phase,
    error: String,
    info: String,
    progress: String,
}

/// `autostart`：窗口一打开就开始翻译（`simulvoice settings --start`，可做成桌面快捷方式）
pub fn run_settings(cfg: crate::config::Config, path: String, autostart: bool) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("SimulVoice 同声传译")
            // SIMULVOICE_UI_SNAPSHOT_HEIGHT：截图检查时临时拉高窗口，看到整页
            .with_inner_size([
                460.0,
                std::env::var("SIMULVOICE_UI_SNAPSHOT_HEIGHT")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(600.0),
            ])
            .with_min_inner_size([400.0, 460.0])
            .with_icon(egui::IconData { rgba: crate::icon::app_icon_rgba(64), width: 64, height: 64 }),
        ..Default::default()
    };
    #[cfg(windows)]
    crate::tray::start();
    let result = eframe::run_native(
        "simulvoice settings",
        options,
        Box::new(move |cc| {
            install_cjk_font(&cc.egui_ctx);
            apply_theme(&cc.egui_ctx);
            let mut app = SettingsApp {
                extra_text: cfg.translate.extra_targets.join(", "),
                cfg,
                path,
                status: String::new(),
                outputs: crate::sink::list_output_devices().unwrap_or_default(),
                inputs: crate::sink::list_input_devices().unwrap_or_default(),
                processes: crate::capture::list_window_processes(),
                session: Arc::new(Mutex::new(Session {
                    phase: Phase::Idle,
                    error: String::new(),
                    info: String::new(),
                    progress: String::new(),
                })),
                subs: new_state(),
                needs_key: String::new(),
                needs: Vec::new(),
                autostart,
                snapshot: Snapshot::from_env(),
                // SIMULVOICE_UI_ADVANCED=1：截图检查时默认展开
                show_advanced: std::env::var("SIMULVOICE_UI_ADVANCED").is_ok(),
                hwnd_registered: false,
                update: Arc::new(Mutex::new(UpdateState::Idle)),
                update_scheduled: false,
            };
            app.auto_pick_meeting();
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("窗口启动失败: {e}"));
    #[cfg(windows)]
    crate::tray::shutdown();
    result
}

/// 用默认浏览器打开网址（只接受 https，避免把别的东西交给 explorer 执行）
fn open_url(url: &str) {
    if !url.starts_with("https://") {
        eprintln!("[ui] 拒绝打开非 https 链接: {url}");
        return;
    }
    if let Err(e) = std::process::Command::new("explorer").arg(url).spawn() {
        eprintln!("[ui] 打开链接失败: {e}");
    }
}

/// 绿色对勾 + 一行说明。勾是画出来的：内置字体（微软雅黑 + egui 默认字体）里没有 ✓，会显示成方框
fn ok_line(ui: &mut egui::Ui, text: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 16.0), egui::Sense::hover());
        let c = rect.center();
        let pts = vec![
            c + egui::vec2(-4.0, 0.2),
            c + egui::vec2(-1.3, 3.0),
            c + egui::vec2(4.2, -3.2),
        ];
        ui.painter().add(egui::Shape::line(pts, Stroke::new(1.7, OK_GREEN)));
        ui.label(RichText::new(text).size(12.0).color(OK_GREEN));
    });
}

/// 检查更新的进度（后台线程写，界面读）
#[derive(Clone)]
enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Available(crate::update::Release),
    Failed(String),
}

struct SettingsApp {
    cfg: crate::config::Config,
    path: String,
    status: String,
    extra_text: String,
    outputs: Vec<String>,
    inputs: Vec<String>,
    processes: Vec<(u32, String, String)>,
    session: Arc<Mutex<Session>>,
    subs: Shared,
    /// needs 的缓存键（语言/引擎变了才重新检查磁盘）
    needs_key: String,
    needs: Vec<crate::models::Need>,
    autostart: bool,
    snapshot: Option<Snapshot>,
    show_advanced: bool,
    /// 主窗口句柄已登记给托盘
    hwnd_registered: bool,
    update: Arc<Mutex<UpdateState>>,
    /// 本次启动的自动检查已经安排过
    update_scheduled: bool,
}

/// 开发用：设 SIMULVOICE_UI_SNAPSHOT=<文件.bmp>，窗口启动若干秒后（SIMULVOICE_UI_SNAPSHOT_DELAY，
/// 默认 2 秒）把自己渲染的画面存成 BMP 并退出。读的是渲染结果，屏幕锁定/窗口被遮挡时也能截。
struct Snapshot {
    path: std::path::PathBuf,
    started: std::time::Instant,
    delay: Duration,
    requested: bool,
}

impl Snapshot {
    fn from_env() -> Option<Self> {
        let path = std::env::var("SIMULVOICE_UI_SNAPSHOT").ok()?;
        let secs = std::env::var("SIMULVOICE_UI_SNAPSHOT_DELAY")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(2.0);
        Some(Self {
            path: path.into(),
            started: std::time::Instant::now(),
            delay: Duration::from_secs_f32(secs.max(0.5)),
            requested: false,
        })
    }

    /// 返回 true 表示截图已保存、窗口应关闭
    fn tick(&mut self, ctx: &egui::Context) -> bool {
        ctx.request_repaint_after(Duration::from_millis(100));
        if !self.requested && self.started.elapsed() >= self.delay {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            self.requested = true;
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        match image {
            Some(img) => {
                if let Err(e) = write_bmp(&self.path, &img) {
                    eprintln!("[ui] 截图保存失败: {e}");
                }
                true
            }
            None => false,
        }
    }
}

/// 32 位自上而下 BMP（无需额外依赖）
fn write_bmp(path: &std::path::Path, img: &egui::ColorImage) -> std::io::Result<()> {
    let (w, h) = (img.size[0] as u32, img.size[1] as u32);
    let data_len = w * h * 4;
    let mut out = Vec::with_capacity(54 + data_len as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + data_len).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(-(h as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    for c in &img.pixels {
        out.extend_from_slice(&[c.b(), c.g(), c.r(), 255]);
    }
    std::fs::write(path, out)
}

impl SettingsApp {
    /// 第一次用、还没选过会议软件：如果正开着某个会议软件，直接替用户选上
    fn auto_pick_meeting(&mut self) {
        if !self.cfg.audio.process.is_empty() {
            return;
        }
        let found = MEETING_APPS
            .iter()
            .filter(|(_, _, auto)| *auto)
            .find_map(|(n, _, _)| self.processes.iter().find(|(_, p, _)| p.eq_ignore_ascii_case(n)));
        if let Some((pid, name, _)) = found {
            self.cfg.audio.input = "pid".into();
            self.cfg.audio.pid = *pid;
            self.cfg.audio.process = name.clone();
        }
    }

    fn phase(&self) -> Phase {
        self.session.lock().map(|s| s.phase).unwrap_or(Phase::Idle)
    }

    fn refresh_needs(&mut self) {
        let key = format!(
            "{}|{}|{}|{}|{}|{}",
            self.cfg.languages.mine,
            self.cfg.languages.theirs,
            self.cfg.stt.provider,
            self.cfg.translate.provider,
            self.cfg.tts.provider,
            self.cfg.realtime.provider
        );
        if key != self.needs_key {
            self.needs_key = key;
            self.needs = crate::models::needed(&self.cfg);
        }
    }

    fn save(&mut self) -> bool {
        self.cfg.translate.extra_targets = self
            .extra_text
            .split([',', '，'])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        self.cfg.apply_languages();
        match self.cfg.save(&self.path) {
            Ok(_) => true,
            Err(e) => {
                self.status = format!("保存设置失败: {e}");
                false
            }
        }
    }

    fn start(&mut self) {
        self.save();
        if !crate::single_instance() {
            if let Ok(mut s) = self.session.lock() {
                s.error = "已有另一个 SimulVoice 在运行（可能在托盘或命令行窗口里），请先关掉它".into();
            }
            return;
        }
        let mut run_cfg = self.cfg.clone();
        run_cfg.ui.subtitles = false;
        let mut info = String::new();
        if run_cfg.audio.input == "pid" {
            let name = run_cfg.audio.process.clone();
            match crate::capture::find_pid(&name) {
                Some(pid) => run_cfg.audio.pid = pid,
                None => {
                    // 选的软件没开：退回整机声音，并在窗口里说明（否则用户只看到"没反应"）
                    run_cfg.audio.input = "loopback".into();
                    run_cfg.audio.pid = 0;
                    run_cfg.audio.process.clear();
                    info = if name.is_empty() {
                        "未选择会议软件，正在收整个电脑的声音".into()
                    } else {
                        format!(
                            "没找到正在运行的{}，改为收整个电脑的声音",
                            meeting_label(&name).unwrap_or(&name)
                        )
                    };
                }
            }
        }
        if run_cfg.audio.duplex {
            run_cfg.audio.mode = "both".into();
            // 没戴耳机时，念给你听的译文会被麦克风收进去再翻给对方；朗读期间暂停收音
            run_cfg.audio.echo_guard = true;
            match cable_device(&self.outputs) {
                Some(dev) => {
                    if run_cfg.audio.remote_output_device.trim().is_empty() {
                        run_cfg.audio.remote_output_device = dev;
                    }
                }
                None => {
                    run_cfg.audio.mute_outbound = true;
                    if !info.is_empty() {
                        info.push('；');
                    }
                    info.push_str("未安装虚拟麦克风：你的话只显示译文字幕，对方听不到译音");
                }
            }
        }
        if let Ok(mut s) = self.subs.lock() {
            *s = SubtitleState::default();
        }
        if let Ok(mut s) = self.session.lock() {
            s.phase = Phase::Preparing;
            s.error.clear();
            s.info = info;
            s.progress = "正在准备…".into();
        }
        let session = self.session.clone();
        let subs = self.subs.clone();
        std::thread::spawn(move || {
            let set = |f: &dyn Fn(&mut Session)| {
                if let Ok(mut s) = session.lock() {
                    f(&mut s);
                }
            };
            crate::STOP.store(false, Ordering::SeqCst);
            let needs = crate::models::needed(&run_cfg);
            let total = needs.len();
            for (i, n) in needs.iter().enumerate() {
                let msg = format!("首次使用，正在下载{}（{}/{}，约 {} MB）", n.label, i + 1, total, n.size_mb);
                set(&|s| s.progress = msg.clone());
                if let Err(e) = crate::models::fetch(&n.id) {
                    let cancelled = crate::STOP.load(Ordering::SeqCst);
                    let err = if cancelled {
                        String::new()
                    } else {
                        format!("下载{}失败：{e}\n请检查网络后再点「开始翻译」，已下完的部分不会重下。", n.label)
                    };
                    set(&|s| {
                        s.phase = Phase::Idle;
                        s.error = err.clone();
                        s.progress.clear();
                    });
                    crate::STOP.store(false, Ordering::SeqCst);
                    return;
                }
            }
            if crate::STOP.load(Ordering::SeqCst) {
                set(&|s| {
                    s.phase = Phase::Idle;
                    s.progress.clear();
                });
                crate::STOP.store(false, Ordering::SeqCst);
                return;
            }
            set(&|s| {
                s.progress = "正在加载模型…".into();
                s.phase = Phase::Running;
            });
            let r = crate::run_session(run_cfg, None, Some(subs));
            let err = match r {
                Ok(()) => String::new(),
                Err(e) => e.to_string(),
            };
            set(&|s| {
                s.phase = Phase::Idle;
                s.error = err.clone();
                s.progress.clear();
            });
            crate::STOP.store(false, Ordering::SeqCst);
        });
    }

    fn stop(&mut self) {
        crate::STOP.store(true, Ordering::SeqCst);
        if let Ok(mut s) = self.session.lock() {
            if s.phase != Phase::Idle {
                s.phase = Phase::Stopping;
                s.progress = "正在停止…".into();
            }
        }
    }

    fn header(&self, ui: &mut egui::Ui, phase: Phase) {
        ui.horizontal(|ui| {
            ui.label(bold("SimulVoice", 19.0).color(TEXT));
            ui.label(RichText::new("会议同声传译").size(12.5).color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (text, dot) = match phase {
                    Phase::Idle => ("未开始", None),
                    Phase::Preparing => ("准备中", Some(Color32::from_rgb(234, 160, 40))),
                    Phase::Running => ("翻译中", Some(OK_GREEN)),
                    Phase::Stopping => ("停止中", Some(MUTED)),
                };
                let dot = dot.map(|c| {
                    if phase == Phase::Running {
                        let t = ui.input(|i| i.time) as f32;
                        ui.ctx().request_repaint_after(Duration::from_millis(50));
                        c.gamma_multiply(0.55 + 0.45 * (t * 2.6).sin().abs())
                    } else {
                        c
                    }
                });
                pill(ui, text, if dot.is_some() { TEXT } else { MUTED }, TILE, dot);
            });
        });
        ui.add_space(4.0);
    }

    fn language_card(&mut self, ui: &mut egui::Ui, locked: bool) {
        card(ui, "", |ui| {
            ui.add_enabled_ui(!locked, |ui| {
                let gap = ui.spacing().item_spacing.x;
                let swap_w = 30.0;
                let col = ((ui.available_width() - swap_w - gap * 2.0) / 2.0).floor();
                // ComboBox::width 是内容宽，外框还要加图标和左右内边距
                let combo_w = (col - 2.0).max(80.0);
                let mut changed = false;
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(egui::vec2(col, 50.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.label(RichText::new("对方说").size(12.0).color(MUTED));
                        changed |= lang_combo(ui, "theirs", &mut self.cfg.languages.theirs, combo_w);
                    });
                    ui.allocate_ui_with_layout(egui::vec2(swap_w, 50.0), egui::Layout::top_down(egui::Align::Center), |ui| {
                        ui.add_space(18.0);
                        if ui
                            .add_sized([swap_w, 28.0], pill_button(RichText::new("↔").size(14.0).color(MUTED), CARD))
                            .on_hover_text("互换两种语言")
                            .clicked()
                        {
                            std::mem::swap(&mut self.cfg.languages.mine, &mut self.cfg.languages.theirs);
                            std::mem::swap(&mut self.cfg.tts.voice, &mut self.cfg.tts.voice_reverse);
                            changed = true;
                        }
                    });
                    ui.allocate_ui_with_layout(egui::vec2(col, 50.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.label(RichText::new("我说").size(12.0).color(MUTED));
                        changed |= lang_combo(ui, "mine", &mut self.cfg.languages.mine, combo_w);
                    });
                });
                if changed {
                    self.cfg.apply_languages();
                }
            });
            if self.cfg.languages.mine == self.cfg.languages.theirs {
                ui.label(RichText::new("两边选了同一种语言，不需要翻译").size(12.0).color(DANGER));
            }
        });
    }

    fn mode_card(&mut self, ui: &mut egui::Ui, locked: bool) {
        let mine = self.cfg.mine().label;
        let theirs = self.cfg.theirs().label;
        card(ui, "翻译方式", |ui| {
            ui.add_enabled_ui(!locked, |ui| {
                let sel = if self.cfg.audio.duplex { 1 } else { 0 };
                if let Some(i) = segmented(ui, "mode", sel, &["只听翻译", "双向翻译"]) {
                    self.cfg.audio.duplex = i == 1;
                    self.cfg.audio.mode = "both".into();
                }
            });
            let desc = if self.cfg.audio.duplex {
                format!("对方的{theirs}译成{mine}念给你，你说的{mine}译成{theirs}送给对方")
            } else {
                format!("对方说的{theirs}实时译成{mine}念给你听")
            };
            ui.label(RichText::new(desc).size(12.0).color(MUTED));
            if self.cfg.audio.duplex {
                match cable_device(&self.outputs) {
                    Some(_) => ok_line(ui, "已检测到虚拟麦克风：在会议软件里把麦克风选成 CABLE Output"),
                    None => hint_box(
                        ui,
                        "让对方听到你的译音，需要先装免费的虚拟麦克风 VB-CABLE（装好后重启本程序）。\
                         没装也能用，你的话会以字幕显示。建议戴耳机。",
                        |ui| {
                            if ui.add(pill_button(bold("打开下载页", 12.0).color(Color32::WHITE), INK)).clicked() {
                                let _ = std::process::Command::new("explorer")
                                    .arg("https://vb-audio.com/Cable/")
                                    .spawn();
                            }
                        },
                    ),
                }
            }
        });
    }

    fn source_card(&mut self, ui: &mut egui::Ui, locked: bool) {
        card(ui, "收听来源", |ui| {
            ui.add_enabled_ui(!locked, |ui| {
                ui.horizontal(|ui| {
                    let current = if self.cfg.audio.input == "pid" && !self.cfg.audio.process.is_empty() {
                        meeting_label(&self.cfg.audio.process)
                            .map(str::to_string)
                            .unwrap_or_else(|| self.cfg.audio.process.clone())
                    } else if self.cfg.audio.input == "mic" {
                        "麦克风".to_string()
                    } else {
                        "整个电脑的声音".to_string()
                    };
                    let procs = self.processes.clone();
                    combo("source")
                        .width((ui.available_width() - 58.0).max(160.0))
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(self.cfg.audio.input == "loopback", "整个电脑的声音")
                                .clicked()
                            {
                                self.cfg.audio.input = "loopback".into();
                                self.cfg.audio.pid = 0;
                                self.cfg.audio.process.clear();
                            }
                            let mut seen = std::collections::HashSet::new();
                            let (meet, other): (Vec<_>, Vec<_>) =
                                procs.iter().partition(|(_, n, _)| meeting_label(n).is_some());
                            for (pid, name, title) in meet.into_iter().chain(other) {
                                if name.is_empty() || !seen.insert(name.to_ascii_lowercase()) {
                                    continue;
                                }
                                let label = match meeting_label(name) {
                                    Some(l) => l.to_string(),
                                    None => {
                                        let t: String = title.chars().take(22).collect();
                                        format!("{name} · {t}")
                                    }
                                };
                                let sel = self.cfg.audio.input == "pid"
                                    && self.cfg.audio.process.eq_ignore_ascii_case(name);
                                if ui.selectable_label(sel, label).clicked() {
                                    self.cfg.audio.input = "pid".into();
                                    self.cfg.audio.pid = *pid;
                                    self.cfg.audio.process = name.clone();
                                }
                            }
                        });
                    if ui
                        .add_sized([50.0, 28.0], pill_button(RichText::new("刷新").size(12.5).color(TEXT), CARD))
                        .on_hover_text("重新查找正在运行的软件和声音设备")
                        .clicked()
                    {
                        self.processes = crate::capture::list_window_processes();
                        self.outputs = crate::sink::list_output_devices().unwrap_or_default();
                        self.inputs = crate::sink::list_input_devices().unwrap_or_default();
                    }
                });
            });
            let tip = if self.cfg.audio.input == "pid" && !self.cfg.audio.process.is_empty() {
                "只收这个软件的声音，念出来的译文不会被重复识别"
            } else {
                "会收到电脑里所有声音。先打开会议软件，再在这里选中它效果更好"
            };
            ui.label(RichText::new(tip).size(12.0).color(MUTED));
        });
    }

    /// 后台查一次新版本。manual=false（启动时自动查）失败或已是最新都不打扰用户
    fn check_update(&self, ctx: &egui::Context, manual: bool) {
        if let Ok(mut g) = self.update.lock() {
            if matches!(*g, UpdateState::Checking) {
                return;
            }
            *g = UpdateState::Checking;
        }
        let state = self.update.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = crate::update::check();
            crate::update::mark_checked();
            let next = match result {
                Ok(Some(r)) => UpdateState::Available(r),
                Ok(None) if manual => UpdateState::UpToDate,
                Err(e) if manual => UpdateState::Failed(e.to_string()),
                Err(e) => {
                    eprintln!("[更新] 自动检查失败: {e}");
                    UpdateState::Idle
                }
                Ok(None) => UpdateState::Idle,
            };
            if let Ok(mut g) = state.lock() {
                *g = next;
            }
            ctx.request_repaint();
        });
    }

    /// 有新版本时在标题下面提示一条
    fn update_banner(&mut self, ui: &mut egui::Ui) {
        let state = self.update.lock().map(|g| g.clone()).unwrap_or(UpdateState::Idle);
        let UpdateState::Available(rel) = state else {
            return;
        };
        let mut dismiss = false;
        egui::Frame::none()
            .fill(TILE)
            .rounding(TILE_R)
            .inner_margin(egui::Margin::symmetric(16.0, 12.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.horizontal(|ui| {
                    ui.label(bold(format!("有新版本 {}", rel.version), 13.5).color(TEXT));
                    ui.label(RichText::new(format!("当前 {}", crate::update::CURRENT)).size(12.0).color(MUTED));
                });
                ui.horizontal(|ui| {
                    if ui.add(pill_button(bold("下载新版", 12.5).color(Color32::WHITE), INK)).clicked() {
                        open_url(&rel.download);
                    }
                    if ui.add(pill_button(RichText::new("更新说明").size(12.5).color(TEXT), CARD)).clicked() {
                        open_url(&rel.page);
                    }
                    if ui.add(pill_button(RichText::new("以后再说").size(12.5).color(MUTED), TILE)).clicked() {
                        dismiss = true;
                    }
                });
            });
        if dismiss {
            if let Ok(mut g) = self.update.lock() {
                *g = UpdateState::Idle;
            }
        }
    }

    fn version_card(&mut self, ui: &mut egui::Ui) {
        let state = self.update.lock().map(|g| g.clone()).unwrap_or(UpdateState::Idle);
        card(ui, "版本", |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("SimulVoice {}", crate::update::CURRENT)).size(13.0).color(TEXT));
                let checking = matches!(state, UpdateState::Checking);
                let label = if checking { "正在检查…" } else { "检查更新" };
                let btn = pill_button(RichText::new(label).size(12.5).color(TEXT), CARD);
                if ui.add_enabled(!checking, btn).clicked() {
                    self.check_update(ui.ctx(), true);
                }
            });
            match &state {
                UpdateState::UpToDate => ok_line(ui, "已是最新版本"),
                UpdateState::Available(r) => {
                    ui.label(RichText::new(format!("有新版本 {}，在窗口顶部可以下载", r.version)).size(12.0).color(OK_GREEN));
                }
                UpdateState::Failed(e) => {
                    ui.label(RichText::new(format!("检查失败：{e}")).size(12.0).color(DANGER));
                }
                UpdateState::Idle | UpdateState::Checking => {}
            }
            ui.checkbox(&mut self.cfg.ui.check_updates, "打开时自动检查（每天最多一次）");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(RichText::new("个人和非商业使用免费，商业使用需要授权 ·").size(12.0).color(MUTED));
                let link = ui.add(
                    egui::Label::new(RichText::new("授权说明").size(12.0).color(TEXT).underline())
                        .sense(egui::Sense::click()),
                );
                if link.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    open_url(&format!("{}#许可与授权", crate::update::REPO_URL));
                }
            });
        });
    }

    fn summary(&self, ui: &mut egui::Ui) {
        let source = if self.cfg.audio.input == "pid" && !self.cfg.audio.process.is_empty() {
            meeting_label(&self.cfg.audio.process)
                .map(str::to_string)
                .unwrap_or_else(|| self.cfg.audio.process.clone())
        } else {
            "整个电脑的声音".to_string()
        };
        let mode = if self.cfg.audio.duplex { "双向翻译" } else { "只听翻译" };
        card(ui, "", |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    bold(format!("{} → {}", self.cfg.theirs().label, self.cfg.mine().label), 13.5)
                        .color(TEXT),
                );
                ui.label(RichText::new(format!("·  {mode}  ·  {source}")).color(MUTED));
            });
        });
    }

    fn start_area(&mut self, ui: &mut egui::Ui, phase: Phase) {
        let (error, info, progress) = self
            .session
            .lock()
            .map(|s| (s.error.clone(), s.info.clone(), s.progress.clone()))
            .unwrap_or_default();
        if phase == Phase::Idle && !self.needs.is_empty() {
            let mb: u32 = self.needs.iter().map(|n| n.size_mb).sum();
            let names: Vec<&str> = self.needs.iter().map(|n| n.label.as_str()).collect();
            ui.label(
                RichText::new(format!(
                    "首次使用会自动下载：{}（约 {} MB，仅一次）",
                    names.join("、"),
                    mb
                ))
                .size(12.0)
                .color(MUTED),
            );
        }
        let same_lang = self.cfg.languages.mine == self.cfg.languages.theirs;
        let (text, fg, fill, enabled) = match phase {
            Phase::Idle => ("开始翻译", Color32::WHITE, INK, !same_lang),
            Phase::Preparing | Phase::Running => ("停止翻译", TEXT, TRACK, true),
            Phase::Stopping => ("正在停止…", MUTED, TRACK, false),
        };
        ui.add_space(2.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 44.0), egui::Sense::hover());
        if phase == Phase::Idle && enabled {
            let shadow = egui::epaint::Shadow {
                offset: egui::vec2(0.0, 8.0),
                blur: 22.0,
                spread: -4.0,
                color: Color32::from_rgba_unmultiplied(23, 24, 26, 60),
            };
            ui.painter().add(shadow.as_shape(rect, egui::Rounding::same(22.0)));
        }
        let btn = pill_button(bold(text, 14.5).color(fg), fill);
        if ui.add_enabled_ui(enabled, |ui| ui.put(rect, btn)).inner.clicked() {
            match phase {
                Phase::Idle => self.start(),
                _ => self.stop(),
            }
        }
        ui.add_space(2.0);
        // 采集线程开始出数据 = 模型已加载、真正在听
        let listening = phase == Phase::Running && crate::capture::last_tick() > 0;
        if listening {
            ui.label(RichText::new("正在听 · 每句说完停顿约半秒出译文").size(12.0).color(OK_GREEN));
        } else if !progress.is_empty() && phase != Phase::Idle {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(&progress).size(12.0).color(MUTED));
            });
            if progress.contains("下载") {
                let pct = crate::models::DOWNLOAD_PCT.load(Ordering::Relaxed) as f32 / 100.0;
                ui.add(egui::ProgressBar::new(pct).fill(INK).rounding(100.0).desired_height(6.0));
                ui.label(RichText::new(format!("{:.0}%", pct * 100.0)).size(12.0).color(MUTED));
            }
        }
        if !info.is_empty() && phase == Phase::Running {
            ui.label(RichText::new(&info).size(12.0).color(WARN_FG));
        }
        if !error.is_empty() {
            egui::Frame::none()
                .fill(DANGER_SOFT)
                .rounding(12.0)
                .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(RichText::new(&error).size(12.5).color(DANGER)).wrap());
                });
        }
    }

    fn transcript_card(&mut self, ui: &mut egui::Ui, phase: Phase) {
        let s = self.subs.lock().map(|g| g.clone()).unwrap_or_default();
        if phase == Phase::Idle && s.history.is_empty() {
            return;
        }
        let mine = self.cfg.mine().label;
        let theirs = self.cfg.theirs().label;
        card(ui, "字幕", |ui| {
            if !s.notice.is_empty() {
                ui.label(RichText::new(&s.notice).size(12.0).color(DANGER));
            }
            // 运行中字幕区撑满窗口剩余高度（至少 240），停下来后回到紧凑高度
            let h = if phase == Phase::Idle {
                240.0
            } else {
                (ui.ctx().screen_rect().height() - 250.0).max(200.0)
            };
            egui::ScrollArea::vertical()
                .max_height(h)
                .min_scrolled_height(h)
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if s.history.is_empty() && s.in_src.is_empty() && s.out_src.is_empty() {
                        ui.label(
                            RichText::new(format!("对方说{theirs}时，这里会出现{mine}译文"))
                                .size(12.5)
                                .color(MUTED),
                        );
                    }
                    for h in &s.history {
                        if h.own {
                            own_line(ui, &h.src);
                        } else {
                            line(ui, h.outbound, &h.src, &h.dst);
                        }
                    }
                    // 实时引擎（Gemini Live 等）不走历史，直接显示当前句
                    let last_in = s.history.iter().rev().find(|h| !h.outbound);
                    if !s.in_src.is_empty() && last_in.map(|h| h.src != s.in_src).unwrap_or(true) {
                        line(ui, false, &s.in_src, if s.in_dst.is_empty() { "…" } else { &s.in_dst });
                    }
                    let last_out = s.history.iter().rev().find(|h| h.outbound);
                    if !s.out_src.is_empty() && last_out.map(|h| h.src != s.out_src).unwrap_or(true) {
                        line(ui, true, &s.out_src, if s.out_dst.is_empty() { "…" } else { &s.out_dst });
                    }
                });
        });

        fn line(ui: &mut egui::Ui, outbound: bool, src: &str, dst: &str) {
            let (who, fg, bg) = if outbound {
                ("我", Color32::WHITE, INK)
            } else {
                ("对方", TEXT, CARD)
            };
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.horizontal(|ui| {
                pill(ui, who, fg, bg, None);
                ui.add(egui::Label::new(RichText::new(src).size(12.5).color(MUTED)).wrap());
            });
            ui.add(egui::Label::new(RichText::new(dst).size(15.5).color(TEXT)).wrap());
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(2.0);
        }

        /// 对方那一路说的就是我的语言：直接给原话，标一句"原话"
        fn own_line(ui: &mut egui::Ui, text: &str) {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.horizontal(|ui| {
                pill(ui, "对方", TEXT, CARD, None);
                ui.label(RichText::new("原话，未翻译").size(12.0).color(INK_5));
            });
            ui.add(egui::Label::new(RichText::new(text).size(15.5).color(TEXT)).wrap());
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(2.0);
        }
    }

    fn advanced(&mut self, ui: &mut egui::Ui, locked: bool) {
        // 不用 CollapsingHeader（会缩进加竖线）；CollapsingState 只管开合动画，头部自己画
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            ui.id().with("advanced"),
            self.show_advanced,
        );
        let openness = state.openness(ui.ctx());
        let clicked = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (rect, icon) = ui.allocate_exact_size(egui::vec2(10.0, 16.0), egui::Sense::click());
                let text = ui.add(
                    egui::Label::new(RichText::new("高级设置").size(13.0).color(MUTED)).sense(egui::Sense::click()),
                );
                let hot = icon.hovered() || text.hovered();
                if hot {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                // 右指三角绕中心转 0→90°
                let c = rect.center();
                let rot = egui::emath::Rot2::from_angle(openness * std::f32::consts::FRAC_PI_2);
                let pts = [egui::vec2(-2.0, -3.5), egui::vec2(-2.0, 3.5), egui::vec2(2.5, 0.0)]
                    .map(|p| c + rot * p)
                    .to_vec();
                let color = if hot { TEXT } else { MUTED };
                ui.painter().add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
                icon.clicked() || text.clicked()
            })
            .inner;
        if clicked {
            state.toggle(ui);
        }
        state.show_body_unindented(ui, |ui| {
            ui.add_enabled_ui(!locked, |ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                self.advanced_body(ui);
            });
        });
    }

    fn advanced_body(&mut self, ui: &mut egui::Ui) {
        card(ui, "引擎", |ui| {
            egui::Grid::new("engines").num_columns(2).min_col_width(LABEL_W).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("语音识别");
                option_combo(ui, "stt", &mut self.cfg.stt.provider, &[
                    ("local", "本地（推荐，自动选模型）"),
                    ("soniox", "Soniox 在线"),
                    ("openai", "OpenAI 在线"),
                    ("zipformer", "Zipformer（旧）"),
                    ("vosk", "Vosk（旧）"),
                ]);
                ui.end_row();
                ui.label("翻译");
                option_combo(ui, "tr", &mut self.cfg.translate.provider, &[
                    ("local", "本地（推荐，自动选模型）"),
                    ("gemini", "Gemini 在线"),
                    ("openai", "OpenAI 在线"),
                    ("qwen", "通义千问 在线"),
                    ("ollama", "Ollama 本地大模型"),
                    ("custom", "自定义接口"),
                    ("none", "不翻译（只转写）"),
                ]);
                ui.end_row();
                if matches!(self.cfg.translate.provider.as_str(), "gemini" | "openai" | "qwen" | "ollama" | "custom") {
                    ui.label("模型");
                    ui.add(egui::TextEdit::singleline(&mut self.cfg.translate.model).desired_width(220.0));
                    ui.end_row();
                }
                if matches!(self.cfg.translate.provider.as_str(), "ollama" | "custom") {
                    ui.label("接口地址");
                    ui.add(egui::TextEdit::singleline(&mut self.cfg.translate.endpoint).desired_width(220.0));
                    ui.end_row();
                }
                ui.label("朗读");
                option_combo(ui, "tts", &mut self.cfg.tts.provider, &[
                    ("edge", "Edge 在线（声音自然）"),
                    ("sherpa", "本地 Piper（离线）"),
                    ("sapi", "Windows 自带语音"),
                    ("openai", "OpenAI 在线"),
                    ("none", "不朗读（只看字幕）"),
                ]);
                ui.end_row();
                ui.label("端到端实时");
                option_combo(ui, "rt", &mut self.cfg.realtime.provider, &[
                    ("none", "关闭（用上面三个引擎）"),
                    ("gemini-live", "Gemini Live"),
                    ("palabra", "Palabra"),
                    ("volcengine", "火山引擎"),
                ]);
                ui.end_row();
            });
        });
        if self.cfg.tts.provider == "edge" {
            let (mine, theirs) = (self.cfg.mine(), self.cfg.theirs());
            card(ui, "音色", |ui| {
                egui::Grid::new("voices").num_columns(2).min_col_width(LABEL_W).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label(format!("念给我（{}）", mine.label));
                    let opts: Vec<String> = crate::lang::edge_voices(mine.code).iter().map(|s| s.to_string()).collect();
                    device_combo(ui, "voice", &mut self.cfg.tts.voice, &opts, mine.edge_voice);
                    ui.end_row();
                    ui.label(format!("念给对方（{}）", theirs.label));
                    let opts: Vec<String> = crate::lang::edge_voices(theirs.code).iter().map(|s| s.to_string()).collect();
                    device_combo(ui, "voicerev", &mut self.cfg.tts.voice_reverse, &opts, theirs.edge_voice);
                    ui.end_row();
                });
            });
        }
        card(ui, "声音设备", |ui| {
            egui::Grid::new("devices").num_columns(2).min_col_width(LABEL_W).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("译文播放到");
                device_combo(ui, "outdev", &mut self.cfg.audio.output_device, &self.outputs, "系统默认");
                ui.end_row();
                ui.label("我的麦克风");
                device_combo(ui, "micdev", &mut self.cfg.audio.mic_device, &self.inputs, "系统默认");
                ui.end_row();
                ui.label("发给对方的设备");
                device_combo(ui, "remotedev", &mut self.cfg.audio.remote_output_device, &self.outputs, "自动（VB-CABLE）");
                ui.end_row();
            });
        });
        card(ui, "其他", |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.spacing_mut().interact_size.y = 22.0;
            ui.checkbox(&mut self.cfg.dsp.denoise, "降噪");
            ui.checkbox(&mut self.cfg.dsp.aec, "回声消除");
            ui.checkbox(&mut self.cfg.dsp.duck, "朗读译文时调低其他声音");
            ui.checkbox(&mut self.cfg.audio.barge_in, "对方开口时停止正在念的译文（实验性）");
            let own_label = format!("对方那边有人说{}时不翻译，只显示原话", self.cfg.mine().label);
            ui.checkbox(&mut self.cfg.audio.skip_own_language, own_label);
            ui.checkbox(&mut self.cfg.ui.keep_translation, "保存译文语音（Ctrl+Alt+F6 回放）");
            ui.checkbox(&mut self.cfg.ui.record_source, "录制原始声音");
            ui.checkbox(&mut self.cfg.ui.minimize_to_tray, "最小化时收到右下角托盘");
            ui.horizontal(|ui| {
                ui.label("额外字幕语言");
                ui.add(
                    egui::TextEdit::singleline(&mut self.extra_text)
                        .hint_text("如 ja, fr（只显示不朗读）")
                        .desired_width(220.0),
                );
            });
            ui.label(
                RichText::new("快捷键：Ctrl+Alt+F8 停止当前译文 · F9 静音 · F10 停止翻译")
                    .size(12.0)
                    .color(MUTED),
            );
        });
        self.version_card(ui);
        card(ui, "数据", |ui| {
            ui.label(RichText::new(format!("模型与录音保存在 {}", crate::paths::data_dir().display())).size(12.0).color(MUTED));
            ui.horizontal(|ui| {
                if ui.add(pill_button(RichText::new("打开数据目录").size(12.5).color(TEXT), CARD)).clicked() {
                    let _ = std::process::Command::new("explorer").arg(crate::paths::data_dir()).spawn();
                }
                if ui.add(pill_button(RichText::new("更改数据目录…").size(12.5).color(TEXT), CARD)).clicked() {
                    if let Some(dir) = pick_folder() {
                        self.status = match crate::paths::set_data_dir(&dir) {
                            Ok(_) => {
                                crate::paths::ensure_layout();
                                self.path = crate::paths::default_config_path().display().to_string();
                                self.needs_key.clear();
                                format!("数据目录已切换到 {dir}（模型需重新下载或手动迁移）")
                            }
                            Err(e) => format!("设置失败: {e}"),
                        };
                    }
                }
                if ui.add(pill_button(RichText::new("保存设置").size(12.5).color(TEXT), CARD)).clicked() && self.save() {
                    self.status = "设置已保存".into();
                }
            });
            if !self.status.is_empty() {
                ui.label(RichText::new(&self.status).size(12.0).color(MUTED));
            }
        });
    }
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        #[cfg(windows)]
        {
            if !self.hwnd_registered {
                use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                if let Ok(h) = frame.window_handle() {
                    if let RawWindowHandle::Win32(w) = h.as_raw() {
                        crate::tray::set_main_window(w.hwnd.get());
                        self.hwnd_registered = true;
                    }
                }
            }
            crate::tray::MINIMIZE_TO_TRAY.store(self.cfg.ui.minimize_to_tray, Ordering::Relaxed);
        }
        #[cfg(not(windows))]
        let _ = frame;
        // SIMULVOICE_UPDATE_JSON（开发用假数据）时不受每天一次和截图模式的限制
        let fake_update = std::env::var("SIMULVOICE_UPDATE_JSON").is_ok();
        if !std::mem::replace(&mut self.update_scheduled, true)
            && self.cfg.ui.check_updates
            && (fake_update || (self.snapshot.is_none() && crate::update::auto_due()))
        {
            self.check_update(ctx, false);
        }
        if let Some(snap) = &mut self.snapshot {
            if snap.tick(ctx) {
                crate::STOP.store(true, Ordering::SeqCst);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            // 关窗口 = 停止翻译；不在这里阻塞等待，进程退出会结束后台线程
            crate::STOP.store(true, Ordering::SeqCst);
            let _ = self.save();
            #[cfg(windows)]
            crate::tray::shutdown();
        }
        if std::mem::take(&mut self.autostart) && self.phase() == Phase::Idle {
            self.start();
        }
        let phase = self.phase();
        if phase != Phase::Idle {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        self.refresh_needs();
        let locked = phase != Phase::Idle;
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin::symmetric(0.0, 0.0)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    let max_w = 520.0f32;
                    let pad = ((ui.available_width() - max_w) / 2.0).max(16.0);
                    egui::Frame::none()
                        .inner_margin(egui::Margin { left: pad, right: pad, top: 14.0, bottom: 18.0 })
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 10.0;
                            self.header(ui, phase);
                            self.update_banner(ui);
                            if locked {
                                // 翻译进行中：设置收成一行，把空间留给字幕
                                self.summary(ui);
                            } else {
                                self.language_card(ui, locked);
                                self.mode_card(ui, locked);
                                self.source_card(ui, locked);
                            }
                            self.start_area(ui, phase);
                            self.transcript_card(ui, phase);
                            if !locked {
                                self.advanced(ui, locked);
                            }
                        });
                });
            });
    }
}
