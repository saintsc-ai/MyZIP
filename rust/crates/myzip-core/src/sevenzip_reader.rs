//! 7z.exe 를 엔진으로 쓰는 읽기 전용 리더 (RAR, 7Z, CAB, ARJ, LZH, ISO).
//!
//! RAR 압축(생성)은 지원하지 않는다. RARLAB 독점 포맷이고 unRAR 라이선스가
//! RAR 압축기 제작에 코드를 쓰는 것을 금지하기 때문이다. 반디집, 7-Zip 도
//! 같은 이유로 RAR 은 해제만 지원한다.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::archive::{ArchiveReader, ExtractJob, ExtractOptions};
use crate::entry::{Entry, Progress};
use crate::error::{Error, Result};
use crate::sevenzip;

/// 함수가 끝나면 임시 폴더를 지운다.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn temp_name(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "{prefix}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// 7z 가 풀어 놓은 위치. 아카이브 내부 경로를 그대로 따라간다.
fn staged(staging: &Path, member: &str) -> PathBuf {
    let mut path = staging.to_path_buf();
    for part in member.split('/').filter(|p| !p.is_empty()) {
        path.push(part);
    }
    path
}

/// 같은 볼륨 안이면 이름만 바꾸고, 아니면 복사한다.
fn move_file(src: &Path, dst: &Path) -> Result<()> {
    if fs::rename(src, dst).is_err() {
        fs::copy(src, dst)?;
        let _ = fs::remove_file(src);
    }
    Ok(())
}

pub struct SevenZipReader {
    path: PathBuf,
    password: Option<String>,
    format_name: String,
    entries: Vec<Entry>,
}

impl SevenZipReader {
    pub const EXTENSIONS: &'static [&'static str] = &[
        ".rar", ".cbr", ".7z", ".cb7", ".iso", ".cab", ".arj", ".lzh",
    ];

    pub fn open(path: &Path, password: Option<&str>, format_name: &str) -> Result<Self> {
        let password = password.filter(|p| !p.is_empty()).map(str::to_owned);
        let entries = sevenzip::list_entries(path, password.as_deref())?
            .into_iter()
            .filter(|e| !e.path.is_empty())
            .enumerate()
            .map(|(index, e)| Entry {
                compressed_size: if e.packed > 0 { e.packed } else { e.size },
                crc: u32::from_str_radix(&e.crc, 16).ok(),
                method: if !e.method.is_empty() {
                    e.method
                } else if e.is_dir {
                    "폴더".into()
                } else {
                    "파일".into()
                },
                raw_name: e.path.as_bytes().to_vec(),
                path: e.path,
                size: e.size,
                mtime: e.mtime,
                is_dir: e.is_dir,
                encrypted: e.encrypted,
                index,
            })
            .collect();
        Ok(Self {
            path: path.to_path_buf(),
            password,
            format_name: format_name.to_owned(),
            entries,
        })
    }
}

impl ArchiveReader for SevenZipReader {
    fn path(&self) -> &Path {
        &self.path
    }

    fn format_name(&self) -> &str {
        &self.format_name
    }

    fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// 7z 는 이미 UTF-8 로 이름을 돌려주므로 재해석이 필요 없다.
    fn encoding(&self) -> &str {
        "utf-8"
    }

    fn set_encoding(&mut self, _encoding: &str) -> Result<()> {
        Ok(())
    }

    fn can_change_encoding(&self) -> bool {
        false
    }

    /// 7z 는 프로세스 단위로만 동작하므로 항목을 하나씩 꺼내면 매우 느리다.
    /// 그래서 한 번의 호출로 모두 꺼낸 다음 충돌 처리를 하며 제자리로 옮긴다.
    /// 임시 폴더를 같은 볼륨에 두면 옮기는 것이 이름 변경뿐이라 사실상 공짜다.
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

        let staging = TempDir(dest_dir.join(format!(".{}", temp_name("myzip-tmp"))));
        let names: Option<Vec<String>> = options
            .members
            .map(|_| targets.iter().map(|e| e.path.clone()).collect());
        sevenzip::extract(
            &self.path,
            &staging.0,
            names.as_deref(),
            self.password.as_deref(),
            Some(&mut *job.progress),
        )?;

        job.progress.done_files = 0;
        for entry in &targets {
            let source = staged(&staging.0, &entry.path);
            if source.symlink_metadata().is_err() {
                job.progress.advance(0, 1)?; // 7z 가 건너뛴 항목
                continue;
            }
            job.process(entry, |dest, _| move_file(&source, dest))?;
        }
        Ok(job.finish())
    }

    fn extract_one(&mut self, entry: &Entry, dest: &Path, _progress: &mut Progress) -> Result<()> {
        let parent = dest.parent().unwrap_or(Path::new("."));
        let staging = TempDir(parent.join(format!(".{}", temp_name("myzip-one"))));
        sevenzip::extract(
            &self.path,
            &staging.0,
            Some(std::slice::from_ref(&entry.path)),
            self.password.as_deref(),
            None,
        )?;
        let source = staged(&staging.0, &entry.path);
        if source.is_file() {
            move_file(&source, dest)?;
        }
        Ok(())
    }

    fn read_bytes(&mut self, entry: &Entry) -> Result<Vec<u8>> {
        let staging = TempDir(std::env::temp_dir().join(temp_name("myzip")));
        sevenzip::extract(
            &self.path,
            &staging.0,
            Some(std::slice::from_ref(&entry.path)),
            self.password.as_deref(),
            None,
        )?;
        let source = staged(&staging.0, &entry.path);
        fs::read(&source)
            .map_err(|_| Error::Archive(format!("항목을 읽지 못했습니다: {}", entry.path)))
    }

    fn test(&mut self, _progress: &mut Progress) -> Result<Vec<String>> {
        let (ok, message) = sevenzip::test_archive(&self.path, self.password.as_deref())?;
        Ok(if ok { Vec::new() } else { vec![message] })
    }
}
