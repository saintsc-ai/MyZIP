"""백그라운드 작업 스레드.

압축/해제는 수십 초가 걸릴 수 있으므로 반드시 별도 스레드에서 돌린다.
UI 스레드에는 시그널로만 소식을 전한다.
"""

from __future__ import annotations

from pathlib import Path
from typing import Callable, Sequence

from PySide6.QtCore import QObject, QThread, Signal

from ..core import (
    ArchiveEntry,
    ArchiveError,
    OperationCancelled,
    PasswordRequired,
    Progress,
    open_archive,
    strip_archive_suffix,
    walk_inputs,
    writer_for,
)


def _part_name(target: Path) -> str:
    """작업 중 임시 파일 이름. 원래 확장자를 그대로 유지한다.

    '자료.tar.gz' -> '자료.myzip-part.tar.gz'
    """
    stem = strip_archive_suffix(target.name)
    suffix = target.name[len(stem):]
    return f"{stem}.myzip-part{suffix}"


class _Task(QThread):
    """공통 뼈대: 진행률 시그널, 취소, 예외 포장."""

    progressed = Signal(int, str, int, int)  # 퍼센트, 현재 파일, 완료 수, 전체 수
    finished_ok = Signal(object)             # 결과
    failed = Signal(str)                     # 사용자에게 보여줄 메시지
    cancelled = Signal()
    password_required = Signal(str)           # 암호를 물어봐야 함

    def __init__(self, parent: QObject | None = None):
        super().__init__(parent)
        self.progress = Progress()
        self.progress.callback = self._on_progress
        self._last_percent = -1

    def cancel(self) -> None:
        self.progress.cancel()

    def _on_progress(self, p: Progress) -> None:
        # 시그널을 초당 수천 번 쏘면 UI 가 오히려 느려진다.
        # 퍼센트가 실제로 바뀔 때만 보낸다.
        percent = int(p.percent)
        if percent != self._last_percent:
            self._last_percent = percent
            self.progressed.emit(percent, p.current, p.done_files, p.total_files)

    def run(self) -> None:
        try:
            result = self.work()
        except OperationCancelled:
            self.cancelled.emit()
        except PasswordRequired as exc:
            # 암호 문제는 '실패'가 아니라 '되물어야 할 일'이다.
            # 오류창 대신 암호 입력을 띄우도록 따로 알린다.
            self.password_required.emit(str(exc))
        except ArchiveError as exc:
            self.failed.emit(str(exc))
        except Exception as exc:  # noqa: BLE001 - 스레드에서 새어나가면 앱이 죽는다
            self.failed.emit(f"예상치 못한 오류가 발생했습니다.\n\n{type(exc).__name__}: {exc}")
        else:
            self.finished_ok.emit(result)

    def work(self):  # pragma: no cover - 하위 클래스가 구현
        raise NotImplementedError


class CompressTask(_Task):
    """파일/폴더들을 아카이브 하나로 묶는다."""

    def __init__(self, sources: Sequence[Path], target: Path, level: int = 6,
                 password: str | None = None, parent: QObject | None = None):
        super().__init__(parent)
        self.sources = list(sources)
        self.target = Path(target)
        self.level = level
        self.password = password

    def work(self) -> Path:
        items = list(walk_inputs(self.sources))
        if not items:
            raise ArchiveError("압축할 파일이 없습니다.")

        self.target.parent.mkdir(parents=True, exist_ok=True)

        # 중간에 실패하거나 취소되면 반쪽짜리 파일이 남지 않게 임시로 만든 뒤 옮긴다.
        # 확장자는 그대로 두어야 한다. writer_for() 가 확장자로 형식을 고르므로
        # 'foo.zip.part' 같은 이름을 쓰면 형식을 알아보지 못한다.
        temp = self.target.with_name(_part_name(self.target))
        temp.unlink(missing_ok=True)
        try:
            with writer_for(temp, self.level, self.password) as writer:
                writer.write(items, self.progress)
            temp.replace(self.target)
        except BaseException:
            temp.unlink(missing_ok=True)
            raise
        return self.target


class ExtractTask(_Task):
    """아카이브를 폴더로 푼다."""

    def __init__(self, archive: Path, dest: Path,
                 members: Sequence[ArchiveEntry] | None = None,
                 password: str | None = None, strip_root: bool = False,
                 encoding: str | None = None,
                 reader=None, parent: QObject | None = None):
        super().__init__(parent)
        self.archive = Path(archive)
        self.dest = Path(dest)
        self.members = list(members) if members is not None else None
        self.password = password
        self.strip_root = strip_root
        self.encoding = encoding
        self._reader = reader          # 이미 열어 둔 리더가 있으면 재사용
        self.conflict_policy = "overwrite"

    def work(self) -> Path:
        if self._reader is not None:
            self._reader.extract(
                self.dest, self.members, self.progress,
                strip_root=self.strip_root,
                on_conflict=self._conflict,
            )
            return self.dest

        with open_archive(self.archive, self.password) as reader:
            if self.encoding:
                reader.set_encoding(self.encoding)
            members = self.members
            if members is not None:
                # 다시 연 리더의 엔트리 객체로 맞춘다.
                wanted = {e.path for e in members}
                members = [e for e in reader.entries if e.path in wanted]
            reader.extract(
                self.dest, members, self.progress,
                strip_root=self.strip_root,
                on_conflict=self._conflict,
            )
        return self.dest

    def _conflict(self, path: Path, entry: ArchiveEntry) -> str:
        return self.conflict_policy


class TestTask(_Task):
    """아카이브 무결성 검사."""

    def __init__(self, reader, parent: QObject | None = None):
        super().__init__(parent)
        self._reader = reader

    def work(self) -> list[str]:
        tester = getattr(self._reader, "test", None)
        if tester is None:
            raise ArchiveError("이 형식은 무결성 검사를 지원하지 않습니다.")
        return tester(self.progress)


class LoadTask(_Task):
    """아카이브를 열어 목록을 읽는다. 항목이 수만 개면 시간이 걸린다."""

    def __init__(self, path: Path, password: str | None = None,
                 encoding: str | None = None, parent: QObject | None = None):
        super().__init__(parent)
        self.path = Path(path)
        self.password = password
        self.encoding = encoding

    def work(self):
        reader = open_archive(self.path, self.password)
        if self.encoding:
            reader.set_encoding(self.encoding)
        reader.entries  # 여기서 실제로 읽는다
        return reader
