//! MyZIP 압축 엔진.
//!
//! - ZIP: 읽기·쓰기 (AES-256 암호), ZipCrypto 는 읽기만
//! - TAR / TAR.GZ / TAR.BZ2 / TAR.XZ: 읽기·쓰기
//! - 단일 .gz / .bz2 / .xz: 읽기
//! - RAR / 7Z / CAB / ARJ / LZH / ISO: 7-Zip(7z.exe) 을 통해 읽기만
//!
//! UI 와 CLI 는 [`open_archive`] 와 [`writer_for`] 로 시작한다.
//!
//! ```no_run
//! use std::path::Path;
//! use myzip_core::{ExtractOptions, Progress, open_archive};
//!
//! let mut reader = open_archive(Path::new("자료.zip"), None)?;
//! for entry in reader.entries() {
//!     println!("{} ({} B)", entry.path, entry.size);
//! }
//! reader.extract(Path::new("풀린곳"), ExtractOptions::default(), &mut Progress::new())?;
//! # Ok::<(), myzip_core::Error>(())
//! ```

mod archive;
pub mod encoding;
mod entry;
mod error;
mod formats;
pub mod path_safety;
pub mod sevenzip;
mod sevenzip_reader;
mod tar_handler;
mod zip_handler;

pub use archive::{
    ArchiveReader, ArchiveWriter, ConflictFn, ExtractOptions, common_root, walk_inputs,
};
pub use entry::{CancelToken, ConflictAction, Entry, Progress, ProgressCallback};
pub use error::{Error, Result};
pub use formats::{
    ASSOCIABLE, READ_ONLY_NOTES, WRITABLE_FORMATS, default_extension, format_of, is_archive,
    open_archive, sniff, strip_archive_suffix, writer_for,
};
pub use path_safety::{format_size, unique_path};
pub use sevenzip_reader::SevenZipReader;
pub use tar_handler::{Compression, SingleFileReader, TarReader, TarWriter};
pub use zip_handler::{ZipReader, ZipWriter};

pub const APP_NAME: &str = "MyZIP";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
