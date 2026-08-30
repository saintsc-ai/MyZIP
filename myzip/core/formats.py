"""포맷 판별과 핸들러 선택.

확장자는 거짓말을 할 수 있으므로(.zip 으로 이름만 바꾼 rar 등) 파일 앞부분의
매직 바이트를 먼저 본다.
"""

from __future__ import annotations

from pathlib import Path

from .base import ArchiveError, ArchiveReader, ArchiveWriter, UnsupportedFormat
from .rar_handler import RarReader
from .tar_handler import SingleFileReader, TarReader, TarWriter
from .zip_handler import ZipReader, ZipWriter

# (오프셋, 매직바이트, 포맷이름)
_MAGIC: tuple[tuple[int, bytes, str], ...] = (
    (0, b"Rar!\x1a\x07\x00", "rar"),      # RAR 4.x
    (0, b"Rar!\x1a\x07\x01\x00", "rar"),  # RAR 5.x
    (0, b"7z\xbc\xaf\x27\x1c", "7z"),
    (0, b"PK\x03\x04", "zip"),
    (0, b"PK\x05\x06", "zip"),            # 빈 아카이브
    (0, b"PK\x07\x08", "zip"),            # 분할 아카이브
    (0, b"\x1f\x8b", "tar.gz"),
    (0, b"BZh", "tar.bz2"),
    (0, b"\xfd7zXZ\x00", "tar.xz"),
    (257, b"ustar", "tar"),               # POSIX tar 헤더
    (0, b"MSCF", "cab"),
    (0, b"\x60\xea", "arj"),
)

# 포맷 -> 리더 클래스
_READERS: dict[str, type[ArchiveReader]] = {
    "zip": ZipReader,
    "tar": TarReader,
    "tar.gz": TarReader,
    "tar.bz2": TarReader,
    "tar.xz": TarReader,
    "rar": RarReader,
    "7z": RarReader,     # 7z 엔진이 같이 처리한다
    "cab": RarReader,
    "arj": RarReader,
}

#: 압축(생성) 가능한 포맷. RAR 은 여기 없다 — 아래 주석 참조.
WRITABLE_FORMATS: tuple[tuple[str, str, type[ArchiveWriter]], ...] = (
    ("zip", "ZIP (.zip)", ZipWriter),
    ("tar.gz", "TAR + GZIP (.tar.gz)", TarWriter),
    ("tgz", "TAR + GZIP (.tgz)", TarWriter),
    ("tar", "TAR (.tar)", TarWriter),
    ("tar.bz2", "TAR + BZIP2 (.tar.bz2)", TarWriter),
    ("tar.xz", "TAR + XZ (.tar.xz)", TarWriter),
)

#: 확장자 연결 대상. 탐색기에서 더블클릭하면 MyZIP 이 열 파일들.
ASSOCIABLE = (
    ".zip", ".tar", ".tgz", ".gz", ".bz2", ".xz",
    ".tar.gz", ".tar.bz2", ".tar.xz", ".tbz2", ".txz",
    ".rar", ".7z", ".cab", ".arj", ".lzh", ".iso",
)

#: 읽기만 되고 만들 수는 없는 포맷 (사용자에게 이유를 알려주기 위함)
READ_ONLY_NOTE = {
    "rar": "RAR 은 RARLAB 독점 포맷이라 압축은 만들 수 없고 해제만 됩니다.",
    "7z": "7z 압축은 아직 지원하지 않습니다. 해제만 됩니다.",
}


def sniff(path: str | Path) -> str | None:
    """파일 앞부분을 읽어 포맷 이름을 알아낸다. 모르면 None."""
    path = Path(path)
    try:
        with open(path, "rb") as fh:
            head = fh.read(512)
    except OSError:
        return None

    for offset, magic, name in _MAGIC:
        if head[offset:offset + len(magic)] == magic:
            # gzip/bzip2/xz 는 tar 를 감싼 것일 수도 있고 단일 파일일 수도 있다.
            # tarfile 이 둘 다 처리하므로 구분하지 않는다.
            return name
    return None


def format_of(path: str | Path) -> str | None:
    """매직 바이트 우선, 실패하면 확장자로 포맷을 판단한다."""
    detected = sniff(path)
    if detected:
        return detected

    name = Path(path).name.lower()
    for suffix in (".tar.gz", ".tar.bz2", ".tar.xz"):
        if name.endswith(suffix):
            return suffix.lstrip(".")

    ext = Path(name).suffix
    for fmt, reader in _READERS.items():
        if ext in reader.extensions:
            return fmt
    return None


def open_archive(path: str | Path, password: str | None = None) -> ArchiveReader:
    """경로에 맞는 리더를 골라 연다."""
    path = Path(path)
    if not path.is_file():
        raise UnsupportedFormat(f"파일을 찾을 수 없습니다: {path}")

    fmt = format_of(path)
    reader_cls = _READERS.get(fmt or "")
    if reader_cls is None:
        raise UnsupportedFormat(
            f"지원하지 않는 형식입니다: {path.name}\n"
            "ZIP, TAR, TGZ, RAR 형식만 열 수 있습니다."
        )

    reader = reader_cls(path, password)

    # .gz/.bz2/.xz 는 tar 를 감싼 것일 수도, 파일 하나만 압축한 것일 수도
    # 있다. tarfile 이 열지 못하면 단일 파일로 다시 시도한다.
    if fmt in ("tar.gz", "tar.bz2", "tar.xz"):
        try:
            reader.entries
        except ArchiveError:
            reader.close()
            reader = SingleFileReader(path, password)

    return reader


def is_archive(path: str | Path) -> bool:
    """MyZIP 이 열 수 있는 파일인지."""
    return format_of(path) in _READERS


def writer_for(path: str | Path, level: int = 6, password: str | None = None,
               **kwargs) -> ArchiveWriter:
    """저장할 파일 이름에서 알맞은 라이터를 고른다."""
    name = Path(path).name.lower()

    for suffix in (".tar.gz", ".tar.bz2", ".tar.xz"):
        if name.endswith(suffix):
            return TarWriter(path, level, None)

    ext = Path(name).suffix
    if ext in (".tgz", ".taz", ".tbz", ".tbz2", ".tb2", ".txz", ".tar"):
        return TarWriter(path, level, None)
    if ext == ".zip":
        return ZipWriter(path, level, password, **kwargs)

    raise UnsupportedFormat(
        f"만들 수 없는 형식입니다: {ext or name}\n"
        "zip, tar, tar.gz(tgz), tar.bz2, tar.xz 로만 압축할 수 있습니다."
    )


def default_extension(fmt: str) -> str:
    """포맷 이름에 대응하는 기본 확장자."""
    return {
        "zip": ".zip",
        "tar": ".tar",
        "tgz": ".tgz",
        "tar.gz": ".tar.gz",
        "tar.bz2": ".tar.bz2",
        "tar.xz": ".tar.xz",
    }.get(fmt, ".zip")


def strip_archive_suffix(name: str) -> str:
    """아카이브 파일 이름에서 확장자를 떼어 기본 폴더 이름을 만든다."""
    lower = name.lower()
    for suffix in (".tar.gz", ".tar.bz2", ".tar.xz"):
        if lower.endswith(suffix):
            return name[: -len(suffix)]
    stem = Path(name).stem
    return stem or name
