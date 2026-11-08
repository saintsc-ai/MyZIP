//! 7-Zip 콘솔(7z.exe) 래퍼.
//!
//! RAR 은 RARLAB 독점 포맷이라 자유롭게 쓸 수 있는 순수 구현이 없다. 해제만이라도
//! 하려면 외부 엔진이 필요하고, 7-Zip 이 가장 널리 쓰이며 재배포도 가능하다.
//! (단, 7-Zip 안의 unRAR 코드 라이선스상 RAR '생성'에는 쓸 수 없다.)

use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use chrono::NaiveDateTime;

use crate::entry::Progress;
use crate::error::{Error, Result, cancelled};

/// 7z 엔진 경로를 직접 지정할 때 쓰는 환경 변수.
pub const ENV_OVERRIDE: &str = "MYZIP_7Z";

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

/// 7z.exe 가 있을 만한 곳들을 우선순위 순으로 내놓는다.
fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();

    if let Some(path) = std::env::var_os(ENV_OVERRIDE) {
        out.push(PathBuf::from(path));
    }

    // 1) 우리가 함께 배포한 것. 개발 중에는 target/debug 위쪽 저장소의 bin/ 도 본다.
    if let Some(dir) = exe_dir() {
        out.push(dir.join("bin").join("7z.exe"));
        out.push(dir.join("7z.exe"));
        for ancestor in dir.ancestors().skip(1).take(5) {
            out.push(ancestor.join("bin").join("7z.exe"));
        }
    }

    // 2) 레지스트리에 등록된 7-Zip 설치 경로
    #[cfg(windows)]
    {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            if let Ok(key) = RegKey::predef(root).open_subkey(r"SOFTWARE\7-Zip")
                && let Ok(path) = key.get_value::<String, _>("Path")
            {
                out.push(PathBuf::from(path).join("7z.exe"));
            }
        }
    }

    // 3) 표준 설치 위치
    for env in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        if let Some(base) = std::env::var_os(env) {
            out.push(PathBuf::from(base).join("7-Zip").join("7z.exe"));
        }
    }

    // 4) PATH
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for name in ["7z.exe", "7z", "7za.exe"] {
                out.push(dir.join(name));
            }
        }
    }
    out
}

static CACHED: Mutex<Option<PathBuf>> = Mutex::new(None);

/// 사용 가능한 7z 실행 파일 경로. 없으면 None.
pub fn find_7z(refresh: bool) -> Option<PathBuf> {
    let mut cached = CACHED.lock().unwrap_or_else(|e| e.into_inner());
    if !refresh && let Some(path) = cached.as_ref().filter(|p| p.is_file()) {
        return Some(path.clone());
    }
    *cached = candidates().into_iter().find(|p| p.is_file());
    cached.clone()
}

pub fn require_7z() -> Result<PathBuf> {
    find_7z(false).ok_or_else(|| {
        Error::SevenZipMissing(
            "RAR 압축 해제 엔진(7z.exe)을 찾지 못했습니다.\n\
             7-Zip 을 설치하거나, MyZIP 설치 폴더의 bin 폴더에 7z.exe 와 7z.dll 을 넣어 주세요."
                .into(),
        )
    })
}

/// 콘솔 창이 뜨지 않는 7z 명령.
fn command(exe: &Path) -> Command {
    let mut cmd = Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.stdin(Stdio::null());
    cmd
}

/// 암호를 주지 않으면 7z 가 콘솔에서 입력을 기다리며 멈춘다.
/// 빈 값으로라도 반드시 넘겨서 대화형 대기를 막는다.
fn password_arg(password: Option<&str>) -> OsString {
    OsString::from(format!("-p{}", password.unwrap_or("")))
}

fn mentions_password(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "wrong password",
        "password is incorrect",
        "cannot open encrypted",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn clip(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SevenZipEntry {
    pub path: String,
    pub size: u64,
    pub packed: u64,
    pub mtime: Option<NaiveDateTime>,
    pub is_dir: bool,
    pub crc: String,
    pub encrypted: bool,
    pub method: String,
}

/// `7z l -slt` 출력을 파싱해 항목 목록을 만든다.
pub fn list_entries(archive: &Path, password: Option<&str>) -> Result<Vec<SevenZipEntry>> {
    let exe = require_7z()?;
    let output = command(&exe)
        .args(["l", "-slt"])
        .arg(archive)
        .args(["-sccUTF-8", "-scsUTF-8"])
        .arg(password_arg(password))
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        let text = format!("{stdout}{}", String::from_utf8_lossy(&output.stderr));
        if mentions_password(&text) {
            return Err(Error::PasswordRequired(
                "암호가 필요하거나 틀렸습니다.".into(),
            ));
        }
        return Err(Error::Archive(format!(
            "목록을 읽지 못했습니다:\n{}",
            clip(text.trim(), 500)
        )));
    }
    Ok(parse_listing(&stdout))
}

/// 헤더와 본문은 '----------' 로 나뉘고, 항목끼리는 빈 줄로 나뉜다.
pub(crate) fn parse_listing(stdout: &str) -> Vec<SevenZipEntry> {
    let mut entries = Vec::new();
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut in_body = false;

    for line in stdout.lines() {
        if !in_body {
            in_body = line.starts_with("----------");
            continue;
        }
        if line.trim().is_empty() {
            if !fields.is_empty() {
                entries.push(to_entry(&fields));
                fields.clear();
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(" = ") {
            fields.push((key.trim().to_owned(), value.trim().to_owned()));
        } else if let Some(key) = line.strip_suffix(" =") {
            fields.push((key.trim().to_owned(), String::new()));
        }
    }
    if !fields.is_empty() {
        entries.push(to_entry(&fields));
    }
    entries
}

fn to_entry(fields: &[(String, String)]) -> SevenZipEntry {
    let get = |key: &str| {
        fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    };
    let int = |key: &str| get(key).parse::<u64>().unwrap_or(0);
    SevenZipEntry {
        path: get("Path").replace('\\', "/"),
        size: int("Size"),
        packed: int("Packed Size"),
        mtime: parse_time(get("Modified")),
        is_dir: get("Folder") == "+" || get("Attributes").starts_with('D'),
        crc: get("CRC").to_owned(),
        encrypted: get("Encrypted") == "+",
        method: get("Method").to_owned(),
    }
}

fn parse_time(value: &str) -> Option<NaiveDateTime> {
    let value: String = value.trim().chars().take(19).collect();
    NaiveDateTime::parse_from_str(&value, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(&value, "%Y-%m-%d %H:%M"))
        .ok()
}

/// 아카이브를 dest 로 푼다. members 가 없으면 전체.
pub fn extract(
    archive: &Path,
    dest: &Path,
    members: Option<&[String]>,
    password: Option<&str>,
    progress: Option<&mut Progress>,
) -> Result<()> {
    let exe = require_7z()?;
    fs::create_dir_all(dest)?;

    let mut out_arg = OsString::from("-o");
    out_arg.push(dest);

    let mut cmd = command(&exe);
    cmd.arg("x")
        .arg(archive)
        .arg(out_arg)
        .args(["-y", "-bsp1", "-bb0", "-sccUTF-8", "-scsUTF-8"])
        .arg(password_arg(password));

    // 파일 이름이 많으면 명령줄 길이 제한(32767자)에 걸리므로 목록 파일을 쓴다.
    let listfile = match members {
        Some(names) if !names.is_empty() => {
            let path = dest.join(format!(".myzip-filelist-{}.txt", std::process::id()));
            fs::write(&path, names.join("\n"))?;
            let mut arg = OsString::from("-i@");
            arg.push(&path);
            cmd.arg(arg);
            Some(path)
        }
        _ => None,
    };

    let result = stream(cmd, progress);
    if let Some(path) = listfile {
        let _ = fs::remove_file(path);
    }
    result
}

/// 출력 한 줄에서 진행률(%)을 찾는다. " 45% 12 - foo.txt" 같은 꼴이다.
fn percent_in(line: &str) -> Option<u64> {
    let pos = line.find('%')?;
    let digits: String = line[..pos]
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() || digits.len() > 3 {
        return None;
    }
    digits
        .chars()
        .rev()
        .collect::<String>()
        .parse::<u64>()
        .ok()
        .map(|p| p.min(100))
}

/// 7z 를 돌리며 진행률을 읽어 progress 에 반영한다.
fn stream(mut cmd: Command, mut progress: Option<&mut Progress>) -> Result<()> {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;

    // stderr 를 따로 읽지 않으면 버퍼가 차서 7z 가 멈출 수 있다.
    let mut stderr = child.stderr.take().expect("stderr piped");
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });

    let mut stdout = child.stdout.take().expect("stdout piped");
    let mut tail: Vec<String> = Vec::new();
    let mut line = Vec::new();
    let mut buf = [0u8; 4096];
    let mut outcome: Result<()> = Ok(());

    'read: loop {
        let n = match stdout.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        for &byte in &buf[..n] {
            if byte != b'\r' && byte != b'\n' && byte != 0x08 {
                line.push(byte);
                continue;
            }
            let text = String::from_utf8_lossy(&line).trim().to_owned();
            line.clear();
            if text.is_empty() {
                continue;
            }
            if let Some(p) = progress.as_deref_mut() {
                if let Some(pct) = percent_in(&text)
                    && p.total_bytes > 0
                {
                    p.done_bytes = p.total_bytes * pct / 100;
                    p.notify();
                }
                if p.is_cancelled() {
                    outcome = Err(cancelled());
                    break 'read;
                }
            }
            tail.push(text);
            if tail.len() > 30 {
                tail.remove(0);
            }
        }
    }

    if outcome.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    let stderr_text = err_reader.join().unwrap_or_default();
    outcome?;

    // 0 = 성공, 1 = 경고(일부 파일 건너뜀)
    match status.code() {
        Some(0 | 1) => Ok(()),
        code => {
            let text = format!("{}\n{}", tail.join("\n"), stderr_text.trim());
            if mentions_password(&text) {
                return Err(Error::PasswordRequired("암호가 틀렸습니다.".into()));
            }
            Err(Error::Archive(format!(
                "압축 해제에 실패했습니다 (코드 {}):\n{}",
                code.map_or_else(|| "?".into(), |c| c.to_string()),
                clip(text.trim(), 500)
            )))
        }
    }
}

/// 무결성 검사. (정상 여부, 7z 가 남긴 메시지).
pub fn test_archive(archive: &Path, password: Option<&str>) -> Result<(bool, String)> {
    let exe = require_7z()?;
    let output = command(&exe)
        .arg("t")
        .arg(archive)
        .args(["-sccUTF-8", "-scsUTF-8"])
        .arg(password_arg(password))
        .output()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok((output.status.success(), text.trim().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_is_parsed() {
        let out = "\
7-Zip 24.09\n\nPath = x.rar\nType = Rar5\n\n----------\n\
Path = 폴더\nFolder = +\nSize = 0\nAttributes = D\n\n\
Path = 폴더\\보고서.txt\nFolder = -\nSize = 120\nPacked Size = 80\n\
Modified = 2026-03-01 12:34:56.1234567\nCRC = 1A2B3C4D\nEncrypted = -\nMethod = v5:128K\n\n";
        let entries = parse_listing(out);
        assert_eq!(entries.len(), 2);
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].path, "폴더/보고서.txt");
        assert_eq!(entries[1].size, 120);
        assert_eq!(entries[1].packed, 80);
        assert_eq!(entries[1].crc, "1A2B3C4D");
        assert_eq!(entries[1].mtime.unwrap().to_string(), "2026-03-01 12:34:56");
    }

    #[test]
    fn percent_parsing() {
        assert_eq!(percent_in("45% 12 - foo.txt"), Some(45));
        assert_eq!(percent_in("100%"), Some(100));
        assert_eq!(percent_in("Everything is Ok"), None);
    }
}
