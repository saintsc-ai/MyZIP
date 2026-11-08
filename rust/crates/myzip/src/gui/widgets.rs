//! 여러 창이 함께 쓰는 부품: 아이콘, 진행률, 암호 입력, 알림, 글자 다듬기.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::NaiveDateTime;
use eframe::egui::{self, Color32, RichText, TextureHandle, TextureOptions};

use crate::notify::Kind;

// ------------------------------------------------------------------ 아이콘

macro_rules! icon_bytes {
    ($name:literal) => {
        include_bytes!(concat!("../../../../../resources/icons/", $name, ".png"))
    };
}

/// 저장소의 resources/icons 에 있는 PNG 들. exe 에 넣어 둔다.
#[derive(Clone)]
pub struct Icons {
    pub app: TextureHandle,
    pub new: TextureHandle,
    pub open: TextureHandle,
    pub extract: TextureHandle,
    pub extract_here: TextureHandle,
    pub test: TextureHandle,
    pub settings: TextureHandle,
    pub zip: TextureHandle,
    pub rar: TextureHandle,
    pub seven: TextureHandle,
    pub tar: TextureHandle,
    pub tgz: TextureHandle,
    pub file: TextureHandle,
}

pub const APP_PNG: &[u8] = icon_bytes!("app");

fn texture(ctx: &egui::Context, name: &str, bytes: &[u8], size: u32) -> TextureHandle {
    let image = image::load_from_memory(bytes)
        .map(|img| {
            img.resize(size, size, image::imageops::FilterType::Lanczos3)
                .to_rgba8()
        })
        .unwrap_or_else(|_| image::RgbaImage::new(1, 1));
    let (w, h) = image.dimensions();
    let color = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], image.as_raw());
    ctx.load_texture(name, color, TextureOptions::LINEAR)
}

impl Icons {
    fn load(ctx: &egui::Context) -> Self {
        // 목록 아이콘은 작게 줄여 둔다. 256px 원본을 매번 줄여 그리면 흐리다.
        let small = |name, bytes| texture(ctx, name, bytes, 32);
        let tool = |name, bytes| texture(ctx, name, bytes, 48);
        Self {
            app: texture(ctx, "app", icon_bytes!("app"), 96),
            new: tool("act-new", icon_bytes!("act-new")),
            open: tool("act-open", icon_bytes!("act-open")),
            extract: tool("act-extract", icon_bytes!("act-extract")),
            extract_here: tool("act-extract-here", icon_bytes!("act-extract-here")),
            test: tool("act-test", icon_bytes!("act-test")),
            settings: tool("act-settings", icon_bytes!("act-settings")),
            zip: small("zip", icon_bytes!("zip")),
            rar: small("rar", icon_bytes!("rar")),
            seven: small("7z", icon_bytes!("7z")),
            tar: small("tar", icon_bytes!("tar")),
            tgz: small("tgz", icon_bytes!("tgz")),
            file: small("file", icon_bytes!("file")),
        }
    }

    /// 한 번만 읽고 egui 메모리에 넣어 둔다.
    pub fn get(ctx: &egui::Context) -> Arc<Icons> {
        let id = egui::Id::unique("myzip-icons");
        if let Some(icons) = ctx.data(|d| d.get_temp::<Arc<Icons>>(id)) {
            return icons;
        }
        let icons = Arc::new(Icons::load(ctx));
        ctx.data_mut(|d| d.insert_temp(id, icons.clone()));
        icons
    }

    /// 파일 이름에 맞는 아이콘. 우리가 가진 압축 형식만 따로 그린다.
    pub fn for_name(&self, name: &str) -> &TextureHandle {
        let lower = name.to_lowercase();
        let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        match ext {
            "zip" | "zipx" => &self.zip,
            "rar" | "cbr" => &self.rar,
            "7z" | "cb7" => &self.seven,
            "tar" => &self.tar,
            "tgz" | "gz" | "bz2" | "xz" | "tbz2" | "txz" => &self.tgz,
            _ => &self.file,
        }
    }
}

/// 폴더 아이콘. 기본 글꼴에 📁 가 없어서 직접 그린다.
pub fn folder_icon(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
    paint_folder(ui.painter(), rect);
    response
}

fn paint_folder(painter: &egui::Painter, rect: egui::Rect) {
    let back = Color32::from_rgb(0xDB, 0xA2, 0x2C);
    let front = Color32::from_rgb(0xF4, 0xC4, 0x50);
    let r = rect.shrink2(egui::vec2(0.5, 2.5));
    // 뒤판과 손잡이
    painter.rect_filled(
        egui::Rect::from_min_size(r.min, egui::vec2(7.0, 4.0)),
        1.0,
        back,
    );
    painter.rect_filled(
        egui::Rect::from_min_max(egui::pos2(r.min.x, r.min.y + 1.5), r.max),
        1.5,
        back,
    );
    // 앞판
    painter.rect_filled(
        egui::Rect::from_min_max(egui::pos2(r.min.x, r.min.y + 3.5), r.max),
        1.5,
        front,
    );
}

/// 표 칸의 아이콘.
pub enum CellIcon<'a> {
    None,
    Folder,
    Texture(&'a TextureHandle),
}

/// 표 칸에 글자를 '그린다'. 위젯을 쓰지 않으므로 클릭과 더블클릭이 모두 행으로 간다.
/// (Label 을 쓰면 글자 위의 더블클릭을 Label 이 가로챈다.)
pub fn paint_cell(ui: &mut egui::Ui, icon: CellIcon<'_>, text: &str, right: bool) {
    let rect = ui.available_rect_before_wrap();
    let painter = ui.painter_at(rect);
    let mut left = rect.left() + 2.0;
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(left + 8.0, rect.center().y),
        egui::vec2(16.0, 16.0),
    );
    match icon {
        CellIcon::None => {}
        CellIcon::Folder => {
            paint_folder(&painter, icon_rect);
            left += 22.0;
        }
        CellIcon::Texture(tex) => {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(tex.id(), icon_rect, uv, Color32::WHITE);
            left += 22.0;
        }
    }
    if text.is_empty() {
        return;
    }
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().text_color();
    let max_width = (rect.right() - left - 4.0).max(8.0);
    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::TextFormat::simple(font, color),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width);
    let galley = painter.layout_job(job);
    let x = if right {
        rect.right() - 4.0 - galley.size().x
    } else {
        left
    };
    let y = rect.center().y - galley.size().y / 2.0;
    painter.galley(egui::pos2(x, y), galley, color);
}

// ------------------------------------------------------------------ 글자

/// 긴 경로를 가운데를 줄여 보여 준다.
pub fn elide(text: &str, limit: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= limit {
        return text.to_owned();
    }
    let head = limit / 2 - 2;
    let tail = limit - head - 3;
    let mut out: String = chars[..head].iter().collect();
    out.push_str("...");
    out.extend(&chars[chars.len() - tail..]);
    out
}

pub fn duration(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    if seconds < 60 {
        return format!("{seconds}초");
    }
    let (minutes, secs) = (seconds / 60, seconds % 60);
    if minutes < 60 {
        return format!("{minutes}분 {secs}초");
    }
    format!("{}시간 {}분", minutes / 60, minutes % 60)
}

pub fn format_time(time: Option<NaiveDateTime>) -> String {
    time.map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).weak()
}

// ------------------------------------------------------------------ 진행률

/// 작업 스레드가 쓰고 UI 가 읽는 진행 상황.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// "자료.zip 푸는 중" 같은 머리말
    pub headline: String,
    /// "(2/5)" 처럼 여러 작업 중 몇 번째인지
    pub step: String,
    pub percent: f32,
    pub current: String,
    pub done: u64,
    pub total: u64,
    /// 지금 작업을 시작한 시각. 남은 시간 어림에 쓴다.
    pub started: Option<Instant>,
}

pub type Shared = Arc<Mutex<Snapshot>>;

/// 작업 스레드에 넘길 Progress. 콜백은 화면 갱신을 초당 20번으로 줄인다.
pub fn tracked_progress(
    ctx: &egui::Context,
    shared: &Shared,
    token: myzip_core::CancelToken,
) -> myzip_core::Progress {
    let mut progress = myzip_core::Progress::with_token(token);
    let ctx = ctx.clone();
    let shared = shared.clone();
    let mut last = Instant::now() - std::time::Duration::from_secs(1);
    progress.set_callback(move |p| {
        if last.elapsed().as_millis() < 50 && p.done_files < p.total_files {
            return;
        }
        last = Instant::now();
        if let Ok(mut s) = shared.lock() {
            s.percent = p.percent() as f32;
            p.current.clone_into(&mut s.current);
            s.done = p.done_files;
            s.total = p.total_files;
        }
        ctx.request_repaint();
    });
    progress
}

/// 진행률 화면. 취소를 눌렀으면 true.
pub fn progress_view(
    ui: &mut egui::Ui,
    snapshot: &Snapshot,
    started: Instant,
    cancelling: bool,
) -> bool {
    let mut cancel = false;
    ui.label(RichText::new(&snapshot.headline).strong().size(15.0));
    if !snapshot.step.is_empty() {
        ui.label(muted(&snapshot.step));
    }
    let current = if cancelling {
        "작업을 정리하고 있습니다...".to_owned()
    } else if snapshot.current.is_empty() {
        "준비 중...".to_owned()
    } else {
        elide(&snapshot.current, 64)
    };
    ui.add(egui::Label::new(current).truncate());
    ui.add(
        egui::ProgressBar::new(snapshot.percent / 100.0)
            .show_percentage()
            .desired_height(18.0),
    );

    let elapsed = snapshot.started.unwrap_or(started).elapsed().as_secs_f64();
    let mut parts = Vec::new();
    if snapshot.total > 0 {
        parts.push(format!("{} / {} 개", snapshot.done, snapshot.total));
    }
    if snapshot.percent > 2.0 && elapsed > 1.0 {
        let remaining = elapsed * f64::from(100.0 - snapshot.percent) / f64::from(snapshot.percent);
        parts.push(format!("남은 시간 약 {}", duration(remaining)));
    }
    parts.push(format!("경과 {}", duration(elapsed)));
    ui.label(muted(parts.join("  ·  ")));

    ui.add_space(4.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let label = if cancelling {
            "취소하는 중..."
        } else {
            "취소"
        };
        if ui
            .add_enabled(!cancelling, egui::Button::new(label))
            .clicked()
            || ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            cancel = true;
        }
    });
    cancel
}

// ------------------------------------------------------------------ 암호

/// 암호 입력.
#[derive(Debug, Clone)]
pub struct PasswordPrompt {
    pub name: String,
    pub wrong: bool,
    pub input: String,
    pub show: bool,
    focused: bool,
}

pub enum PromptResult {
    Pending,
    Entered(String),
    Cancelled,
}

impl PasswordPrompt {
    pub fn new(name: impl Into<String>, wrong: bool) -> Self {
        Self {
            name: name.into(),
            wrong,
            input: String::new(),
            show: false,
            focused: false,
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) -> PromptResult {
        ui.label(RichText::new("암호 필요").strong().size(15.0));
        if self.wrong {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!(
                    "암호가 맞지 않습니다. '{}' 의 암호를 다시 입력하세요.",
                    self.name
                ),
            );
        } else {
            ui.label(format!("'{}' 은(는) 암호로 보호되어 있습니다.", self.name));
        }
        let edit = ui.add(
            egui::TextEdit::singleline(&mut self.input)
                .password(!self.show)
                .hint_text("암호")
                .desired_width(f32::INFINITY),
        );
        if !self.focused {
            edit.request_focus();
            self.focused = true;
        }
        let entered = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.checkbox(&mut self.show, "암호 보기");

        let mut result = PromptResult::Pending;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("취소").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                result = PromptResult::Cancelled;
            }
            let ok = ui.add_enabled(!self.input.is_empty(), egui::Button::new("확인"));
            if (ok.clicked() || entered) && !self.input.is_empty() {
                result = PromptResult::Entered(self.input.clone());
            }
        });
        result
    }
}

// ------------------------------------------------------------------ 알림

/// 알림 하나. 버튼을 고르면 그 번호를 돌려준다.
#[derive(Debug, Clone)]
pub struct Notice {
    pub kind: Kind,
    pub title: String,
    pub text: String,
    pub buttons: Vec<String>,
}

impl Notice {
    pub fn new(kind: Kind, title: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            text: text.into(),
            buttons: vec!["확인".into()],
        }
    }

    pub fn with_buttons(mut self, buttons: &[&str]) -> Self {
        self.buttons = buttons.iter().map(|b| (*b).to_owned()).collect();
        self
    }

    /// 누른 버튼 번호. Esc 는 마지막 버튼(닫기/취소)으로 친다.
    pub fn ui(&self, ui: &mut egui::Ui) -> Option<usize> {
        let (mark, color) = match self.kind {
            Kind::Info => ("i", Color32::from_rgb(0x2F, 0x80, 0xED)),
            Kind::Warning => ("!", Color32::from_rgb(0xE0, 0x9B, 0x1A)),
            Kind::Error => ("×", Color32::from_rgb(0xD9, 0x3B, 0x3B)),
        };
        ui.horizontal(|ui| {
            badge(ui, mark, color);
            ui.label(RichText::new(&self.title).strong().size(15.0));
        });
        ui.add_space(2.0);
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                ui.add(egui::Label::new(&self.text).wrap());
            });
        ui.add_space(6.0);
        let mut pressed = None;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (i, label) in self.buttons.iter().enumerate().rev() {
                if ui.button(label).clicked() {
                    pressed = Some(i);
                }
            }
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            pressed = Some(self.buttons.len() - 1);
        }
        if pressed.is_none()
            && self.buttons.len() == 1
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
        {
            pressed = Some(0);
        }
        pressed
    }
}

/// 동그라미 안에 글자 하나. 기본 글꼴에 ℹ ⚠ ✖ 가 없어서 직접 그린다.
fn badge(ui: &mut egui::Ui, mark: &str, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.circle_filled(rect.center(), 10.0, color);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        mark,
        egui::FontId::proportional(15.0),
        Color32::WHITE,
    );
}

/// 가운데에 띄우는 모달 틀.
pub fn modal<R>(
    ctx: &egui::Context,
    id: &str,
    width: f32,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Modal::new(egui::Id::unique(id))
        .show(ctx, |ui| {
            ui.set_width(width);
            content(ui)
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elide_keeps_ends() {
        assert_eq!(elide("짧음", 10), "짧음");
        let long = "가".repeat(30) + "끝.txt";
        let out = elide(&long, 20);
        assert_eq!(out.chars().count(), 20);
        assert!(out.ends_with("끝.txt") && out.contains("..."));
    }

    #[test]
    fn durations() {
        assert_eq!(duration(5.0), "5초");
        assert_eq!(duration(125.0), "2분 5초");
        assert_eq!(duration(3725.0), "1시간 2분");
    }
}
