"""아카이브 포맷 공통 인터페이스."""

from __future__ import annotations

import os
import threading
from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path, PurePosixPath
from typing import Callable, Iterable, Iterator, Sequence


class ArchiveError(Exception):
    """아카이브 처리 중 발생한 오류."""


class PasswordRequired(ArchiveError):
    """암호가 필요하거나 틀렸다."""


class UnsupportedFormat(ArchiveError):
    """지원하지 않는 포맷."""


class OperationCancelled(ArchiveError):
    """사용자가 취소했다."""


@dataclass(slots=True)
class ArchiveEntry:
    """아카이브 안의 항목 하나."""

    path: str                      # 슬래시로 구분된 아카이브 내부 경로
    size: int = 0                  # 원본 크기
    compressed_size: int = 0
    mtime: datetime | None = None
    is_dir: bool = False
    crc: int | None = None
    encrypted: bool = False
    method: str = ""               # 압축 방식 표시용
    raw_name: bytes | None = None  # 인코딩 재해석용 원본 바이트
    handle: object | None = None   # 핸들러 전용 데이터 (ZipInfo, TarInfo 등)

    @property
    def name(self) -> str:
        """경로를 제외한 이름."""
        return PurePosixPath(self.path).name or self.path

    @property
    def parent(self) -> str:
        """상위 폴더 경로. 최상위면 빈 문자열."""
        p = str(PurePosixPath(self.path).parent)
        return "" if p == "." else p

    @property
    def suffix(self) -> str:
        return PurePosixPath(self.path).suffix.lower()

    @property
    def ratio(self) -> float:
        """압축률 (0.0 ~ 1.0). 클수록 많이 줄었다."""
        if self.size <= 0:
            return 0.0
        return max(0.0, 1.0 - self.compressed_size / self.size)


@dataclass
class Progress:
    """진행 상황 보고 + 취소 신호 전달."""

    total_bytes: int = 0
    done_bytes: int = 0
    total_files: int = 0
    done_files: int = 0
    current: str = ""
    _cancel: threading.Event = field(default_factory=threading.Event)
    callback: Callable[["Progress"], None] | None = None

    def cancel(self) -> None:
        self._cancel.set()

    @property
    def cancelled(self) -> bool:
        return self._cancel.is_set()

    def check_cancel(self) -> None:
        if self._cancel.is_set():
            raise OperationCancelled("사용자가 작업을 취소했습니다.")

    def advance(self, nbytes: int = 0, files: int = 0, current: str | None = None) -> None:
        self.done_bytes += nbytes
        self.done_files += files
        if current is not None:
            self.current = current
        self.check_cancel()
        if self.callback:
            self.callback(self)

    @property
    def percent(self) -> float:
        if self.total_bytes <= 0:
            if self.total_files <= 0:
                return 0.0
            return 100.0 * self.done_files / self.total_files
        return min(100.0, 100.0 * self.done_bytes / self.total_bytes)


class ArchiveReader(ABC):
    """읽기 전용 아카이브 접근자.

    구현체는 컨텍스트 매니저로 쓸 수 있어야 한다.
    """

    #: 이 리더가 처리하는 확장자 (소문자, 점 포함)
    extensions: tuple[str, ...] = ()
    #: 사람이 읽는 포맷 이름
    format_name: str = ""

    def __init__(self, path: str | os.PathLike, password: str | None = None):
        self.path = Path(path)
        self.password = password
        self.encoding: str = "utf-8"
        self._entries: list[ArchiveEntry] | None = None

    # -- 하위 클래스가 구현할 것 --------------------------------------

    @abstractmethod
    def _load_entries(self) -> list[ArchiveEntry]:
        """아카이브를 열고 항목 목록을 만든다."""

    @abstractmethod
    def _extract_one(self, entry: ArchiveEntry, dest: Path, progress: Progress) -> None:
        """항목 하나를 dest 파일 경로로 꺼낸다. 상위 폴더는 이미 만들어져 있다."""

    @abstractmethod
    def read_bytes(self, entry: ArchiveEntry) -> bytes:
        """항목 내용을 메모리로 읽는다. 미리보기용."""

    def close(self) -> None:  # pragma: no cover - 기본은 할 일 없음
        pass

    # -- 공통 동작 ----------------------------------------------------

    @property
    def entries(self) -> list[ArchiveEntry]:
        if self._entries is None:
            self._entries = self._load_entries()
        return self._entries

    def set_encoding(self, encoding: str) -> None:
        """파일명 인코딩을 바꾸고 목록을 다시 만든다."""
        if encoding == self.encoding:
            return
        self.encoding = encoding
        self._entries = None

    @property
    def has_encrypted(self) -> bool:
        return any(e.encrypted for e in self.entries)

    def total_size(self) -> int:
        return sum(e.size for e in self.entries if not e.is_dir)

    def extract(
        self,
        dest_dir: str | os.PathLike,
        members: Sequence[ArchiveEntry] | None = None,
        progress: Progress | None = None,
        strip_root: bool = False,
        on_conflict: Callable[[Path, ArchiveEntry], str] | None = None,
    ) -> list[Path]:
        """항목들을 dest_dir 아래로 꺼낸다.

        Args:
            members: None 이면 전체.
            strip_root: 아카이브 최상위가 폴더 하나뿐이면 그 폴더를 벗겨낸다.
            on_conflict: 이미 파일이 있을 때 호출. overwrite/skip/rename/cancel 반환.

        Returns:
            실제로 만들어진 경로 목록.
        """
        from .path_safety import safe_join, unique_path

        dest_dir = Path(dest_dir)
        targets = list(members if members is not None else self.entries)
        progress = progress or Progress()

        prefix = _common_root(self.entries) if strip_root else ""

        progress.total_files = len(targets)
        progress.total_bytes = sum(e.size for e in targets if not e.is_dir)

        written: list[Path] = []
        for entry in targets:
            progress.check_cancel()
            rel = entry.path
            if prefix and rel.startswith(prefix):
                rel = rel[len(prefix):].lstrip("/")
                if not rel:
                    continue

            out = safe_join(dest_dir, rel)
            progress.current = entry.path

            if entry.is_dir:
                out.mkdir(parents=True, exist_ok=True)
                progress.advance(files=1)
                written.append(out)
                continue

            out.parent.mkdir(parents=True, exist_ok=True)

            if out.exists():
                action = on_conflict(out, entry) if on_conflict else "overwrite"
                if action == "skip":
                    progress.advance(nbytes=entry.size, files=1)
                    continue
                if action == "rename":
                    out = unique_path(out)
                elif action == "cancel":
                    raise OperationCancelled("덮어쓰기 확인에서 취소했습니다.")

            self._extract_one(entry, out, progress)
            written.append(out)
            progress.advance(files=1)

            if entry.mtime:
                try:
                    ts = entry.mtime.timestamp()
                    os.utime(out, (ts, ts))
                except (OSError, OverflowError, ValueError):
                    pass  # 타임스탬프는 있으면 좋고 없어도 그만

        return written

    # -- 컨텍스트 매니저 ----------------------------------------------

    def __enter__(self) -> "ArchiveReader":
        return self

    def __exit__(self, *exc) -> None:
        self.close()


class ArchiveWriter(ABC):
    """아카이브 생성기."""

    extensions: tuple[str, ...] = ()
    format_name: str = ""
    #: 압축 강도 지원 여부
    supports_level: bool = True
    #: 암호 지원 여부
    supports_password: bool = False

    def __init__(
        self,
        path: str | os.PathLike,
        level: int = 6,
        password: str | None = None,
        encoding: str = "utf-8",
    ):
        self.path = Path(path)
        self.level = level
        self.password = password
        self.encoding = encoding

    @abstractmethod
    def write(self, items: Sequence[tuple[Path, str]], progress: Progress) -> None:
        """(원본 절대경로, 아카이브 내부 경로) 쌍들을 기록한다."""

    def __enter__(self) -> "ArchiveWriter":
        return self

    def __exit__(self, *exc) -> None:
        pass


def _common_root(entries: Iterable[ArchiveEntry]) -> str:
    """모든 항목이 같은 최상위 폴더 하나에 들어있으면 그 이름을 반환."""
    roots = set()
    entries = list(entries)
    for e in entries:
        head = e.path.split("/", 1)[0]
        if not head:
            continue
        roots.add(head)
        if len(roots) > 1:
            return ""
    if len(roots) != 1:
        return ""
    root = roots.pop()
    # 최상위에 파일이 하나 덜렁 있는 경우는 벗기면 안 된다
    if any(e.path == root and not e.is_dir for e in entries):
        return ""
    return root


def walk_inputs(paths: Sequence[str | os.PathLike]) -> Iterator[tuple[Path, str]]:
    """입력 경로들을 (절대경로, 아카이브 내부 경로) 쌍으로 펼친다.

    폴더는 재귀적으로 들어가고, 각 입력의 이름이 아카이브 최상위가 된다.
    """
    for raw in paths:
        p = Path(raw).resolve()
        if p.is_dir():
            base = p.parent
            yield p, p.name
            for sub in sorted(p.rglob("*")):
                yield sub, sub.relative_to(base).as_posix()
        elif p.exists():
            yield p, p.name
