//! 컨텍스트 메뉴가 호출하는 명령들이 실제로 동작하는지 검증.
//! Python 판 tests/test_cli.py 를 옮기고, 여러 파일을 고른 경우를 더했다.
//!
//! 모든 테스트는
//! * MYZIP_NO_SHELL  — 탐색기 창과 메시지 상자를 띄우지 않는다
//! * MYZIP_REG_ROOT  — 레지스트리를 테스트마다 다른 샌드박스 키 아래로 옮긴다
//!   (사용자의 실제 탐색기 설정과 MyZIP 설정을 건드리지 않는다)
//!
//! 를 켜고 돈다.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

const EXE: &str = env!("CARGO_BIN_EXE_MyZIP");

/// 테스트 하나의 작업 공간과 레지스트리 샌드박스.
struct Sandbox {
    dir: tempfile::TempDir,
    reg_root: String,
}

impl Sandbox {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        Self {
            dir: tempfile::tempdir().unwrap(),
            reg_root: format!(r"Software\MyZIP-Test\{}-{n}", std::process::id()),
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(EXE);
        cmd.args(args)
            .env("MYZIP_NO_SHELL", "1")
            .env("MYZIP_REG_ROOT", &self.reg_root)
            .env("MYZIP_NO_IPC", "1")
            .current_dir(self.path());
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn reg(&self) -> RegKey {
        RegKey::predef(HKEY_CURRENT_USER)
    }

    fn reg_get(&self, path: &str, name: &str) -> Option<String> {
        self.reg()
            .open_subkey(format!(r"{}\{path}", self.reg_root))
            .ok()?
            .get_value(name)
            .ok()
    }

    fn set_setting(&self, key: &str, value: &str) {
        let (group, name) = key.split_once('/').unwrap();
        let (k, _) = self
            .reg()
            .create_subkey(format!(r"{}\Software\MyZIP\MyZIP\{group}", self.reg_root))
            .unwrap();
        k.set_value(name, &value).unwrap();
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = self.reg().delete_subkey_all(&self.reg_root);
    }
}

fn text(out: &Output) -> String {
    format!(
        "rc={:?}\nstdout={}\nstderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn arg(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// 압축할 원본 트리: 자료\메모.txt, 자료\안쪽\data.bin
fn make_tree(root: &Path) -> PathBuf {
    let src = root.join("자료");
    fs::create_dir_all(src.join("안쪽")).unwrap();
    fs::write(src.join("메모.txt"), "테스트 내용\n".repeat(50)).unwrap();
    fs::write(src.join("안쪽").join("data.bin"), [0u8, 1, 2].repeat(10000)).unwrap();
    src
}

#[test]
fn help_and_version() {
    let sb = Sandbox::new();
    let out = sb.run(&["--version"]);
    assert!(
        out.status.success() && String::from_utf8_lossy(&out.stdout).contains("MyZIP"),
        "{}",
        text(&out)
    );

    let out = sb.run(&["--help"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("압축") && stdout.contains("--extract-to"),
        "{}",
        text(&out)
    );

    let out = sb.run(&["--extract-here", "--test", "x.zip"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
}

#[test]
fn compress_then_extract_here_and_to() {
    let sb = Sandbox::new();
    let src = make_tree(sb.path());

    for (flag, name) in [
        ("--compress-zip", "자료.zip"),
        ("--compress-tgz", "자료.tar.gz"),
    ] {
        let out = sb.run(&[flag, arg(&src)]);
        let archive = sb.path().join(name);
        assert!(
            out.status.success() && archive.metadata().map(|m| m.len() > 0).unwrap_or(false),
            "{flag}: {}",
            text(&out)
        );

        // --extract-here: 아카이브가 있는 폴더에 그대로 푼다 (자료\ 가 생긴다).
        let here = sb.path().join(format!("here-{name}"));
        fs::create_dir_all(&here).unwrap();
        let copied = here.join(name);
        fs::copy(&archive, &copied).unwrap();
        let out = sb.run(&["--extract-here", arg(&copied)]);
        assert!(out.status.success(), "{}", text(&out));
        assert_eq!(
            fs::read(here.join("자료").join("메모.txt")).unwrap(),
            fs::read(src.join("메모.txt")).unwrap()
        );

        // --extract-to: 아카이브 이름의 폴더. 안의 중복 '자료' 폴더는 벗겨진다.
        let to = sb.path().join(format!("to-{name}"));
        fs::create_dir_all(&to).unwrap();
        let copied = to.join(name);
        fs::copy(&archive, &copied).unwrap();
        let out = sb.run(&["--extract-to", arg(&copied)]);
        assert!(out.status.success(), "{}", text(&out));
        assert!(to.join("자료").join("메모.txt").is_file(), "{}", text(&out));
        assert!(to.join("자료").join("안쪽").join("data.bin").is_file());
        assert!(
            !to.join("자료").join("자료").exists(),
            "중복 폴더가 벗겨지지 않음"
        );
    }
}

/// 여러 아카이브를 골라 '압축 풀기 (폴더 생성)' — 각자 자기 이름의 폴더로.
#[test]
fn extract_to_many_archives_each_into_own_folder() {
    let sb = Sandbox::new();
    let src = make_tree(sb.path());
    let names = ["첫째.zip", "둘째.tar.gz", "셋째.tar.xz", "넷째.tgz"];
    let archives: Vec<PathBuf> = names
        .iter()
        .map(|n| sb.path().join("모음").join(n))
        .collect();
    fs::create_dir_all(sb.path().join("모음")).unwrap();
    for archive in &archives {
        let mut writer = myzip_core::writer_for(archive, 6, None).unwrap();
        writer
            .write(
                &myzip_core::walk_inputs(&[&src]),
                &mut myzip_core::Progress::new(),
            )
            .unwrap();
    }

    let mut args = vec!["--extract-to"];
    args.extend(archives.iter().map(|a| arg(a)));
    let out = sb.run(&args);
    assert!(out.status.success(), "{}", text(&out));

    for folder in ["첫째", "둘째", "셋째", "넷째"] {
        let dest = sb.path().join("모음").join(folder);
        assert!(dest.join("메모.txt").is_file(), "{folder}: {}", text(&out));
        assert!(dest.join("안쪽").join("data.bin").is_file(), "{folder}");
    }
}

/// 여러 아카이브를 골라 '여기에 압축 풀기' — 같은 이름이 겹치면 설정(기본: 이름 바꾸기)대로.
#[test]
fn extract_here_many_archives_with_conflicts() {
    let sb = Sandbox::new();
    let work = sb.path().join("겹침");
    fs::create_dir_all(&work).unwrap();
    let mut archives = Vec::new();
    for (i, name) in ["가.zip", "나.zip"].iter().enumerate() {
        let file = sb.path().join(format!("같은이름-{i}.txt"));
        fs::write(&file, format!("내용 {i}")).unwrap();
        let archive = work.join(name);
        let mut writer = myzip_core::writer_for(&archive, 6, None).unwrap();
        writer
            .write(
                &[(file, "같은이름.txt".into())],
                &mut myzip_core::Progress::new(),
            )
            .unwrap();
        archives.push(archive);
    }

    let out = sb.run(&["--extract-here", arg(&archives[0]), arg(&archives[1])]);
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        fs::read_to_string(work.join("같은이름.txt")).unwrap(),
        "내용 0"
    );
    assert_eq!(
        fs::read_to_string(work.join("같은이름 (2).txt")).unwrap(),
        "내용 1"
    );

    // 설정을 '건너뛰기'로 바꾸면 더 생기지 않는다.
    sb.set_setting("extract/conflict", "skip");
    let out = sb.run(&["--extract-here", arg(&archives[1])]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(!work.join("같은이름 (3).txt").exists());
}

/// 하나가 망가져도 나머지는 풀고, 실패한 것만 알린다.
#[test]
fn batch_continues_after_failure() {
    let sb = Sandbox::new();
    let src = make_tree(sb.path());
    let good = sb.path().join("멀쩡.zip");
    let mut writer = myzip_core::writer_for(&good, 6, None).unwrap();
    writer
        .write(
            &myzip_core::walk_inputs(&[&src]),
            &mut myzip_core::Progress::new(),
        )
        .unwrap();
    let broken = sb.path().join("망가짐.zip");
    fs::write(&broken, b"PK\x03\x04 this is not really a zip").unwrap();
    let missing = sb.path().join("없음.zip");

    let out = sb.run(&["--extract-to", arg(&broken), arg(&good), arg(&missing)]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(
        sb.path().join("멀쩡").join("메모.txt").is_file(),
        "{}",
        text(&out)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("망가짐.zip") && stderr.contains("없음.zip"),
        "{}",
        text(&out)
    );
    assert!(stderr.contains("3개 중 2개"), "{}", text(&out));
}

#[test]
fn strip_redundant_setting_is_honoured() {
    let sb = Sandbox::new();
    let src = make_tree(sb.path());
    let out = sb.run(&["--compress-zip", arg(&src)]);
    assert!(out.status.success(), "{}", text(&out));
    let stage = sb.path().join("stage");
    fs::create_dir_all(&stage).unwrap();
    fs::copy(sb.path().join("자료.zip"), stage.join("자료.zip")).unwrap();

    sb.set_setting("extract/strip_redundant", "false");
    let out = sb.run(&["--extract-to", arg(&stage.join("자료.zip"))]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(
        stage.join("자료").join("자료").join("메모.txt").is_file(),
        "{}",
        text(&out)
    );
}

#[test]
fn compress_many_files_names_archive_after_folder() {
    let sb = Sandbox::new();
    let folder = sb.path().join("프로젝트");
    fs::create_dir_all(&folder).unwrap();
    let files: Vec<PathBuf> = (1..=3)
        .map(|i| folder.join(format!("문서{i}.txt")))
        .collect();
    for f in &files {
        fs::write(f, "x").unwrap();
    }
    let mut args = vec!["--compress-zip"];
    args.extend(files.iter().map(|f| arg(f)));
    let out = sb.run(&args);
    assert!(out.status.success(), "{}", text(&out));

    let reader = myzip_core::open_archive(&folder.join("프로젝트.zip"), None).unwrap();
    let mut names: Vec<&str> = reader.entries().iter().map(|e| e.path.as_str()).collect();
    names.sort();
    assert_eq!(names, ["문서1.txt", "문서2.txt", "문서3.txt"]);

    // 한 번 더 하면 덮어쓰지 않고 (2) 를 붙인다.
    let out = sb.run(&args);
    assert!(out.status.success(), "{}", text(&out));
    assert!(folder.join("프로젝트 (2).zip").is_file());
}

#[test]
fn password_flow() {
    let sb = Sandbox::new();
    let src = make_tree(sb.path());
    let out = sb.run(&["--compress-zip", "-p", "비밀1234", arg(&src)]);
    assert!(out.status.success(), "{}", text(&out));
    let archive = sb.path().join("자료.zip");
    let stage = sb.path().join("암호");
    fs::create_dir_all(&stage).unwrap();
    fs::copy(&archive, stage.join("자료.zip")).unwrap();
    let copied = stage.join("자료.zip");

    let out = sb.run(&["--extract-to", arg(&copied)]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("암호"),
        "{}",
        text(&out)
    );
    assert!(!stage.join("자료").join("메모.txt").exists());

    let out = sb.run(&["--extract-to", "--password", "틀림", arg(&copied)]);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("암호가 틀렸습니다"),
        "{}",
        text(&out)
    );

    let out = sb.run(&["--extract-to", "--password", "비밀1234", arg(&copied)]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(stage.join("자료").join("메모.txt").is_file());
}

#[test]
fn test_mode_reports_damage() {
    let sb = Sandbox::new();
    let src = make_tree(sb.path());
    assert!(sb.run(&["--compress-tgz", arg(&src)]).status.success());
    let archive = sb.path().join("자료.tar.gz");

    let out = sb.run(&["--test", arg(&archive)]);
    assert!(
        out.status.success() && String::from_utf8_lossy(&out.stdout).contains("이상이 없습니다"),
        "{}",
        text(&out)
    );

    let mut blob = fs::read(&archive).unwrap();
    let mid = blob.len() / 2;
    blob[mid..mid + 32].iter_mut().for_each(|b| *b ^= 0xFF);
    fs::write(&archive, blob).unwrap();
    let out = sb.run(&["--test", arg(&archive)]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
}

#[test]
fn list_prints_entries() {
    let sb = Sandbox::new();
    let src = make_tree(sb.path());
    assert!(sb.run(&["--compress-zip", arg(&src)]).status.success());
    let out = sb.run(&["--list", arg(&sb.path().join("자료.zip"))]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("자료/메모.txt") && stdout.contains("ZIP"),
        "{}",
        text(&out)
    );
}

/// MYZIP_NO_SHELL 에서는 창을 띄우지 않는다. 창이 필요한 동작은 그렇다고 알리고 실패한다.
#[test]
fn window_modes_do_not_open_windows_under_no_shell() {
    let sb = Sandbox::new();
    for args in [&[][..], &["--extract", "x.zip"], &["--compress", "x.txt"]] {
        let out = sb.run(args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {}", text(&out));
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("창"),
            "{args:?}: {}",
            text(&out)
        );
    }
}

/// 풀다가 실패하면 이번에 만든 빈 폴더를 남기지 않는다.
#[test]
fn failed_extract_leaves_no_empty_folder() {
    let sb = Sandbox::new();
    let src = make_tree(sb.path());
    assert!(sb.run(&["--compress-zip", arg(&src)]).status.success());
    let stage = sb.path().join("깨짐");
    fs::create_dir_all(&stage).unwrap();
    let archive = stage.join("자료.zip");
    // 목록(중앙 디렉터리)은 멀쩡하고 내용만 망가진 ZIP.
    let mut blob = fs::read(sb.path().join("자료.zip")).unwrap();
    for b in &mut blob[40..400] {
        *b ^= 0x5A;
    }
    fs::write(&archive, blob).unwrap();

    let out = sb.run(&["--extract-to", arg(&archive)]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(
        !stage.join("자료").exists(),
        "빈 폴더가 남음: {}",
        text(&out)
    );
}

// ---------------------------------------------------------- 여러 프로세스

/// 탐색기처럼 파일마다 프로세스를 하나씩, 거의 동시에 띄운다.
fn spawn_like_explorer(sb: &Sandbox, ipc: &str, flag: &str, paths: &[PathBuf]) -> Vec<Output> {
    let children: Vec<_> = paths
        .iter()
        .map(|p| {
            let mut cmd = sb.command(&[flag, arg(p)]);
            cmd.env_remove("MYZIP_NO_IPC").env("MYZIP_IPC_NAME", ipc);
            cmd.stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            cmd.spawn().unwrap()
        })
        .collect();
    children
        .into_iter()
        .map(|c| c.wait_with_output().unwrap())
        .collect()
}

fn ipc_name(tag: &str) -> String {
    format!("MyZIP.Test.{tag}.{}", std::process::id())
}

/// 파일 6개를 골라 'ZIP 으로 압축' — ZIP 이 6개가 아니라 하나만 생겨야 한다.
#[test]
fn compress_from_many_processes_makes_one_archive() {
    let sb = Sandbox::new();
    let folder = sb.path().join("선택");
    fs::create_dir_all(&folder).unwrap();
    let files: Vec<PathBuf> = (1..=6)
        .map(|i| folder.join(format!("파일{i}.txt")))
        .collect();
    for f in &files {
        fs::write(f, format!("내용 {}", f.display())).unwrap();
    }

    let outputs = spawn_like_explorer(&sb, &ipc_name("compress"), "--compress-zip", &files);
    for out in &outputs {
        assert!(out.status.success(), "{}", text(out));
    }

    let zips: Vec<_> = fs::read_dir(&folder)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".zip"))
        .collect();
    assert_eq!(zips, ["선택.zip"], "아카이브가 하나로 모이지 않음");
    let reader = myzip_core::open_archive(&folder.join("선택.zip"), None).unwrap();
    assert_eq!(reader.entries().len(), 6);
}

/// 아카이브 5개를 골라 '압축 풀기 (폴더 생성)' — 각자 자기 폴더로, 한 프로세스가 차례로.
#[test]
fn extract_to_from_many_processes() {
    let sb = Sandbox::new();
    let src = make_tree(sb.path());
    let folder = sb.path().join("받은파일");
    fs::create_dir_all(&folder).unwrap();
    let archives: Vec<PathBuf> = (1..=5)
        .map(|i| folder.join(format!("묶음{i}.zip")))
        .collect();
    for archive in &archives {
        let mut writer = myzip_core::writer_for(archive, 6, None).unwrap();
        writer
            .write(
                &myzip_core::walk_inputs(&[&src]),
                &mut myzip_core::Progress::new(),
            )
            .unwrap();
    }

    let outputs = spawn_like_explorer(&sb, &ipc_name("extract"), "--extract-to", &archives);
    for out in &outputs {
        assert!(out.status.success(), "{}", text(out));
    }
    for i in 1..=5 {
        let dest = folder.join(format!("묶음{i}"));
        assert!(dest.join("메모.txt").is_file(), "묶음{i}");
        assert!(!dest.join("자료").exists(), "묶음{i}: 중복 폴더");
    }
    // 한 프로세스가 모아서 처리했다면 '풀었습니다' 줄이 한 곳에 다 모인다.
    let busiest = outputs
        .iter()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .matches("압축을 풀었습니다")
                .count()
        })
        .max();
    assert_eq!(busiest, Some(5), "요청이 한 프로세스로 모이지 않음");
}

// ------------------------------------------------------------- 레지스트리

#[test]
fn registry_roundtrip_in_sandbox() {
    let sb = Sandbox::new();
    let out = sb.run(&["--register"]);
    assert!(out.status.success(), "{}", text(&out));

    let compress = r"Software\Classes\*\shell\MyZIP.compress";
    assert_eq!(
        sb.reg_get(compress, "MultiSelectModel").as_deref(),
        Some("Player")
    );
    assert!(
        sb.reg_get(r"Software\Classes\MyZIP.zip", "").is_some(),
        "ProgID 생성"
    );
    assert_eq!(
        sb.reg_get(r"Software\Classes\.zip", "").as_deref(),
        Some("MyZIP.zip"),
        "확장자 연결"
    );

    // 압축 풀기 메뉴에도 Player 가 있어야 16개 이상 골라도 메뉴가 보인다.
    let extract = r"Software\Classes\SystemFileAssociations\.rar\shell\MyZIP";
    assert_eq!(
        sb.reg_get(extract, "MultiSelectModel").as_deref(),
        Some("Player")
    );
    let command = sb
        .reg_get(&format!(r"{extract}\shell\03folder\command"), "")
        .unwrap();
    assert!(
        command.contains("--extract-to \"%1\"") && command.contains("MyZIP"),
        "{command}"
    );
    assert!(
        !command.contains(r"\\?\"),
        "탐색기가 실행 못 하는 경로: {command}"
    );

    let out = sb.run(&["--unregister"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(sb.reg_get(compress, "MUIVerb").is_none(), "메뉴 제거");
    assert!(
        sb.reg_get(r"Software\Classes\MyZIP.zip", "").is_none(),
        "ProgID 제거"
    );
    assert_eq!(
        sb.reg_get(r"Software\Classes\.zip", "").as_deref(),
        Some(""),
        "연결 해제"
    );
}

/// 원래 다른 프로그램에 연결돼 있던 확장자는 제거할 때 되돌린다.
#[test]
fn unregister_restores_previous_association() {
    let sb = Sandbox::new();
    let (k, _) = sb
        .reg()
        .create_subkey(format!(r"{}\Software\Classes\.zip", sb.reg_root))
        .unwrap();
    k.set_value("", &"CompressedFolder").unwrap();

    assert!(sb.run(&["--register"]).status.success());
    assert_eq!(
        sb.reg_get(r"Software\Classes\.zip", "").as_deref(),
        Some("MyZIP.zip")
    );
    assert!(sb.run(&["--unregister"]).status.success());
    assert_eq!(
        sb.reg_get(r"Software\Classes\.zip", "").as_deref(),
        Some("CompressedFolder")
    );
}
