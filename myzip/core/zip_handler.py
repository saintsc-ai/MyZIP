"""ZIP 읽기/쓰기.

pyzipper 를 쓰면 표준 zipfile 과 같은 API 로 ZipCrypto(구식 암호)와
AES-256 암호를 모두 다룰 수 있다.
"""

from __future__ import annotations

import zipfile
from datetime import datetime
from pathlib import Path
from typing import Sequence

import pyzipper

from .base import (
    ArchiveEntry,
    ArchiveReader,
    ArchiveWriter,
    ArchiveError,
    PasswordRequired,
    Progress,
)
from .encoding import decode_best, detect_archive_encoding

CHUNK = 1 << 20  # 1 MiB

# ZipInfo.compress_type -> 표시 이름
_METHODS = {
    0: "저장",
    8: "Deflate",
    9: "Deflate64",
    12: "BZip2",
    14: "LZMA",
    93: "Zstandard",
    95: "XZ",
    98: "PPMd",
}

# 압축 강도(0~9) -> (compress_type, compresslevel)
def _method_for_level(level: int) -> tuple[int, int | None]:
    if level <= 0:
        return zipfile.ZIP_STORED, None
    return zipfile.ZIP_DEFLATED, max(1, min(9, level))


class ZipReader(ArchiveReader):
    extensions = (".zip", ".zipx", ".jar", ".apk", ".epub", ".docx", ".xlsx", ".pptx")
    format_name = "ZIP"

    def __init__(self, path, password: str | None = None):
        super().__init__(path, password)
        self._zf: pyzipper.AESZipFile | None = None
        self.encoding = ""  # 아직 판별 전

    def _open(self) -> pyzipper.AESZipFile:
        if self._zf is None:
            try:
                self._zf = pyzipper.AESZipFile(self.path, "r")
            except (zipfile.BadZipFile, OSError) as exc:
                raise ArchiveError(f"ZIP 파일을 열 수 없습니다: {exc}") from exc
            if self.password:
                self._zf.setpassword(self.password.encode("utf-8"))
        return self._zf

    def _load_entries(self) -> list[ArchiveEntry]:
        zf = self._open()
        infos = zf.infolist()

        # 1) 각 항목의 원본 파일명 바이트를 복원한다.
        raws: list[bytes] = []
        utf8_flags: list[bool] = []
        for info in infos:
            is_utf8 = bool(info.flag_bits & 0x800)
            utf8_flags.append(is_utf8)
            # zipfile 은 UTF-8 플래그가 없으면 cp437 로 디코딩해 둔다.
            enc = "utf-8" if is_utf8 else "cp437"
            try:
                raws.append(info.filename.encode(enc))
            except UnicodeEncodeError:
                raws.append(info.filename.encode("utf-8", "replace"))

        # 2) 인코딩이 지정되지 않았으면 아카이브 전체를 보고 한 번에 정한다.
        if not self.encoding:
            legacy = [r for r, u in zip(raws, utf8_flags) if not u]
            self.encoding = detect_archive_encoding(legacy) if legacy else "utf-8"

        entries: list[ArchiveEntry] = []
        for info, raw, is_utf8 in zip(infos, raws, utf8_flags):
            if is_utf8:
                name = raw.decode("utf-8", "replace")
            else:
                try:
                    name = raw.decode(self.encoding)
                except (UnicodeDecodeError, LookupError):
                    name, _ = decode_best(raw)

            name = name.replace("\\", "/")
            is_dir = info.is_dir() or name.endswith("/")
            entries.append(
                ArchiveEntry(
                    path=name.rstrip("/") if is_dir else name,
                    size=info.file_size,
                    compressed_size=info.compress_size,
                    mtime=_zip_mtime(info),
                    is_dir=is_dir,
                    crc=info.CRC if not is_dir else None,
                    encrypted=bool(info.flag_bits & 0x1),
                    method=_METHODS.get(info.compress_type, f"방식 {info.compress_type}"),
                    raw_name=raw,
                    handle=info,
                )
            )
        return entries

    def _member_pw(self) -> bytes | None:
        return self.password.encode("utf-8") if self.password else None

    def _extract_one(self, entry: ArchiveEntry, dest: Path, progress: Progress) -> None:
        zf = self._open()
        info = entry.handle
        try:
            src = zf.open(info, "r", pwd=self._member_pw())
        except RuntimeError as exc:
            if "password" in str(exc).lower():
                raise PasswordRequired(f"암호가 필요합니다: {entry.path}") from exc
            raise ArchiveError(str(exc)) from exc

        try:
            with src, open(dest, "wb") as out:
                while True:
                    progress.check_cancel()
                    buf = src.read(CHUNK)
                    if not buf:
                        break
                    out.write(buf)
                    progress.advance(nbytes=len(buf))
        except (RuntimeError, zipfile.BadZipFile) as exc:
            msg = str(exc).lower()
            if "password" in msg or "bad" in msg and entry.encrypted:
                raise PasswordRequired(f"암호가 틀렸습니다: {entry.path}") from exc
            raise ArchiveError(f"{entry.path}: {exc}") from exc

    def read_bytes(self, entry: ArchiveEntry) -> bytes:
        zf = self._open()
        try:
            return zf.read(entry.handle, pwd=self._member_pw())
        except RuntimeError as exc:
            raise PasswordRequired(str(exc)) from exc

    def test(self, progress: Progress) -> list[str]:
        """CRC 검사로 손상된 항목을 찾는다. 손상된 항목 경로 목록을 반환."""
        zf = self._open()
        bad: list[str] = []
        files = [e for e in self.entries if not e.is_dir]
        progress.total_files = len(files)
        progress.total_bytes = sum(e.size for e in files)
        for entry in files:
            progress.current = entry.path
            try:
                with zf.open(entry.handle, "r", pwd=self._member_pw()) as src:
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
        if self._zf is not None:
            self._zf.close()
            self._zf = None


class ZipWriter(ArchiveWriter):
    extensions = (".zip",)
    format_name = "ZIP"
    supports_password = True

    def __init__(self, path, level: int = 6, password: str | None = None,
                 encoding: str = "utf-8", aes: bool = True):
        super().__init__(path, level, password, encoding)
        self.aes = aes

    def write(self, items: Sequence[tuple[Path, str]], progress: Progress) -> None:
        compress_type, complevel = _method_for_level(self.level)

        files = [(src, arc) for src, arc in items if src.is_file()]
        progress.total_files = len(items)
        progress.total_bytes = sum(_safe_size(s) for s, _ in files)

        if self.password and not self.aes:
            # 구식 ZipCrypto 는 쓰기가 지원되지 않는다(읽기만 가능).
            # 어차피 수 초면 뚫리는 방식이라 AES-256 으로 강제한다.
            self.aes = True

        with pyzipper.AESZipFile(self.path, "w", compression=compress_type,
                                 compresslevel=complevel) as zf:
            if self.password:
                zf.setpassword(self.password.encode("utf-8"))
                zf.setencryption(pyzipper.WZ_AES, nbits=256)

            for src, arc in items:
                progress.check_cancel()
                progress.current = arc
                if src.is_dir():
                    info = zf.zipinfo_cls(arc.rstrip("/") + "/", _mtime_tuple(src))
                    info.external_attr = 0o40775 << 16 | 0x10
                    info.compress_type = zipfile.ZIP_STORED
                    zf.writestr(info, b"")
                    progress.advance(files=1)
                    continue

                self._write_file(zf, src, arc, compress_type, progress)
                progress.advance(files=1)

    def _write_file(self, zf, src: Path, arc: str, compress_type: int,
                    progress: Progress) -> None:
        """큰 파일도 메모리에 다 올리지 않도록 조각내어 기록한다."""
        info = zf.zipinfo_cls(arc, _mtime_tuple(src))
        info.compress_type = compress_type
        info.external_attr = 0o600 << 16
        try:
            info.file_size = src.stat().st_size
        except OSError:
            info.file_size = 0

        try:
            with zf.open(info, "w") as dst, open(src, "rb") as fh:
                while True:
                    progress.check_cancel()
                    buf = fh.read(CHUNK)
                    if not buf:
                        break
                    dst.write(buf)
                    progress.advance(nbytes=len(buf))
        except PermissionError as exc:
            raise ArchiveError(f"읽을 수 없는 파일입니다: {src} ({exc})") from exc


def _zip_mtime(info) -> datetime | None:
    try:
        return datetime(*info.date_time)
    except (ValueError, TypeError):
        return None


def _mtime_tuple(path: Path) -> tuple[int, int, int, int, int, int]:
    try:
        dt = datetime.fromtimestamp(path.stat().st_mtime)
    except (OSError, OverflowError, ValueError):
        dt = datetime.now()
    # ZIP 은 1980년 이전 날짜를 표현할 수 없다.
    if dt.year < 1980:
        dt = dt.replace(year=1980, month=1, day=1)
    return (dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second)


def _safe_size(path: Path) -> int:
    try:
        return path.stat().st_size
    except OSError:
        return 0
