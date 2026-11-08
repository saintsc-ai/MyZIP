//! 셸 통합 작업: 요청을 작업 목록으로 바꾸고, 하나씩 처리하고, 결과를 정리한다.
//!
//! 탐색기에서 여러 파일을 골라 메뉴를 누르면 요청이 여러 개 모인다.
//!
//! * 압축(--compress-zip/tgz): 같은 동작의 요청을 모두 합쳐 아카이브 하나로 묶는다.
//! * 압축 풀기(--extract-here/to)와 검사(--test): 아카이브마다 따로 처리한다.
//!   --extract-to 는 아카이브마다 자기 이름의 폴더를 만들어 푼다.
//!
//! 하나가 실패해도 나머지는 계속 처리하고, 끝에 한 번만 알린다.
//! 처리 자체는 창이 있든 없든 같다. 창(GUI)은 진행률과 암호 입력을 보탤 뿐이다.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use myzip_core::{
    ConflictAction, Entry, Error, ExtractOptions, Progress, open_archive, strip_archive_suffix,
    unique_path, walk_inputs, writer_for,
};

use crate::cli::{Mode, Request};
use crate::config;
use crate::console;
use crate::notify::{self, Kind};

/// 압축 풀 위치와 방식.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractPlan {
    /// 풀 폴더. None 이면 아카이브가 있는 폴더.
    pub base: Option<PathBuf>,
    /// base 아래에 아카이브 이름의 폴더를 만들어 푼다.
    pub subfolder: bool,
    /// 아카이브 안이 폴더 하나뿐이면 벗겨 낸다 (subfolder 일 때만).
    pub strip_root: bool,
    pub conflict: ConflictAction,
    /// 끝나면 탐색기로 보여 준다.
    pub open_after: bool,
}

impl ExtractPlan {
    /// 탐색기 메뉴의 빠른 동작. 설정을 따른다.
    pub fn for_mode(mode: Mode) -> Self {
        Self {
            base: None,
            subfolder: mode == Mode::ExtractTo,
            strip_root: config::get_bool("extract/strip_redundant"),
            conflict: conflict_policy(),
            open_after: config::get_bool("extract/open_after"),
        }
    }
}

/// 만들 아카이브 이름.
#[derive(Debug, Clone, PartialEq)]
pub enum CompressTarget {
    /// 고른 것의 이름을 따서 그 옆에 만든다. 값은 확장자(".zip").
    Auto(&'static str),
    /// 사용자가 정한 경로. 이미 있으면 덮어쓴다 (대화상자에서 확인을 받았다).
    Exact(PathBuf),
}

/// 실제로 할 일 하나.
#[derive(Debug, Clone, PartialEq)]
pub enum Job {
    Extract {
        archive: PathBuf,
        password: Option<String>,
        plan: ExtractPlan,
    },
    Test {
        archive: PathBuf,
        password: Option<String>,
    },
    Compress {
        sources: Vec<PathBuf>,
        target: CompressTarget,
        level: i32,
        password: Option<String>,
    },
    /// 창이 필요한 동작 (열기, 대화상자).
    Window { mode: Mode, paths: Vec<PathBuf> },
}

impl Job {
    /// 사용자에게 보여 줄 짧은 설명.
    pub fn label(&self) -> String {
        match self {
            Job::Extract { archive, .. } => format!("{} 푸는 중", file_name(archive)),
            Job::Test { archive, .. } => format!("{} 검사 중", file_name(archive)),
            Job::Compress { sources, .. } if sources.len() == 1 => {
                format!("{} 압축하는 중", file_name(&sources[0]))
            }
            Job::Compress { sources, .. } => format!("{}개 항목 압축하는 중", sources.len()),
            Job::Window { .. } => String::new(),
        }
    }

    /// 암호를 바꿔 다시 시도할 작업.
    pub fn with_password(&self, new: String) -> Job {
        let mut job = self.clone();
        match &mut job {
            Job::Extract { password, .. } | Job::Test { password, .. } => *password = Some(new),
            Job::Compress { .. } | Job::Window { .. } => {}
        }
        job
    }
}

/// 같은 파일인지 비교할 열쇠. Windows 경로는 대소문자를 가리지 않는다.
fn path_key(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

/// 모인 요청들을 작업 목록으로 바꾼다. 도착 순서를 지킨다.
pub fn plan(requests: Vec<Request>) -> Vec<Job> {
    let mut jobs: Vec<Job> = Vec::new();
    let mut seen: HashSet<(Mode, String)> = HashSet::new();
    let level = config::get_int("compress/level").clamp(0, 9);

    for request in requests {
        let Request {
            mode,
            paths,
            password,
        } = request;
        match mode {
            Mode::CompressZip | Mode::CompressTgz if !paths.is_empty() => {
                let ext = if mode == Mode::CompressZip {
                    ".zip"
                } else {
                    ".tar.gz"
                };
                // 같은 동작·같은 암호의 압축은 하나로 합친다.
                let index = jobs.iter().position(|job| {
                    matches!(job, Job::Compress { target: CompressTarget::Auto(e), password: p, .. }
                        if *e == ext && *p == password)
                });
                let index = index.unwrap_or_else(|| {
                    jobs.push(Job::Compress {
                        sources: Vec::new(),
                        target: CompressTarget::Auto(ext),
                        level,
                        password: password.clone(),
                    });
                    jobs.len() - 1
                });
                if let Job::Compress { sources, .. } = &mut jobs[index] {
                    for path in paths {
                        if seen.insert((mode, path_key(&path))) {
                            sources.push(path);
                        }
                    }
                }
            }
            Mode::ExtractHere | Mode::ExtractTo | Mode::Test if !paths.is_empty() => {
                for archive in paths {
                    if !seen.insert((mode, path_key(&archive))) {
                        continue; // 같은 파일이 두 번 들어왔다
                    }
                    jobs.push(if mode == Mode::Test {
                        Job::Test {
                            archive,
                            password: password.clone(),
                        }
                    } else {
                        Job::Extract {
                            archive,
                            password: password.clone(),
                            plan: ExtractPlan::for_mode(mode),
                        }
                    });
                }
            }
            _ => jobs.push(Job::Window { mode, paths }),
        }
    }
    jobs
}

/// 작업 하나의 결과.
#[derive(Debug, Clone)]
pub enum Outcome {
    Extracted {
        archive: PathBuf,
        dest: PathBuf,
        /// 탐색기로 보여 줄 것
        reveal: PathBuf,
        open_after: bool,
    },
    Compressed {
        target: PathBuf,
        note: Option<String>,
    },
    Tested {
        archive: PathBuf,
        bad: Vec<String>,
    },
    /// 암호가 필요하거나 틀렸다. 창이 있으면 물어보고 다시 한다.
    NeedsPassword {
        subject: PathBuf,
        wrong: bool,
    },
    Failed {
        subject: PathBuf,
        message: String,
    },
    Cancelled,
    NeedsWindow,
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// 사용자에게 보여 줄 오류 문장.
pub fn describe(err: &Error) -> String {
    match err {
        Error::PasswordRequired(_) => "암호가 필요하거나 틀렸습니다.".into(),
        Error::Io(e) => format!("파일을 다루지 못했습니다: {e}"),
        other => other.to_string(),
    }
}

/// 오류를 결과로 바꾼다. 암호와 취소는 따로 다룬다.
fn failure(subject: &Path, err: Error, password_given: bool) -> Outcome {
    match err {
        Error::PasswordRequired(_) => Outcome::NeedsPassword {
            subject: subject.to_path_buf(),
            wrong: password_given,
        },
        Error::Cancelled(_) => Outcome::Cancelled,
        other => Outcome::Failed {
            subject: subject.to_path_buf(),
            message: describe(&other),
        },
    }
}

/// 설정의 충돌 처리 방식. 'ask' 는 이름 바꾸기로 처리한다.
pub fn conflict_policy() -> ConflictAction {
    ConflictAction::parse(&config::get("extract/conflict"))
        .filter(|a| *a != ConflictAction::Cancel)
        .unwrap_or(ConflictAction::Rename)
}

/// 풀 위치. subfolder 면 아카이브 이름의 폴더다 (자료.tar.gz -> 자료\).
pub fn destination(archive: &Path, plan: &ExtractPlan) -> myzip_core::Result<(PathBuf, bool)> {
    let base = match &plan.base {
        Some(base) => base.clone(),
        None => archive.parent().unwrap_or(Path::new(".")).to_path_buf(),
    };
    if !plan.subfolder {
        return Ok((base, false));
    }
    let mut dest = base.join(strip_archive_suffix(&file_name(archive)));
    if dest.exists() && !dest.is_dir() {
        // 같은 이름의 '파일'이 있으면 폴더를 만들 수 없다. 이름을 비켜 간다.
        dest = unique_path(&dest)?;
    }
    // 아카이브 안이 폴더 하나뿐이면 벗겨 낸다. 자료\자료\... 가 되지 않게.
    Ok((dest, plan.strip_root))
}

/// 해제가 끝난 뒤 탐색기에서 보여 줄 것. 최상위에 항목이 하나만 생겼으면 그것을
/// 선택하고, 아니면 풀린 폴더를 연다.
pub fn to_reveal(dest: &Path, written: &[PathBuf]) -> PathBuf {
    let tops: HashSet<PathBuf> = written
        .iter()
        .filter_map(|p| p.strip_prefix(dest).ok())
        .filter_map(|rel| rel.components().next())
        .map(|first| dest.join(first))
        .collect();
    match tops.len() {
        1 => tops
            .into_iter()
            .next()
            .unwrap_or_else(|| dest.to_path_buf()),
        _ => dest.to_path_buf(),
    }
}

fn extract(
    archive: &Path,
    password: Option<&str>,
    plan: &ExtractPlan,
    progress: &mut Progress,
) -> Outcome {
    if !archive.is_file() {
        return Outcome::Failed {
            subject: archive.to_path_buf(),
            message: "파일을 찾을 수 없습니다.".into(),
        };
    }
    let mut created: Option<PathBuf> = None;
    let result = (|| {
        let mut reader = open_archive(archive, password)?;
        // ZIP 은 목록은 암호 없이 읽힌다. 풀기 전에 암호부터 물어야
        // 빈 폴더만 만들어 놓고 실패하는 일이 없다.
        if reader.has_encrypted() && password.is_none() {
            return Err(Error::PasswordRequired(String::new()));
        }
        let (dest, strip_root) = destination(archive, plan)?;
        created = (!dest.exists()).then(|| dest.clone());
        let policy = plan.conflict;
        let mut ask = |_: &Path, _: &Entry| policy;
        let options = ExtractOptions {
            members: None,
            strip_root,
            on_conflict: Some(&mut ask),
        };
        let written = reader.extract(&dest, options, progress)?;
        let reveal = if plan.subfolder {
            dest.clone()
        } else {
            to_reveal(&dest, &written)
        };
        Ok::<_, Error>((dest, reveal))
    })();
    match result {
        Ok((dest, reveal)) => Outcome::Extracted {
            archive: archive.to_path_buf(),
            dest,
            reveal,
            open_after: plan.open_after,
        },
        Err(err) => {
            if let Some(dir) = created {
                remove_if_no_files(&dir);
            }
            failure(archive, err, password.is_some())
        }
    }
}

/// 파일이 하나도 없는 폴더 트리면 지운다. 하나라도 있으면 그대로 둔다.
fn remove_if_no_files(dir: &Path) {
    fn has_files(dir: &Path) -> bool {
        let Ok(read) = std::fs::read_dir(dir) else {
            return true;
        };
        read.flatten().any(|e| match e.file_type() {
            Ok(t) if t.is_dir() => has_files(&e.path()),
            _ => true,
        })
    }
    if !has_files(dir) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn test(archive: &Path, password: Option<&str>, progress: &mut Progress) -> Outcome {
    let result = open_archive(archive, password).and_then(|mut r| {
        if r.has_encrypted() && password.is_none() {
            return Err(Error::PasswordRequired(String::new()));
        }
        r.test(progress)
    });
    match result {
        Ok(bad) => Outcome::Tested {
            archive: archive.to_path_buf(),
            bad,
        },
        Err(err) => failure(archive, err, password.is_some()),
    }
}

/// 만들 아카이브 이름. Python 판과 같은 규칙이다.
///
/// * 하나면 그 이름: 폴더 `자료` -> `자료.zip`, 파일 `보고서.hwp` -> `보고서.zip`
/// * 여러 개면 들어 있는 폴더 이름: `D:\프로젝트\a.txt, b.txt` -> `프로젝트.zip`
pub fn suggest_name(sources: &[PathBuf], extension: &str) -> Option<PathBuf> {
    let first = sources.first()?;
    let parent = first.parent().unwrap_or(Path::new("."));
    let stem = if sources.len() == 1 {
        if first.is_dir() {
            first.file_name()
        } else {
            first.file_stem()
        }
    } else {
        parent.file_name()
    };
    let stem = stem
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "압축파일".into());
    Some(parent.join(format!("{stem}{extension}")))
}

fn compress(
    sources: &[PathBuf],
    target: &CompressTarget,
    level: i32,
    password: Option<&str>,
    progress: &mut Progress,
) -> Outcome {
    let subject = sources.first().cloned().unwrap_or_default();
    let mut note = None;
    let result = (|| {
        let existing: Vec<PathBuf> = sources.iter().filter(|p| p.exists()).cloned().collect();
        if existing.is_empty() {
            return Err(Error::Archive("압축할 파일이 없습니다.".into()));
        }
        let path = match target {
            CompressTarget::Auto(ext) => {
                let suggested = suggest_name(&existing, ext).unwrap_or_else(|| PathBuf::from(ext));
                unique_path(&suggested)?
            }
            CompressTarget::Exact(path) => path.clone(),
        };
        let mut writer = writer_for(&path, level, password)?;
        if password.is_some() && !writer.supports_password() {
            note = Some(format!(
                "{} 는 암호를 지원하지 않아 암호 없이 만들었습니다.",
                writer.format_name()
            ));
        }
        writer.write(&walk_inputs(&existing), progress)?;
        Ok(path)
    })();
    match result {
        Ok(target) => Outcome::Compressed { target, note },
        Err(err) => failure(&subject, err, password.is_some()),
    }
}

/// 작업 하나를 처리한다.
pub fn run_job(job: &Job, progress: &mut Progress) -> Outcome {
    match job {
        Job::Extract {
            archive,
            password,
            plan,
        } => extract(archive, password.as_deref(), plan, progress),
        Job::Test { archive, password } => test(archive, password.as_deref(), progress),
        Job::Compress {
            sources,
            target,
            level,
            password,
        } => compress(sources, target, *level, password.as_deref(), progress),
        Job::Window { .. } => Outcome::NeedsWindow,
    }
}

/// 여러 작업의 결과를 사람이 읽을 모양으로 정리한 것.
#[derive(Debug, Default, Clone)]
pub struct Summary {
    /// (보여 줄 경로, 끝나면 열지)
    pub extracted: Vec<(PathBuf, bool)>,
    pub compressed: Vec<PathBuf>,
    pub notes: Vec<String>,
    pub tested_ok: Vec<String>,
    pub damaged: Vec<String>,
    pub failures: Vec<String>,
    pub cancelled: bool,
    pub needs_window: bool,
    /// 처리하려고 한 작업 수 (창이 필요한 것 제외)
    pub total: usize,
}

impl Summary {
    pub fn new(outcomes: &[Outcome]) -> Self {
        let mut s = Summary::default();
        for outcome in outcomes {
            if !matches!(outcome, Outcome::NeedsWindow) {
                s.total += 1;
            }
            match outcome {
                Outcome::Extracted {
                    reveal, open_after, ..
                } => s.extracted.push((reveal.clone(), *open_after)),
                Outcome::Compressed { target, note } => {
                    s.compressed.push(target.clone());
                    s.notes.extend(note.clone());
                }
                Outcome::Tested { archive, bad } if bad.is_empty() => {
                    s.tested_ok.push(file_name(archive))
                }
                Outcome::Tested { archive, bad } => {
                    let mut lines =
                        format!("{} — 문제가 있는 항목 {}개:", file_name(archive), bad.len());
                    for item in bad.iter().take(20) {
                        lines.push_str(&format!("\n  {item}"));
                    }
                    s.damaged.push(lines);
                }
                Outcome::NeedsPassword { subject, wrong } => s.failures.push(format!(
                    "{}\n  {}",
                    file_name(subject),
                    if *wrong {
                        "암호가 틀렸습니다."
                    } else {
                        "암호가 필요합니다. --password 로 암호를 주세요."
                    }
                )),
                Outcome::Failed { subject, message } => {
                    s.failures
                        .push(format!("{}\n  {message}", file_name(subject)));
                }
                Outcome::Cancelled => s.cancelled = true,
                Outcome::NeedsWindow => s.needs_window = true,
            }
        }
        s
    }

    /// 무결성 검사 결과 알림. 검사를 안 했으면 None.
    pub fn test_report(&self) -> Option<(Kind, &'static str, String)> {
        if self.tested_ok.is_empty() && self.damaged.is_empty() {
            return None;
        }
        if self.damaged.is_empty() {
            let message = if self.tested_ok.len() == 1 {
                format!("{} — 이상이 없습니다.", self.tested_ok[0])
            } else {
                format!("{}개 아카이브 모두 이상이 없습니다.", self.tested_ok.len())
            };
            return Some((Kind::Info, "검사 완료", message));
        }
        let mut message = self.damaged.join("\n\n");
        if !self.tested_ok.is_empty() {
            message.push_str(&format!("\n\n이상 없음: {}", self.tested_ok.join(", ")));
        }
        Some((Kind::Warning, "손상 발견", message))
    }

    /// 실패 알림. 실패가 없으면 None.
    pub fn failure_report(&self) -> Option<String> {
        if self.failures.is_empty() {
            return None;
        }
        let head = if self.total > 1 {
            format!(
                "{}개 중 {}개를 처리하지 못했습니다.\n\n",
                self.total,
                self.failures.len()
            )
        } else {
            String::new()
        };
        let shown: Vec<&str> = self.failures.iter().take(15).map(String::as_str).collect();
        let mut message = head + &shown.join("\n\n");
        if self.failures.len() > shown.len() {
            message.push_str(&format!("\n\n… 외 {}개", self.failures.len() - shown.len()));
        }
        Some(message)
    }

    /// 탐색기로 보여 줄 것. 하나만 풀었을 때만 연다. 여러 개를 풀었다면
    /// 사용자는 이미 그 폴더를 보고 있고, 창이 여러 개 뜨면 오히려 성가시다.
    pub fn to_open(&self) -> Option<&Path> {
        match self.extracted.as_slice() {
            [(path, true)] if self.failures.is_empty() && !self.cancelled => Some(path),
            _ => None,
        }
    }

    pub fn exit_code(&self) -> i32 {
        if self.failures.is_empty() && self.damaged.is_empty() && !self.cancelled {
            0
        } else {
            1
        }
    }
}

/// 창 없이 처리하고 결과를 알린다 (콘솔 또는 메시지 상자). 종료 코드를 돌려준다.
pub fn execute(requests: Vec<Request>) -> i32 {
    let jobs = plan(requests);
    let outcomes: Vec<Outcome> = jobs
        .iter()
        .map(|job| run_job(job, &mut Progress::new()))
        .collect();

    if console::has_output() {
        for outcome in &outcomes {
            match outcome {
                Outcome::Extracted { archive, dest, .. } => console::say(&format!(
                    "압축을 풀었습니다: {} -> {}",
                    archive.display(),
                    dest.display()
                )),
                Outcome::Compressed { target, .. } => {
                    console::say(&format!("압축했습니다: {}", target.display()));
                }
                _ => {}
            }
        }
    }

    let summary = Summary::new(&outcomes);
    for note in &summary.notes {
        notify::show(Kind::Warning, "알림", note);
    }
    if let Some(path) = summary.to_open() {
        notify::open_in_explorer(path);
    }
    if let Some((kind, title, message)) = summary.test_report() {
        notify::show(kind, title, &message);
    }
    if let Some(message) = summary.failure_report() {
        notify::show(Kind::Error, "오류", &message);
    }
    if summary.needs_window {
        notify::show(
            Kind::Info,
            "창 없음",
            "창을 띄우는 기능(열기, 압축 풀기..., 압축하기...)은 MYZIP_NO_SHELL 환경에서 쓰지 않습니다.",
        );
        return 1;
    }
    summary.exit_code()
}

/// --list: 아카이브 목록을 콘솔에 출력한다.
pub fn list(request: &Request) -> i32 {
    let mut code = 0;
    for archive in &request.paths {
        match open_archive(archive, request.password.as_deref()) {
            Ok(reader) => {
                console::say(&format!(
                    "{} [{}, 파일명 인코딩 {}]",
                    archive.display(),
                    reader.format_name(),
                    reader.encoding()
                ));
                for entry in reader.entries() {
                    let date = entry
                        .mtime
                        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_default();
                    let size = if entry.is_dir {
                        "<폴더>".to_owned()
                    } else {
                        entry.size.to_string()
                    };
                    let lock = if entry.encrypted { "*" } else { " " };
                    console::say(&format!("{date:>16} {size:>14} {lock} {}", entry.path));
                }
                let files = reader.entries().iter().filter(|e| !e.is_dir).count();
                console::say(&format!(
                    "파일 {files}개, {}\n",
                    myzip_core::format_size(reader.total_size())
                ));
            }
            Err(err) => {
                console::error(&format!("{}: {}", archive.display(), describe(&err)));
                code = 1;
            }
        }
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(mode: Mode, paths: &[&str]) -> Request {
        Request {
            mode,
            paths: paths.iter().map(PathBuf::from).collect(),
            password: None,
        }
    }

    #[test]
    fn compress_requests_merge_into_one() {
        let jobs = plan(vec![
            req(Mode::CompressZip, &[r"C:\d\a.txt"]),
            req(Mode::CompressZip, &[r"C:\d\b.txt"]),
            req(Mode::CompressZip, &[r"C:\D\A.TXT"]), // 같은 파일
            req(Mode::CompressTgz, &[r"C:\d\c.txt"]),
        ]);
        assert_eq!(jobs.len(), 2);
        let Job::Compress { sources, .. } = &jobs[0] else {
            panic!()
        };
        assert_eq!(sources.len(), 2);
    }

    #[test]
    fn extract_requests_stay_separate() {
        let jobs = plan(vec![
            req(Mode::ExtractTo, &[r"C:\d\a.zip"]),
            req(Mode::ExtractTo, &[r"C:\d\b.zip", r"C:\d\c.rar"]),
            req(Mode::ExtractTo, &[r"C:\d\a.zip"]),
            req(Mode::Test, &[r"C:\d\a.zip"]),
        ]);
        assert_eq!(jobs.len(), 4);
        assert!(matches!(&jobs[3], Job::Test { .. }));
        let Job::Extract { plan, .. } = &jobs[0] else {
            panic!()
        };
        assert!(plan.subfolder);
    }

    #[test]
    fn window_modes_are_deferred() {
        let jobs = plan(vec![
            req(Mode::Open, &[r"C:\a.zip"]),
            req(Mode::ExtractHere, &[]),
        ]);
        assert!(jobs.iter().all(|j| matches!(j, Job::Window { .. })));
    }

    #[test]
    fn reveal_single_top_item() {
        let dest = Path::new(r"C:\d");
        let written = [
            PathBuf::from(r"C:\d\자료"),
            PathBuf::from(r"C:\d\자료\a.txt"),
        ];
        assert_eq!(to_reveal(dest, &written), PathBuf::from(r"C:\d\자료"));
        let written = [PathBuf::from(r"C:\d\a.txt"), PathBuf::from(r"C:\d\b.txt")];
        assert_eq!(to_reveal(dest, &written), PathBuf::from(r"C:\d"));
    }

    #[test]
    fn password_retry_keeps_plan() {
        let job = Job::Extract {
            archive: "a.zip".into(),
            password: None,
            plan: ExtractPlan::for_mode(Mode::ExtractTo),
        };
        let Job::Extract { password, plan, .. } = job.with_password("비밀".into()) else {
            panic!()
        };
        assert_eq!(password.as_deref(), Some("비밀"));
        assert!(plan.subfolder);
    }

    #[test]
    fn summary_counts() {
        let s = Summary::new(&[
            Outcome::Extracted {
                archive: "a.zip".into(),
                dest: "a".into(),
                reveal: "a".into(),
                open_after: true,
            },
            Outcome::Failed {
                subject: "b.zip".into(),
                message: "깨짐".into(),
            },
        ]);
        assert_eq!(s.total, 2);
        assert!(s.failure_report().unwrap().contains("2개 중 1개"));
        assert!(s.to_open().is_none(), "실패가 있으면 탐색기를 열지 않는다");
        assert_eq!(s.exit_code(), 1);
    }
}
