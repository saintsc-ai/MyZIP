"""MyZIP 빌드 스크립트 (Rust 판).

    python build.py              exe 빌드 + 배포 폴더(dist/MyZIP) 만들기 + 동작 확인
    python build.py --installer  위 + Inno Setup 설치 파일
    python build.py --release    처음부터 다시: 설치 파일 + 포터블 ZIP + 체크섬 (release/)
    python build.py --clean      빌드 산출물 정리

필요한 것: Rust(MSVC 툴체인), 설치 파일을 만들 때는 Inno Setup 6.
Python 표준 라이브러리만 쓴다.

배포 폴더는 이렇게 생겼다. 아이콘은 exe 안에 들어 있다.

    MyZIP/
      MyZIP.exe
      bin/7z.exe, bin/7z.dll     RAR/7Z 해제 엔진 (tools/fetch_7zip.py 가 받아 둔 것)
      LICENSE.txt, THIRD-PARTY-NOTICES.md
"""

from __future__ import annotations

import argparse
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent
RUST = ROOT / "rust"
DIST = ROOT / "dist"
APP_DIR = DIST / "MyZIP"
RELEASE = ROOT / "release"
INSTALLER_OUT = ROOT / "installer" / "output"
APP_NAME = "MyZIP"


def version() -> str:
    with open(RUST / "Cargo.toml", "rb") as f:
        return tomllib.load(f)["workspace"]["package"]["version"]


def run(cmd: list[str], cwd: Path = ROOT, **kwargs) -> subprocess.CompletedProcess:
    print(f"$ {' '.join(str(c) for c in cmd)}", flush=True)
    return subprocess.run([str(c) for c in cmd], cwd=str(cwd), **kwargs)


def cargo() -> str | None:
    found = shutil.which("cargo")
    if found:
        return found
    candidate = Path.home() / ".cargo" / "bin" / "cargo.exe"
    return str(candidate) if candidate.exists() else None


# ------------------------------------------------------------------ 단계

def fetch_engine() -> None:
    print("\n== RAR/7Z 엔진 확인 ==")
    if run([sys.executable, "tools/fetch_7zip.py"]).returncode != 0:
        print("경고: RAR/7Z 엔진 없이 빌드합니다. ZIP/TAR 은 정상 동작합니다.")


def build_exe() -> bool:
    print("\n== 실행 파일 빌드 ==")
    exe = cargo()
    if exe is None:
        print("cargo 를 찾지 못했습니다. https://rustup.rs 에서 Rust 를 설치하세요.")
        return False
    if run([exe, "build", "--release", "--locked"], cwd=RUST).returncode != 0:
        return False

    built = RUST / "target" / "release" / f"{APP_NAME}.exe"
    if not built.exists():
        print(f"실행 파일이 만들어지지 않았습니다: {built}")
        return False

    if APP_DIR.exists():
        shutil.rmtree(APP_DIR)
    APP_DIR.mkdir(parents=True)
    shutil.copy2(built, APP_DIR / built.name)
    shutil.copy2(ROOT / "LICENSE", APP_DIR / "LICENSE.txt")
    shutil.copy2(ROOT / "THIRD-PARTY-NOTICES.md", APP_DIR / "THIRD-PARTY-NOTICES.md")

    engine = ROOT / "bin"
    if (engine / "7z.exe").exists() and (engine / "7z.dll").exists():
        (APP_DIR / "bin").mkdir()
        for name in ("7z.exe", "7z.dll"):
            shutil.copy2(engine / name, APP_DIR / "bin" / name)
    else:
        print("경고: bin/7z.exe 가 없어 RAR/7Z 엔진 없이 배포 폴더를 만듭니다.")

    print(f"\n배포 폴더: {APP_DIR}  ({folder_size(APP_DIR) / 1024 / 1024:.1f} MB)")
    return True


def folder_size(path: Path) -> int:
    return sum(f.stat().st_size for f in path.rglob("*") if f.is_file())


def quiet_env() -> dict[str, str]:
    """만든 exe 를 시험할 때 쓰는 환경. 탐색기 창, 메시지 상자, 사용자 설정을 건드리지 않는다."""
    return dict(
        os.environ,
        MYZIP_NO_SHELL="1",
        MYZIP_NO_IPC="1",
        MYZIP_REG_ROOT=r"Software\MyZIP-Build",
    )


def myzip(*args: str | Path) -> subprocess.CompletedProcess:
    # 출력을 받아 두면(파이프) MyZIP 은 창 없이 동작한다.
    return run([APP_DIR / f"{APP_NAME}.exe", *args], env=quiet_env(),
               capture_output=True, text=True, encoding="utf-8", errors="replace",
               timeout=300)


def smoke_test() -> bool:
    """배포 폴더의 exe 가 실제로 압축하고 풀리는지 확인한다."""
    print("\n== 배포본 동작 확인 ==")
    expected = f"{APP_NAME} {version()}"
    out = myzip("--version")
    if out.returncode != 0 or expected not in out.stdout:
        print(f"버전이 다릅니다: {out.stdout.strip()!r} (기대: {expected!r})")
        return False

    with tempfile.TemporaryDirectory(prefix="myzip-build-") as tmp:
        work = Path(tmp)
        src = work / "자료"
        (src / "하위").mkdir(parents=True)
        (src / "한글.txt").write_text("안녕하세요\n" * 200, encoding="utf-8")
        (src / "하위" / "data.bin").write_bytes(os.urandom(4096))

        out = work / "풀기"
        out.mkdir()
        steps = [
            ("ZIP 압축", ["--compress-zip", src]),
            ("무결성 검사", ["--test", work / "자료.zip"]),
        ]
        for label, args in steps:
            result = myzip(*args)
            if result.returncode != 0:
                print(f"[실패] {label}\n{result.stdout}{result.stderr}")
                return False
            print(f"[통과] {label}")

        shutil.copy2(work / "자료.zip", out / "자료.zip")
        result = myzip("--extract-to", out / "자료.zip")
        restored = out / "자료"
        if result.returncode != 0 or not restored.is_dir():
            print(f"[실패] 폴더 만들어 풀기\n{result.stdout}{result.stderr}")
            return False
        same = all(
            (restored / rel).read_bytes() == (src / rel).read_bytes()
            for rel in ("한글.txt", "하위/data.bin")
        )
        if not same:
            print("[실패] 푼 내용이 원본과 다릅니다.")
            return False
        print("[통과] 폴더 만들어 풀기, 내용 일치")

        if (APP_DIR / "bin" / "7z.exe").exists():
            # 7-Zip 엔진이 번들 위치(bin/)에서 잡히는지: 7z 로 만든 아카이브를 풀어 본다.
            seven = work / "seven.7z"
            made = run([APP_DIR / "bin" / "7z.exe", "a", "-bd", seven, src / "한글.txt"],
                       capture_output=True)
            if made.returncode == 0:
                result = myzip("--test", seven)
                if result.returncode != 0:
                    print(f"[실패] 7Z 검사 (bin/7z.exe)\n{result.stdout}{result.stderr}")
                    return False
                print("[통과] 7Z 검사 (bin/7z.exe)")
    return True


def find_inno() -> Path | None:
    """ISCC.exe(Inno Setup 컴파일러)를 찾는다. winget 설치본은 사용자 폴더에 있다."""
    roots = [
        os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)"),
        os.environ.get("ProgramFiles", r"C:\Program Files"),
        str(Path(os.environ.get("LOCALAPPDATA", "")) / "Programs"),
    ]
    for root in roots:
        for name in ("Inno Setup 6", "Inno Setup 7"):
            path = Path(root) / name / "ISCC.exe"
            if path.exists():
                return path
    found = shutil.which("ISCC.exe")
    return Path(found) if found else None


def build_installer() -> bool:
    print("\n== 설치 파일 빌드 ==")
    iscc = find_inno()
    if iscc is None:
        print("Inno Setup 6 을 찾지 못했습니다. https://jrsoftware.org/isdl.php")
        print("(설치 파일 없이 dist/MyZIP 폴더를 그대로 배포할 수도 있습니다.)")
        return False
    script = ROOT / "installer" / "myzip.iss"
    if run([iscc, f"/DAppVersion={version()}", script]).returncode != 0:
        return False
    setup = INSTALLER_OUT / f"{APP_NAME}-{version()}-setup.exe"
    if not setup.exists():
        print(f"설치 파일이 없습니다: {setup}")
        return False
    print(f"설치 파일: {setup}  ({setup.stat().st_size / 1024 / 1024:.1f} MB)")
    return True


PORTABLE_NOTE = """MyZIP {version} — 포터블 버전

설치가 필요 없습니다. 이 폴더를 원하는 곳에 통째로 두고
MyZIP.exe 를 실행하면 됩니다.

탐색기 우클릭 메뉴와 확장자 연결을 쓰려면 한 번만 등록하세요.

    MyZIP.exe --register

되돌리려면:

    MyZIP.exe --unregister

등록 정보는 전부 HKEY_CURRENT_USER 아래에만 쓰이므로
관리자 권한이 필요 없고, 폴더를 옮겼다면 --register 를 다시 실행하세요.

Windows 11 에서는 우클릭 후 '추가 옵션 표시'(Shift+F10) 안에 나타납니다.

압축  : ZIP(AES-256 암호), TAR, TAR.GZ/TGZ, TAR.BZ2, TAR.XZ
해제  : 위 형식 + RAR, 7Z, CAB, ARJ, LZH, ISO

RAR 은 RARLAB 독점 형식이라 해제만 가능합니다.
"""


def make_portable() -> Path | None:
    """배포 폴더를 포터블 ZIP 으로 묶는다. 묶는 일도 방금 만든 MyZIP.exe 가 한다."""
    print("\n== 포터블 ZIP 만들기 ==")
    note = APP_DIR / "사용법.txt"
    note.write_text(PORTABLE_NOTE.format(version=version()), encoding="utf-8")
    made = DIST / f"{APP_NAME}.zip"
    made.unlink(missing_ok=True)
    try:
        result = myzip("--compress-zip", APP_DIR)
    finally:
        note.unlink(missing_ok=True)
    if result.returncode != 0 or not made.exists():
        print(f"포터블 ZIP 을 만들지 못했습니다.\n{result.stdout}{result.stderr}")
        return None
    if myzip("--test", made).returncode != 0:
        print("포터블 ZIP 검사에 실패했습니다.")
        return None

    RELEASE.mkdir(parents=True, exist_ok=True)
    target = RELEASE / f"{APP_NAME}-{version()}-portable.zip"
    target.unlink(missing_ok=True)
    shutil.move(made, target)
    print(f"포터블: {target}  ({target.stat().st_size / 1024 / 1024:.1f} MB)")
    return target


def collect_release() -> list[Path]:
    """산출물을 release/ 로 모으고 체크섬을 적는다."""
    RELEASE.mkdir(parents=True, exist_ok=True)
    artifacts: list[Path] = []

    setup = INSTALLER_OUT / f"{APP_NAME}-{version()}-setup.exe"
    if setup.exists():
        shutil.copy2(setup, RELEASE / setup.name)
        artifacts.append(RELEASE / setup.name)
    portable = RELEASE / f"{APP_NAME}-{version()}-portable.zip"
    if portable.exists():
        artifacts.append(portable)
    if not artifacts:
        return []

    lines = [f"{APP_NAME} {version()} SHA-256", ""]
    for path in artifacts:
        lines.append(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}")
    checksums = RELEASE / "SHA256SUMS.txt"
    checksums.write_text("\n".join(lines) + "\n", encoding="utf-8")

    print("\n== 배포 산출물 ==")
    for path in artifacts:
        print(f"  {path.name:42} {path.stat().st_size / 1024 / 1024:6.1f} MB")
    print(f"  {checksums.name:42} {checksums.stat().st_size:6d} bytes")
    return artifacts


def clean() -> None:
    for path in (DIST, RELEASE, INSTALLER_OUT, ROOT / "build"):
        if path.exists():
            shutil.rmtree(path)
            print(f"삭제: {path}")


def main() -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")

    parser = argparse.ArgumentParser(description=f"{APP_NAME} 빌드")
    parser.add_argument("--installer", action="store_true", help="Inno Setup 설치 파일까지")
    parser.add_argument("--release", action="store_true",
                        help="처음부터 다시 빌드해 설치 파일 + 포터블 ZIP + 체크섬")
    parser.add_argument("--clean", action="store_true", help="산출물 정리")
    parser.add_argument("--skip-engine", action="store_true", help="7-Zip 엔진 확인 건너뛰기")
    args = parser.parse_args()

    if args.clean:
        clean()
        return 0
    if args.release:
        clean()

    print(f"{APP_NAME} {version()}")
    if not args.skip_engine:
        fetch_engine()
    if not build_exe() or not smoke_test():
        return 1
    if (args.installer or args.release) and not build_installer():
        return 1
    if args.release:
        if make_portable() is None or not collect_release():
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
