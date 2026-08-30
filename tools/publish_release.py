"""GitHub 릴리스 생성 및 배포 파일 업로드.

토큰은 Git 자격증명 관리자에서 꺼내 쓴다. 별도로 저장하지 않는다.

    python tools/publish_release.py             릴리스 생성 + 자산 업로드
    python tools/publish_release.py --dry-run   무엇을 할지만 출력
"""

from __future__ import annotations

import argparse
import json
import mimetypes
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))

REPO = "saintsc-ai/MyZIP"
API = "https://api.github.com"
UPLOADS = "https://uploads.github.com"


def get_token() -> str:
    """Git 자격증명 관리자에서 github.com 토큰을 꺼낸다."""
    proc = subprocess.run(
        ["git", "credential", "fill"],
        input="protocol=https\nhost=github.com\n\n",
        capture_output=True, text=True, timeout=60,
    )
    for line in proc.stdout.splitlines():
        if line.startswith("password="):
            return line.split("=", 1)[1]
    raise SystemExit(
        "GitHub 토큰을 찾지 못했습니다.\n"
        "한 번이라도 git push 를 해서 자격증명이 저장되어 있어야 합니다."
    )


def request(method: str, url: str, token: str, data=None,
            content_type: str | None = None):
    headers = {
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "MyZIP-release",
    }
    body = None
    if isinstance(data, (bytes, bytearray)):
        body = data
        headers["Content-Type"] = content_type or "application/octet-stream"
    elif data is not None:
        body = json.dumps(data).encode("utf-8")
        headers["Content-Type"] = "application/json"

    req = urllib.request.Request(url, data=body, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=600) as response:
            raw = response.read()
            return json.loads(raw) if raw else {}
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode("utf-8", "replace")[:500]
        raise SystemExit(f"{method} {url}\nHTTP {exc.code}: {detail}") from exc


def release_notes(version: str, artifacts: list[Path]) -> str:
    checksums = ROOT / "release" / "SHA256SUMS.txt"
    sums = ""
    if checksums.exists():
        lines = [
            line for line in checksums.read_text(encoding="utf-8").splitlines()
            if line.strip() and not line.startswith("MyZIP")
        ]
        sums = "\n".join(lines)

    return f"""반디집 / KingZip 스타일의 가벼운 Windows 압축 프로그램입니다.

## 받을 파일

| 파일 | 설명 |
| --- | --- |
| `MyZIP-{version}-setup.exe` | **설치 마법사** — 관리자 권한 없이 설치됩니다 (UAC 창 없음) |
| `MyZIP-{version}-portable.zip` | **포터블** — 압축만 풀면 바로 실행 |

포터블은 탐색기 우클릭 메뉴를 쓰려면 한 번만 `MyZIP.exe --register` 를 실행하세요.

## 지원 형식

| 형식 | 압축 | 해제 |
| --- | :---: | :---: |
| ZIP (AES-256 암호) | O | O |
| TAR / TAR.GZ / TGZ / TAR.BZ2 / TAR.XZ | O | O |
| GZ / BZ2 / XZ (단일 파일) | — | O |
| RAR (RAR4 / RAR5) | X | O |
| 7Z / CAB / ARJ / LZH / ISO | X | O |

RAR 압축은 지원하지 않습니다. RARLAB 의 독점 형식이고 unRAR 라이선스가
RAR 압축기 제작에 그 코드를 쓰는 것을 금지하기 때문입니다.
7-Zip 과 반디집도 같은 제약을 받습니다.

## 주요 기능

- **탐색기 우클릭 메뉴** — 압축하기 / 여기에 압축 풀기 / 폴더 생성 후 풀기 / 무결성 검사.
  기본 연결 프로그램이 MyZIP 이 아니어도 나옵니다.
- **한글 파일명 자동 복원** — UTF-8 / CP949 / CP932 / GBK 를 아카이브 단위로 판별합니다.
  ZIP 규격에는 파일명 인코딩을 적는 자리가 없어서, 예전 한국·일본 프로그램이
  만든 아카이브는 이름이 깨집니다. 깨져 보이면 툴바에서 직접 고를 수도 있습니다.
- **풀지 않고 들여다보기** — 폴더 트리로 안을 돌아다니고, 파일을 더블클릭하면
  임시 폴더로 꺼내 연결된 프로그램으로 엽니다.
- **경로 탈출 차단** — 아카이브 안의 `../../windows/system32` 같은 악성 경로를
  무력화합니다.
- **깨끗한 제거** — 레지스트리는 `HKEY_CURRENT_USER` 에만 씁니다.
  제거하면 확장자 연결이 이전 프로그램으로 되돌아갑니다.

## 알아두실 점

**Windows 11 에서는** 우클릭 메뉴가 **'추가 옵션 표시'**(`Shift`+`F10`) 안에
나타납니다. 1차 메뉴에 넣으려면 MSIX 패키지와 서명된 COM 확장이 필요한데,
이 프로젝트는 그 복잡도를 택하지 않았습니다.

**확장자 연결이 안 바뀌면** 다른 압축 프로그램이 Windows 기본 앱으로 고정된
상태입니다. Windows 8 부터 프로그램이 이를 임의로 바꿀 수 없게 막혀 있습니다.
설정 화면이 이 상태를 감지해 알려 주고 기본 앱 설정으로 안내합니다.

**코드 서명이 되어 있지 않아** 처음 실행할 때 SmartScreen 경고가 뜹니다.
'추가 정보' → '실행'을 눌러 주세요.

## 무결성 확인

```
{sums}
```

PowerShell 에서 확인:

```powershell
Get-FileHash .\\MyZIP-{version}-setup.exe -Algorithm SHA256
```

---

전체 사용법은 [docs/사용법.md](https://github.com/{REPO}/blob/main/docs/%EC%82%AC%EC%9A%A9%EB%B2%95.md) 를 보세요.
"""


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--version", default=None)
    args = parser.parse_args()

    from myzip import __version__

    version = args.version or __version__
    tag = f"v{version}"

    release_dir = ROOT / "release"
    artifacts = [
        release_dir / f"MyZIP-{version}-setup.exe",
        release_dir / f"MyZIP-{version}-portable.zip",
        release_dir / "SHA256SUMS.txt",
    ]
    missing = [p for p in artifacts if not p.exists()]
    if missing:
        raise SystemExit(
            "배포 파일이 없습니다:\n  "
            + "\n  ".join(str(p) for p in missing)
            + "\n\n먼저 python build.py --release 를 실행하세요."
        )

    notes = release_notes(version, artifacts)

    if args.dry_run:
        print(f"태그: {tag}")
        print("자산:")
        for path in artifacts:
            print(f"  {path.name}  ({path.stat().st_size / 1024 / 1024:.1f} MB)")
        print("\n--- 릴리스 노트 ---")
        print(notes)
        return 0

    token = get_token()

    # 같은 태그의 릴리스가 이미 있으면 지우고 다시 만든다.
    try:
        existing = request("GET", f"{API}/repos/{REPO}/releases/tags/{tag}", token)
        print(f"기존 릴리스 발견 (id={existing['id']}) — 지우고 다시 만듭니다.")
        request("DELETE", f"{API}/repos/{REPO}/releases/{existing['id']}", token)
    except SystemExit as exc:
        if "HTTP 404" not in str(exc):
            raise

    release = request("POST", f"{API}/repos/{REPO}/releases", token, {
        "tag_name": tag,
        "target_commitish": "main",
        "name": f"MyZIP {version}",
        "body": notes,
        "draft": False,
        "prerelease": False,
    })
    print(f"릴리스 생성: {release['html_url']}")

    for path in artifacts:
        size = path.stat().st_size
        ctype = mimetypes.guess_type(path.name)[0] or "application/octet-stream"
        print(f"  업로드 중: {path.name} ({size / 1024 / 1024:.1f} MB)...", flush=True)
        asset = request(
            "POST",
            f"{UPLOADS}/repos/{REPO}/releases/{release['id']}/assets?name={path.name}",
            token, path.read_bytes(), ctype,
        )
        print(f"    완료: {asset['browser_download_url']}")

    print(f"\n릴리스 주소: {release['html_url']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
