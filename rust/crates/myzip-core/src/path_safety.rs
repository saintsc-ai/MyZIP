//! 압축 해제 시 경로 탈출(Zip Slip) 방지 및 경로 유틸.

use std::io;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Windows 예약 장치명.
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Windows 에서 파일명에 쓸 수 없는 문자.
fn is_illegal(ch: char) -> bool {
    matches!(
        ch,
        '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\x00'..='\x1f'
    )
}

/// 경로 조각 하나를 Windows 에서 안전한 이름으로 바꾼다.
pub fn sanitize_component(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| if is_illegal(c) { '_' } else { c })
        .collect();
    let trimmed = replaced.trim_end_matches([' ', '.']);
    if trimmed.is_empty() {
        return "_".into();
    }
    let stem = trimmed.split('.').next().unwrap_or("").to_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        format!("_{trimmed}")
    } else {
        trimmed.to_owned()
    }
}

/// 아카이브 내부 경로를 쪼개 안전한 조각들로 만든다.
///
/// 상위 이동, 절대경로, 드라이브 문자, UNC 경로를 모두 무력화한다.
pub fn safe_components(member: &str) -> Vec<String> {
    let normalized = member.replace('\\', "/");
    let mut parts: Vec<String> = Vec::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            // 상위로 올라가려는 시도는 통째로 버린다
            continue;
        }
        let mut chars = part.chars();
        if let (Some(_), Some(':'), None) = (chars.next(), chars.next(), chars.next()) {
            continue; // C: 같은 드라이브 지정
        }
        parts.push(sanitize_component(part));
    }
    if parts.is_empty() {
        parts.push("_".into());
    }
    parts
}

/// 아카이브 내부 경로를 base 아래로만 떨어지도록 붙인다.
pub fn safe_join(base: &Path, member: &str) -> Result<PathBuf> {
    let base = std::path::absolute(base)?;
    let mut result = base.clone();
    for part in safe_components(member) {
        result.push(part);
    }

    // 이미 있는 폴더가 심볼릭 링크(정션)라면 여전히 밖으로 샐 수 있다.
    // 실제로 존재하는 가장 깊은 조상을 따라가 base 안인지 확인한다.
    if let Ok(real_base) = dunce::canonicalize(&base) {
        let mut probe = result.clone();
        while probe.starts_with(&base) && probe != base {
            if probe.symlink_metadata().is_ok() {
                let real = dunce::canonicalize(&probe).unwrap_or_else(|_| probe.clone());
                if !real.starts_with(&real_base) {
                    return Err(Error::Archive(format!(
                        "안전하지 않은 경로입니다: {member}"
                    )));
                }
                break;
            }
            if !probe.pop() {
                break;
            }
        }
    }
    Ok(result)
}

/// 이미 있는 경로면 " (2)", " (3)" 을 붙여 비어 있는 경로를 찾는다.
pub fn unique_path(path: &Path) -> Result<PathBuf> {
    if path.symlink_metadata().is_err() {
        return Ok(path.to_path_buf());
    }
    let parent = path.parent().unwrap_or(Path::new(""));
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let suffix = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for i in 2..10000 {
        let candidate = parent.join(format!("{stem} ({i}){suffix}"));
        if candidate.symlink_metadata().is_err() {
            return Ok(candidate);
        }
    }
    Err(Error::Io(io::Error::other(format!(
        "사용 가능한 이름을 찾지 못했습니다: {}",
        path.display()
    ))))
}

/// 사람이 읽는 크기 문자열.
pub fn format_size(nbytes: u64) -> String {
    if nbytes < 1024 {
        return format!("{nbytes} B");
    }
    let mut value = nbytes as f64;
    for unit in ["KB", "MB", "GB", "TB"] {
        value /= 1024.0;
        if value < 1024.0 || unit == "TB" {
            return format!("{} {unit}", group_thousands(&format!("{value:.1}")));
        }
    }
    unreachable!()
}

/// "12345.6" -> "12,345.6"
fn group_thousands(number: &str) -> String {
    let (int, frac) = number.split_once('.').unwrap_or((number, ""));
    let mut out = String::with_capacity(number.len() + int.len() / 3);
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if !frac.is_empty() {
        out.push('.');
        out.push_str(frac);
    }
    out
}

/// 확장자를 뗀 이름. 점으로 시작하는 이름(".bashrc")은 그대로 둔다.
pub(crate) fn stem_of(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traversal_is_neutralised() {
        assert_eq!(
            safe_components("../../windows/system32/x.dll"),
            ["windows", "system32", "x.dll"]
        );
        assert_eq!(safe_components("C:\\evil.txt"), ["evil.txt"]);
        assert_eq!(
            safe_components("\\\\server\\share\\a"),
            ["server", "share", "a"]
        );
        assert_eq!(safe_components("../"), ["_"]);
    }

    #[test]
    fn reserved_and_illegal_names() {
        assert_eq!(sanitize_component("CON.txt"), "_CON.txt");
        assert_eq!(sanitize_component("a<b>?.txt"), "a_b__.txt");
        assert_eq!(sanitize_component("trailing. "), "trailing");
        assert_eq!(sanitize_component("..."), "_");
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(10), "10 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(
            format_size(5 * 1024 * 1024 * 1024 * 1024 * 1024),
            "5,120.0 TB"
        );
    }
}
