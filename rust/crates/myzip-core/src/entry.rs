//! 아카이브 항목, 진행 상황, 충돌 처리 방식.

use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::NaiveDateTime;

use crate::error::{Error, Result, cancelled, io_cancelled};

/// 스트림 복사 단위 (1 MiB).
pub(crate) const CHUNK: usize = 1 << 20;

/// 아카이브 안의 항목 하나.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    /// 슬래시로 구분된 아카이브 내부 경로.
    pub path: String,
    /// 원본 크기.
    pub size: u64,
    pub compressed_size: u64,
    pub mtime: Option<NaiveDateTime>,
    pub is_dir: bool,
    pub crc: Option<u32>,
    pub encrypted: bool,
    /// 압축 방식 표시용.
    pub method: String,
    /// 인코딩 재해석용 원본 바이트.
    pub raw_name: Vec<u8>,
    /// 핸들러 안에서 이 항목을 다시 찾을 때 쓰는 번호.
    pub index: usize,
}

impl Entry {
    /// 경로를 제외한 이름.
    pub fn name(&self) -> &str {
        match self.path.rsplit('/').next() {
            Some(name) if !name.is_empty() => name,
            _ => &self.path,
        }
    }

    /// 상위 폴더 경로. 최상위면 빈 문자열.
    pub fn parent(&self) -> &str {
        match self.path.rfind('/') {
            Some(i) => &self.path[..i],
            None => "",
        }
    }

    /// 소문자 확장자 (점 포함). 없으면 빈 문자열.
    pub fn suffix(&self) -> String {
        let name = self.name();
        match name.rfind('.') {
            Some(i) if i > 0 && i + 1 < name.len() => name[i..].to_lowercase(),
            _ => String::new(),
        }
    }

    /// 압축률 (0.0 ~ 1.0). 클수록 많이 줄었다.
    pub fn ratio(&self) -> f64 {
        if self.size == 0 {
            return 0.0;
        }
        (1.0 - self.compressed_size as f64 / self.size as f64).max(0.0)
    }
}

/// 이미 같은 이름의 파일이 있을 때 할 일.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictAction {
    Overwrite,
    Skip,
    Rename,
    Cancel,
}

impl ConflictAction {
    /// 설정 파일에 적힌 이름("overwrite", "skip", "rename")을 해석한다.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "overwrite" => Some(Self::Overwrite),
            "skip" => Some(Self::Skip),
            "rename" => Some(Self::Rename),
            "cancel" => Some(Self::Cancel),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Overwrite => "overwrite",
            Self::Skip => "skip",
            Self::Rename => "rename",
            Self::Cancel => "cancel",
        }
    }
}

/// 다른 스레드(UI)에서 작업을 멈추게 하는 신호.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

pub type ProgressCallback = Box<dyn FnMut(&Progress) + Send>;

/// 진행 상황 보고 + 취소 신호 전달.
#[derive(Default)]
pub struct Progress {
    pub total_bytes: u64,
    pub done_bytes: u64,
    pub total_files: u64,
    pub done_files: u64,
    pub current: String,
    token: CancelToken,
    callback: Option<ProgressCallback>,
}

impl std::fmt::Debug for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Progress")
            .field("total_bytes", &self.total_bytes)
            .field("done_bytes", &self.done_bytes)
            .field("total_files", &self.total_files)
            .field("done_files", &self.done_files)
            .field("current", &self.current)
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl Progress {
    pub fn new() -> Self {
        Self::default()
    }

    /// 바깥에서 만든 취소 신호를 공유한다.
    pub fn with_token(token: CancelToken) -> Self {
        Self {
            token,
            ..Self::default()
        }
    }

    /// 진행될 때마다 불린다. 호출이 잦으므로 UI 는 알아서 솎아 내야 한다.
    pub fn set_callback(&mut self, callback: impl FnMut(&Progress) + Send + 'static) {
        self.callback = Some(Box::new(callback));
    }

    pub fn token(&self) -> CancelToken {
        self.token.clone()
    }

    pub fn cancel(&self) {
        self.token.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    pub fn check_cancel(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(cancelled())
        } else {
            Ok(())
        }
    }

    pub fn set_current(&mut self, current: &str) {
        current.clone_into(&mut self.current);
    }

    /// 진행량을 더하고, 취소됐으면 멈추고, 아니면 콜백을 부른다.
    pub fn advance(&mut self, nbytes: u64, files: u64) -> Result<()> {
        self.done_bytes += nbytes;
        self.done_files += files;
        self.check_cancel()?;
        self.notify();
        Ok(())
    }

    /// 값만 바꾼 뒤 콜백을 부르고 싶을 때.
    pub fn notify(&mut self) {
        if let Some(mut callback) = self.callback.take() {
            callback(self);
            self.callback = Some(callback);
        }
    }

    pub fn percent(&self) -> f64 {
        if self.total_bytes == 0 {
            if self.total_files == 0 {
                return 0.0;
            }
            return 100.0 * self.done_files as f64 / self.total_files as f64;
        }
        (100.0 * self.done_bytes as f64 / self.total_bytes as f64).min(100.0)
    }
}

/// 진행률을 보고하며 src 를 dst 로 복사한다.
pub(crate) fn copy_with_progress<R, W>(
    src: &mut R,
    dst: &mut W,
    progress: &mut Progress,
) -> Result<u64>
where
    R: Read + ?Sized,
    W: Write + ?Sized,
{
    copy_mapped(src, dst, progress, Error::from)
}

/// [`copy_with_progress`] 와 같되, 읽기 쪽 오류를 따로 해석한다.
///
/// ZIP 은 암호가 틀리면 읽는 도중에 체크섬 오류가 난다. 그것을 디스크 쓰기
/// 오류와 구분해야 "암호가 틀렸습니다" 라고 알려 줄 수 있다.
pub(crate) fn copy_mapped<R, W, F>(
    src: &mut R,
    dst: &mut W,
    progress: &mut Progress,
    on_read_error: F,
) -> Result<u64>
where
    R: Read + ?Sized,
    W: Write + ?Sized,
    F: Fn(io::Error) -> Error,
{
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        progress.check_cancel()?;
        let n = match src.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                let err = Error::from(e);
                return Err(match err {
                    Error::Io(e) => on_read_error(e),
                    other => other,
                });
            }
        };
        dst.write_all(&buf[..n])?;
        total += n as u64;
        progress.advance(n as u64, 0)?;
    }
    Ok(total)
}

/// 읽히는 만큼 진행률을 올리는 래퍼. 압축 라이브러리가 직접 읽어 갈 때 쓴다.
pub(crate) struct ProgressReader<'a, R> {
    pub inner: R,
    pub progress: &'a mut Progress,
}

impl<R: Read> Read for ProgressReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.progress.is_cancelled() {
            return Err(io_cancelled());
        }
        let n = self.inner.read(buf)?;
        if n > 0 && self.progress.advance(n as u64, 0).is_err() {
            return Err(io_cancelled());
        }
        Ok(n)
    }
}

/// 파일의 수정 시각을 아카이브에 적힌 값으로 맞춘다. 실패해도 그만이다.
pub(crate) fn apply_mtime(path: &Path, mtime: Option<NaiveDateTime>) {
    let Some(mtime) = mtime else { return };
    // 아카이브의 시각은 대개 현지 시각이다 (ZIP 의 DOS 시각이 그렇다).
    let local = match mtime.and_local_timezone(chrono::Local) {
        chrono::LocalResult::Single(t) => t,
        chrono::LocalResult::Ambiguous(t, _) => t,
        chrono::LocalResult::None => return,
    };
    let ft = filetime::FileTime::from_unix_time(local.timestamp(), 0);
    let _ = filetime::set_file_times(path, ft, ft);
}
