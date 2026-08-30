"""7-Zip 콘솔(7z.exe) 래퍼.

RAR 은 RARLAB 독점 포맷이라 파이썬 순수 구현이 없다. 해제만이라도 하려면
외부 엔진이 필요하고, 7-Zip 이 가장 널리 쓰이며 재배포도 가능하다.
(단, 7-Zip 안의 unRAR 코드 라이선스상 RAR '생성'에는 쓸 수 없다.)
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Callable, Iterator, Sequence

from .base import ArchiveError, OperationCancelled, PasswordRequired, Progress

# 콘솔 창이 뜨지 않게 한다.
_NO_WINDOW = 0x08000000 if sys.platform == "win32" else 0


class SevenZipMissing(ArchiveError):
    """7z.exe 를 찾지 못했다."""


def app_dir() -> Path:
    """실행 파일(또는 소스)이 있는 폴더."""
    if getattr(sys, "frozen", False):
        return Path(sys.executable).parent
    return Path(__file__).resolve().parent.parent.parent


def bundle_dir() -> Path:
    """번들된 자원이 풀려 있는 폴더.

    PyInstaller onedir 는 실행 파일 옆 _internal/ 에, onefile 은 임시 폴더에
    자원을 둔다. 둘 다 sys._MEIPASS 로 가리켜진다.
    """
    if getattr(sys, "frozen", False):
        return Path(getattr(sys, "_MEIPASS", Path(sys.executable).parent))
    return app_dir()


def _candidates() -> Iterator[Path]:
    """7z.exe 가 있을 만한 곳들을 우선순위 순으로 내놓는다."""
    # 1) 우리가 함께 배포한 것
    yield bundle_dir() / "bin" / "7z.exe"
    yield app_dir() / "bin" / "7z.exe"
    yield app_dir() / "7z.exe"

    # 2) 레지스트리에 등록된 7-Zip 설치 경로
    if sys.platform == "win32":
        try:
            import winreg

            for root in (winreg.HKEY_CURRENT_USER, winreg.HKEY_LOCAL_MACHINE):
                try:
                    with winreg.OpenKey(root, r"SOFTWARE\7-Zip") as key:
                        path, _ = winreg.QueryValueEx(key, "Path")
                        yield Path(path) / "7z.exe"
                except OSError:
                    continue
        except ImportError:
            pass

    # 3) 표준 설치 위치
    for env in ("ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"):
        base = os.environ.get(env)
        if base:
            yield Path(base) / "7-Zip" / "7z.exe"

    # 4) PATH
    from shutil import which

    for name in ("7z.exe", "7z", "7za.exe"):
        found = which(name)
        if found:
            yield Path(found)


_cached_exe: Path | None = None


def find_7z(refresh: bool = False) -> Path | None:
    """사용 가능한 7z 실행 파일 경로. 없으면 None."""
    global _cached_exe
    if _cached_exe is not None and not refresh and _cached_exe.exists():
        return _cached_exe
    for candidate in _candidates():
        if candidate.is_file():
            _cached_exe = candidate
            return candidate
    _cached_exe = None
    return None


def require_7z() -> Path:
    exe = find_7z()
    if exe is None:
        raise SevenZipMissing(
            "RAR 압축 해제 엔진(7z.exe)을 찾지 못했습니다.\n"
            "7-Zip 을 설치하거나, MyZIP 설치 폴더의 bin 폴더에 "
            "7z.exe 와 7z.dll 을 넣어 주세요."
        )
    return exe


@dataclass
class SevenZipEntry:
    path: str
    size: int = 0
    packed: int = 0
    mtime: datetime | None = None
    is_dir: bool = False
    crc: str = ""
    encrypted: bool = False
    method: str = ""


_ERR_PASSWORD = re.compile(r"wrong password|password is incorrect|Cannot open encrypted",
                           re.IGNORECASE)
_PROGRESS = re.compile(r"(\d{1,3})%")


def _run(args: Sequence[str], password: str | None) -> subprocess.CompletedProcess:
    exe = require_7z()
    cmd = [str(exe), *args, "-sccUTF-8", "-scsUTF-8"]
    # 암호를 주지 않으면 7z 가 콘솔에서 입력을 기다리며 멈춘다.
    # 빈 값으로라도 반드시 넘겨서 대화형 대기를 막는다.
    cmd.append(f"-p{password}" if password else "-p")
    return subprocess.run(
        cmd,
        capture_output=True,
        encoding="utf-8",
        errors="replace",
        creationflags=_NO_WINDOW,
    )


def list_entries(archive: Path, password: str | None = None) -> list[SevenZipEntry]:
    """7z l -slt 출력을 파싱해 항목 목록을 만든다."""
    proc = _run(["l", "-slt", str(archive)], password)

    if proc.returncode != 0:
        text = (proc.stdout or "") + (proc.stderr or "")
        if _ERR_PASSWORD.search(text):
            raise PasswordRequired("암호가 필요하거나 틀렸습니다.")
        raise ArchiveError(f"목록을 읽지 못했습니다:\n{text.strip()[:500]}")

    entries: list[SevenZipEntry] = []
    current: dict[str, str] = {}
    in_body = False

    for line in (proc.stdout or "").splitlines():
        if not in_body:
            # 헤더와 본문은 '----------' 로 나뉜다.
            if line.startswith("----------"):
                in_body = True
            continue

        if not line.strip():
            if current:
                entries.append(_to_entry(current))
                current = {}
            continue

        key, sep, value = line.partition(" = ")
        if sep:
            current[key.strip()] = value.strip()

    if current:
        entries.append(_to_entry(current))

    return entries


def _to_entry(fields: dict[str, str]) -> SevenZipEntry:
    attrs = fields.get("Attributes", "")
    folder = fields.get("Folder", "")
    is_dir = folder == "+" or attrs.startswith("D")

    return SevenZipEntry(
        path=fields.get("Path", "").replace("\\", "/"),
        size=_int(fields.get("Size")),
        packed=_int(fields.get("Packed Size")),
        mtime=_parse_time(fields.get("Modified")),
        is_dir=is_dir,
        crc=fields.get("CRC", ""),
        encrypted=fields.get("Encrypted", "") == "+",
        method=fields.get("Method", ""),
    )


def _int(value: str | None) -> int:
    try:
        return int((value or "0").strip() or 0)
    except ValueError:
        return 0


def _parse_time(value: str | None) -> datetime | None:
    if not value:
        return None
    for fmt in ("%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"):
        try:
            return datetime.strptime(value.strip()[:19], fmt)
        except ValueError:
            continue
    return None


def extract(
    archive: Path,
    dest: Path,
    members: Sequence[str] | None = None,
    password: str | None = None,
    progress: Progress | None = None,
    keep_paths: bool = True,
) -> None:
    """아카이브를 dest 로 푼다. members 가 없으면 전체."""
    exe = require_7z()
    dest.mkdir(parents=True, exist_ok=True)

    cmd = [
        str(exe),
        "x" if keep_paths else "e",
        str(archive),
        f"-o{dest}",
        "-y",        # 모든 질문에 예
        "-bsp1",     # 진행률을 표준출력으로
        "-bb0",      # 파일 이름 로그는 끔
        "-sccUTF-8",
        "-scsUTF-8",
        f"-p{password}" if password else "-p",
    ]

    listfile: Path | None = None
    if members:
        # 파일 이름이 많으면 명령줄 길이 제한(32767자)에 걸리므로 목록 파일을 쓴다.
        listfile = dest / f".myzip-filelist-{os.getpid()}.txt"
        listfile.write_text("\n".join(members), encoding="utf-8")
        cmd.append(f"-i@{listfile}")

    try:
        _stream(cmd, progress)
    finally:
        if listfile is not None:
            listfile.unlink(missing_ok=True)


def _stream(cmd: Sequence[str], progress: Progress | None) -> None:
    """7z 를 돌리며 진행률(%)을 읽어 progress 에 반영한다."""
    proc = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        encoding="utf-8",
        errors="replace",
        bufsize=1,
        creationflags=_NO_WINDOW,
    )

    tail: list[str] = []
    buffer = ""
    try:
        assert proc.stdout is not None
        while True:
            char = proc.stdout.read(1)
            if not char:
                break
            if char in ("\r", "\n"):
                line = buffer.strip()
                buffer = ""
                if not line:
                    continue
                tail.append(line)
                del tail[:-30]
                if progress is not None:
                    match = _PROGRESS.search(line)
                    if match and progress.total_bytes:
                        pct = min(100, int(match.group(1)))
                        progress.done_bytes = progress.total_bytes * pct // 100
                        if progress.callback:
                            progress.callback(progress)
                    if progress.cancelled:
                        proc.kill()
                        raise OperationCancelled("사용자가 작업을 취소했습니다.")
            else:
                buffer += char
    finally:
        if proc.poll() is None:
            proc.kill()
        proc.wait()

    if proc.returncode not in (0, 1):   # 1 = 경고(일부 파일 건너뜀)
        text = "\n".join(tail)
        if _ERR_PASSWORD.search(text):
            raise PasswordRequired("암호가 틀렸습니다.")
        raise ArchiveError(f"압축 해제에 실패했습니다 (코드 {proc.returncode}):\n{text[:500]}")


def test_archive(archive: Path, password: str | None = None) -> tuple[bool, str]:
    """무결성 검사. (정상 여부, 메시지)."""
    proc = _run(["t", str(archive)], password)
    text = ((proc.stdout or "") + (proc.stderr or "")).strip()
    return proc.returncode == 0, text
