//! 셸 통합 작업: 요청을 작업 목록으로 바꾸고, 차례로 처리하고, 결과를 알린다.
//!
//! 탐색기에서 여러 파일을 골라 메뉴를 누르면 요청이 여러 개 모인다.
//!
//! * 압축(--compress-zip/tgz): 같은 동작의 요청을 모두 합쳐 아카이브 하나로 묶는다.
//! * 압축 풀기(--extract-here/to)와 검사(--test): 아카이브마다 따로 처리한다.
//!   --extract-to 는 아카이브마다 자기 이름의 폴더를 만들어 푼다.
//!
//! 하나가 실패해도 나머지는 계속 처리하고, 끝에 한 번만 알린다.
//! 알림창을 파일마다 띄우면 30개를 골랐을 때 30번 눌러야 한다.

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

/// 실제로 할 일 하나.
#[derive(Debug, Clone, PartialEq)]
pub enum Job {
    Extract {
        mode: Mode,
        archive: PathBuf,
        password: Option<String>,
    },
    Test {
        archive: PathBuf,
        password: Option<String>,
    },
    Compress {
        mode: Mode,
        sources: Vec<PathBuf>,
        password: Option<String>,
    },
    /// 창이 필요한 동작 (열기, 대화상자). GUI 단계에서 구현한다.
    Window { mode: Mode, paths: Vec<PathBuf> },
}

/// 같은 파일인지 비교할 열쇠. Windows 경로는 대소문자를 가리지 않는다.
fn path_key(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

/// 모인 요청들을 작업 목록으로 바꾼다. 도착 순서를 지킨다.
pub fn plan(requests: Vec<Request>) -> Vec<Job> {
    let mut jobs: Vec<Job> = Vec::new();
    let mut seen: HashSet<(Mode, String)> = HashSet::new();

    for request in requests {
        let Request {
            mode,
            paths,
            password,
        } = request;
        match mode {
            Mode::CompressZip | Mode::CompressTgz if !paths.is_empty() => {
                // 같은 동작·같은 암호의 압축은 하나로 합친다.
                let group = jobs.iter_mut().find_map(|job| match job {
                    Job::Compress {
                        mode: m,
                        sources,
                        password: p,
                    } if *m == mode && *p == password => Some(sources),
                    _ => None,
                });
                let sources = match group {
                    Some(sources) => sources,
                    None => {
                        jobs.push(Job::Compress {
                            mode,
                            sources: Vec::new(),
                            password,
                        });
                        match jobs.last_mut() {
                            Some(Job::Compress { sources, .. }) => sources,
                            _ => unreachable!(),
                        }
                    }
                };
                for path in paths {
                    if seen.insert((mode, path_key(&path))) {
                        sources.push(path);
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
                            mode,
                            archive,
                            password: password.clone(),
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
#[derive(Debug)]
enum Outcome {
    Extracted {
        archive: PathBuf,
        dest: PathBuf,
        opened: PathBuf,
    },
    Compressed {
        target: PathBuf,
        note: Option<String>,
    },
    Tested {
        archive: PathBuf,
        bad: Vec<String>,
    },
    Failed {
        subject: PathBuf,
        message: String,
    },
    NeedsWindow,
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// 사용자에게 보여 줄 오류 문장.
fn describe(err: &Error, password_given: bool) -> String {
    match err {
        Error::PasswordRequired(_) if !password_given => {
            "암호가 걸린 아카이브입니다. 암호 입력 창은 다음 단계(GUI)에서 추가됩니다.\n\
             지금은 명령줄에서 --password 로 암호를 주세요."
                .into()
        }
        Error::PasswordRequired(_) => "암호가 틀렸습니다.".into(),
        Error::Io(e) => format!("파일을 다루지 못했습니다: {e}"),
        other => other.to_string(),
    }
}

/// 설정의 충돌 처리 방식. 'ask' 는 묻는 창이 생기기 전까지 이름 바꾸기로 처리한다.
fn conflict_policy() -> ConflictAction {
    ConflictAction::parse(&config::get("extract/conflict"))
        .filter(|a| *a != ConflictAction::Cancel)
        .unwrap_or(ConflictAction::Rename)
}

/// 풀 위치. --extract-to 는 아카이브 이름의 폴더다 (자료.tar.gz -> 자료\).
fn destination(mode: Mode, archive: &Path) -> myzip_core::Result<(PathBuf, bool)> {
    let parent = archive.parent().unwrap_or(Path::new(".")).to_path_buf();
    if mode != Mode::ExtractTo {
        return Ok((parent, false));
    }
    let mut dest = parent.join(strip_archive_suffix(&file_name(archive)));
    if dest.exists() && !dest.is_dir() {
        // 같은 이름의 '파일'이 있으면 폴더를 만들 수 없다. 이름을 비켜 간다.
        dest = unique_path(&dest)?;
    }
    // 아카이브 안이 폴더 하나뿐이면 벗겨 낸다. 자료\자료\... 가 되지 않게.
    Ok((dest, config::get_bool("extract/strip_redundant")))
}

/// 해제가 끝난 뒤 탐색기에서 보여 줄 것. 최상위에 항목이 하나만 생겼으면 그것을
/// 선택하고, 아니면 풀린 폴더를 연다.
fn to_reveal(dest: &Path, written: &[PathBuf]) -> PathBuf {
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

fn extract(mode: Mode, archive: &Path, password: Option<&str>) -> Outcome {
    let result = (|| {
        let mut reader = open_archive(archive, password)?;
        let (dest, strip_root) = destination(mode, archive)?;
        let policy = conflict_policy();
        let mut ask = |_: &Path, _: &Entry| policy;
        let options = ExtractOptions {
            members: None,
            strip_root,
            on_conflict: Some(&mut ask),
        };
        let written = reader.extract(&dest, options, &mut Progress::new())?;
        let opened = if mode == Mode::ExtractTo {
            dest.clone()
        } else {
            to_reveal(&dest, &written)
        };
        Ok::<_, Error>((dest, opened))
    })();
    match result {
        Ok((dest, opened)) => Outcome::Extracted {
            archive: archive.to_path_buf(),
            dest,
            opened,
        },
        Err(err) => Outcome::Failed {
            subject: archive.to_path_buf(),
            message: describe(&err, password.is_some()),
        },
    }
}

fn test(archive: &Path, password: Option<&str>) -> Outcome {
    match open_archive(archive, password).and_then(|mut r| r.test(&mut Progress::new())) {
        Ok(bad) => Outcome::Tested {
            archive: archive.to_path_buf(),
            bad,
        },
        Err(err) => Outcome::Failed {
            subject: archive.to_path_buf(),
            message: describe(&err, password.is_some()),
        },
    }
}

/// 만들 아카이브 이름. Python 판과 같은 규칙이다.
///
/// * 하나면 그 이름: 폴더 `자료` -> `자료.zip`, 파일 `보고서.hwp` -> `보고서.zip`
/// * 여러 개면 들어 있는 폴더 이름: `D:\프로젝트\a.txt, b.txt` -> `프로젝트.zip`
fn compress_target(sources: &[PathBuf], extension: &str) -> myzip_core::Result<PathBuf> {
    let first = &sources[0];
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
        .filter(|s| !s.is_empty());
    let stem = stem.unwrap_or_else(|| "압축파일".into());
    unique_path(&parent.join(format!("{stem}{extension}")))
}

fn compress(mode: Mode, sources: &[PathBuf], password: Option<&str>) -> Outcome {
    let zip = mode == Mode::CompressZip;
    let subject = sources.first().cloned().unwrap_or_default();
    let result = (|| {
        let existing: Vec<PathBuf> = sources.iter().filter(|p| p.exists()).cloned().collect();
        if existing.is_empty() {
            return Err(Error::Archive("압축할 파일이 없습니다.".into()));
        }
        let target = compress_target(&existing, if zip { ".zip" } else { ".tar.gz" })?;
        let items = walk_inputs(&existing);
        let level = config::get_int("compress/level").clamp(0, 9);
        let password = if zip { password } else { None };
        writer_for(&target, level, password)?.write(&items, &mut Progress::new())?;
        Ok(target)
    })();
    match result {
        Ok(target) => Outcome::Compressed {
            target,
            note: (!zip && password.is_some())
                .then(|| "TAR.GZ 는 암호를 지원하지 않아 암호 없이 만들었습니다.".to_owned()),
        },
        Err(err) => Outcome::Failed {
            subject,
            message: describe(&err, password.is_some()),
        },
    }
}

fn run_job(job: &Job) -> Outcome {
    match job {
        Job::Extract {
            mode,
            archive,
            password,
        } => {
            if !archive.is_file() {
                return Outcome::Failed {
                    subject: archive.clone(),
                    message: "파일을 찾을 수 없습니다.".into(),
                };
            }
            extract(*mode, archive, password.as_deref())
        }
        Job::Test { archive, password } => test(archive, password.as_deref()),
        Job::Compress {
            mode,
            sources,
            password,
        } => compress(*mode, sources, password.as_deref()),
        Job::Window { .. } => Outcome::NeedsWindow,
    }
}

/// 요청들을 처리하고 결과를 알린다. 프로세스 종료 코드를 돌려준다.
pub fn execute(requests: Vec<Request>) -> i32 {
    let jobs = plan(requests);
    let outcomes: Vec<Outcome> = jobs.iter().map(run_job).collect();
    report(&outcomes)
}

fn report(outcomes: &[Outcome]) -> i32 {
    let mut failures: Vec<String> = Vec::new();
    let mut damaged: Vec<String> = Vec::new();
    let mut tested_ok: Vec<String> = Vec::new();
    let mut extracted: Vec<&PathBuf> = Vec::new();
    let mut needs_window = false;
    let console_mode = console::has_output();

    for outcome in outcomes {
        match outcome {
            Outcome::Extracted {
                archive,
                dest,
                opened,
            } => {
                if console_mode {
                    console::say(&format!(
                        "압축을 풀었습니다: {} -> {}",
                        archive.display(),
                        dest.display()
                    ));
                }
                extracted.push(opened);
            }
            Outcome::Compressed { target, note } => {
                if console_mode {
                    console::say(&format!("압축했습니다: {}", target.display()));
                }
                if let Some(note) = note {
                    notify::show(Kind::Warning, "알림", note);
                }
            }
            Outcome::Tested { archive, bad } if bad.is_empty() => {
                tested_ok.push(file_name(archive))
            }
            Outcome::Tested { archive, bad } => {
                let mut lines =
                    format!("{} — 문제가 있는 항목 {}개:", file_name(archive), bad.len());
                for item in bad.iter().take(20) {
                    lines.push_str(&format!("\n  {item}"));
                }
                damaged.push(lines);
            }
            Outcome::Failed { subject, message } => {
                failures.push(format!("{}\n  {message}", file_name(subject)))
            }
            Outcome::NeedsWindow => needs_window = true,
        }
    }

    // 압축 풀기 결과는 하나일 때만 탐색기로 보여 준다. 여러 개를 풀었다면
    // 사용자는 이미 그 폴더를 보고 있고, 창이 여러 개 뜨면 오히려 성가시다.
    if extracted.len() == 1 && failures.is_empty() && config::get_bool("extract/open_after") {
        notify::open_in_explorer(extracted[0]);
    }

    if !tested_ok.is_empty() || !damaged.is_empty() {
        if damaged.is_empty() {
            let message = if tested_ok.len() == 1 {
                format!("{} — 이상이 없습니다.", tested_ok[0])
            } else {
                format!("{}개 아카이브 모두 이상이 없습니다.", tested_ok.len())
            };
            notify::show(Kind::Info, "검사 완료", &message);
        } else {
            let mut message = damaged.join("\n\n");
            if !tested_ok.is_empty() {
                message.push_str(&format!("\n\n이상 없음: {}", tested_ok.join(", ")));
            }
            notify::show(Kind::Warning, "손상 발견", &message);
        }
    }

    if !failures.is_empty() {
        let total = outcomes
            .iter()
            .filter(|o| !matches!(o, Outcome::NeedsWindow))
            .count();
        let head = if total > 1 {
            format!(
                "{total}개 중 {}개를 처리하지 못했습니다.\n\n",
                failures.len()
            )
        } else {
            String::new()
        };
        let shown: Vec<&String> = failures.iter().take(15).collect();
        let mut message = head
            + &shown
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
        if failures.len() > shown.len() {
            message.push_str(&format!("\n\n… 외 {}개", failures.len() - shown.len()));
        }
        notify::show(Kind::Error, "오류", &message);
    }

    if needs_window {
        notify::show(
            Kind::Info,
            "준비 중",
            "창을 띄우는 기능(열기, 압축 풀기..., 압축하기...)은 다음 단계(GUI)에서 추가됩니다.\n\
             지금은 '여기에 압축 풀기', '압축 풀기 (폴더 생성)', 'ZIP/TAR.GZ 로 압축', '무결성 검사'를 쓸 수 있습니다.",
        );
    }

    if failures.is_empty() && damaged.is_empty() && !needs_window {
        0
    } else {
        1
    }
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
                console::error(&format!(
                    "{}: {}",
                    archive.display(),
                    describe(&err, request.password.is_some())
                ));
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
}
