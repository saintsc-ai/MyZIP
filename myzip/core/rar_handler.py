"""RAR 읽기 전용 핸들러.

RAR 압축(생성)은 지원하지 않는다 — RARLAB 독점 포맷이고 unRAR 라이선스가
RAR 압축기 제작에 코드를 쓰는 것을 금지하기 때문이다. 반디집, 7-Zip 도
같은 이유로 RAR 은 해제만 지원한다.
"""

from __future__ import annotations

import os
import shutil
from pathlib import Path
from typing import Callable, Sequence

from . import sevenzip
from .base import (
    ArchiveEntry,
    ArchiveError,
    ArchiveReader,
    OperationCancelled,
    Progress,
)
from .path_safety import safe_join, unique_path


class RarReader(ArchiveReader):
    """7z.exe 를 엔진으로 쓰는 RAR 리더.

    7z 는 프로세스 단위로만 동작하므로 항목 하나씩 꺼내는 것은 매우 느리다.
    그래서 extract() 를 통째로 재정의해 한 번의 호출로 모두 꺼낸 다음
    충돌 처리를 하며 제자리로 옮긴다.
    """

    extensions = (".rar", ".cbr", ".7z", ".cb7", ".iso", ".cab", ".arj", ".lzh")
    format_name = "RAR"

    def _load_entries(self) -> list[ArchiveEntry]:
        raw_entries = sevenzip.list_entries(self.path, self.password)
        entries: list[ArchiveEntry] = []
        for e in raw_entries:
            if not e.path:
                continue
            entries.append(
                ArchiveEntry(
                    path=e.path,
                    size=e.size,
                    compressed_size=e.packed or e.size,
                    mtime=e.mtime,
                    is_dir=e.is_dir,
                    crc=int(e.crc, 16) if e.crc else None,
                    encrypted=e.encrypted,
                    method=e.method or ("폴더" if e.is_dir else "파일"),
                    raw_name=e.path.encode("utf-8"),
                    handle=e.path,
                )
            )
        # 7z 는 이미 UTF-8 로 이름을 돌려주므로 재해석이 필요 없다.
        self.encoding = "utf-8"
        return entries

    def set_encoding(self, encoding: str) -> None:
        # 7z 엔진이 인코딩을 이미 처리했다. 바꿀 것이 없다.
        return

    # -- 추출 --------------------------------------------------------

    def extract(
        self,
        dest_dir: str | os.PathLike,
        members: Sequence[ArchiveEntry] | None = None,
        progress: Progress | None = None,
        strip_root: bool = False,
        on_conflict: Callable[[Path, ArchiveEntry], str] | None = None,
    ) -> list[Path]:
        from .base import _common_root

        dest_dir = Path(dest_dir)
        dest_dir.mkdir(parents=True, exist_ok=True)
        targets = list(members if members is not None else self.entries)
        progress = progress or Progress()
        progress.total_files = len(targets)
        progress.total_bytes = sum(e.size for e in targets if not e.is_dir)

        prefix = _common_root(self.entries) if strip_root else ""

        # 같은 볼륨에 임시 폴더를 두면 나중에 옮기는 것이 단순한 이름 변경이라
        # 사실상 공짜다. 덕분에 충돌 처리와 경로 검증을 우리가 직접 할 수 있다.
        staging = Path(dest_dir) / f".myzip-tmp-{os.getpid()}"
        if staging.exists():
            shutil.rmtree(staging, ignore_errors=True)

        try:
            names = None if members is None else [e.path for e in targets]
            sevenzip.extract(
                self.path, staging, names, self.password, progress
            )
            return self._relocate(staging, dest_dir, targets, prefix, progress, on_conflict)
        finally:
            shutil.rmtree(staging, ignore_errors=True)

    def _relocate(
        self,
        staging: Path,
        dest_dir: Path,
        targets: Sequence[ArchiveEntry],
        prefix: str,
        progress: Progress,
        on_conflict: Callable[[Path, ArchiveEntry], str] | None,
    ) -> list[Path]:
        """임시 폴더에서 최종 위치로 옮기며 충돌을 처리한다."""
        written: list[Path] = []
        progress.done_files = 0

        for entry in targets:
            progress.check_cancel()
            rel = entry.path
            if prefix and rel.startswith(prefix):
                rel = rel[len(prefix):].lstrip("/")
                if not rel:
                    continue

            source = staging / Path(*[p for p in entry.path.split("/") if p])
            if not source.exists():
                progress.advance(files=1)
                continue  # 7z 가 건너뛴 항목

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
                    progress.advance(files=1)
                    continue
                if action == "rename":
                    out = unique_path(out)
                elif action == "cancel":
                    raise OperationCancelled("덮어쓰기 확인에서 취소했습니다.")
                else:
                    out.unlink(missing_ok=True)

            try:
                os.replace(source, out)
            except OSError:
                shutil.move(str(source), str(out))
            written.append(out)
            progress.advance(files=1)

        return written

    def _extract_one(self, entry: ArchiveEntry, dest: Path, progress: Progress) -> None:
        # extract() 를 통째로 재정의했으므로 여기로 오는 경우는 미리보기 뿐이다.
        staging = dest.parent / f".myzip-one-{os.getpid()}"
        try:
            sevenzip.extract(self.path, staging, [entry.path], self.password, None)
            source = staging / Path(*[p for p in entry.path.split("/") if p])
            if source.exists():
                os.replace(source, dest)
        finally:
            shutil.rmtree(staging, ignore_errors=True)

    def read_bytes(self, entry: ArchiveEntry) -> bytes:
        import tempfile

        with tempfile.TemporaryDirectory(prefix="myzip-") as tmp:
            sevenzip.extract(self.path, Path(tmp), [entry.path], self.password, None)
            source = Path(tmp) / Path(*[p for p in entry.path.split("/") if p])
            if not source.exists():
                raise ArchiveError(f"항목을 읽지 못했습니다: {entry.path}")
            return source.read_bytes()

    def test(self, progress: Progress) -> list[str]:
        ok, message = sevenzip.test_archive(self.path, self.password)
        return [] if ok else [message]
