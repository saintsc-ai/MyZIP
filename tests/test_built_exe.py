"""빌드된 실행 파일이 실제로 동작하는지 검증.

소스에서 되는 것과 PyInstaller 로 묶은 것이 되는 것은 다른 문제다.
번들 경로(_internal), 아이콘, 7z 엔진, 레지스트리 등록까지 확인한다.
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

EXE = ROOT / "dist" / "MyZIP" / "MyZIP.exe"
WORK = work_dir("myzip-exe-tests")

PASSED: list[str] = []
FAILED: list[str] = []


def check(name: str, ok: bool, detail: str = "") -> None:
    (PASSED if ok else FAILED).append(name)
    print(f"[{'PASS' if ok else 'FAIL'}] {name}" + (f"  -- {detail}" if detail and not ok else ""))


def run(args: list[str], timeout: int = 90) -> subprocess.CompletedProcess:
    import os

    env = dict(os.environ, MYZIP_NO_SHELL="1")
    return subprocess.run(
        [str(EXE), *args], capture_output=True, text=True,
        encoding="utf-8", errors="replace", timeout=timeout, env=env,
    )


def make_source() -> Path:
    src = WORK / "실행테스트"
    if src.exists():
        shutil.rmtree(src)
    (src / "폴더").mkdir(parents=True)
    (src / "문서.txt").write_text("빌드된 exe 테스트\n" * 60, encoding="utf-8")
    (src / "폴더" / "데이터.bin").write_bytes(bytes(range(256)) * 800)
    return src


def test_startup_speed() -> None:
    """탐색기에서 더블클릭했을 때 답답하지 않아야 한다."""
    start = time.monotonic()
    proc = run(["--version"], timeout=60)
    elapsed = time.monotonic() - start
    check("exe 실행 및 --version", proc.returncode == 0 and "MyZIP" in proc.stdout,
          f"rc={proc.returncode} out={proc.stdout!r} err={proc.stderr[:300]!r}")
    print(f"       시작 시간: {elapsed:.2f}초")
    check("시작 시간 3초 이내", elapsed < 3.0, f"{elapsed:.2f}초")


def test_bundled_engine() -> None:
    """번들된 7z 엔진을 찾는지 (RAR 지원 여부)."""
    internal = EXE.parent / "_internal" / "bin" / "7z.exe"
    beside = EXE.parent / "bin" / "7z.exe"
    check("7z 엔진이 배포에 포함됨", internal.exists() or beside.exists(),
          f"찾은 위치 없음: {internal}")


def test_icons_bundled() -> None:
    internal = EXE.parent / "_internal" / "resources" / "icons"
    beside = EXE.parent / "resources" / "icons"
    folder = internal if internal.exists() else beside
    icons = list(folder.glob("*.ico")) if folder.exists() else []
    check("아이콘이 배포에 포함됨", len(icons) >= 10, f"{folder}: {len(icons)}개")


def test_compress_and_extract() -> None:
    src = make_source()

    archive = src.parent / "실행테스트.zip"
    archive.unlink(missing_ok=True)
    proc = run(["--compress-zip", str(src)])
    check("exe 로 ZIP 압축", archive.exists() and archive.stat().st_size > 0,
          f"rc={proc.returncode} err={proc.stderr[:400]!r}")
    if not archive.exists():
        return

    stage = WORK / "풀기"
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    copied = stage / archive.name
    shutil.copy(archive, copied)

    proc = run(["--extract-here", str(copied)])
    restored = stage / "실행테스트" / "문서.txt"
    check("exe 로 압축 해제", restored.exists(),
          f"rc={proc.returncode} 결과={[p.name for p in stage.rglob('*')][:10]}")

    if restored.exists():
        check("해제 내용이 원본과 동일",
              restored.read_bytes() == (src / "문서.txt").read_bytes())


def test_registry_paths() -> None:
    """exe 로 등록했을 때 레지스트리에 유효한 경로가 들어가는지."""
    from myzip.shell import registry

    run(["--register"])

    command = registry._get(
        r"Software\Classes\SystemFileAssociations\.zip\shell\MyZIP\shell"
        r"\02here\command"
    )
    check("컨텍스트 메뉴 명령 등록됨", command is not None, str(command))

    if command:
        exe_in_registry = command.split('"')[1] if '"' in command else ""
        check("등록된 경로가 빌드된 exe",
              Path(exe_in_registry).name.lower() == "myzip.exe",
              f"got={exe_in_registry}")
        check("등록된 exe 가 실제로 존재", Path(exe_in_registry).exists(),
              exe_in_registry)

    icon = registry._get(
        r"Software\Classes\SystemFileAssociations\.zip\shell\MyZIP", "Icon"
    )
    if icon:
        icon_path = Path(icon.rsplit(",", 1)[0])
        check("등록된 아이콘 파일이 실제로 존재", icon_path.exists(), str(icon_path))
    else:
        check("아이콘 등록됨", False, "Icon 값이 없습니다")


def main() -> int:
    if not EXE.exists():
        print(f"빌드된 실행 파일이 없습니다: {EXE}")
        print("먼저 python build.py 를 실행하세요.")
        return 1

    WORK.mkdir(parents=True, exist_ok=True)
    print(f"검증 대상: {EXE}")
    print()

    test_startup_speed()
    test_bundled_engine()
    test_icons_bundled()
    test_compress_and_extract()
    test_registry_paths()

    print()
    print(f"통과 {len(PASSED)} / 실패 {len(FAILED)}")
    if FAILED:
        print("실패 항목:", ", ".join(FAILED))
    return 1 if FAILED else 0


if __name__ == "__main__":
    raise SystemExit(main())
