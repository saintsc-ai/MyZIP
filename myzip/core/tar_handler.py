"""TAR 계열 읽기/쓰기 (tar, tar.gz/tgz, tar.bz2, tar.xz)."""

from __future__ import annotations

import tarfile
from datetime import datetime
from pathlib import Path
from typing import Sequence

from .base import (
    ArchiveEntry,
    ArchiveError,
    ArchiveReader,
    ArchiveWriter,
    Progress,
)
from .encoding import decode_best, detect_archive_encoding

CHUNK = 1 << 20

# 확장자 -> tarfile 모드 접미사
_COMPRESSION = {
    ".tar": "",
    ".tgz": "gz",
    ".taz": "gz",
    ".tbz": "bz2",
    ".tbz2": "bz2",
    ".tb2": "bz2",
    ".txz": "xz",
}

_SUFFIX_COMPRESSION = {
    ".tar.gz": "gz",
    ".tar.bz2": "bz2",
    ".tar.xz": "xz",
}

_LABELS = {"": "TAR", "gz": "TAR.GZ", "bz2": "TAR.BZ2", "xz": "TAR.XZ"}


def compression_for(path: Path) -> str:
    """경로에서 tar 압축 방식을 알아낸다. 알 수 없으면 빈 문자열."""
    lower = path.name.lower()
    for suffix, comp in _SUFFIX_COMPRESSION.items():
        if lower.endswith(suffix):
            return comp
    return _COMPRESSION.get(path.suffix.lower(), "")


class TarReader(ArchiveReader):
    extensions = (".tar", ".tgz", ".taz", ".tbz", ".tbz2", ".tb2", ".txz",
                  ".gz", ".bz2", ".xz")
    format_name = "TAR"

    def __init__(self, path, password: str | None = None):
        super().__init__(path, password)
        self._tf: tarfile.TarFile | None = None
        self.encoding = ""

    def _open(self) -> tarfile.TarFile:
        if self._tf is None:
            try:
                # surrogateescape 로 열면 원본 파일명 바이트를 그대로 되살릴 수 있다.
                self._tf = tarfile.open(
                    self.path, "r:*", encoding="utf-8", errors="surrogateescape"
                )
            except (tarfile.TarError, OSError, EOFError) as exc:
                raise ArchiveError(f"TAR 파일을 열 수 없습니다: {exc}") from exc
        return self._tf

    def _load_entries(self) -> list[ArchiveEntry]:
        tf = self._open()
        members = tf.getmembers()

        raws = [m.name.encode("utf-8", "surrogateescape") for m in members]

        if not self.encoding:
            self.encoding = detect_archive_encoding(raws)

        entries: list[ArchiveEntry] = []
        for member, raw in zip(members, raws):
            try:
                name = raw.decode(self.encoding)
            except (UnicodeDecodeError, LookupError):
                name, _ = decode_best(raw)

            name = name.replace("\\", "/").lstrip("./").rstrip("/")
            if not name:
                continue

            entries.append(
                ArchiveEntry(
                    path=name,
                    size=member.size,
                    # tar 은 항목별 압축 크기가 없다. 원본 크기를 그대로 쓴다.
                    compressed_size=member.size,
                    mtime=_tar_mtime(member),
                    is_dir=member.isdir(),
                    encrypted=False,
                    method=_member_kind(member),
                    raw_name=raw,
                    handle=member,
                )
            )
        return entries

    def _extract_one(self, entry: ArchiveEntry, dest: Path, progress: Progress) -> None:
        tf = self._open()
        member = entry.handle

        if member.issym() or member.islnk():
            # 링크는 대상 내용을 텍스트로 남긴다. Windows 에서 링크 생성은
            # 관리자 권한이 필요하고 보안상 위험하므로 만들지 않는다.
            dest.write_text(member.linkname, encoding="utf-8")
            return
        if not member.isfile():
            return  # 장치 파일 등은 건너뛴다

        src = tf.extractfile(member)
        if src is None:
            return
        with src, open(dest, "wb") as out:
            while True:
                progress.check_cancel()
                buf = src.read(CHUNK)
                if not buf:
                    break
                out.write(buf)
                progress.advance(nbytes=len(buf))

    def read_bytes(self, entry: ArchiveEntry) -> bytes:
        tf = self._open()
        src = tf.extractfile(entry.handle)
        if src is None:
            return b""
        with src:
            return src.read()

    def test(self, progress: Progress) -> list[str]:
        """전 항목을 끝까지 읽어 스트림 손상 여부를 본다."""
        bad: list[str] = []
        files = [e for e in self.entries if not e.is_dir]
        progress.total_files = len(files)
        progress.total_bytes = sum(e.size for e in files)
        tf = self._open()
        for entry in files:
            progress.current = entry.path
            try:
                src = tf.extractfile(entry.handle)
                if src is not None:
                    with src:
                        while True:
                            progress.check_cancel()
                            buf = src.read(CHUNK)
                            if not buf:
                                break
                            progress.advance(nbytes=len(buf))
            except Exception:
                bad.append(entry.path)
            progress.advance(files=1)
        return bad

    def close(self) -> None:
        if self._tf is not None:
            self._tf.close()
            self._tf = None


class SingleFileReader(ArchiveReader):
    """tar 로 감싸지 않은 단일 압축 파일 (data.gz, dump.xz 등).

    .gz/.bz2/.xz 는 파일 하나만 압축한 것일 수도 있고 tar 를 감싼 것일
    수도 있다. tarfile 이 열지 못하면 여기로 넘어와 항목 하나짜리
    아카이브처럼 다룬다.
    """

    extensions = (".gz", ".bz2", ".xz")
    format_name = "단일 압축 파일"

    _OPENERS = {
        "gz": ("gzip", "open"),
        "bz2": ("bz2", "open"),
        "xz": ("lzma", "open"),
    }

    def __init__(self, path, password: str | None = None):
        super().__init__(path, password)
        self.encoding = "utf-8"

    def _compression(self) -> str:
        comp = compression_for(self.path)
        if comp:
            return comp
        # 확장자가 없거나 낯설면 매직 바이트로 판단한다.
        with open(self.path, "rb") as fh:
            head = fh.read(6)
        if head[:2] == b"\x1f\x8b":
            return "gz"
        if head[:3] == b"BZh":
            return "bz2"
        if head[:6] == b"\xfd7zXZ\x00":
            return "xz"
        raise ArchiveError(f"압축 형식을 알 수 없습니다: {self.path.name}")

    def _open_stream(self):
        import importlib

        module_name, func_name = self._OPENERS[self._compression()]
        module = importlib.import_module(module_name)
        return getattr(module, func_name)(self.path, "rb")

    def _inner_name(self) -> str:
        """압축을 풀었을 때의 파일 이름. 확장자 하나만 떼어낸다."""
        name = self.path.name
        for suffix in (".gz", ".bz2", ".xz", ".gzip", ".bzip2"):
            if name.lower().endswith(suffix):
                return name[: -len(suffix)] or "data"
        return name + ".out"

    def _load_entries(self) -> list[ArchiveEntry]:
        # 원본 크기를 알려면 끝까지 읽어야 한다. 아주 큰 파일에서 비싸므로
        # gzip 은 꼬리 4바이트에 적힌 크기를 쓰고, 나머지는 스트리밍한다.
        comp = self._compression()
        size = 0
        if comp == "gz":
            try:
                with open(self.path, "rb") as fh:
                    fh.seek(-4, 2)
                    size = int.from_bytes(fh.read(4), "little")
            except OSError:
                size = 0
        else:
            try:
                with self._open_stream() as stream:
                    while True:
                        chunk = stream.read(CHUNK)
                        if not chunk:
                            break
                        size += len(chunk)
            except (OSError, EOFError) as exc:
                raise ArchiveError(f"압축 파일을 읽을 수 없습니다: {exc}") from exc

        try:
            stat = self.path.stat()
            mtime = datetime.fromtimestamp(stat.st_mtime)
            packed = stat.st_size
        except OSError:
            mtime, packed = None, 0

        name = self._inner_name()
        return [
            ArchiveEntry(
                path=name,
                size=size,
                compressed_size=packed,
                mtime=mtime,
                is_dir=False,
                method=_LABELS.get(comp, comp).replace("TAR.", ""),
                raw_name=name.encode("utf-8"),
            )
        ]

    def _extract_one(self, entry: ArchiveEntry, dest: Path, progress: Progress) -> None:
        with self._open_stream() as src, open(dest, "wb") as out:
            while True:
                progress.check_cancel()
                buf = src.read(CHUNK)
                if not buf:
                    break
                out.write(buf)
                progress.advance(nbytes=len(buf))

    def read_bytes(self, entry: ArchiveEntry) -> bytes:
        with self._open_stream() as src:
            return src.read()

    def test(self, progress: Progress) -> list[str]:
        try:
            with self._open_stream() as src:
                while True:
                    progress.check_cancel()
                    if not src.read(CHUNK):
                        break
        except (OSError, EOFError):
            return [self.path.name]
        return []


class TarWriter(ArchiveWriter):
    extensions = (".tar", ".tgz", ".tar.gz", ".tar.bz2", ".tar.xz", ".txz", ".tbz2")
    format_name = "TAR"
    supports_password = False

    def write(self, items: Sequence[tuple[Path, str]], progress: Progress) -> None:
        comp = compression_for(self.path)
        mode = f"w:{comp}" if comp else "w"

        kwargs: dict = {}
        if comp == "gz":
            kwargs["compresslevel"] = max(1, min(9, self.level)) if self.level > 0 else 1
        elif comp == "bz2":
            kwargs["compresslevel"] = max(1, min(9, self.level)) if self.level > 0 else 1
        elif comp == "xz":
            kwargs["preset"] = max(0, min(9, self.level))

        progress.total_files = len(items)
        progress.total_bytes = sum(_safe_size(s) for s, _ in items if s.is_file())

        try:
            with tarfile.open(self.path, mode, format=tarfile.PAX_FORMAT,
                              encoding="utf-8", **kwargs) as tf:
                for src, arc in items:
                    progress.check_cancel()
                    progress.current = arc
                    try:
                        info = tf.gettarinfo(str(src), arcname=arc)
                    except OSError as exc:
                        raise ArchiveError(f"읽을 수 없는 파일입니다: {src} ({exc})") from exc

                    if info.isfile():
                        with open(src, "rb") as fh:
                            tf.addfile(info, _ProgressReader(fh, progress))
                    else:
                        tf.addfile(info)
                    progress.advance(files=1)
        except tarfile.TarError as exc:
            raise ArchiveError(f"TAR 생성 실패: {exc}") from exc


class _ProgressReader:
    """tarfile.addfile 이 읽어가는 동안 진행률을 보고하는 래퍼."""

    def __init__(self, fh, progress: Progress):
        self._fh = fh
        self._progress = progress

    def read(self, size: int = -1) -> bytes:
        self._progress.check_cancel()
        buf = self._fh.read(size)
        if buf:
            self._progress.advance(nbytes=len(buf))
        return buf


def _tar_mtime(member: tarfile.TarInfo) -> datetime | None:
    try:
        return datetime.fromtimestamp(member.mtime)
    except (OSError, OverflowError, ValueError):
        return None


def _member_kind(member: tarfile.TarInfo) -> str:
    if member.isdir():
        return "폴더"
    if member.issym():
        return "심볼릭 링크"
    if member.islnk():
        return "하드 링크"
    if member.isfile():
        return "파일"
    return "특수"


def _safe_size(path: Path) -> int:
    try:
        return path.stat().st_size
    except OSError:
        return 0
