//! ZIP 읽기/쓰기. ZipCrypto(구식 암호)는 읽기만, AES-256 은 읽기·쓰기 모두 된다.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};
use zip::result::ZipError;
use zip::write::SimpleFileOptions;
use zip::{AesMode, CompressionMethod, ZipArchive};

use crate::archive::{ArchiveReader, ArchiveWriter, write_atomically};
use crate::encoding::{decode_with, detect_archive_encoding};
use crate::entry::{Entry, Progress, copy_mapped, copy_with_progress};
use crate::error::{Error, Result};

const UTF8_FLAG: u16 = 0x800;

impl From<ZipError> for Error {
    fn from(err: ZipError) -> Self {
        match err {
            ZipError::Io(e) => Error::from(e),
            ZipError::InvalidPassword => Error::PasswordRequired("암호가 틀렸습니다.".into()),
            ZipError::UnsupportedArchive(msg) if msg == ZipError::PASSWORD_REQUIRED => {
                Error::PasswordRequired("암호가 필요합니다.".into())
            }
            other => Error::Archive(format!("ZIP 처리 오류: {other}")),
        }
    }
}

fn method_name(code: u16) -> String {
    match code {
        0 => "저장".into(),
        8 => "Deflate".into(),
        9 => "Deflate64".into(),
        12 => "BZip2".into(),
        14 => "LZMA".into(),
        93 => "Zstandard".into(),
        95 => "XZ".into(),
        98 => "PPMd".into(),
        other => format!("방식 {other}"),
    }
}

/// 중앙 디렉터리에서 읽은 항목 정보. 인코딩을 바꿀 때 다시 쓴다.
struct RawInfo {
    name: Vec<u8>,
    utf8: bool,
    size: u64,
    compressed: u64,
    mtime: Option<NaiveDateTime>,
    dir_flag: bool,
    crc: u32,
    encrypted: bool,
    method: u16,
}

pub struct ZipReader {
    path: PathBuf,
    archive: ZipArchive<BufReader<File>>,
    password: Option<String>,
    encoding: String,
    raws: Vec<RawInfo>,
    entries: Vec<Entry>,
}

impl ZipReader {
    pub const EXTENSIONS: &'static [&'static str] = &[
        ".zip", ".zipx", ".jar", ".apk", ".epub", ".docx", ".xlsx", ".pptx",
    ];

    pub fn open(path: &Path, password: Option<&str>) -> Result<Self> {
        let file = File::open(path)?;
        let mut archive = ZipArchive::new(BufReader::new(file))
            .map_err(|e| Error::Archive(format!("ZIP 파일을 열 수 없습니다: {e}")))?;

        let mut raws = Vec::with_capacity(archive.len());
        for i in 0..archive.len() {
            // raw 로 열면 암호 없이도 메타데이터를 읽을 수 있다.
            let f = archive
                .by_index_raw(i)
                .map_err(|e| Error::Archive(format!("ZIP 항목을 읽을 수 없습니다: {e}")))?;
            raws.push(RawInfo {
                name: f.name_raw().to_vec(),
                utf8: f.flags().as_u16() & UTF8_FLAG != 0,
                size: f.size(),
                compressed: f.compressed_size(),
                mtime: f.last_modified().and_then(zip_to_naive),
                dir_flag: f.is_dir(),
                crc: f.crc32(),
                encrypted: f.encrypted(),
                // 규격상의 방식 번호(APPNOTE 4.4.5)가 필요한데, 숫자를 꺼내는 공개
                // API 는 이것뿐이다. 지원하지 않는 방식도 번호로 보여 줄 수 있다.
                #[allow(deprecated)]
                method: f.compression().to_u16(),
            });
        }

        // 인코딩은 아카이브 전체를 보고 한 번에 정한다.
        let legacy = raws.iter().filter(|r| !r.utf8).map(|r| r.name.as_slice());
        let encoding = detect_archive_encoding(legacy).to_owned();

        let mut reader = Self {
            path: path.to_path_buf(),
            archive,
            password: password.filter(|p| !p.is_empty()).map(str::to_owned),
            encoding,
            raws,
            entries: Vec::new(),
        };
        reader.rebuild_entries();
        Ok(reader)
    }

    fn rebuild_entries(&mut self) {
        self.entries = self
            .raws
            .iter()
            .enumerate()
            .map(|(index, raw)| {
                let name = if raw.utf8 {
                    String::from_utf8_lossy(&raw.name).into_owned()
                } else {
                    decode_with(&raw.name, &self.encoding)
                };
                let name = name.replace('\\', "/");
                let is_dir = raw.dir_flag || name.ends_with('/');
                Entry {
                    path: if is_dir {
                        name.trim_end_matches('/').to_owned()
                    } else {
                        name
                    },
                    size: raw.size,
                    compressed_size: raw.compressed,
                    mtime: raw.mtime,
                    is_dir,
                    crc: (!is_dir).then_some(raw.crc),
                    encrypted: raw.encrypted,
                    method: method_name(raw.method),
                    raw_name: raw.name.clone(),
                    index,
                }
            })
            .collect();
    }

    /// 항목을 열어 읽기 함수에 넘긴다. 암호 처리를 한곳에 모았다.
    fn with_member<T>(
        &mut self,
        entry: &Entry,
        read: impl FnOnce(&mut dyn Read, &dyn Fn(io::Error) -> Error) -> Result<T>,
    ) -> Result<T> {
        let path = entry.path.clone();
        let encrypted = entry.encrypted;
        let mut member = if encrypted {
            let Some(password) = self.password.as_deref() else {
                return Err(Error::PasswordRequired(format!(
                    "암호가 필요합니다: {path}"
                )));
            };
            self.archive
                .by_index_decrypt(entry.index, password.as_bytes())
                .map_err(|e| match Error::from(e) {
                    Error::PasswordRequired(_) => {
                        Error::PasswordRequired(format!("암호가 틀렸습니다: {path}"))
                    }
                    other => other,
                })?
        } else {
            self.archive.by_index(entry.index)?
        };

        // 암호가 틀려도 검증 바이트를 우연히 통과할 수 있다(ZipCrypto 는 1/256).
        // 그러면 읽는 도중 CRC/HMAC 오류가 난다.
        let on_read_error = move |e: io::Error| {
            if encrypted {
                Error::PasswordRequired(format!("암호가 틀렸습니다: {path}"))
            } else {
                Error::Archive(format!("{path}: {e}"))
            }
        };
        read(&mut member, &on_read_error)
    }
}

impl ArchiveReader for ZipReader {
    fn path(&self) -> &Path {
        &self.path
    }

    fn format_name(&self) -> &str {
        "ZIP"
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
        self.with_member(entry, |src, on_err| {
            let mut out = BufWriter::new(File::create(dest)?);
            copy_mapped(src, &mut out, progress, on_err)?;
            out.flush()?;
            Ok(())
        })
    }

    fn read_bytes(&mut self, entry: &Entry) -> Result<Vec<u8>> {
        self.with_member(entry, |src, on_err| {
            let mut buf = Vec::with_capacity(entry.size.min(64 << 20) as usize);
            copy_mapped(src, &mut buf, &mut Progress::new(), on_err)?;
            Ok(buf)
        })
    }

    fn test(&mut self, progress: &mut Progress) -> Result<Vec<String>> {
        let files: Vec<Entry> = self.entries.iter().filter(|e| !e.is_dir).cloned().collect();
        progress.total_files = files.len() as u64;
        progress.total_bytes = files.iter().map(|e| e.size).sum();

        let mut bad = Vec::new();
        for entry in &files {
            progress.set_current(&entry.path);
            let result = self.with_member(entry, |src, on_err| {
                copy_mapped(src, &mut io::sink(), progress, on_err).map(drop)
            });
            match result {
                Ok(()) => {}
                Err(e) if e.is_cancelled() => return Err(e),
                Err(_) => bad.push(entry.path.clone()),
            }
            progress.advance(0, 1)?;
        }
        Ok(bad)
    }
}

pub struct ZipWriter {
    path: PathBuf,
    level: i32,
    password: Option<String>,
}

impl ZipWriter {
    /// level: 0 이면 저장만, 1~9 는 Deflate 강도.
    /// 암호를 주면 AES-256 으로 암호화한다. 구식 ZipCrypto 는 수 초면
    /// 뚫리는 방식이라 만들지 않는다.
    pub fn new(path: &Path, level: i32, password: Option<&str>) -> Self {
        Self {
            path: path.to_path_buf(),
            level,
            password: password.filter(|p| !p.is_empty()).map(str::to_owned),
        }
    }
}

impl ArchiveWriter for ZipWriter {
    fn format_name(&self) -> &str {
        "ZIP"
    }

    fn supports_password(&self) -> bool {
        true
    }

    fn write(&mut self, items: &[(PathBuf, String)], progress: &mut Progress) -> Result<()> {
        progress.total_files = items.len() as u64;
        progress.total_bytes = items
            .iter()
            .filter(|(s, _)| s.is_file())
            .map(|(s, _)| file_size(s))
            .sum();

        let base = if self.level <= 0 {
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored)
        } else {
            SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .compression_level(Some(i64::from(self.level.clamp(1, 9))))
        };
        let password = self.password.as_deref();

        write_atomically(&self.path, |file| {
            let mut zip = zip::ZipWriter::new(BufWriter::new(file));
            for (src, arc) in items {
                progress.check_cancel()?;
                progress.set_current(arc);

                let meta = fs::metadata(src).map_err(|e| {
                    Error::Archive(format!("읽을 수 없는 파일입니다: {} ({e})", src.display()))
                })?;
                let options = base.last_modified_time(zip_time(&meta));

                if meta.is_dir() {
                    zip.add_directory(arc.trim_end_matches('/'), options.unix_permissions(0o755))?;
                    progress.advance(0, 1)?;
                    continue;
                }

                let mut options = options
                    .unix_permissions(0o644)
                    .large_file(meta.len() >= u64::from(u32::MAX));
                if let Some(pw) = password {
                    options = options.with_aes_encryption(AesMode::Aes256, pw);
                }
                let mut input = File::open(src).map_err(|e| {
                    Error::Archive(format!("읽을 수 없는 파일입니다: {} ({e})", src.display()))
                })?;
                zip.start_file(arc.as_str(), options)?;
                copy_with_progress(&mut input, &mut zip, progress)?;
                progress.advance(0, 1)?;
            }
            let mut out = zip.finish()?;
            out.flush()?;
            Ok(())
        })
    }
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn zip_to_naive(dt: zip::DateTime) -> Option<NaiveDateTime> {
    NaiveDate::from_ymd_opt(
        i32::from(dt.year()),
        u32::from(dt.month()),
        u32::from(dt.day()),
    )?
    .and_hms_opt(
        u32::from(dt.hour()),
        u32::from(dt.minute()),
        u32::from(dt.second()),
    )
}

/// 파일의 수정 시각을 ZIP 시각으로. ZIP 은 1980년 이전을 표현하지 못한다.
fn zip_time(meta: &fs::Metadata) -> zip::DateTime {
    let local: chrono::DateTime<chrono::Local> = meta
        .modified()
        .map(chrono::DateTime::from)
        .unwrap_or_else(|_| chrono::Local::now());
    let year = local.year().clamp(1980, 2107) as u16;
    let (month, day) = if local.year() < 1980 {
        (1, 1)
    } else {
        (local.month() as u8, local.day() as u8)
    };
    zip::DateTime::from_date_and_time(
        year,
        month,
        day,
        local.hour() as u8,
        local.minute() as u8,
        local.second() as u8,
    )
    .unwrap_or_default()
}
