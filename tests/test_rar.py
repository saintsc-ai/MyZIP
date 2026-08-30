"""RAR / 7Z 해제 검증.

RAR 압축기는 만들 수 없으므로(라이선스), 테스트용 RAR 을 직접 만들 수 없다.
대신 7z.exe 로 7Z 아카이브를 만들어 RarReader(7z 엔진 경로)를 검증하고,
RAR 파일이 있으면 그것도 함께 확인한다.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from paths import work_dir  # noqa: E402
from myzip.core import Progress, format_of, open_archive  # noqa: E402
from myzip.core.sevenzip import find_7z  # noqa: E402

WORK = work_dir("myzip-rar-tests")
PASSED: list[str] = []
FAILED: list[str] = []
SKIPPED: list[str] = []


def check(name: str, ok: bool, detail: str = "") -> None:
    (PASSED if ok else FAILED).append(name)
    print(f"[{'PASS' if ok else 'FAIL'}] {name}" + (f"  -- {detail}" if detail and not ok else ""))


def skip(name: str, why: str) -> None:
    SKIPPED.append(name)
    print(f"[SKIP] {name}  -- {why}")


def make_source() -> Path:
    src = WORK / "샘플"
    if src.exists():
        shutil.rmtree(src)
    (src / "안쪽폴더").mkdir(parents=True)
    (src / "읽어보기.txt").write_text("한글 내용 테스트\n" * 40, encoding="utf-8")
    (src / "안쪽폴더" / "이진.dat").write_bytes(bytes(range(256)) * 300)
    (src / "안쪽폴더" / "spaces in name.log").write_text("log line\n" * 200,
                                                        encoding="utf-8")
    return src


def snapshot(root: Path) -> dict[str, int]:
    return {
        p.relative_to(root).as_posix(): (-1 if p.is_dir() else p.stat().st_size)
        for p in sorted(root.rglob("*"))
    }


def build_with_7z(src: Path, archive: Path, password: str | None = None) -> bool:
    exe = find_7z()
    if exe is None:
        return False
    archive.unlink(missing_ok=True)
    cmd = [str(exe), "a", str(archive), str(src), "-y", "-bso0", "-bsp0"]
    if password:
        cmd.append(f"-p{password}")
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=180)
    return result.returncode == 0 and archive.exists()


def verify_roundtrip(archive: Path, src: Path, label: str,
                     password: str | None = None) -> None:
    fmt = format_of(archive)
    check(f"{label}: 형식 판별", fmt is not None, f"got={fmt}")

    dest = WORK / f"out-{archive.stem}"
    if dest.exists():
        shutil.rmtree(dest)

    with open_archive(archive, password) as reader:
        entries = reader.entries
        names = [e.path for e in entries]
        check(f"{label}: 목록 읽기", len(entries) > 0, f"entries={len(entries)}")
        check(f"{label}: 한글 파일명 유지",
              any("읽어보기.txt" in n for n in names), str(names))
        reader.extract(dest, progress=Progress())

    before = snapshot(src)
    after = snapshot(dest / src.name)
    check(f"{label}: 내용 일치", before == after,
          f"before={before} after={after}")


def test_7z() -> None:
    src = make_source()
    archive = WORK / "샘플.7z"
    if not build_with_7z(src, archive):
        skip("7Z 해제", "7z.exe 를 찾지 못했습니다")
        return
    verify_roundtrip(archive, src, "7Z")


def test_7z_password() -> None:
    src = WORK / "샘플"
    archive = WORK / "샘플-암호.7z"
    if not build_with_7z(src, archive, password="비밀1234"):
        skip("암호 7Z", "7z.exe 를 찾지 못했습니다")
        return

    with open_archive(archive, "비밀1234") as reader:
        entries = reader.entries
        target = next(e for e in entries if e.path.endswith("읽어보기.txt"))
        data = reader.read_bytes(target)
    original = (src / "읽어보기.txt").read_bytes()
    check("암호 7Z 해제", data == original)

    blocked = False
    try:
        with open_archive(archive, "틀린암호") as reader:
            reader.read_bytes(next(e for e in reader.entries if not e.is_dir))
    except Exception:
        blocked = True
    check("암호 7Z: 틀린 암호 거부", blocked)


def oem_codepage() -> int:
    """Windows 의 OEM 코드페이지. 한국어 Windows 는 949."""
    try:
        import ctypes

        return int(ctypes.windll.kernel32.GetOEMCP())
    except Exception:
        return 0


def test_rar_files() -> None:
    """진짜 RAR 컨테이너로 해제 경로를 검증한다.

    RAR 압축기는 만들 수 없으므로 무압축(store) RAR 을 직접 조립해 쓴다.

    파일 이름은 ASCII 로 둔다. RAR4 는 유니코드 플래그가 없으면 이름을
    '만든 PC 의 OEM 코드페이지' 로 해석하기로 되어 있어서, 한글 이름을
    쓰면 검사 결과가 돌리는 PC 의 로케일에 따라 달라진다.
    (한글 이름 처리는 test_rar_korean_names 에서 따로 본다.)
    """
    from rar_fixture import build_rar

    text = b"plain ascii payload\n" * 40
    binary = bytes(range(256)) * 300

    archive = build_rar(
        WORK / "assembled.rar",
        {
            "sample/readme.txt": text,
            "sample/inner/data.dat": binary,
        },
        directories=("sample", "sample/inner"),
    )

    check("RAR 형식 판별 (매직 바이트)", format_of(archive) == "rar",
          f"got={format_of(archive)}")

    dest = WORK / "rar-out"
    if dest.exists():
        shutil.rmtree(dest)

    with open_archive(archive) as reader:
        entries = reader.entries
        names = sorted(e.path for e in entries)
        check("RAR 목록 읽기", len(entries) == 4, f"entries={names}")
        check("RAR 경로 구조 읽기",
              names == ["sample", "sample/inner", "sample/inner/data.dat",
                        "sample/readme.txt"], str(names))
        reader.extract(dest, progress=Progress())

    restored_text = dest / "sample" / "readme.txt"
    restored_bin = dest / "sample" / "inner" / "data.dat"

    check("RAR 텍스트 내용 일치",
          restored_text.exists() and restored_text.read_bytes() == text,
          f"exists={restored_text.exists()}")
    check("RAR 이진 내용 일치",
          restored_bin.exists() and restored_bin.read_bytes() == binary,
          f"exists={restored_bin.exists()}")
    check("RAR 폴더 구조 유지", (dest / "sample" / "inner").is_dir())


def test_rar_korean_names() -> None:
    """한글 이름이 든 RAR 을 제대로 읽는지.

    RAR4 는 이름을 OEM 코드페이지로 저장한다. 우리 픽스처는 CP949 로
    쓰므로, 이 검사는 OEM 코드페이지가 949 인 PC 에서만 의미가 있다.
    """
    codepage = oem_codepage()
    if codepage != 949:
        skip("RAR 한글 파일명",
             f"이 PC 의 OEM 코드페이지가 {codepage or '알 수 없음'} 입니다 "
             "(CP949 한국어 Windows 에서만 검사)")
        return

    from rar_fixture import build_rar

    text = "한글 내용 테스트\n".encode("utf-8") * 40
    archive = build_rar(
        WORK / "조립-한글.rar",
        {"샘플/읽어보기.txt": text},
        directories=("샘플",),
    )

    dest = WORK / "rar-korean-out"
    if dest.exists():
        shutil.rmtree(dest)

    with open_archive(archive) as reader:
        names = sorted(e.path for e in reader.entries)
        check("RAR 한글 파일명", any("읽어보기.txt" in n for n in names), str(names))
        reader.extract(dest, progress=Progress())

    restored = dest / "샘플" / "읽어보기.txt"
    check("RAR 한글 경로로 해제",
          restored.exists() and restored.read_bytes() == text,
          f"exists={restored.exists()}")


def test_header_encrypted() -> None:
    """헤더까지 암호화된 아카이브는 목록조차 못 읽는다.

    이때 일반 오류가 아니라 PasswordRequired 로 신호해야 UI 가
    오류창 대신 암호 입력을 띄울 수 있다.
    """
    from myzip.core import PasswordRequired

    exe = find_7z()
    if exe is None:
        skip("헤더 암호화 아카이브", "7z.exe 를 찾지 못했습니다")
        return

    src = WORK / "샘플"
    archive = WORK / "헤더암호.7z"
    archive.unlink(missing_ok=True)
    result = subprocess.run(
        [str(exe), "a", str(archive), str(src), "-mhe=on", "-p테스트암호",
         "-y", "-bso0", "-bsp0"],
        capture_output=True, text=True, timeout=180,
    )
    if result.returncode != 0 or not archive.exists():
        skip("헤더 암호화 아카이브", "테스트 파일을 만들지 못했습니다")
        return

    signalled = False
    try:
        with open_archive(archive) as reader:
            reader.entries
    except PasswordRequired:
        signalled = True
    except Exception as exc:
        check("헤더 암호화: 암호 필요 신호", False,
              f"엉뚱한 예외 {type(exc).__name__}: {exc}")
        return
    check("헤더 암호화: 암호 필요 신호", signalled)

    with open_archive(archive, "테스트암호") as reader:
        check("헤더 암호화: 암호 주면 열림", len(reader.entries) > 0)


def test_rar_write_refused() -> None:
    """RAR 로 압축을 시도하면 이유를 알려주며 거절해야 한다."""
    from myzip.core import UnsupportedFormat, writer_for

    try:
        writer_for(WORK / "안됨.rar")
        check("RAR 압축 거절", False, "예외가 발생하지 않았습니다")
    except UnsupportedFormat as exc:
        check("RAR 압축 거절", "zip" in str(exc).lower(), str(exc))


def test_engine_missing_message() -> None:
    """엔진이 없을 때 사용자에게 쓸모 있는 메시지가 나오는지."""
    import myzip.core.sevenzip as sz

    saved_cache, saved_candidates = sz._cached_exe, sz._candidates
    try:
        sz._cached_exe = None
        sz._candidates = lambda: iter(())
        try:
            sz.require_7z()
            check("엔진 없음 안내", False, "예외가 발생하지 않았습니다")
        except sz.SevenZipMissing as exc:
            message = str(exc)
            check("엔진 없음 안내",
                  "7z.exe" in message and "bin" in message, message)
    finally:
        sz._cached_exe, sz._candidates = saved_cache, saved_candidates


def main() -> int:
    WORK.mkdir(parents=True, exist_ok=True)
    exe = find_7z()
    print(f"7z 엔진: {exe or '없음'}")
    print()

    test_7z()
    test_7z_password()
    test_rar_files()
    test_rar_korean_names()
    test_header_encrypted()
    test_rar_write_refused()
    test_engine_missing_message()

    print()
    print(f"통과 {len(PASSED)} / 실패 {len(FAILED)} / 건너뜀 {len(SKIPPED)}")
    if FAILED:
        print("실패 항목:", ", ".join(FAILED))
    return 1 if FAILED else 0


if __name__ == "__main__":
    raise SystemExit(main())
