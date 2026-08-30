"""RAR/7Z 해제 엔진(7-Zip) 준비.

MyZIP 은 ZIP/TAR 을 파이썬 표준 라이브러리로 직접 처리하지만, RAR 은
순수 파이썬 구현이 존재하지 않아 외부 엔진이 필요하다. 7-Zip 의 7z.dll 이
RAR4/RAR5 해제 코덱을 갖고 있고 재배포도 허용된다.

  주의: unRAR 라이선스는 그 코드를 RAR '압축기' 제작에 쓰는 것을 금지한다.
  그래서 MyZIP 도, 7-Zip 도, 반디집도 RAR 은 해제만 지원한다.

준비 순서
  1. 이미 bin/ 에 있으면 그대로 둔다.
  2. 시스템에 설치된 7-Zip 이 있으면 거기서 복사한다.
  3. 공식 MSI 를 받아 '관리 설치'로 파일만 뽑아낸다.
     (msiexec /a 는 시스템에 설치하지 않고 내용물만 펼친다.)
"""

from __future__ import annotations

import hashlib
import shutil
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "bin"

VERSION = "2409"
MSI_URL = f"https://www.7-zip.org/a/7z{VERSION}-x64.msi"

NEEDED = ("7z.exe", "7z.dll")


def already_present() -> bool:
    return all((BIN / name).exists() for name in NEEDED)


def report() -> None:
    for name in NEEDED:
        path = BIN / name
        print(f"  bin/{name}  ({path.stat().st_size:,} bytes)")


def copy_from_installed() -> bool:
    """시스템에 설치된 7-Zip 에서 가져온다."""
    sys.path.insert(0, str(ROOT))
    from myzip.core.sevenzip import find_7z

    exe = find_7z(refresh=True)
    if exe is None or not exe.name.lower() == "7z.exe":
        return False

    dll = exe.with_name("7z.dll")
    if not dll.exists():
        return False

    BIN.mkdir(parents=True, exist_ok=True)
    print(f"설치된 7-Zip 에서 복사: {exe.parent}")
    shutil.copy2(exe, BIN / "7z.exe")
    shutil.copy2(dll, BIN / "7z.dll")
    return True


def download(url: str, dest: Path) -> Path:
    print(f"내려받는 중: {url}")
    request = urllib.request.Request(url, headers={"User-Agent": "MyZIP-setup/1.0"})
    with urllib.request.urlopen(request, timeout=180) as response:
        data = response.read()
    dest.write_bytes(data)
    print(f"  {len(data):,} bytes  sha256={hashlib.sha256(data).hexdigest()[:16]}...")
    return dest


def extract_msi(msi: Path, target: Path) -> bool:
    """msiexec 관리 설치로 MSI 안의 파일만 펼친다.

    /a 는 네트워크 배포용 '관리 설치' 모드다. 레지스트리를 건드리거나
    프로그램 목록에 등록하지 않고 TARGETDIR 아래에 파일만 푼다.
    """
    print("MSI 에서 파일 추출 중 (시스템에 설치하지 않음)...")
    result = subprocess.run(
        ["msiexec", "/a", str(msi), "/qn", f"TARGETDIR={target}"],
        capture_output=True, text=True, timeout=300,
    )
    if result.returncode != 0:
        print(f"  msiexec 실패 (코드 {result.returncode})")
        return False
    return True


def install_from(staging: Path) -> bool:
    """펼쳐진 폴더에서 필요한 파일만 bin/ 으로 옮긴다."""
    BIN.mkdir(parents=True, exist_ok=True)
    found = 0
    for name in NEEDED:
        matches = sorted(staging.rglob(name))
        if not matches:
            print(f"  찾지 못함: {name}")
            continue
        shutil.copy2(matches[0], BIN / name)
        found += 1
    return found == len(NEEDED)


def main() -> int:
    if already_present():
        print(f"이미 준비되어 있습니다: {BIN}")
        report()
        return 0

    if copy_from_installed():
        report()
        return 0

    try:
        with tempfile.TemporaryDirectory(prefix="myzip-7z-") as tmp:
            tmp_path = Path(tmp)
            msi = download(MSI_URL, tmp_path / "7zip.msi")
            staging = tmp_path / "extracted"
            staging.mkdir()
            if extract_msi(msi, staging) and install_from(staging):
                report()
                return 0
    except Exception as exc:
        print(f"자동 준비 실패: {type(exc).__name__}: {exc}")

    print()
    print("=" * 62)
    print("RAR/7Z 엔진을 자동으로 준비하지 못했습니다.")
    print()
    print("수동 해결 (둘 중 하나):")
    print("  a) https://www.7-zip.org 에서 7-Zip 을 설치하면 MyZIP 이 자동으로 찾습니다.")
    print(f"  b) 7-Zip 설치 폴더의 7z.exe 와 7z.dll 을 다음 위치에 복사:")
    print(f"     {BIN}")
    print()
    print("이 엔진이 없어도 ZIP / TAR / TGZ 는 완전히 정상 동작합니다.")
    print("RAR 과 7Z 파일만 열 수 없습니다.")
    print("=" * 62)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
