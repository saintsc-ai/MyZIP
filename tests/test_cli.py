"""컨텍스트 메뉴가 호출하는 명령들이 실제로 동작하는지 검증.

GUI 대화상자를 띄우지 않는 경로만 자동으로 확인한다.
(--extract-here / --compress-zip / --compress-tgz 는 대화상자 없이 끝난다.)
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from paths import work_dir  # noqa: E402

WORK = work_dir("myzip-cli-tests")
PYTHON = sys.executable
APP = ROOT / "myzip_app.py"

PASSED: list[str] = []
FAILED: list[str] = []


def check(name: str, ok: bool, detail: str = "") -> None:
    (PASSED if ok else FAILED).append(name)
    print(f"[{'PASS' if ok else 'FAIL'}] {name}" + (f"  -- {detail}" if detail and not ok else ""))


def run(args: list[str], timeout: int = 60) -> subprocess.CompletedProcess:
    """앱을 한 번 돌리고 끝날 때까지 기다린다."""
    return subprocess.run(
        [PYTHON, str(APP), *args],
        capture_output=True, text=True, encoding="utf-8",
        errors="replace", timeout=timeout, cwd=str(ROOT),
    )


def make_tree() -> Path:
    src = WORK / "자료"
    if src.exists():
        shutil.rmtree(src)
    (src / "안쪽").mkdir(parents=True)
    (src / "메모.txt").write_text("테스트 내용\n" * 50, encoding="utf-8")
    (src / "안쪽" / "data.bin").write_bytes(b"\x00\x01\x02" * 10000)
    return src


def test_help_and_version() -> None:
    proc = run(["--version"], timeout=30)
    check("--version 동작", proc.returncode == 0 and "MyZIP" in proc.stdout,
          f"rc={proc.returncode} out={proc.stdout!r} err={proc.stderr[:300]!r}")

    proc = run(["--help"], timeout=30)
    check("--help 동작", proc.returncode == 0 and "압축" in proc.stdout,
          f"rc={proc.returncode}")


def test_compress_zip() -> Path | None:
    src = make_tree()
    target = src.parent / "자료.zip"
    target.unlink(missing_ok=True)

    proc = run(["--compress-zip", str(src)], timeout=120)
    ok = target.exists() and target.stat().st_size > 0
    check("--compress-zip 으로 아카이브 생성", ok,
          f"rc={proc.returncode} err={proc.stderr[:400]!r}")
    return target if ok else None


def test_compress_tgz() -> Path | None:
    src = WORK / "자료"
    target = src.parent / "자료.tar.gz"
    target.unlink(missing_ok=True)

    proc = run(["--compress-tgz", str(src)], timeout=120)
    ok = target.exists() and target.stat().st_size > 0
    check("--compress-tgz 으로 아카이브 생성", ok,
          f"rc={proc.returncode} err={proc.stderr[:400]!r}")
    return target if ok else None


def test_extract_here(archive: Path) -> None:
    """--extract-here 는 아카이브가 있는 폴더에 그대로 푼다."""
    stage = WORK / "here"
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    copied = stage / archive.name
    shutil.copy(archive, copied)

    proc = run(["--extract-here", str(copied)], timeout=120)
    extracted = stage / "자료" / "메모.txt"
    check("--extract-here 로 압축 해제", extracted.exists(),
          f"rc={proc.returncode} 목록={[p.name for p in stage.rglob('*')][:12]} "
          f"err={proc.stderr[:400]!r}")


def test_extract_to(archive: Path) -> None:
    """--extract-to 는 아카이브 이름의 폴더를 만들어 푼다."""
    stage = WORK / "to"
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    copied = stage / archive.name
    shutil.copy(archive, copied)

    proc = run(["--extract-to", str(copied)], timeout=120)
    # 자료.zip -> 자료/ 폴더. 안에 중복 '자료' 폴더는 벗겨진다.
    made = stage / "자료"
    ok = made.is_dir() and (made / "메모.txt").exists()
    check("--extract-to 로 폴더 만들어 해제", ok,
          f"rc={proc.returncode} 목록={[str(p.relative_to(stage)) for p in stage.rglob('*')][:12]} "
          f"err={proc.stderr[:400]!r}")


def test_roundtrip_content(archive: Path) -> None:
    """푼 내용이 원본과 같은지 바이트 단위로 확인."""
    from myzip.core import open_archive

    stage = WORK / "verify"
    if stage.exists():
        shutil.rmtree(stage)
    with open_archive(archive) as reader:
        reader.extract(stage)

    original = WORK / "자료" / "메모.txt"
    restored = stage / "자료" / "메모.txt"
    ok = restored.exists() and restored.read_bytes() == original.read_bytes()
    check("해제 결과가 원본과 동일", ok)


def test_legacy_console_encoding() -> None:
    """한글을 못 담는 콘솔 인코딩에서도 죽지 않아야 한다.

    성공 메시지를 찍다가 UnicodeEncodeError 로 죽은 적이 있다.
    콘솔 없는 배포본에서는 그 예외가 모달 오류창으로 떠서 프로세스가
    영원히 멈춘다 — 컨텍스트 메뉴가 먹통이 된다는 뜻이다.
    """
    import os

    for encoding in ("cp1252", "ascii"):
        env = dict(os.environ, PYTHONIOENCODING=encoding, MYZIP_NO_SHELL="1")
        proc = subprocess.run(
            [PYTHON, str(APP), "--register"],
            capture_output=True, text=True, encoding="utf-8",
            errors="replace", timeout=90, cwd=str(ROOT), env=env,
        )
        check(f"{encoding} 콘솔에서 --register 정상 종료",
              proc.returncode == 0,
              f"rc={proc.returncode} err={proc.stderr[:300]!r}")

        proc = subprocess.run(
            [PYTHON, str(APP), "--help"],
            capture_output=True, text=True, encoding="utf-8",
            errors="replace", timeout=60, cwd=str(ROOT), env=env,
        )
        check(f"{encoding} 콘솔에서 --help 정상 종료",
              proc.returncode == 0,
              f"rc={proc.returncode} err={proc.stderr[:300]!r}")


def test_registry_roundtrip() -> None:
    """--register / --unregister 가 실제로 키를 만들고 지우는지."""
    from myzip.shell import registry

    run(["--register"], timeout=60)
    installed = registry.context_menu_installed()
    zip_status = registry.status(".zip")
    check("--register 로 컨텍스트 메뉴 설치", installed)
    check("--register 로 ProgID 생성",
          registry._get(r"Software\Classes\MyZIP.zip") is not None)

    run(["--unregister"], timeout=60)
    check("--unregister 로 메뉴 제거", not registry.context_menu_installed())
    check("--unregister 로 ProgID 제거",
          registry._get(r"Software\Classes\MyZIP.zip") is None)

    # 검사 후 다시 설치해 둔다
    run(["--register"], timeout=60)


def main() -> int:
    WORK.mkdir(parents=True, exist_ok=True)

    test_help_and_version()
    zip_archive = test_compress_zip()
    tgz_archive = test_compress_tgz()

    if zip_archive:
        test_extract_here(zip_archive)
        test_extract_to(zip_archive)
        test_roundtrip_content(zip_archive)
    if tgz_archive:
        test_extract_here(tgz_archive)

    test_legacy_console_encoding()
    test_registry_roundtrip()

    print()
    print(f"통과 {len(PASSED)} / 실패 {len(FAILED)}")
    if FAILED:
        print("실패 항목:", ", ".join(FAILED))
    return 1 if FAILED else 0


if __name__ == "__main__":
    raise SystemExit(main())
