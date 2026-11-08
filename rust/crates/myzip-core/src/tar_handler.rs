//! TAR 계열 읽기/쓰기 (tar, tar.gz/tgz, tar.bz2, tar.xz) 와 단일 압축 파일(.gz/.bz2/.xz).
//!
//! TAR 은 앞에서부터 흘려 읽는 형식이다. 압축까지 씌워져 있으면 중간으로
//! 건너뛸 수 없으므로, 여러 항목을 꺼낼 때는 스트림을 한 번만 훑으며
//! 필요한 항목을 만날 때마다 꺼낸다.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDateTime};
use tar::{EntryType, Header, HeaderMode};

use crate::archive::{ArchiveReader, ArchiveWriter, ExtractJob, ExtractOptions, write_atomically};
use crate::encoding::{decode_with, detect_archive_encoding};
use crate::entry::{Entry, Progress, ProgressReader, copy_with_progress};
use crate::error::{Error, Result};

/// TAR 을 감싼 압축 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
    Bzip2,
    Xz,
}

impl Compression {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "TAR",
            Self::Gzip => "TAR.GZ",
            Self::Bzip2 => "TAR.BZ2",
            Self::Xz => "TAR.XZ",
        }
    }

    /// 파일 앞부분의 매직 바이트로 판단한다. 확장자는 거짓말을 할 수 있다.
    pub fn sniff(path: &Path) -> Result<Self> {
        let mut head = [0u8; 6];
        let mut file = File::open(path)?;
        let n = read_up_to(&mut file, &mut head)?;
        let head = &head[..n];
        Ok(if head.starts_with(b"\x1f\x8b") {
            Self::Gzip
        } else if head.starts_with(b"BZh") {
            Self::Bzip2
        } else if head.starts_with(b"\xfd7zXZ\x00") {
            Self::Xz
        } else {
            Self::None
        })
    }

    /// 저장할 파일 이름에서 압축 방식을 고른다.
    pub fn for_name(path: &Path) -> Self {
        let lower = path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") || lower.ends_with(".taz") {
            Self::Gzip
        } else if lower.ends_with(".tar.bz2")
            || lower.ends_with(".tbz")
            || lower.ends_with(".tbz2")
            || lower.ends_with(".tb2")
        {
            Self::Bzip2
        } else if lower.ends_with(".tar.xz") || lower.ends_with(".txz") {
            Self::Xz
        } else {
            Self::None
        }
    }
}

fn read_up_to(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

/// 압축을 풀면서 읽는 스트림.
fn open_decoder(path: &Path, compression: Compression) -> Result<Box<dyn Read + Send>> {
    let file = BufReader::new(File::open(path)?);
    Ok(match compression {
        Compression::None => Box::new(file),
        // 여러 조각을 이어 붙인 파일(cat a.gz b.gz)도 끝까지 읽는다.
        Compression::Gzip => Box::new(flate2::read::MultiGzDecoder::new(file)),
        Compression::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(file)),
        Compression::Xz => Box::new(liblzma::read::XzDecoder::new_multi_decoder(file)),
    })
}

fn tar_error(err: io::Error) -> Error {
    match Error::from(err) {
        Error::Io(e) => Error::Archive(format!("TAR 파일을 읽을 수 없습니다: {e}")),
        other => other,
    }
}

fn unix_to_local(secs: u64) -> Option<NaiveDateTime> {
    let dt = DateTime::from_timestamp(i64::try_from(secs).ok()?, 0)?;
    Some(dt.with_timezone(&Local).naive_local())
}

/// 스트림에서 읽은 항목 정보. 인코딩을 바꿀 때 다시 쓴다.
struct RawMember {
    index: usize,
    name: Vec<u8>,
    kind: EntryType,
    size: u64,
    mtime: Option<NaiveDateTime>,
}

fn kind_label(kind: EntryType) -> &'static str {
    if kind.is_dir() {
        "폴더"
    } else if kind.is_symlink() {
        "심볼릭 링크"
    } else if kind.is_hard_link() {
        "하드 링크"
    } else if kind.is_file() || kind == EntryType::Continuous {
        "파일"
    } else {
        "특수"
    }
}

/// tar 의 메타 항목. 실제 파일이 아니라 다음 항목을 설명하는 머리말이다.
fn is_meta(kind: EntryType) -> bool {
    matches!(
        kind,
        EntryType::XGlobalHeader
            | EntryType::XHeader
            | EntryType::GNULongName
            | EntryType::GNULongLink
    )
}

/// "./a/b/" -> "a/b"
fn normalize_member_name(name: &str) -> String {
    let mut s = name.replace('\\', "/");
    loop {
        let trimmed = s.trim_start_matches('/');
        let trimmed = trimmed.strip_prefix("./").unwrap_or(trimmed);
        if trimmed.len() == s.len() {
            break;
        }
        s = trimmed.to_owned();
    }
    if s == "." {
        s.clear();
    }
    s.trim_end_matches('/').to_owned()
}

/// 항목 내용을 dest 에 쓴다.
fn write_member<R: Read>(
    member: &mut tar::Entry<'_, R>,
    dest: &Path,
    progress: &mut Progress,
) -> Result<()> {
    let kind = member.header().entry_type();
    if kind.is_symlink() || kind.is_hard_link() {
        // 링크는 대상 경로를 텍스트로 남긴다. Windows 에서 링크를 만들려면
        // 관리자 권한이 필요하고, 아카이브 밖을 가리키게 만드는 공격에도 쓰인다.
        let target = member
            .link_name_bytes()
            .map(|b| b.into_owned())
            .unwrap_or_default();
        fs::write(dest, String::from_utf8_lossy(&target).as_bytes())?;
        return Ok(());
    }
    if !(kind.is_file() || kind == EntryType::Continuous) {
        return Ok(()); // 장치 파일 등은 건너뛴다
    }
    let mut out = BufWriter::new(File::create(dest)?);
    copy_with_progress(member, &mut out, progress).map_err(|e| match e {
        Error::Io(e) => tar_error(e),
        other => other,
    })?;
    out.flush()?;
    Ok(())
}

pub struct TarReader {
    path: PathBuf,
    compression: Compression,
    encoding: String,
    members: Vec<RawMember>,
    entries: Vec<Entry>,
}

impl TarReader {
    pub const EXTENSIONS: &'static [&'static str] = &[
        ".tar", ".tgz", ".taz", ".tbz", ".tbz2", ".tb2", ".txz", ".gz", ".bz2", ".xz",
    ];

    pub fn open(path: &Path) -> Result<Self> {
        let compression = Compression::sniff(path)?;
        let mut archive = tar::Archive::new(open_decoder(path, compression)?);

        let mut members = Vec::new();
        for (index, item) in archive.entries().map_err(tar_error)?.enumerate() {
            let member = item.map_err(tar_error)?;
            let header = member.header();
            let kind = header.entry_type();
            if is_meta(kind) {
                continue;
            }
            members.push(RawMember {
                index,
                name: member.path_bytes().into_owned(),
                kind,
                size: member.size(),
                mtime: header.mtime().ok().and_then(unix_to_local),
            });
        }

        // 압축만 된 단일 파일(data.gz)을 tar 로 읽으면 대개 헤더 검사에서
        // 실패하지만, 짧으면 '빈 tar' 처럼 보일 수도 있다. 그런 경우도 tar 가 아니다.
        if members.is_empty() && compression != Compression::None {
            return Err(Error::Archive("TAR 형식이 아닙니다.".into()));
        }

        let encoding =
            detect_archive_encoding(members.iter().map(|m| m.name.as_slice())).to_owned();
        let mut reader = Self {
            path: path.to_path_buf(),
            compression,
            encoding,
            members,
            entries: Vec::new(),
        };
        reader.rebuild_entries();
        Ok(reader)
    }

    pub fn compression(&self) -> Compression {
        self.compression
    }

    fn rebuild_entries(&mut self) {
        self.entries = self
            .members
            .iter()
            .filter_map(|m| {
                let name = normalize_member_name(&decode_with(&m.name, &self.encoding));
                if name.is_empty() {
                    return None;
                }
                Some(Entry {
                    path: name,
                    size: m.size,
                    // tar 는 항목별 압축 크기가 없다. 원본 크기를 그대로 쓴다.
                    compressed_size: m.size,
                    mtime: m.mtime,
                    is_dir: m.kind.is_dir(),
                    crc: None,
                    encrypted: false,
                    method: kind_label(m.kind).into(),
                    raw_name: m.name.clone(),
                    index: m.index,
                })
            })
            .collect();
    }

    /// 스트림을 처음부터 훑어 index 번째 항목에서 `f` 를 부른다.
    fn with_member<T>(
        &self,
        index: usize,
        f: impl FnOnce(&mut tar::Entry<'_, Box<dyn Read + Send>>) -> Result<T>,
    ) -> Result<T> {
        let mut archive = tar::Archive::new(open_decoder(&self.path, self.compression)?);
        for (i, item) in archive.entries().map_err(tar_error)?.enumerate() {
            let mut member = item.map_err(tar_error)?;
            if i == index {
                return f(&mut member);
            }
        }
        Err(Error::Archive("항목을 찾지 못했습니다.".into()))
    }
}

impl ArchiveReader for TarReader {
    fn path(&self) -> &Path {
        &self.path
    }

    fn format_name(&self) -> &str {
        self.compression.label()
    }

    fn entries(&self) -> &[Entry] {
        &self.entries
    }

    fn encoding(&self) -> &str {
        &self.encoding
    }

    fn set_encoding(&mut self, encoding: &str) -> Result<()> {
        if encoding != self.encoding {
            encoding.clone_into(&mut self.encoding);
            self.rebuild_entries();
        }
        Ok(())
    }

    fn extract_one(&mut self, entry: &Entry, dest: &Path, progress: &mut Progress) -> Result<()> {
        self.with_member(entry.index, |member| write_member(member, dest, progress))
    }

    fn read_bytes(&mut self, entry: &Entry) -> Result<Vec<u8>> {
        self.with_member(entry.index, |member| {
            let mut buf = Vec::new();
            member.read_to_end(&mut buf).map_err(tar_error)?;
            Ok(buf)
        })
    }

    /// 스트림을 한 번만 훑으며 필요한 항목을 꺼낸다.
    fn extract(
        &mut self,
        dest_dir: &Path,
        options: ExtractOptions<'_>,
        progress: &mut Progress,
    ) -> Result<Vec<PathBuf>> {
        let targets = options
            .members
            .map_or_else(|| self.entries.clone(), <[Entry]>::to_vec);
        let mut job = ExtractJob::new(
            dest_dir,
            &self.entries,
            &targets,
            options.strip_root,
            options.on_conflict,
            progress,
        )?;

        let mut wanted: HashMap<usize, &Entry> = targets.iter().map(|e| (e.index, e)).collect();
        if !wanted.is_empty() {
            let mut archive = tar::Archive::new(open_decoder(&self.path, self.compression)?);
            for (i, item) in archive.entries().map_err(tar_error)?.enumerate() {
                let mut member = item.map_err(tar_error)?;
                let Some(entry) = wanted.remove(&i) else {
                    continue;
                };
                job.process(entry, |dest, progress| {
                    write_member(&mut member, dest, progress)
                })?;
                if wanted.is_empty() {
                    break;
                }
            }
        }
        Ok(job.finish())
    }

    /// 전 항목을 끝까지 읽어 스트림 손상 여부를 본다.
    fn test(&mut self, progress: &mut Progress) -> Result<Vec<String>> {
        let names: HashMap<usize, &Entry> = self.entries.iter().map(|e| (e.index, e)).collect();
        progress.total_files = self.entries.iter().filter(|e| !e.is_dir).count() as u64;
        progress.total_bytes = self
            .entries
            .iter()
            .filter(|e| !e.is_dir)
            .map(|e| e.size)
            .sum();

        let mut bad = Vec::new();
        let mut archive = tar::Archive::new(open_decoder(&self.path, self.compression)?);
        let iter = match archive.entries() {
            Ok(iter) => iter,
            Err(e) => return Ok(vec![format!("{}: {e}", self.path.display())]),
        };
        for (i, item) in iter.enumerate() {
            let mut member = match item {
                Ok(m) => m,
                Err(e) => {
                    // 압축 스트림이 깨지면 그 뒤로는 읽을 수 없다.
                    bad.push(format!("스트림 손상: {e}"));
                    break;
                }
            };
            let Some(entry) = names.get(&i) else { continue };
            if entry.is_dir {
                continue;
            }
            progress.set_current(&entry.path);
            match copy_with_progress(&mut member, &mut io::sink(), progress) {
                Ok(_) => {}
                Err(e) if e.is_cancelled() => return Err(e),
                Err(_) => {
                    bad.push(entry.path.clone());
                    break;
                }
            }
            progress.advance(0, 1)?;
        }
        Ok(bad)
    }
}

/// tar 로 감싸지 않은 단일 압축 파일 (data.gz, dump.xz 등).
///
/// .gz/.bz2/.xz 는 파일 하나만 압축한 것일 수도 있고 tar 를 감싼 것일
/// 수도 있다. tar 로 열지 못하면 여기로 넘어와 항목 하나짜리 아카이브처럼 다룬다.
pub struct SingleFileReader {
    path: PathBuf,
    compression: Compression,
    entries: Vec<Entry>,
}

impl SingleFileReader {
    pub fn open(path: &Path) -> Result<Self> {
        let compression = Compression::sniff(path)?;
        if compression == Compression::None {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            return Err(Error::Archive(format!(
                "압축 형식을 알 수 없습니다: {name}"
            )));
        }

        // 원본 크기를 알려면 끝까지 읽어야 한다. 아주 큰 파일에서 비싸므로
        // gzip 은 꼬리 4바이트에 적힌 크기를 쓰고, 나머지는 끝까지 읽어 센다.
        let size = if compression == Compression::Gzip {
            gzip_isize(path).unwrap_or(0)
        } else {
            let mut stream = open_decoder(path, compression)?;
            io::copy(&mut stream, &mut io::sink())
                .map_err(|e| Error::Archive(format!("압축 파일을 읽을 수 없습니다: {e}")))?
        };

        let meta = fs::metadata(path)?;
        let mtime = meta
            .modified()
            .ok()
            .map(|t| DateTime::<Local>::from(t).naive_local());
        let name = inner_name(path);
        let entry = Entry {
            path: name.clone(),
            size,
            compressed_size: meta.len(),
            mtime,
            is_dir: false,
            method: compression.label().trim_start_matches("TAR.").into(),
            raw_name: name.into_bytes(),
            ..Entry::default()
        };
        Ok(Self {
            path: path.to_path_buf(),
            compression,
            entries: vec![entry],
        })
    }
}

/// gzip 꼬리에 적힌 원본 크기 (4GiB 로 나눈 나머지).
fn gzip_isize(path: &Path) -> io::Result<u64> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::End(-4))?;
    let mut tail = [0u8; 4];
    file.read_exact(&mut tail)?;
    Ok(u64::from(u32::from_le_bytes(tail)))
}

/// 압축을 풀었을 때의 파일 이름. 확장자 하나만 떼어 낸다.
fn inner_name(path: &Path) -> String {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let lower = name.to_lowercase();
    for suffix in [".gz", ".bz2", ".xz", ".gzip", ".bzip2"] {
        if lower.ends_with(suffix) {
            let stem = &name[..name.len() - suffix.len()];
            return if stem.is_empty() {
                "data".into()
            } else {
                stem.into()
            };
        }
    }
    format!("{name}.out")
}

impl ArchiveReader for SingleFileReader {
    fn path(&self) -> &Path {
        &self.path
    }

    fn format_name(&self) -> &str {
        "단일 압축 파일"
    }

    fn entries(&self) -> &[Entry] {
        &self.entries
    }

    fn encoding(&self) -> &str {
        "utf-8"
    }

    fn set_encoding(&mut self, _encoding: &str) -> Result<()> {
        Ok(())
    }

    fn can_change_encoding(&self) -> bool {
        false
    }

    fn extract_one(&mut self, _entry: &Entry, dest: &Path, progress: &mut Progress) -> Result<()> {
        let mut src = open_decoder(&self.path, self.compression)?;
        let mut out = BufWriter::new(File::create(dest)?);
        copy_with_progress(&mut src, &mut out, progress)?;
        out.flush()?;
        Ok(())
    }

    fn read_bytes(&mut self, _entry: &Entry) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        open_decoder(&self.path, self.compression)?.read_to_end(&mut buf)?;
        Ok(buf)
    }

    fn test(&mut self, progress: &mut Progress) -> Result<Vec<String>> {
        progress.total_files = 1;
        progress.total_bytes = self.entries[0].size;
        let mut src = open_decoder(&self.path, self.compression)?;
        match copy_with_progress(&mut src, &mut io::sink(), progress) {
            Ok(_) => Ok(Vec::new()),
            Err(e) if e.is_cancelled() => Err(e),
            Err(_) => Ok(vec![self.entries[0].path.clone()]),
        }
    }
}

pub struct TarWriter {
    path: PathBuf,
    level: i32,
    compression: Compression,
}

impl TarWriter {
    pub fn new(path: &Path, level: i32) -> Self {
        Self {
            path: path.to_path_buf(),
            level,
            compression: Compression::for_name(path),
        }
    }
}

impl ArchiveWriter for TarWriter {
    fn format_name(&self) -> &str {
        self.compression.label()
    }

    fn write(&mut self, items: &[(PathBuf, String)], progress: &mut Progress) -> Result<()> {
        progress.total_files = items.len() as u64;
        progress.total_bytes = items
            .iter()
            .filter_map(|(s, _)| {
                fs::metadata(s)
                    .ok()
                    .filter(|m| m.is_file())
                    .map(|m| m.len())
            })
            .sum();

        // gzip/bzip2 는 1~9, xz 는 프리셋 0~9.
        let level = self.level.clamp(1, 9) as u32;
        let compression = self.compression;
        let xz_preset = self.level.clamp(0, 9) as u32;

        write_atomically(&self.path, |file| {
            let out = BufWriter::new(file);
            let out = match compression {
                Compression::None => build_tar(out, items, progress)?,
                Compression::Gzip => {
                    let enc = flate2::write::GzEncoder::new(out, flate2::Compression::new(level));
                    build_tar(enc, items, progress)?.finish()?
                }
                Compression::Bzip2 => {
                    let enc = bzip2::write::BzEncoder::new(out, bzip2::Compression::new(level));
                    build_tar(enc, items, progress)?.finish()?
                }
                Compression::Xz => {
                    let enc = liblzma::write::XzEncoder::new(out, xz_preset);
                    build_tar(enc, items, progress)?.finish()?
                }
            };
            out.into_inner()
                .map_err(|e| Error::from(e.into_error()))?
                .sync_all()?;
            Ok(())
        })
    }
}

fn build_tar<W: Write>(out: W, items: &[(PathBuf, String)], progress: &mut Progress) -> Result<W> {
    let mut builder = tar::Builder::new(out);
    builder.mode(HeaderMode::Complete);
    builder.follow_symlinks(false);

    for (src, arc) in items {
        progress.check_cancel()?;
        progress.set_current(arc);
        let unreadable = |e: io::Error| {
            Error::Archive(format!("읽을 수 없는 파일입니다: {} ({e})", src.display()))
        };
        let meta = fs::symlink_metadata(src).map_err(unreadable)?;

        if meta.is_dir() {
            builder.append_dir(arc, src)?;
        } else if meta.is_file() {
            let input = File::open(src).map_err(unreadable)?;
            let mut header = Header::new_gnu();
            header.set_metadata_in_mode(&meta, HeaderMode::Complete);
            header.set_size(meta.len());
            builder.append_data(
                &mut header,
                arc,
                ProgressReader {
                    inner: input,
                    progress: &mut *progress,
                },
            )?;
        } else {
            builder.append_path_with_name(src, arc)?;
        }
        progress.advance(0, 1)?;
    }
    Ok(builder.into_inner()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_names() {
        assert_eq!(normalize_member_name("./a/b/"), "a/b");
        assert_eq!(normalize_member_name("/abs/x"), "abs/x");
        assert_eq!(normalize_member_name("."), "");
        assert_eq!(normalize_member_name(".hidden"), ".hidden");
    }

    #[test]
    fn inner_names() {
        assert_eq!(inner_name(Path::new("보고서.txt.gz")), "보고서.txt");
        assert_eq!(inner_name(Path::new(".gz")), "data");
        assert_eq!(inner_name(Path::new("blob")), "blob.out");
    }
}
