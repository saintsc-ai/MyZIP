//! 압축하기 / 압축 풀기 대화상자의 입력 양식.
//!
//! 탐색기 메뉴에서 바로 뜨는 작업 창과 메인 창이 같은 양식을 쓴다.
//! Python 판 ui/compress_dialog.py, ui/extract_dialog.py 와 같은 항목, 같은 설정 키다.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};
use myzip_core::{
    ConflictAction, WRITABLE_FORMATS, default_extension, format_size, strip_archive_suffix,
};

use super::widgets::{Icons, Notice, elide, folder_icon, muted};
use crate::config;
use crate::jobs::{CompressTarget, ExtractPlan, Job, file_name, suggest_name};
use crate::notify::Kind;

/// 양식에서 누른 것.
pub enum FormResult<T> {
    Pending,
    Submit(T),
    Cancel,
}

/// 압축 강도 눈금: (값, 이름, 설명)
const LEVELS: &[(i32, &str, &str)] = &[
    (0, "저장 (압축 안 함)", "가장 빠름. 이미 압축된 파일에 적합"),
    (1, "가장 빠르게", "속도 우선"),
    (3, "빠르게", ""),
    (6, "표준", "속도와 크기의 균형 (권장)"),
    (9, "최대 압축", "가장 작지만 오래 걸림"),
];

const CONFLICTS: &[(ConflictAction, &str)] = &[
    (ConflictAction::Overwrite, "덮어쓰기"),
    (ConflictAction::Skip, "건너뛰기"),
    (ConflictAction::Rename, "다른 이름으로 저장"),
];

fn remember_dir(kind: &str, path: &Path) {
    let dir = if path.is_dir() {
        Some(path)
    } else {
        path.parent()
    };
    if let Some(dir) = dir.filter(|d| d.is_dir()) {
        let _ = config::set(&format!("{kind}/last_dir"), &dir.to_string_lossy());
    }
}

fn last_dir(kind: &str) -> Option<PathBuf> {
    let raw = config::get(&format!("{kind}/last_dir"));
    let path = PathBuf::from(raw);
    path.is_dir().then_some(path)
}

/// 확장자를 바꿔 끼운다. "자료.tar.gz" -> "자료.zip"
fn swap_extension(path: &str, new_ext: &str) -> String {
    let p = Path::new(path);
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = strip_archive_suffix(&name);
    p.with_file_name(format!("{stem}{new_ext}"))
        .to_string_lossy()
        .into_owned()
}

// ---------------------------------------------------------------- 압축하기

pub struct CompressForm {
    pub sources: Vec<PathBuf>,
    selected: Option<usize>,
    summary: String,
    target: String,
    format: usize,
    level: usize,
    password: String,
    password2: String,
    show_password: bool,
    /// 덮어쓰기 확인 중
    confirm_overwrite: bool,
    problem: Option<Notice>,
}

impl CompressForm {
    pub fn new(sources: Vec<PathBuf>) -> Self {
        let saved_format = config::get("compress/format");
        let format = WRITABLE_FORMATS
            .iter()
            .position(|(k, _)| *k == saved_format)
            .unwrap_or(0);
        let saved_level = config::get_int("compress/level");
        let level = LEVELS
            .iter()
            .position(|(v, _, _)| *v == saved_level)
            .unwrap_or(3);
        let mut form = Self {
            sources: Vec::new(),
            selected: None,
            summary: String::new(),
            target: String::new(),
            format,
            level,
            password: String::new(),
            password2: String::new(),
            show_password: false,
            confirm_overwrite: false,
            problem: None,
        };
        form.append(sources);
        form
    }

    fn format_key(&self) -> &'static str {
        WRITABLE_FORMATS[self.format].0
    }

    fn is_zip(&self) -> bool {
        self.format_key() == "zip"
    }

    fn append(&mut self, paths: Vec<PathBuf>) {
        let before = self.sources.len();
        for path in paths {
            if !self.sources.contains(&path) {
                self.sources.push(path);
            }
        }
        if self.sources.len() != before {
            self.refresh_summary();
            if self.target.trim().is_empty() {
                self.suggest_target();
            }
        }
    }

    /// 하나면 그 이름, 여러 개면 들어 있는 폴더 이름 (반디집과 같은 규칙).
    fn suggest_target(&mut self) {
        if let Some(path) = suggest_name(&self.sources, default_extension(self.format_key())) {
            self.target = path.to_string_lossy().into_owned();
        }
    }

    fn refresh_summary(&mut self) {
        let (mut files, mut folders, mut total) = (0u64, 0u64, 0u64);
        for path in &self.sources {
            if path.is_dir() {
                folders += 1;
                for entry in walkdir(path) {
                    files += 1;
                    total += entry;
                }
            } else if let Ok(meta) = path.metadata() {
                files += 1;
                total += meta.len();
            }
        }
        let mut bits = Vec::new();
        if folders > 0 {
            bits.push(format!("폴더 {folders}개"));
        }
        bits.push(format!("파일 {files}개"));
        bits.push(format!("총 {}", format_size(total)));
        self.summary = bits.join("  ·  ");
    }

    fn start_dir(&self) -> Option<PathBuf> {
        match self.sources.first() {
            Some(first) if first.is_dir() => Some(first.clone()),
            Some(first) => first.parent().map(Path::to_path_buf),
            None => last_dir("compress"),
        }
    }

    fn mismatch(&self) -> bool {
        self.is_zip() && !self.password.is_empty() && self.password != self.password2
    }

    /// 확인을 눌렀을 때의 검사. 통과하면 작업을 돌려준다.
    fn validate(&mut self, overwrite_ok: bool) -> Option<Job> {
        let target = PathBuf::from(self.target.trim());
        if !target.parent().is_some_and(Path::is_dir) {
            let parent = target
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            self.problem = Some(Notice::new(
                Kind::Warning,
                "저장 위치 없음",
                format!("폴더가 존재하지 않습니다:\n{parent}"),
            ));
            return None;
        }
        // 자기 자신을 압축 대상에 넣으면 무한히 커진다.
        let resolved = std::path::absolute(&target).unwrap_or(target.clone());
        if self.sources.iter().any(|s| {
            std::path::absolute(s)
                .map(|s| s == resolved)
                .unwrap_or(false)
        }) {
            self.problem = Some(Notice::new(
                Kind::Warning,
                "잘못된 대상",
                "저장할 파일이 압축 대상에 포함되어 있습니다.\n다른 이름이나 위치를 선택하세요.",
            ));
            return None;
        }
        if target.exists() && !overwrite_ok {
            self.confirm_overwrite = true;
            return None;
        }

        let level = LEVELS[self.level].0;
        let _ = config::set("compress/format", self.format_key());
        let _ = config::set("compress/level", &level.to_string());
        remember_dir("compress", &target);

        Some(Job::Compress {
            sources: self.sources.clone(),
            target: CompressTarget::Exact(target),
            level,
            password: (self.is_zip() && !self.password.is_empty()).then(|| self.password.clone()),
        })
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) -> FormResult<Job> {
        if let Some(problem) = &self.problem {
            if problem.ui(ui).is_some() {
                self.problem = None;
            }
            return FormResult::Pending;
        }
        if self.confirm_overwrite {
            let name = file_name(Path::new(self.target.trim()));
            let notice = Notice::new(
                Kind::Warning,
                "덮어쓰기 확인",
                format!("이미 파일이 있습니다.\n\n{name}\n\n덮어쓸까요?"),
            )
            .with_buttons(&["덮어쓰기", "취소"]);
            match notice.ui(ui) {
                Some(0) => {
                    self.confirm_overwrite = false;
                    if let Some(job) = self.validate(true) {
                        return FormResult::Submit(job);
                    }
                }
                Some(_) => self.confirm_overwrite = false,
                None => {}
            }
            return FormResult::Pending;
        }

        // 본문은 스크롤되고, 버튼은 늘 아래에 보인다.
        let body_height = (ui.available_height() - FOOTER).max(160.0);
        egui::ScrollArea::vertical()
            .max_height(body_height)
            .auto_shrink([false, true])
            .show(ui, |ui| self.body(ui));

        ui.add_space(6.0);
        let mut result = FormResult::Pending;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("취소").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                result = FormResult::Cancel;
            }
            let ready =
                !self.sources.is_empty() && !self.target.trim().is_empty() && !self.mismatch();
            if ui
                .add_enabled(
                    ready,
                    egui::Button::new(RichText::new("압축 시작").strong()),
                )
                .clicked()
                && let Some(job) = self.validate(false)
            {
                result = FormResult::Submit(job);
            }
        });
        result
    }

    fn body(&mut self, ui: &mut egui::Ui) {
        ui.set_max_width(ui.available_width());
        ui.label(RichText::new("압축할 대상").strong());
        ui.horizontal(|ui| {
            let list_width = (ui.available_width() - 150.0).max(160.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(list_width);
                ui.set_height(96.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            let icons = Icons::get(ui.ctx());
                            // 긴 경로는 가운데를 줄인다. 글자 하나를 7.5px 쯤으로 어림한다.
                            let max_chars = ((list_width - 30.0) / 7.5) as usize;
                            for (i, path) in self.sources.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    if path.is_dir() {
                                        folder_icon(ui);
                                    } else {
                                        let name = file_name(path);
                                        ui.add(egui::Image::new((
                                            icons.for_name(&name).id(),
                                            egui::vec2(16.0, 16.0),
                                        )));
                                    }
                                    let text =
                                        elide(&path.display().to_string(), max_chars.max(20));
                                    let response =
                                        ui.selectable_label(self.selected == Some(i), text);
                                    if response.on_hover_text(path.display().to_string()).clicked()
                                    {
                                        self.selected = Some(i);
                                    }
                                });
                            }
                        });
                    });
            });
            ui.vertical(|ui| {
                let width = egui::vec2(100.0, 0.0);
                if ui
                    .add(egui::Button::new("파일 추가...").min_size(width))
                    .clicked()
                {
                    let mut dialog = rfd::FileDialog::new().set_title("압축할 파일 선택");
                    if let Some(dir) = self.start_dir() {
                        dialog = dialog.set_directory(dir);
                    }
                    if let Some(files) = dialog.pick_files() {
                        self.append(files);
                    }
                }
                if ui
                    .add(egui::Button::new("폴더 추가...").min_size(width))
                    .clicked()
                {
                    let mut dialog = rfd::FileDialog::new().set_title("압축할 폴더 선택");
                    if let Some(dir) = self.start_dir() {
                        dialog = dialog.set_directory(dir);
                    }
                    if let Some(folder) = dialog.pick_folder() {
                        self.append(vec![folder]);
                    }
                }
                if ui
                    .add_enabled(
                        self.selected.is_some(),
                        egui::Button::new("제외").min_size(width),
                    )
                    .clicked()
                    && let Some(i) = self.selected.take()
                {
                    self.sources.remove(i);
                    self.refresh_summary();
                }
            });
        });
        ui.label(muted(&self.summary));
        ui.add_space(8.0);

        ui.label(RichText::new("설정").strong());
        field(ui, "저장 위치", |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.target)
                    .desired_width((ui.available_width() - 116.0).max(120.0)),
            );
            if ui.button("찾아보기...").clicked() {
                let ext = default_extension(self.format_key());
                let current = PathBuf::from(self.target.trim());
                let mut dialog = rfd::FileDialog::new()
                    .set_title("저장할 위치")
                    .add_filter("압축 파일", &[ext.rsplit('.').next().unwrap_or("zip")]);
                if let Some(dir) = current.parent().filter(|d| d.is_dir()) {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(name) = current.file_name() {
                    dialog = dialog.set_file_name(name.to_string_lossy());
                }
                if let Some(path) = dialog.save_file() {
                    self.target = path.to_string_lossy().into_owned();
                }
            }
        });

        let before = self.format;
        field(ui, "압축 형식", |ui| {
            egui::ComboBox::from_id_salt("format")
                .selected_text(WRITABLE_FORMATS[self.format].1)
                .width(220.0)
                .show_ui(ui, |ui| {
                    for (i, (_, label)) in WRITABLE_FORMATS.iter().enumerate() {
                        ui.selectable_value(&mut self.format, i, *label);
                    }
                });
        });
        if before != self.format && !self.target.trim().is_empty() {
            self.target = swap_extension(self.target.trim(), default_extension(self.format_key()));
        }

        field(ui, "압축 강도", |ui| {
            ui.add(egui::Slider::new(&mut self.level, 0..=LEVELS.len() - 1).show_value(false));
            let (_, name, hint) = LEVELS[self.level];
            ui.label(name);
            if !hint.is_empty() {
                ui.label(muted(hint));
            }
        });

        let zip = self.is_zip();
        if !zip {
            self.password.clear();
            self.password2.clear();
        }
        field(ui, "암호", |ui| {
            ui.add_enabled(
                zip,
                egui::TextEdit::singleline(&mut self.password)
                    .password(!self.show_password)
                    .hint_text("비워 두면 암호를 걸지 않습니다")
                    .desired_width(260.0),
            );
        });
        field(ui, "암호 확인", |ui| {
            ui.add_enabled(
                zip,
                egui::TextEdit::singleline(&mut self.password2)
                    .password(!self.show_password)
                    .hint_text("확인을 위해 한 번 더")
                    .desired_width(260.0),
            );
            ui.add_enabled(
                zip,
                egui::Checkbox::new(&mut self.show_password, "암호 보기"),
            );
        });

        let note = if !self.is_zip() {
            "TAR 계열은 형식 자체에 암호 기능이 없습니다. 암호를 걸려면 ZIP 을 선택하세요."
        } else if self.mismatch() {
            "두 암호가 서로 다릅니다."
        } else if !self.password.is_empty() {
            "AES-256 으로 암호화합니다."
        } else {
            ""
        };
        if !note.is_empty() {
            field(ui, "", |ui| {
                ui.label(muted(note));
            });
        }
    }
}

/// 양식 아래쪽 버튼 줄의 높이.
const FOOTER: f32 = 40.0;
/// 양식 왼쪽 이름 칸의 너비.
const LABEL_WIDTH: f32 = 76.0;

/// "이름  [입력]" 한 줄.
fn field(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized([LABEL_WIDTH, 24.0], egui::Label::new(label));
        add(ui);
    });
}

/// 폴더 안 파일들의 크기. 요약 표시용이라 읽을 수 없는 것은 건너뛴다.
fn walkdir(root: &Path) -> Vec<u64> {
    let mut sizes = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(entry.path()),
                Ok(t) if t.is_file() => sizes.push(entry.metadata().map(|m| m.len()).unwrap_or(0)),
                _ => {}
            }
        }
    }
    sizes
}

// ---------------------------------------------------------------- 압축 풀기

/// 압축 풀기 양식이 돌려주는 것.
pub struct ExtractChoice {
    pub plan: ExtractPlan,
    /// 메인 창에서 고른 항목만 풀지
    pub only_selected: bool,
}

pub struct ExtractForm {
    archives: Vec<PathBuf>,
    dest: String,
    subfolder: bool,
    strip: bool,
    conflict: ConflictAction,
    open_after: bool,
    selection_count: usize,
    only_selected: bool,
    problem: Option<Notice>,
}

impl ExtractForm {
    pub fn new(archives: Vec<PathBuf>, selection_count: usize) -> Self {
        let base = last_dir("extract")
            .or_else(|| {
                archives
                    .first()
                    .and_then(|a| a.parent().map(Path::to_path_buf))
            })
            .unwrap_or_default();
        Self {
            archives,
            dest: base.to_string_lossy().into_owned(),
            subfolder: config::get("extract/mode") != "here",
            strip: config::get_bool("extract/strip_redundant"),
            conflict: crate::jobs::conflict_policy(),
            open_after: config::get_bool("extract/open_after"),
            selection_count,
            only_selected: selection_count > 0,
            problem: None,
        }
    }

    fn preview(&self) -> String {
        let base = PathBuf::from(self.dest.trim());
        let mut note = match (self.subfolder, self.archives.as_slice()) {
            (true, [one]) => format!(
                "→ {}",
                base.join(strip_archive_suffix(&file_name(one))).display()
            ),
            (true, _) => format!("→ {}\\<각 압축 파일 이름>", base.display()),
            (false, _) => format!("→ {}", base.display()),
        };
        if self.subfolder && self.strip {
            note.push_str("\n   (압축 파일 안이 폴더 하나뿐이면 그 폴더는 생략됩니다)");
        }
        note
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) -> FormResult<ExtractChoice> {
        if let Some(problem) = &self.problem {
            if problem.ui(ui).is_some() {
                self.problem = None;
            }
            return FormResult::Pending;
        }

        let body_height = (ui.available_height() - FOOTER).max(160.0);
        egui::ScrollArea::vertical()
            .max_height(body_height)
            .auto_shrink([false, true])
            .show(ui, |ui| self.body(ui));

        ui.add_space(6.0);
        let mut result = FormResult::Pending;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("취소").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                result = FormResult::Cancel;
            }
            if ui.button(RichText::new("압축 풀기").strong()).clicked() {
                result = self.submit();
            }
        });
        result
    }

    fn submit(&mut self) -> FormResult<ExtractChoice> {
        let base = PathBuf::from(self.dest.trim());
        if !base.exists() && !base.parent().is_some_and(Path::is_dir) {
            self.problem = Some(Notice::new(
                Kind::Warning,
                "폴더 없음",
                format!("폴더를 찾을 수 없습니다:\n{}", base.display()),
            ));
            return FormResult::Pending;
        }
        let _ = config::set(
            "extract/mode",
            if self.subfolder { "subfolder" } else { "here" },
        );
        let _ = config::set(
            "extract/strip_redundant",
            if self.strip { "true" } else { "false" },
        );
        let _ = config::set(
            "extract/open_after",
            if self.open_after { "true" } else { "false" },
        );
        let _ = config::set("extract/conflict", self.conflict.as_str());
        remember_dir("extract", &base);
        FormResult::Submit(ExtractChoice {
            plan: ExtractPlan {
                base: Some(base),
                subfolder: self.subfolder,
                strip_root: self.subfolder && self.strip,
                conflict: self.conflict,
                open_after: self.open_after,
            },
            only_selected: self.selection_count > 0 && self.only_selected,
        })
    }

    fn body(&mut self, ui: &mut egui::Ui) {
        let title = match self.archives.as_slice() {
            [one] => file_name(one),
            many => format!("압축 파일 {}개", many.len()),
        };
        ui.label(RichText::new(title).strong().size(15.0));
        if self.archives.len() > 1 {
            let names: Vec<String> = self.archives.iter().take(6).map(|a| file_name(a)).collect();
            let more = if self.archives.len() > 6 {
                format!(" 외 {}개", self.archives.len() - 6)
            } else {
                String::new()
            };
            ui.label(muted(format!("{}{more}", names.join(", "))));
        }
        ui.add_space(4.0);

        ui.label(RichText::new("풀어낼 위치").strong());
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.dest)
                    .desired_width(ui.available_width() - 100.0),
            );
            if ui.button("찾아보기...").clicked() {
                let mut dialog = rfd::FileDialog::new().set_title("풀어낼 폴더 선택");
                let current = PathBuf::from(self.dest.trim());
                if current.is_dir() {
                    dialog = dialog.set_directory(current);
                }
                if let Some(folder) = dialog.pick_folder() {
                    self.dest = folder.to_string_lossy().into_owned();
                }
            }
        });
        if let Some(parent) = self.archives.first().and_then(|a| a.parent())
            && ui.small_button("압축 파일이 있는 폴더").clicked()
        {
            self.dest = parent.to_string_lossy().into_owned();
        }
        ui.radio_value(&mut self.subfolder, false, "선택한 폴더에 그대로 풀기");
        ui.radio_value(
            &mut self.subfolder,
            true,
            "압축 파일 이름으로 폴더를 만들어 풀기",
        );
        ui.add_enabled(self.subfolder, egui::Checkbox::new(&mut self.strip, "압축 파일 안이 폴더 하나뿐이면 중복 폴더 만들지 않기"))
            .on_hover_text("예: 자료.zip 안에 '자료' 폴더 하나만 있을 때\n자료\\자료\\... 처럼 두 겹이 되는 것을 막습니다.");
        ui.add(egui::Label::new(muted(self.preview())).wrap());
        ui.add_space(6.0);

        ui.label(RichText::new("옵션").strong());
        ui.horizontal(|ui| {
            ui.label("같은 파일이 있으면");
            let current = CONFLICTS
                .iter()
                .find(|(a, _)| *a == self.conflict)
                .map_or("다른 이름으로 저장", |(_, l)| *l);
            egui::ComboBox::from_id_salt("conflict")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (action, label) in CONFLICTS {
                        ui.selectable_value(&mut self.conflict, *action, *label);
                    }
                });
        });
        ui.checkbox(&mut self.open_after, "작업이 끝나면 폴더 열기");
        if self.selection_count > 0 {
            ui.checkbox(
                &mut self.only_selected,
                format!("선택한 {}개 항목만 풀기", self.selection_count),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_swap() {
        assert_eq!(
            swap_extension(r"D:\a\자료.tar.gz", ".zip"),
            r"D:\a\자료.zip"
        );
        assert_eq!(
            swap_extension(r"D:\a\자료.zip", ".tar.xz"),
            r"D:\a\자료.tar.xz"
        );
    }
}
