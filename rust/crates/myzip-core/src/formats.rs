//! 포맷 판별과 핸들러 선택.
//!
//! 확장자는 거짓말을 할 수 있으므로(.zip 으로 이름만 바꾼 rar 등) 파일 앞부분의
//! 매직 바이트를 먼저 본다.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::archive::{ArchiveReader, ArchiveWriter};
use crate::error::{Error, Result};
use crate::path_safety::stem_of;
use crate::sevenzip_reader::SevenZipReader;
use crate::tar_handler::{SingleFileReader, TarReader, TarWriter};
use crate::zip_handler::{ZipReader, ZipWriter};

/// (오프셋, 매직바이트, 포맷이름)
const MAGIC: &[(usize, &[u8], &str)] = &[
    (0, b"Rar!\x1a\x07\x00", "rar"),     // RAR 4.x
    (0, b"Rar!\x1a\x07\x01\x00", "rar"), // RAR 5.x
    (0, b"7z\xbc\xaf\x27\x1c", "7z"),
    (0, b"PK\x03\x04", "zip"),
    (0, b"PK\x05\x06", "zip"), // 빈 아카이브
    (0, b"PK\x07\x08", "zip"), // 분할 아카이브
    (0, b"\x1f\x8b", "tar.gz"),
    (0, b"BZh", "tar.bz2"),
    (0, b"\xfd7zXZ\x00", "tar.xz"),
    (257, b"ustar", "tar"), // POSIX tar 헤더
    (0, b"MSCF", "cab"),
    (0, b"\x60\xea", "arj"),
];

/// 확장자로만 알 수 있는 포맷.
const EXTENSION_FORMATS: &[(&str, &str)] = &[
    (".rar", "rar"),
    (".cbr", "rar"),
    (".7z", "7z"),
    (".cb7", "7z"),
    (".iso", "iso"),
    (".cab", "cab"),
    (".arj", "arj"),
    (".lzh", "lzh"),
];

/// 압축(생성) 가능한 포맷: (포맷 이름, 표시 이름).
/// RAR 은 여기 없다 — [`READ_ONLY_NOTES`] 참조.
pub const WRITABLE_FORMATS: &[(&str, &str)] = &[
    ("zip", "ZIP (.zip)"),
    ("tar.gz", "TAR + GZIP (.tar.gz)"),
    ("tgz", "TAR + GZIP (.tgz)"),
    ("tar", "TAR (.tar)"),
    ("tar.bz2", "TAR + BZIP2 (.tar.bz2)"),
    ("tar.xz", "TAR + XZ (.tar.xz)"),
];

/// 확장자 연결 대상. 탐색기에서 더블클릭하면 MyZIP 이 열 파일들.
pub const ASSOCIABLE: &[&str] = &[
    ".zip", ".tar", ".tgz", ".gz", ".bz2", ".xz", ".tar.gz", ".tar.bz2", ".tar.xz", ".tbz2",
    ".txz", ".rar", ".7z", ".cab", ".arj", ".lzh", ".iso",
];

/// 읽기만 되고 만들 수는 없는 포맷 (사용자에게 이유를 알려 주기 위함).
pub const READ_ONLY_NOTES: &[(&str, &str)] = &[
    (
        "rar",
        "RAR 은 RARLAB 독점 포맷이라 압축은 만들 수 없고 해제만 됩니다.",
    ),
    ("7z", "7z 압축은 아직 지원하지 않습니다. 해제만 됩니다."),
];

const TAR_SUFFIXES: &[&str] = &[".tar.gz", ".tar.bz2", ".tar.xz"];

fn lower_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn extension_of(lower_name: &str) -> &str {
    match lower_name.rfind('.') {
        Some(i) if i > 0 => &lower_name[i..],
        _ => "",
    }
}

/// 파일 앞부분을 읽어 포맷 이름을 알아낸다. 모르면 None.
pub fn sniff(path: &Path) -> Option<&'static str> {
    let mut head = Vec::with_capacity(512);
    File::open(path)
        .ok()?
        .take(512)
        .read_to_end(&mut head)
        .ok()?;
    // gzip/bzip2/xz 는 tar 를 감싼 것일 수도, 단일 파일일 수도 있다.
    // 여는 쪽(open_archive)에서 구분한다.
    MAGIC
        .iter()
        .find(|(offset, magic, _)| head.get(*offset..offset + magic.len()) == Some(*magic))
        .map(|(_, _, name)| *name)
}

/// 매직 바이트 우선, 실패하면 확장자로 포맷을 판단한다.
pub fn format_of(path: &Path) -> Option<&'static str> {
    if let Some(found) = sniff(path) {
        return Some(found);
    }
    let name = lower_name(path);
    for suffix in TAR_SUFFIXES {
        if name.ends_with(suffix) {
            return Some(&suffix[1..]);
        }
    }
    let ext = extension_of(&name);
    if ZipReader::EXTENSIONS.contains(&ext) {
        return Some("zip");
    }
    if TarReader::EXTENSIONS.contains(&ext) {
        return Some("tar");
    }
    EXTENSION_FORMATS
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, f)| *f)
}

/// MyZIP 이 열 수 있는 파일인지.
pub fn is_archive(path: &Path) -> bool {
    format_of(path).is_some()
}

/// 경로에 맞는 리더를 골라 연다.
pub fn open_archive(path: &Path, password: Option<&str>) -> Result<Box<dyn ArchiveReader>> {
    if !path.is_file() {
        return Err(Error::Unsupported(format!(
            "파일을 찾을 수 없습니다: {}",
            path.display()
        )));
    }
    let unsupported = || {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        Error::Unsupported(format!(
            "지원하지 않는 형식입니다: {name}\nZIP, TAR, TGZ, RAR, 7Z 형식만 열 수 있습니다."
        ))
    };

    Ok(match format_of(path).ok_or_else(unsupported)? {
        "zip" => Box::new(ZipReader::open(path, password)?),
        "tar" => Box::new(TarReader::open(path)?),
        // .gz/.bz2/.xz 는 tar 를 감싼 것일 수도, 파일 하나만 압축한 것일 수도
        // 있다. tar 로 열지 못하면 단일 파일로 다시 시도한다.
        "tar.gz" | "tar.bz2" | "tar.xz" => match TarReader::open(path) {
            Ok(reader) => Box::new(reader),
            Err(_) => Box::new(SingleFileReader::open(path)?),
        },
        fmt => Box::new(SevenZipReader::open(path, password, &fmt.to_uppercase())?),
    })
}

/// 저장할 파일 이름에서 알맞은 라이터를 고른다.
///
/// level: 0(저장만) ~ 9(최대). 암호는 ZIP 에서만 쓰인다.
pub fn writer_for(
    path: &Path,
    level: i32,
    password: Option<&str>,
) -> Result<Box<dyn ArchiveWriter>> {
    let name = lower_name(path);
    if TAR_SUFFIXES.iter().any(|s| name.ends_with(s)) {
        return Ok(Box::new(TarWriter::new(path, level)));
    }
    let ext = extension_of(&name);
    match ext {
        ".tgz" | ".taz" | ".tbz" | ".tbz2" | ".tb2" | ".txz" | ".tar" => {
            Ok(Box::new(TarWriter::new(path, level)))
        }
        ".zip" => Ok(Box::new(ZipWriter::new(path, level, password))),
        _ => Err(Error::Unsupported(format!(
            "만들 수 없는 형식입니다: {}\nzip, tar, tar.gz(tgz), tar.bz2, tar.xz 로만 압축할 수 있습니다.",
            if ext.is_empty() { name.as_str() } else { ext }
        ))),
    }
}

/// 포맷 이름에 대응하는 기본 확장자.
pub fn default_extension(fmt: &str) -> &'static str {
    match fmt {
        "tar" => ".tar",
        "tgz" => ".tgz",
        "tar.gz" => ".tar.gz",
        "tar.bz2" => ".tar.bz2",
        "tar.xz" => ".tar.xz",
        _ => ".zip",
    }
}

/// 아카이브 파일 이름에서 확장자를 떼어 기본 폴더 이름을 만든다.
pub fn strip_archive_suffix(name: &str) -> String {
    let lower = name.to_lowercase();
    for suffix in TAR_SUFFIXES {
        if lower.ends_with(suffix) && name.len() >= suffix.len() {
            return name[..name.len() - suffix.len()].to_owned();
        }
    }
    let stem = stem_of(name);
    if stem.is_empty() {
        name.to_owned()
    } else {
        stem.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names() {
        assert_eq!(strip_archive_suffix("자료.zip"), "자료");
        assert_eq!(strip_archive_suffix("backup.TAR.GZ"), "backup");
        assert_eq!(strip_archive_suffix("noext"), "noext");
    }

    #[test]
    fn writers() {
        assert_eq!(
            writer_for(Path::new("a.tar.xz"), 6, None)
                .unwrap()
                .format_name(),
            "TAR.XZ"
        );
        assert_eq!(
            writer_for(Path::new("a.tgz"), 6, None)
                .unwrap()
                .format_name(),
            "TAR.GZ"
        );
        assert_eq!(
            writer_for(Path::new("a.zip"), 6, None)
                .unwrap()
                .format_name(),
            "ZIP"
        );
        assert!(writer_for(Path::new("a.rar"), 6, None).is_err());
    }
}
