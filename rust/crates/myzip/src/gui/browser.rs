//! 메인 창: 반디집 스타일의 아카이브 탐색기.
//!
//! 왼쪽에 폴더 트리, 오른쪽에 파일 목록을 두고 아카이브 안을 실제 폴더처럼
//! 돌아다닌다. Python 판 ui/main_window.py 를 옮겼다.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};
use myzip_core::encoding::{CANDIDATES, decode_best, display_name};
use myzip_core::path_safety::safe_join;
use myzip_core::{
    ArchiveReader, Entry, Error, ExtractOptions, format_size, is_archive, open_archive,
};

use super::forms::{CompressForm, ExtractForm, FormResult};
use super::settings::Settings;
use super::task::Task;
use super::widgets::{
    CellIcon, Icons, Notice, PasswordPrompt, PromptResult, folder_icon, format_time, modal, muted,
    paint_cell,
};
use crate::jobs::{self, ExtractPlan, Job, Outcome, file_name};
use crate::notify::{self, Kind};
use myzip_core::APP_NAME;

/// 미리보기 전에 물어보는 크기.
const BIG_PREVIEW: u64 = 400 * 1024 * 1024;
/// 텍스트로 복사할 수 있는 최대 크기.
const MAX_COPY: u64 = 4 * 1024 * 1024;

// ---------------------------------------------------------------- 폴더 색인

/// 항목들을 폴더별로 묶어 둔다. 매 프레임 전체를 훑지 않기 위해서다.
#[derive(Default)]
struct Index {
    /// 폴더 -> 그 안의 항목 (폴더 포함)
    children: HashMap<String, Vec<Entry>>,
    /// 폴더 -> 하위 폴더 (트리용, 이름순)
    subfolders: HashMap<String, Vec<String>>,
}

fn parent_of(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

impl Index {
    fn build(entries: &[Entry]) -> Self {
        let mut children: HashMap<String, Vec<Entry>> = HashMap::new();
        let mut dirs: BTreeSet<String> = BTreeSet::new();
        children.insert(String::new(), Vec::new());

        for entry in entries {
            // 중간 폴더가 아카이브에 적혀 있지 않을 수도 있으므로 경로를 따라가며 만든다.
            let mut acc = String::new();
            for part in entry.parent().split('/').filter(|p| !p.is_empty()) {
                if !acc.is_empty() {
                    acc.push('/');
                }
                acc.push_str(part);
                dirs.insert(acc.clone());
            }
            if entry.is_dir {
                dirs.insert(entry.path.clone());
                children.entry(entry.path.clone()).or_default();
            }
            children
                .entry(entry.parent().to_owned())
                .or_default()
                .push(entry.clone());
        }

        // 명시적으로 기록되지 않은 폴더도 목록에 나타나야 한다.
        let mut subfolders: HashMap<String, Vec<String>> = HashMap::new();
        for folder in &dirs {
            let parent = parent_of(folder).to_owned();
            let siblings = children.entry(parent.clone()).or_default();
            if !siblings.iter().any(|e| e.path == *folder) {
                siblings.push(Entry {
                    path: folder.clone(),
                    is_dir: true,
                    method: "폴더".into(),
                    ..Entry::default()
                });
            }
            children.entry(folder.clone()).or_default();
            subfolders.entry(parent).or_default().push(folder.clone());
        }
        Self {
            children,
            subfolders,
        }
    }
}

// ---------------------------------------------------------------- 정렬

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Name,
    Size,
    Packed,
    Ratio,
    Kind,
    Modified,
}

const COLUMNS: &[(SortKey, &str)] = &[
    (SortKey::Name, "이름"),
    (SortKey::Size, "크기"),
    (SortKey::Packed, "압축 크기"),
    (SortKey::Ratio, "압축률"),
    (SortKey::Kind, "종류"),
    (SortKey::Modified, "수정한 날짜"),
];

fn kind_of(entry: &Entry) -> String {
    if entry.is_dir {
        return "폴더".into();
    }
    let suffix = entry.suffix();
    let base = if suffix.is_empty() {
        "파일".to_owned()
    } else {
        format!("{} 파일", suffix[1..].to_uppercase())
    };
    if entry.encrypted {
        format!("{base} · 암호")
    } else {
        base
    }
}

fn sort_rows(rows: &mut [Entry], key: SortKey, ascending: bool) {
    rows.sort_by(|a, b| {
        // 폴더가 항상 먼저
        b.is_dir.cmp(&a.is_dir).then_with(|| {
            let order = match key {
                SortKey::Name => a.name().to_lowercase().cmp(&b.name().to_lowercase()),
                SortKey::Size => a.size.cmp(&b.size),
                SortKey::Packed => a.compressed_size.cmp(&b.compressed_size),
                SortKey::Ratio => a.ratio().total_cmp(&b.ratio()),
                SortKey::Kind => kind_of(a).cmp(&kind_of(b)),
                SortKey::Modified => a.mtime.cmp(&b.mtime),
            };
            if ascending { order } else { order.reverse() }
        })
    });
}

// ---------------------------------------------------------------- 상태

struct Opened {
    path: PathBuf,
    /// 작업 스레드가 빌려 간 동안은 None
    reader: Option<Box<dyn ArchiveReader>>,
    format: String,
    /// 열 때 자동으로 판별된 인코딩
    detected: String,
    /// 지금 쓰는 인코딩 ("auto" 또는 인코딩 이름)
    encoding: String,
    can_change_encoding: bool,
    entries: Vec<Entry>,
    index: Index,
}

/// 메인 창이 기다리는 작업.
enum Busy {
    Opening(
        Task<Result<Box<dyn ArchiveReader>, Error>>,
        PathBuf,
        Option<String>,
    ),
    /// 리더를 빌려 간 작업. 끝나면 돌려받는다.
    Reader(Task<(Box<dyn ArchiveReader>, Done)>),
    Compress(Task<Outcome>),
}

/// 리더를 빌려 간 작업의 결과.
enum Done {
    Extracted {
        result: Result<Vec<PathBuf>, Error>,
        dest: PathBuf,
        subfolder: bool,
        open_after: bool,
        quiet: bool,
    },
    Tested(Result<Vec<String>, Error>),
    Preview(Result<PathBuf, Error>),
}

enum Modal {
    Password(PasswordPrompt, PathBuf),
    Extract(ExtractForm),
    Compress(CompressForm),
    Notice(Notice, After),
    Settings(Settings),
}

/// 알림의 버튼을 누른 뒤 할 일.
enum After {
    Nothing,
    /// 압축 완료: [폴더 열기, 아카이브 열기, 닫기]
    Compressed(PathBuf),
    /// 큰 파일 미리보기 확인: [열기, 취소]
    Preview(Entry),
}

pub struct Browser {
    opened: Option<Opened>,
    current: String,
    rows: Vec<Entry>,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    sort: (SortKey, bool),
    busy: Option<Busy>,
    modal: Option<Modal>,
    flash: Option<(String, Instant)>,
    preview_dir: Option<PathBuf>,
    preview_count: u32,
    /// 첫 프레임에 열 아카이브
    pending: Option<(PathBuf, Option<String>)>,
    /// 트리에서 펼쳐 둘 폴더 (목록에서 들어간 폴더의 조상)
    reveal_in_tree: Option<String>,
}

impl Browser {
    pub fn new(archive: Option<PathBuf>, password: Option<String>) -> Self {
        Self {
            opened: None,
            current: String::new(),
            rows: Vec::new(),
            selected: BTreeSet::new(),
            anchor: None,
            sort: (SortKey::Name, true),
            busy: None,
            modal: None,
            flash: None,
            preview_dir: None,
            preview_count: 0,
            pending: archive.map(|a| (a, password)),
            reveal_in_tree: None,
        }
    }

    /// 창을 닫을 때 미리보기 임시 폴더를 지운다.
    pub fn cleanup(&mut self) {
        if let Some(dir) = self.preview_dir.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    fn notice(&mut self, kind: Kind, title: &str, text: impl Into<String>) {
        self.modal = Some(Modal::Notice(
            Notice::new(kind, title, text),
            After::Nothing,
        ));
    }

    fn error(&mut self, err: &Error) {
        self.notice(Kind::Error, "오류", jobs::describe(err));
    }

    // ------------------------------------------------------------ 열기

    pub fn open(&mut self, ctx: &egui::Context, path: PathBuf, password: Option<String>) {
        if !path.is_file() {
            self.notice(
                Kind::Error,
                "오류",
                format!("파일을 찾을 수 없습니다:\n{}", path.display()),
            );
            return;
        }
        if !is_archive(&path) {
            self.notice(
                Kind::Error,
                "오류",
                format!(
                    "압축 파일이 아니거나 지원하지 않는 형식입니다:\n{}",
                    file_name(&path)
                ),
            );
            return;
        }
        let task_path = path.clone();
        let task_password = password.clone();
        let task = Task::spawn(ctx, format!("{} 여는 중", file_name(&path)), move |_| {
            let mut reader = open_archive(&task_path, task_password.as_deref())?;
            if reader.has_encrypted() {
                let Some(_) = &task_password else {
                    return Err(Error::PasswordRequired(String::new()));
                };
                // ZIP 은 암호가 틀려도 목록은 읽힌다. 가장 작은 암호 걸린 파일을 읽어
                // 미리 확인해 둔다. 풀다가 틀렸다고 하는 것보다 지금 묻는 편이 낫다.
                let probe = reader
                    .entries()
                    .iter()
                    .filter(|e| e.encrypted && !e.is_dir && e.size <= 16 << 20)
                    .min_by_key(|e| e.size)
                    .cloned();
                if let Some(entry) = probe {
                    reader.read_bytes(&entry)?;
                }
            }
            Ok(reader)
        });
        self.busy = Some(Busy::Opening(task, path, password));
    }

    fn opened_ok(&mut self, ctx: &egui::Context, path: PathBuf, reader: Box<dyn ArchiveReader>) {
        let entries = reader.entries().to_vec();
        let _ = crate::config::set(
            "extract/last_dir",
            &path.parent().unwrap_or(Path::new("")).to_string_lossy(),
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "{} - {APP_NAME}",
            file_name(&path)
        )));
        self.opened = Some(Opened {
            index: Index::build(&entries),
            format: reader.format_name().to_owned(),
            detected: reader.encoding().to_owned(),
            encoding: "auto".into(),
            can_change_encoding: reader.can_change_encoding(),
            entries,
            path,
            reader: Some(reader),
        });
        self.navigate(String::new());
    }

    fn set_encoding(&mut self, choice: String) {
        let Some(opened) = &mut self.opened else {
            return;
        };
        let Some(reader) = &mut opened.reader else {
            return;
        };
        let encoding = if choice == "auto" {
            opened.detected.clone()
        } else {
            choice.clone()
        };
        if reader.set_encoding(&encoding).is_ok() {
            opened.encoding = choice;
            opened.entries = reader.entries().to_vec();
            opened.index = Index::build(&opened.entries);
            self.navigate(String::new());
        }
    }

    // ------------------------------------------------------------ 이동

    fn navigate(&mut self, folder: String) {
        self.rows = self
            .opened
            .as_ref()
            .and_then(|o| o.index.children.get(&folder))
            .cloned()
            .unwrap_or_default();
        sort_rows(&mut self.rows, self.sort.0, self.sort.1);
        self.current = folder;
        self.selected.clear();
        self.anchor = None;
        self.reveal_in_tree = Some(self.current.clone());
    }

    fn go_up(&mut self) {
        if !self.current.is_empty() {
            self.navigate(parent_of(&self.current).to_owned());
        }
    }

    /// 목록의 행 번호. 하위 폴더에 있으면 0번은 '..' 이다.
    fn row_offset(&self) -> usize {
        usize::from(!self.current.is_empty())
    }

    fn row(&self, index: usize) -> Option<&Entry> {
        index
            .checked_sub(self.row_offset())
            .and_then(|i| self.rows.get(i))
    }

    fn activate(&mut self, ctx: &egui::Context, index: usize) {
        if index < self.row_offset() {
            self.go_up();
            return;
        }
        let Some(entry) = self.row(index).cloned() else {
            return;
        };
        if entry.is_dir {
            self.navigate(entry.path);
        } else {
            self.preview(ctx, entry, false);
        }
    }

    /// 선택된 항목들. 폴더를 고르면 그 안의 모든 항목까지 포함한다.
    fn selected_entries(&self) -> Vec<Entry> {
        let Some(opened) = &self.opened else {
            return Vec::new();
        };
        let mut chosen = Vec::new();
        let mut seen = BTreeSet::new();
        for &index in &self.selected {
            let Some(entry) = self.row(index) else {
                continue;
            };
            if entry.is_dir {
                let prefix = format!("{}/", entry.path);
                for candidate in &opened.entries {
                    if (candidate.path == entry.path || candidate.path.starts_with(&prefix))
                        && seen.insert(candidate.path.clone())
                    {
                        chosen.push(candidate.clone());
                    }
                }
            } else if seen.insert(entry.path.clone()) {
                chosen.push(entry.clone());
            }
        }
        chosen
    }

    fn click(&mut self, index: usize, modifiers: egui::Modifiers) {
        if index < self.row_offset() {
            self.selected.clear();
            return;
        }
        if modifiers.shift {
            let anchor = self.anchor.unwrap_or(index);
            let (a, b) = if anchor <= index {
                (anchor, index)
            } else {
                (index, anchor)
            };
            if !modifiers.command {
                self.selected.clear();
            }
            let offset = self.row_offset();
            self.selected.extend((a..=b).filter(|&i| i >= offset));
        } else if modifiers.command {
            if !self.selected.remove(&index) {
                self.selected.insert(index);
            }
            self.anchor = Some(index);
        } else {
            self.selected = BTreeSet::from([index]);
            self.anchor = Some(index);
        }
    }

    fn select_all(&mut self) {
        let offset = self.row_offset();
        self.selected = (offset..offset + self.rows.len()).collect();
    }

    // ------------------------------------------------------------ 작업

    /// 리더를 빌려 스레드에서 돌린다.
    fn with_reader<F>(&mut self, ctx: &egui::Context, headline: String, work: F)
    where
        F: FnOnce(&mut dyn ArchiveReader, &mut myzip_core::Progress) -> Done + Send + 'static,
    {
        let Some(mut reader) = self.opened.as_mut().and_then(|o| o.reader.take()) else {
            return;
        };
        let task = Task::spawn(ctx, headline, move |progress| {
            let done = work(reader.as_mut(), progress);
            (reader, done)
        });
        self.busy = Some(Busy::Reader(task));
    }

    fn extract(
        &mut self,
        ctx: &egui::Context,
        plan: ExtractPlan,
        members: Option<Vec<Entry>>,
        quiet: bool,
    ) {
        let Some(opened) = &self.opened else { return };
        let (dest, strip_root) = match jobs::destination(&opened.path, &plan) {
            Ok(v) => v,
            Err(err) => return self.error(&err),
        };
        let headline = format!("{} 푸는 중", file_name(&opened.path));
        let policy = plan.conflict;
        let subfolder = plan.subfolder;
        let open_after = plan.open_after;
        self.with_reader(ctx, headline, move |reader, progress| {
            let mut ask = |_: &Path, _: &Entry| policy;
            let options = ExtractOptions {
                members: members.as_deref(),
                strip_root,
                on_conflict: Some(&mut ask),
            };
            let result = reader.extract(&dest, options, progress);
            Done::Extracted {
                result,
                dest,
                subfolder,
                open_after,
                quiet,
            }
        });
    }

    fn extract_here(&mut self, ctx: &egui::Context) {
        let mut plan = ExtractPlan::for_mode(crate::cli::Mode::ExtractHere);
        plan.open_after = crate::config::get_bool("extract/open_after");
        self.extract(ctx, plan, None, true);
    }

    fn extract_dialog(&mut self) {
        let Some(opened) = &self.opened else { return };
        let count = self.selected_entries().len();
        self.modal = Some(Modal::Extract(ExtractForm::new(
            vec![opened.path.clone()],
            count,
        )));
    }

    fn test(&mut self, ctx: &egui::Context) {
        let Some(opened) = &self.opened else { return };
        let headline = format!("{} 검사 중", file_name(&opened.path));
        self.with_reader(ctx, headline, |reader, progress| {
            Done::Tested(reader.test(progress))
        });
    }

    /// 항목을 임시 폴더로 꺼내 연결된 프로그램으로 연다.
    fn preview(&mut self, ctx: &egui::Context, entry: Entry, confirmed: bool) {
        if entry.size > BIG_PREVIEW && !confirmed {
            let notice = Notice::new(
                Kind::Warning,
                "큰 파일",
                format!(
                    "{} 은(는) {} 입니다.\n임시 폴더로 꺼내는 데 시간이 걸릴 수 있습니다.\n\n계속할까요?",
                    entry.name(),
                    format_size(entry.size)
                ),
            )
            .with_buttons(&["열기", "취소"]);
            self.modal = Some(Modal::Notice(notice, After::Preview(entry)));
            return;
        }
        let root = self
            .preview_dir
            .get_or_insert_with(|| {
                std::env::temp_dir().join(format!("myzip-view-{}", std::process::id()))
            })
            .clone();
        // 같은 이름을 여러 번 열어도 앞에서 연 것을 덮어쓰지 않게 매번 다른 폴더에 푼다.
        self.preview_count += 1;
        let dest = root.join(self.preview_count.to_string());
        let headline = format!("{} 여는 중", entry.name());
        self.with_reader(ctx, headline, move |reader, progress| {
            let members = [entry.clone()];
            let options = ExtractOptions {
                members: Some(&members),
                ..ExtractOptions::default()
            };
            let result = reader
                .extract(&dest, options, progress)
                .and_then(|_| safe_join(&dest, &entry.path));
            Done::Preview(result)
        });
    }

    fn copy_text(&mut self, ctx: &egui::Context, entry: &Entry) {
        if entry.size > MAX_COPY {
            self.notice(
                Kind::Warning,
                "너무 큼",
                "4MB 이하 파일만 복사할 수 있습니다.",
            );
            return;
        }
        let Some(reader) = self.opened.as_mut().and_then(|o| o.reader.as_mut()) else {
            return;
        };
        match reader.read_bytes(entry) {
            Ok(raw) => {
                ctx.copy_text(decode_best(&raw).0);
                self.flash = Some((
                    format!("{} 내용을 복사했습니다.", entry.name()),
                    Instant::now(),
                ));
            }
            Err(err) => self.error(&err),
        }
    }

    fn compress_dialog(&mut self, sources: Vec<PathBuf>) {
        self.modal = Some(Modal::Compress(CompressForm::new(sources)));
    }

    fn pick_and_compress(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("압축할 파일 선택");
        if let Some(dir) = crate::config::get("compress/last_dir")
            .parse::<PathBuf>()
            .ok()
            .filter(|d| d.is_dir())
        {
            dialog = dialog.set_directory(dir);
        }
        if let Some(files) = dialog.pick_files() {
            self.compress_dialog(files);
        }
    }

    fn pick_and_open(&mut self, ctx: &egui::Context) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("압축 파일 열기")
            .add_filter(
                "압축 파일",
                &[
                    "zip", "tar", "tgz", "gz", "bz2", "xz", "tbz2", "txz", "rar", "7z", "cab",
                    "arj", "lzh", "iso",
                ],
            )
            .add_filter("모든 파일", &["*"]);
        if let Some(dir) = crate::config::get("extract/last_dir")
            .parse::<PathBuf>()
            .ok()
            .filter(|d| d.is_dir())
        {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.open(ctx, path, None);
        }
    }

    // ------------------------------------------------------------ 작업 결과

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(busy) = &self.busy else { return };
        match busy {
            Busy::Opening(task, ..) => {
                let Some(result) = task.poll() else { return };
                let Some(Busy::Opening(_, path, password)) = self.busy.take() else {
                    return;
                };
                match result {
                    Ok(reader) => self.opened_ok(ctx, path, reader),
                    Err(Error::PasswordRequired(_)) => {
                        let prompt = PasswordPrompt::new(file_name(&path), password.is_some());
                        self.modal = Some(Modal::Password(prompt, path));
                    }
                    Err(err) => self.error(&err),
                }
            }
            Busy::Reader(task) => {
                let Some((reader, done)) = task.poll() else {
                    return;
                };
                self.busy = None;
                if let Some(opened) = &mut self.opened {
                    opened.reader = Some(reader);
                }
                self.finished(done);
            }
            Busy::Compress(task) => {
                let Some(outcome) = task.poll() else { return };
                self.busy = None;
                match outcome {
                    Outcome::Compressed { target, note } => {
                        let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
                        let mut text = format!(
                            "{}\n{}\n\n{}",
                            file_name(&target),
                            format_size(size),
                            target
                                .parent()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default()
                        );
                        if let Some(note) = note {
                            text.push_str(&format!("\n\n{note}"));
                        }
                        let notice = Notice::new(Kind::Info, "압축 완료", text).with_buttons(&[
                            "폴더 열기",
                            "아카이브 열기",
                            "닫기",
                        ]);
                        self.modal = Some(Modal::Notice(notice, After::Compressed(target)));
                    }
                    Outcome::Failed { message, .. } => self.notice(Kind::Error, "오류", message),
                    _ => {} // 취소
                }
            }
        }
    }

    fn finished(&mut self, done: Done) {
        match done {
            Done::Extracted {
                result: Ok(written),
                dest,
                subfolder,
                open_after,
                quiet,
            } => {
                if open_after {
                    let reveal = if subfolder {
                        dest.clone()
                    } else {
                        jobs::to_reveal(&dest, &written)
                    };
                    notify::open_in_explorer(&reveal);
                } else if !quiet {
                    self.notice(
                        Kind::Info,
                        "완료",
                        format!("압축을 풀었습니다.\n\n{}", dest.display()),
                    );
                }
            }
            Done::Extracted {
                result: Err(err), ..
            }
            | Done::Tested(Err(err))
            | Done::Preview(Err(err)) => {
                if !err.is_cancelled() {
                    self.error(&err);
                }
            }
            Done::Tested(Ok(bad)) if bad.is_empty() => {
                self.notice(Kind::Info, "검사 완료", "아카이브에 이상이 없습니다.")
            }
            Done::Tested(Ok(bad)) => {
                let mut text = format!("{}개 항목에 문제가 있습니다.\n", bad.len());
                for item in bad.iter().take(200) {
                    text.push_str(&format!("\n{item}"));
                }
                self.notice(Kind::Warning, "손상 발견", text);
            }
            Done::Preview(Ok(path)) => {
                if !path.exists() {
                    self.notice(Kind::Error, "오류", "항목을 꺼내지 못했습니다.");
                } else if !notify::shell_open(path.as_os_str()) {
                    self.notice(
                        Kind::Error,
                        "오류",
                        format!(
                            "연결된 프로그램이 없어 열 수 없습니다:\n{}",
                            file_name(&path)
                        ),
                    );
                }
            }
        }
    }

    // ------------------------------------------------------------ 그리기

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some((path, password)) = self.pending.take() {
            self.open(&ctx, path, password);
        }
        self.poll(&ctx);
        self.handle_drop(&ctx);

        let icons = Icons::get(&ctx);
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui, &icons));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));

        if self.opened.is_some() {
            egui::Panel::left("tree")
                .resizable(true)
                .default_size(220.0)
                .show(ui, |ui| self.tree(ui, &icons));
            egui::CentralPanel::default().show(ui, |ui| self.list(ui, &icons));
        } else {
            egui::CentralPanel::default().show(ui, |ui| empty_hint(ui, &icons));
        }

        self.shortcuts(&ctx);
        self.overlays(&ctx);
    }

    fn handle_drop(&mut self, ctx: &egui::Context) {
        if self.busy.is_some() || self.modal.is_some() {
            return;
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        let paths: Vec<PathBuf> = dropped.into_iter().filter(|p| p.exists()).collect();
        match paths.as_slice() {
            [] => {}
            // 압축 파일 하나만 떨어뜨렸으면 여는 것이 자연스럽다.
            [one] if one.is_file() && is_archive(one) => self.open(ctx, one.clone(), None),
            _ => self.compress_dialog(paths),
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, icons: &Icons) {
        let has = self.opened.as_ref().is_some_and(|o| o.reader.is_some());
        let ctx = ui.ctx().clone();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let tool = |ui: &mut egui::Ui,
                        tex: &egui::TextureHandle,
                        text: &str,
                        tip: &str,
                        enabled: bool| {
                let image = egui::Image::new((tex.id(), egui::vec2(22.0, 22.0)));
                ui.add_enabled(enabled, egui::Button::image_and_text(image, text))
                    .on_hover_text(tip)
                    .clicked()
            };
            if tool(
                ui,
                &icons.new,
                "새 압축",
                "파일이나 폴더를 새 압축 파일로 만들기 (Ctrl+N)",
                true,
            ) {
                self.pick_and_compress();
            }
            if tool(ui, &icons.open, "열기", "압축 파일 열기 (Ctrl+O)", true) {
                self.pick_and_open(&ctx);
            }
            ui.separator();
            if tool(
                ui,
                &icons.extract,
                "압축 풀기",
                "위치를 정해서 풀기 (Ctrl+E)",
                has,
            ) {
                self.extract_dialog();
            }
            if tool(
                ui,
                &icons.extract_here,
                "여기에 풀기",
                "압축 파일이 있는 폴더에 바로 풀기 (Ctrl+H)",
                has,
            ) {
                self.extract_here(&ctx);
            }
            if tool(
                ui,
                &icons.test,
                "검사",
                "압축 파일이 손상되지 않았는지 확인 (Ctrl+T)",
                has,
            ) {
                self.test(&ctx);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if tool(
                    ui,
                    &icons.settings,
                    "설정",
                    "확장자 연결과 탐색기 메뉴",
                    true,
                ) {
                    self.modal = Some(Modal::Settings(Settings::new()));
                }
                ui.separator();
                self.encoding_box(ui);
            });
        });
        ui.add_space(2.0);
    }

    fn encoding_box(&mut self, ui: &mut egui::Ui) {
        let Some(opened) = &self.opened else { return };
        let current = opened.encoding.clone();
        let label = |enc: &str| {
            if enc == "auto" {
                format!("파일명 인코딩: 자동 ({})", display_name(&opened.detected))
            } else {
                format!("파일명: {}", display_name(enc))
            }
        };
        let mut chosen = current.clone();
        ui.add_enabled_ui(
            opened.can_change_encoding && opened.reader.is_some(),
            |ui| {
                egui::ComboBox::from_id_salt("encoding")
                    .selected_text(label(&current))
                    .width(230.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut chosen, "auto".to_owned(), label("auto"));
                        for enc in CANDIDATES.iter().chain(&["cp437"]) {
                            ui.selectable_value(&mut chosen, (*enc).to_owned(), label(enc));
                        }
                    })
                    .response
                    .on_hover_text("파일 이름이 깨져 보이면 직접 고르세요");
            },
        );
        if chosen != current {
            self.set_encoding(chosen);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some((text, at)) = &self.flash {
                if at.elapsed() < Duration::from_secs(3) {
                    ui.label(text);
                    ui.ctx().request_repaint_after(Duration::from_millis(500));
                    return;
                }
                self.flash = None;
            }
            let Some(opened) = &self.opened else {
                ui.label("열린 압축 파일이 없습니다.");
                return;
            };
            let files: Vec<&Entry> = opened.entries.iter().filter(|e| !e.is_dir).collect();
            let total: u64 = files.iter().map(|e| e.size).sum();
            let packed: u64 = files.iter().map(|e| e.compressed_size).sum();
            let ratio = if total > 0 {
                (1.0 - packed as f64 / total as f64) * 100.0
            } else {
                0.0
            };
            if self.selected.is_empty() {
                ui.label(format!(
                    "파일 {}개  ·  원본 {}",
                    files.len(),
                    format_size(total)
                ));
            } else {
                let picked: u64 = self
                    .selected_entries()
                    .iter()
                    .filter(|e| !e.is_dir)
                    .map(|e| e.size)
                    .sum();
                ui.label(format!(
                    "{}개 선택  ·  {}",
                    self.selected.len(),
                    format_size(picked)
                ));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let encoding = if opened.encoding == "auto" {
                    &opened.detected
                } else {
                    &opened.encoding
                };
                ui.label(muted(format!(
                    "{}  ·  압축 후 {}  ·  {ratio:.1}% 절약  ·  {}",
                    opened.format,
                    format_size(packed),
                    display_name(encoding)
                )));
            });
        });
    }

    fn tree(&mut self, ui: &mut egui::Ui, icons: &Icons) {
        let Some(opened) = &self.opened else { return };
        let mut target: Option<String> = None;
        let reveal = self.reveal_in_tree.take();
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let root_selected = self.current.is_empty();
                ui.horizontal(|ui| {
                    ui.add(egui::Image::new((
                        icons.for_name(&file_name(&opened.path)).id(),
                        egui::vec2(16.0, 16.0),
                    )));
                    if ui
                        .selectable_label(root_selected, file_name(&opened.path))
                        .clicked()
                    {
                        target = Some(String::new());
                    }
                });
                ui.indent("tree-root", |ui| {
                    tree_children(
                        ui,
                        &opened.index,
                        "",
                        &self.current,
                        reveal.as_deref(),
                        &mut target,
                    );
                });
            });
        if let Some(folder) = target
            && folder != self.current
        {
            self.navigate(folder);
            self.reveal_in_tree = None;
        }
    }

    fn list(&mut self, ui: &mut egui::Ui, icons: &Icons) {
        let ctx = ui.ctx().clone();
        let offset = self.row_offset();
        let count = self.rows.len() + offset;
        let mut clicked: Option<(usize, egui::Modifiers)> = None;
        let mut activated: Option<usize> = None;
        let mut action: Option<RowAction> = None;
        let mut sort_by: Option<SortKey> = None;
        let selected = &self.selected;
        let rows = &self.rows;
        let sort = self.sort;
        let modifiers = ui.input(|i| i.modifiers);

        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(250.0).at_least(120.0).clip(true))
            .column(Column::initial(85.0).at_least(56.0))
            .column(Column::initial(90.0).at_least(56.0))
            .column(Column::initial(62.0).at_least(48.0))
            .column(Column::initial(100.0).at_least(56.0).clip(true))
            .column(Column::remainder().at_least(128.0))
            .header(24.0, |mut header| {
                for (key, label) in COLUMNS {
                    header.col(|ui| {
                        let arrow = match (sort.0 == *key, sort.1) {
                            (true, true) => " ▲",
                            (true, false) => " ▼",
                            _ => "",
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(format!("{label}{arrow}")).strong(),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            sort_by = Some(*key);
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(22.0, count, |mut row| {
                    let index = row.index();
                    let entry = index.checked_sub(offset).and_then(|i| rows.get(i));
                    row.set_selected(selected.contains(&index));
                    match entry {
                        None => {
                            row.col(|ui| paint_cell(ui, CellIcon::Folder, "..", false));
                            for text in ["", "", "", "상위 폴더", ""] {
                                row.col(|ui| paint_cell(ui, CellIcon::None, text, false));
                            }
                        }
                        Some(entry) => {
                            let icon = if entry.is_dir {
                                CellIcon::Folder
                            } else {
                                CellIcon::Texture(icons.for_name(entry.name()))
                            };
                            row.col(|ui| paint_cell(ui, icon, entry.name(), false));
                            let numbers = if entry.is_dir {
                                [String::new(), String::new(), String::new()]
                            } else {
                                [
                                    format_size(entry.size),
                                    format_size(entry.compressed_size),
                                    if entry.size > 0 {
                                        format!("{:.0}%", entry.ratio() * 100.0)
                                    } else {
                                        String::new()
                                    },
                                ]
                            };
                            for text in numbers {
                                row.col(|ui| paint_cell(ui, CellIcon::None, &text, true));
                            }
                            row.col(|ui| paint_cell(ui, CellIcon::None, &kind_of(entry), false));
                            row.col(|ui| {
                                paint_cell(ui, CellIcon::None, &format_time(entry.mtime), false)
                            });
                        }
                    }

                    let response = row.response();
                    if response.double_clicked() {
                        activated = Some(index);
                    } else if response.clicked() {
                        clicked = Some((index, modifiers));
                    } else if response.secondary_clicked() && !selected.contains(&index) {
                        // 고르지 않은 줄을 우클릭하면 그 줄을 고른다 (탐색기와 같다).
                        clicked = Some((index, egui::Modifiers::NONE));
                    }
                    response.context_menu(|ui| {
                        if let Some(choice) = row_menu(ui, entry) {
                            action = Some(choice);
                        }
                    });
                });
            });

        if let Some(key) = sort_by {
            self.sort = if self.sort.0 == key {
                (key, !self.sort.1)
            } else {
                (key, true)
            };
            sort_rows(&mut self.rows, self.sort.0, self.sort.1);
            self.selected.clear();
        }
        if let Some((index, modifiers)) = clicked {
            self.click(index, modifiers);
        }
        if let Some(index) = activated {
            self.activate(&ctx, index);
        }
        if let Some(action) = action {
            self.row_action(&ctx, action);
        }
    }

    fn row_action(&mut self, ctx: &egui::Context, action: RowAction) {
        match action {
            RowAction::ExtractSelected | RowAction::ExtractAll => {
                if action == RowAction::ExtractAll {
                    self.selected.clear();
                }
                self.extract_dialog();
            }
            RowAction::ExtractHere => self.extract_here(ctx),
            RowAction::Open(entry) => self.preview(ctx, entry, false),
            RowAction::Copy(entry) => self.copy_text(ctx, &entry),
            RowAction::SelectAll => self.select_all(),
            RowAction::Test => self.test(ctx),
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.modal.is_some() || self.busy.is_some() || ctx.egui_wants_keyboard_input() {
            return;
        }
        use egui::{Key, KeyboardShortcut, Modifiers};
        let pressed = |key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, key)))
        };
        if pressed(Key::N) {
            self.pick_and_compress();
        }
        if pressed(Key::O) {
            self.pick_and_open(ctx);
        }
        let has = self.opened.as_ref().is_some_and(|o| o.reader.is_some());
        if !has {
            return;
        }
        if pressed(Key::E) {
            self.extract_dialog();
        }
        if pressed(Key::H) {
            self.extract_here(ctx);
        }
        if pressed(Key::T) {
            self.test(ctx);
        }
        if pressed(Key::A) {
            self.select_all();
        }
        if ctx.input(|i| i.key_pressed(Key::Backspace)) {
            self.go_up();
        }
        if ctx.input(|i| i.key_pressed(Key::Enter))
            && let Some(&index) = self.selected.first()
        {
            self.activate(ctx, index);
        }
    }

    /// 모달(진행률, 대화상자, 알림).
    fn overlays(&mut self, ctx: &egui::Context) {
        if let Some(busy) = &mut self.busy {
            modal(ctx, "busy", 440.0, |ui| match busy {
                Busy::Opening(task, ..) => task.ui(ui),
                Busy::Reader(task) => task.ui(ui),
                Busy::Compress(task) => task.ui(ui),
            });
            return;
        }

        let Some(current) = &mut self.modal else {
            return;
        };
        let mut close = false;
        let mut then: Option<Then> = None;
        match current {
            Modal::Password(prompt, path) => {
                match modal(ctx, "password", 380.0, |ui| prompt.ui(ui)) {
                    PromptResult::Entered(password) => {
                        let path = path.clone();
                        let ctx = ctx.clone();
                        close = true;
                        then = Some(Box::new(move |b: &mut Browser| {
                            b.open(&ctx, path, Some(password))
                        }));
                    }
                    PromptResult::Cancelled => close = true,
                    PromptResult::Pending => {}
                }
            }
            Modal::Extract(form) => match modal(ctx, "extract", 520.0, |ui| form.ui(ui)) {
                FormResult::Submit(choice) => {
                    close = true;
                    let ctx = ctx.clone();
                    then = Some(Box::new(move |b: &mut Browser| {
                        let members = choice.only_selected.then(|| b.selected_entries());
                        b.extract(&ctx, choice.plan, members, false);
                    }));
                }
                FormResult::Cancel => close = true,
                FormResult::Pending => {}
            },
            Modal::Compress(form) => match modal(ctx, "compress", 580.0, |ui| form.ui(ui)) {
                FormResult::Submit(job) => {
                    close = true;
                    let ctx = ctx.clone();
                    then = Some(Box::new(move |b: &mut Browser| b.start_compress(&ctx, job)));
                }
                FormResult::Cancel => close = true,
                FormResult::Pending => {}
            },
            Modal::Notice(notice, after) => {
                if let Some(button) = modal(ctx, "notice", 420.0, |ui| notice.ui(ui)) {
                    close = true;
                    let ctx = ctx.clone();
                    match std::mem::replace(after, After::Nothing) {
                        After::Nothing => {}
                        After::Compressed(target) => match button {
                            0 => notify::open_in_explorer(&target),
                            1 => {
                                then = Some(Box::new(move |b: &mut Browser| {
                                    b.open(&ctx, target, None)
                                }))
                            }
                            _ => {}
                        },
                        After::Preview(entry) if button == 0 => {
                            then = Some(Box::new(move |b: &mut Browser| {
                                b.preview(&ctx, entry, true)
                            }));
                        }
                        After::Preview(_) => {}
                    }
                }
            }
            Modal::Settings(settings) => {
                if modal(ctx, "settings", 560.0, |ui| settings.ui(ui)) {
                    close = true;
                }
            }
        }
        if close {
            self.modal = None;
        }
        if let Some(then) = then {
            then(self);
        }
    }

    fn start_compress(&mut self, ctx: &egui::Context, job: Job) {
        let headline = job.label();
        let task = Task::spawn(ctx, headline, move |progress| jobs::run_job(&job, progress));
        self.busy = Some(Busy::Compress(task));
    }
}

/// 모달을 닫은 뒤 이어서 할 일.
type Then = Box<dyn FnOnce(&mut Browser)>;

#[derive(PartialEq)]
enum RowAction {
    ExtractSelected,
    ExtractAll,
    ExtractHere,
    Open(Entry),
    Copy(Entry),
    SelectAll,
    Test,
}

fn row_menu(ui: &mut egui::Ui, entry: Option<&Entry>) -> Option<RowAction> {
    let mut action = None;
    if let Some(entry) = entry {
        if ui.button("선택 항목 풀기...").clicked() {
            action = Some(RowAction::ExtractSelected);
        }
        if !entry.is_dir {
            if ui.button("열기").clicked() {
                action = Some(RowAction::Open(entry.clone()));
            }
            if ui.button("내용 복사 (텍스트)").clicked() {
                action = Some(RowAction::Copy(entry.clone()));
            }
        }
        ui.separator();
    }
    if ui.button("전체 풀기...").clicked() {
        action = Some(RowAction::ExtractAll);
    }
    if ui.button("여기에 전부 풀기").clicked() {
        action = Some(RowAction::ExtractHere);
    }
    ui.separator();
    if ui.button("모두 선택").clicked() {
        action = Some(RowAction::SelectAll);
    }
    if ui.button("무결성 검사").clicked() {
        action = Some(RowAction::Test);
    }
    if action.is_some() {
        ui.close();
    }
    action
}

/// 트리의 하위 폴더들을 그린다.
fn tree_children(
    ui: &mut egui::Ui,
    index: &Index,
    folder: &str,
    current: &str,
    reveal: Option<&str>,
    target: &mut Option<String>,
) {
    let Some(children) = index.subfolders.get(folder) else {
        return;
    };
    for child in children {
        let id = ui.make_persistent_id(("tree", child));
        let mut state =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
        // 목록에서 들어간 폴더는 트리에서도 보이게 조상을 펼친다.
        if reveal.is_some_and(|r| r.starts_with(&format!("{child}/"))) {
            state.set_open(true);
        }
        let has_children = index.subfolders.contains_key(child);
        let header = state.show_header(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            folder_icon(ui);
            if ui
                .selectable_label(child == current, name_of(child))
                .clicked()
            {
                *target = Some(child.clone());
            }
        });
        if has_children {
            header.body(|ui| tree_children(ui, index, child, current, reveal, target));
        } else {
            header.body(|_| {});
        }
    }
}

fn empty_hint(ui: &mut egui::Ui, icons: &Icons) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - 110.0).max(20.0));
        ui.add(egui::Image::new((icons.app.id(), egui::vec2(72.0, 72.0))));
        ui.label(RichText::new(APP_NAME).strong().size(22.0));
        ui.add_space(6.0);
        ui.label(muted("압축 파일을 여기에 끌어다 놓거나"));
        ui.label(muted("'새 압축' 으로 파일을 압축하세요."));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, is_dir: bool) -> Entry {
        Entry {
            path: path.into(),
            is_dir,
            ..Entry::default()
        }
    }

    #[test]
    fn index_creates_implicit_folders() {
        // 'a' 와 'a/b' 폴더는 아카이브에 적혀 있지 않다.
        let index = Index::build(&[entry("a/b/c.txt", false), entry("d.txt", false)]);
        let root: Vec<&str> = index.children[""].iter().map(|e| e.path.as_str()).collect();
        assert!(root.contains(&"a") && root.contains(&"d.txt"));
        assert_eq!(index.subfolders[""], ["a"]);
        assert_eq!(index.subfolders["a"], ["a/b"]);
        assert_eq!(index.children["a/b"][0].path, "a/b/c.txt");
    }

    #[test]
    fn folders_sort_first() {
        let mut rows = vec![
            entry("b.txt", false),
            entry("z", true),
            entry("a.txt", false),
        ];
        sort_rows(&mut rows, SortKey::Name, false);
        assert_eq!(rows[0].path, "z");
        assert_eq!(rows[1].path, "b.txt");
    }
}
