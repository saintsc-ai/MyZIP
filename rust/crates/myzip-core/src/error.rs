//! 오류 타입.
//!
//! Python 판의 예외 계층(ArchiveError 와 그 하위 클래스들)을 enum 하나로 옮겼다.
//! UI 는 종류에 따라 다르게 반응한다. 암호가 필요하면 암호 창을 띄우고,
//! 취소면 아무 말 없이 넘어간다.

use std::fmt;
use std::io;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 아카이브 처리 중 발생한 일반 오류.
    #[error("{0}")]
    Archive(String),

    /// 암호가 필요하거나 틀렸다.
    #[error("{0}")]
    PasswordRequired(String),

    /// 지원하지 않는 포맷.
    #[error("{0}")]
    Unsupported(String),

    /// 사용자가 취소했다.
    #[error("{0}")]
    Cancelled(String),

    /// RAR/7Z 해제 엔진(7z.exe)을 찾지 못했다.
    #[error("{0}")]
    SevenZipMissing(String),

    #[error("{0}")]
    Io(io::Error),
}

impl Error {
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Error::Cancelled(_))
    }

    pub fn is_password(&self) -> bool {
        matches!(self, Error::PasswordRequired(_))
    }
}

pub(crate) fn cancelled() -> Error {
    Error::Cancelled("사용자가 작업을 취소했습니다.".into())
}

/// `Read`/`Write` 구현 안에서 취소를 알릴 때 쓰는 표식.
///
/// 압축 라이브러리들은 io::Error 만 돌려보내므로, 취소를 io::Error 에 실어
/// 보냈다가 바깥에서 다시 [`Error::Cancelled`] 로 되살린다.
#[derive(Debug)]
pub(crate) struct CancelMarker;

impl fmt::Display for CancelMarker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for CancelMarker {}

pub(crate) fn io_cancelled() -> io::Error {
    io::Error::other(CancelMarker)
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        if err
            .get_ref()
            .is_some_and(|inner| inner.is::<CancelMarker>())
        {
            cancelled()
        } else {
            Error::Io(err)
        }
    }
}
