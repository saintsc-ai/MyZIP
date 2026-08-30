"""사용자 설정 저장.

Windows 레지스트리(HKCU\\Software\\MyZIP)에 QSettings 로 저장한다.
설치 폴더에 파일을 쓰지 않으므로 Program Files 에 설치해도 문제없다.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from PySide6.QtCore import QSettings

from . import APP_NAME

ORG = "MyZIP"

DEFAULTS: dict[str, Any] = {
    # 압축
    "compress/format": "zip",
    "compress/level": 6,
    "compress/last_dir": "",
    # 해제
    "extract/mode": "subfolder",     # here | subfolder | ask
    "extract/conflict": "rename",    # overwrite | skip | rename
    "extract/last_dir": "",
    "extract/open_after": True,
    "extract/strip_redundant": True,
    # 보기
    "view/encoding": "auto",
    "view/columns": "",
    "view/geometry": "",
    "view/splitter": "",
    "view/show_tree": True,
    # 동작
    "general/confirm_close": False,
    "general/theme": "auto",         # auto | light | dark
}


def settings() -> QSettings:
    return QSettings(QSettings.NativeFormat, QSettings.UserScope, ORG, APP_NAME)


def get(key: str, default: Any = None) -> Any:
    s = settings()
    fallback = DEFAULTS.get(key, default)
    value = s.value(key, fallback)

    # QSettings 는 레지스트리에서 값을 문자열로 돌려주기도 한다.
    # 기본값의 타입에 맞춰 되돌린다.
    if isinstance(fallback, bool):
        if isinstance(value, str):
            return value.lower() in ("true", "1", "yes")
        return bool(value)
    if isinstance(fallback, int) and not isinstance(fallback, bool):
        try:
            return int(value)
        except (TypeError, ValueError):
            return fallback
    return value


def put(key: str, value: Any) -> None:
    settings().setValue(key, value)


def last_dir(kind: str) -> Path:
    """마지막으로 쓴 폴더. 없으면 문서 폴더."""
    raw = get(f"{kind}/last_dir", "")
    if raw:
        path = Path(raw)
        if path.is_dir():
            return path
    return Path.home() / "Documents"


def remember_dir(kind: str, path: Path | str) -> None:
    path = Path(path)
    if path.is_file():
        path = path.parent
    if path.is_dir():
        put(f"{kind}/last_dir", str(path))
