"""MyZIP 빌드 스크립트.

    python build.py            아이콘 생성 + exe 빌드
    python build.py --installer  위 + Inno Setup 설치 파일까지
    python build.py --clean      빌드 산출물 정리

PyInstaller 의 onedir 모드를 쓴다. onefile 은 실행할 때마다 임시 폴더로
전체를 풀기 때문에 시작이 1초 이상 느려지는데, 압축 프로그램은 탐색기에서
더블클릭했을 때 즉시 떠야 하므로 그 대가가 너무 크다.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DIST = ROOT / "dist"
BUILD = ROOT / "build"
RELEASE = ROOT / "release"
APP_NAME = "MyZIP"

# 포터블 ZIP 을 만들 때 MyZIP 자신을 import 한다. 어느 폴더에서 실행하든
# 프로젝트 루트를 찾을 수 있도록 미리 넣어 둔다.
sys.path.insert(0, str(ROOT))


def run(cmd: list[str], **kwargs) -> int:
    print(f"$ {' '.join(cmd)}")
    return subprocess.run(cmd, cwd=str(ROOT), **kwargs).returncode


def ensure_tools() -> bool:
    try:
        import PyInstaller  # noqa: F401
    except ImportError:
        print("PyInstaller 가 없습니다. 설치합니다...")
        if run([sys.executable, "-m", "pip", "install", "pyinstaller"]) != 0:
            print("PyInstaller 설치에 실패했습니다.")
            return False
    return True


def make_icons() -> None:
    print("\n== 아이콘 생성 ==")
    run([sys.executable, "tools/make_icons.py"])
    run([sys.executable, "tools/make_action_icons.py"])


def fetch_engine() -> None:
    print("\n== RAR/7Z 엔진 확인 ==")
    code = run([sys.executable, "tools/fetch_7zip.py"])
    if code != 0:
        print("경고: RAR/7Z 엔진 없이 빌드합니다. ZIP/TAR 은 정상 동작합니다.")


def build_exe() -> bool:
    print("\n== 실행 파일 빌드 ==")

    icon = ROOT / "resources" / "icons" / "app.ico"
    separator = ";" if sys.platform == "win32" else ":"

    args = [
        sys.executable, "-m", "PyInstaller",
        "--noconfirm",
        "--clean",
        "--windowed",                      # 콘솔 창 없이
        "--name", APP_NAME,
        "--icon", str(icon),
        "--add-data", f"resources{separator}resources",
        # Qt 에서 실제로 쓰지 않는 무거운 모듈을 걷어낸다.
        "--exclude-module", "PySide6.QtWebEngineCore",
        "--exclude-module", "PySide6.QtWebEngineWidgets",
        "--exclude-module", "PySide6.Qt3DCore",
        "--exclude-module", "PySide6.QtMultimedia",
        "--exclude-module", "PySide6.QtQuick",
        "--exclude-module", "PySide6.QtQml",
        "--exclude-module", "PySide6.QtCharts",
        "--exclude-module", "PySide6.QtDataVisualization",
        "--exclude-module", "tkinter",
        "--exclude-module", "unittest",
        "--exclude-module", "pydoc",
        "myzip_app.py",
    ]

    bin_dir = ROOT / "bin"
    if (bin_dir / "7z.exe").exists():
        args.extend(["--add-binary", f"bin/7z.exe{separator}bin"])
        args.extend(["--add-binary", f"bin/7z.dll{separator}bin"])

    if run(args) != 0:
        return False

    target = DIST / APP_NAME / f"{APP_NAME}.exe"
    if not target.exists():
        print(f"실행 파일이 만들어지지 않았습니다: {target}")
        return False

    before = folder_size(DIST / APP_NAME)
    prune()
    after = folder_size(DIST / APP_NAME)

    print(f"\n빌드 완료: {target}")
    print(f"배포 폴더 크기: {after / 1024 / 1024:.1f} MB "
          f"({(before - after) / 1024 / 1024:.1f} MB 덜어냄)")
    return True


def folder_size(path: Path) -> int:
    return sum(f.stat().st_size for f in path.rglob("*") if f.is_file())


# PySide6 가 무조건 끌고 오지만 위젯 전용 앱에는 쓰이지 않는 것들.
# --exclude-module 로는 DLL 이 걸러지지 않아 빌드 후에 직접 지운다.
PRUNE_FILES = (
    # 소프트웨어 OpenGL 폴백 (20MB). 위젯은 래스터 엔진으로 그린다.
    "PySide6/opengl32sw.dll",
    # QML / Quick 계열 — 우리는 위젯만 쓴다
    "PySide6/Qt6Qml.dll",
    "PySide6/Qt6QmlModels.dll",
    "PySide6/Qt6QmlMeta.dll",
    "PySide6/Qt6QmlWorkerScript.dll",
    "PySide6/Qt6Quick.dll",
    "PySide6/Qt6QuickControls2.dll",
    "PySide6/Qt6QuickTemplates2.dll",
    "PySide6/Qt6QuickWidgets.dll",
    # PDF 렌더러
    "PySide6/Qt6Pdf.dll",
    "PySide6/Qt6PdfWidgets.dll",
    # Direct3D 셰이더 컴파일러 (QML 전용)
    "PySide6/d3dcompiler_47.dll",
    "PySide6/Qt6ShaderTools.dll",
    # 가상 키보드, 원격 데스크톱 등
    "PySide6/Qt6VirtualKeyboard.dll",
)

PRUNE_DIRS = (
    "PySide6/qml",
    "PySide6/plugins/qmltooling",
    "PySide6/plugins/virtualkeyboard",
    "PySide6/plugins/sqldrivers",
    "PySide6/plugins/multimedia",
    "PySide6/plugins/canbus",
    "PySide6/plugins/position",
    "PySide6/plugins/renderers",
    "PySide6/plugins/sceneparsers",
    "PySide6/plugins/geometryloaders",
)


def prune() -> None:
    """배포 폴더에서 쓰지 않는 Qt 구성요소를 덜어낸다."""
    root = DIST / APP_NAME / "_internal"
    if not root.exists():
        root = DIST / APP_NAME

    removed = 0
    for relative in PRUNE_FILES:
        path = root / relative
        if path.exists():
            removed += path.stat().st_size
            path.unlink()

    for relative in PRUNE_DIRS:
        path = root / relative
        if path.is_dir():
            removed += folder_size(path)
            shutil.rmtree(path)

    if removed:
        print(f"불필요한 Qt 구성요소 {removed / 1024 / 1024:.1f} MB 제거")


def find_inno() -> Path | None:
    """ISCC.exe(Inno Setup 컴파일러)를 찾는다.

    winget 으로 설치하면 관리자 권한 없이 사용자 폴더에 들어가므로
    Program Files 만 봐서는 못 찾는다.
    """
    import os

    roots = [
        os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)"),
        os.environ.get("ProgramFiles", r"C:\Program Files"),
        str(Path(os.environ.get("LOCALAPPDATA", "")) / "Programs"),
    ]
    for root in roots:
        if not root:
            continue
        for version in ("Inno Setup 6", "Inno Setup 7", "Inno Setup 5"):
            path = Path(root) / version / "ISCC.exe"
            if path.exists():
                return path

    from shutil import which

    found = which("ISCC.exe")
    return Path(found) if found else None


def build_installer() -> bool:
    print("\n== 설치 파일 빌드 ==")
    iscc = find_inno()
    if iscc is None:
        print("Inno Setup 6 을 찾지 못했습니다.")
        print("  https://jrsoftware.org/isdl.php 에서 설치한 뒤 다시 실행하세요.")
        print("  (설치 파일 없이도 dist/MyZIP 폴더를 그대로 배포할 수 있습니다.)")
        return False

    script = ROOT / "installer" / "myzip.iss"
    if run([str(iscc), str(script)]) != 0:
        return False

    outputs = sorted((ROOT / "installer" / "output").glob("*.exe"))
    for path in outputs:
        print(f"설치 파일: {path}  ({path.stat().st_size / 1024 / 1024:.1f} MB)")
    return bool(outputs)


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
    """배포 폴더를 포터블 ZIP 으로 묶는다.

    묶는 데 MyZIP 자신의 코드를 쓴다. 릴리스마다 압축 경로를 실제
    데이터로 한 번 더 검증하는 셈이다.
    """
    print("\n== 포터블 ZIP 만들기 ==")
    source = DIST / APP_NAME
    if not source.is_dir():
        print(f"배포 폴더가 없습니다: {source}")
        return None

    sys.path.insert(0, str(ROOT))
    from myzip import __version__
    from myzip.core import Progress, walk_inputs, writer_for

    RELEASE.mkdir(parents=True, exist_ok=True)

    # 사용법 안내를 폴더에 넣어 함께 묶는다
    note = source / "사용법.txt"
    note.write_text(PORTABLE_NOTE.format(version=__version__), encoding="utf-8")

    target = RELEASE / f"{APP_NAME}-{__version__}-portable.zip"
    target.unlink(missing_ok=True)

    items = list(walk_inputs([source]))
    with writer_for(target, level=9) as writer:
        writer.write(items, Progress())

    note.unlink(missing_ok=True)
    print(f"포터블: {target}  ({target.stat().st_size / 1024 / 1024:.1f} MB)")
    return target


def collect_release() -> list[Path]:
    """산출물을 release/ 로 모으고 체크섬을 적는다."""
    import hashlib

    from myzip import __version__

    RELEASE.mkdir(parents=True, exist_ok=True)

    artifacts: list[Path] = []

    setup = ROOT / "installer" / "output" / f"{APP_NAME}-{__version__}-setup.exe"
    if setup.exists():
        destination = RELEASE / setup.name
        if setup.resolve() != destination.resolve():
            shutil.copy2(setup, destination)
        artifacts.append(destination)

    portable = RELEASE / f"{APP_NAME}-{__version__}-portable.zip"
    if portable.exists():
        artifacts.append(portable)

    if not artifacts:
        return []

    lines = [f"{APP_NAME} {__version__} SHA-256", ""]
    for path in artifacts:
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        lines.append(f"{digest}  {path.name}")
    checksums = RELEASE / "SHA256SUMS.txt"
    checksums.write_text("\n".join(lines) + "\n", encoding="utf-8")

    print("\n== 배포 산출물 ==")
    for path in artifacts:
        print(f"  {path.name:42} {path.stat().st_size / 1024 / 1024:6.1f} MB")
    print(f"  {checksums.name:42} {checksums.stat().st_size:6d} bytes")
    return artifacts


def clean() -> None:
    for path in (DIST, BUILD, RELEASE, ROOT / "installer" / "output"):
        if path.exists():
            shutil.rmtree(path)
            print(f"삭제: {path}")
    for spec in ROOT.glob("*.spec"):
        spec.unlink()
        print(f"삭제: {spec}")


def main() -> int:
    parser = argparse.ArgumentParser(description=f"{APP_NAME} 빌드")
    parser.add_argument("--installer", action="store_true",
                        help="Inno Setup 설치 파일까지 만들기")
    parser.add_argument("--release", action="store_true",
                        help="처음부터 다시 빌드해 설치본 + 포터블 ZIP + 체크섬 생성")
    parser.add_argument("--clean", action="store_true", help="산출물 정리")
    parser.add_argument("--skip-icons", action="store_true",
                        help="아이콘 재생성 건너뛰기")
    parser.add_argument("--skip-engine", action="store_true",
                        help="RAR 엔진 확인 건너뛰기")
    args = parser.parse_args()

    if args.clean:
        clean()
        return 0

    if not ensure_tools():
        return 1

    if args.release:
        # 배포본은 남은 찌꺼기 없이 처음부터 만든다.
        clean()

    if not args.skip_icons:
        make_icons()
    if not args.skip_engine:
        fetch_engine()

    if not build_exe():
        return 1

    if (args.installer or args.release) and not build_installer():
        return 1

    if args.release:
        make_portable()
        if not collect_release():
            print("배포 산출물을 모으지 못했습니다.")
            return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
