"""UI 공통 부품: 아이콘 로딩, 스타일, 자잘한 헬퍼."""

from __future__ import annotations

import sys
from datetime import datetime
from functools import lru_cache
from pathlib import Path

from PySide6.QtCore import QSize, Qt
from PySide6.QtGui import QColor, QIcon, QPainter, QPalette, QPixmap
from PySide6.QtWidgets import QApplication, QMessageBox, QStyle, QWidget

from .. import APP_NAME


def resource_dir() -> Path:
    """번들된 리소스 폴더. PyInstaller onefile 도 고려한다."""
    if getattr(sys, "frozen", False):
        base = Path(getattr(sys, "_MEIPASS", Path(sys.executable).parent))
    else:
        base = Path(__file__).resolve().parent.parent.parent
    return base / "resources"


@lru_cache(maxsize=64)
def app_icon(name: str = "app") -> QIcon:
    """resources/icons 에서 아이콘을 읽는다."""
    path = resource_dir() / "icons" / f"{name}.ico"
    if path.exists():
        return QIcon(str(path))
    png = resource_dir() / "icons" / f"{name}.png"
    if png.exists():
        return QIcon(str(png))
    return QIcon()


# 확장자 -> 우리가 가진 아이콘 이름
_ICON_BY_EXT = {
    ".zip": "zip", ".tar": "tar", ".tgz": "tgz", ".gz": "tgz",
    ".bz2": "tgz", ".xz": "tgz", ".tbz2": "tgz", ".txz": "tgz",
    ".rar": "rar", ".7z": "7z",
}


def icon_for_name(name: str, is_dir: bool = False) -> QIcon:
    """파일 이름에 맞는 아이콘. 시스템 아이콘을 최대한 활용한다."""
    if is_dir:
        return _dir_icon()
    return _icon_for_suffix(Path(name).suffix.lower())


@lru_cache(maxsize=1)
def _dir_icon() -> QIcon:
    return QApplication.style().standardIcon(QStyle.SP_DirIcon)


@lru_cache(maxsize=256)
def _icon_for_suffix(ext: str) -> QIcon:
    """확장자 단위로 캐시한다.

    파일 이름 단위로 캐시하면 항목이 수만 개인 아카이브에서 캐시가 계속
    밀려나 목록을 그릴 때마다 시스템 아이콘을 다시 조회하게 된다.
    아이콘은 어차피 확장자로만 정해지므로 확장자를 키로 쓴다.
    """
    if ext in _ICON_BY_EXT:
        return app_icon(_ICON_BY_EXT[ext])

    # 나머지는 Windows 가 등록해 둔 파일 형식 아이콘을 빌려 쓴다.
    from PySide6.QtCore import QFileInfo

    icon = _icon_provider().icon(QFileInfo(f"dummy{ext}"))
    if not icon.isNull() and icon.availableSizes():
        return icon
    return QApplication.style().standardIcon(QStyle.SP_FileIcon)


@lru_cache(maxsize=1)
def _icon_provider():
    from PySide6.QtWidgets import QFileIconProvider

    return QFileIconProvider()


def format_time(dt: datetime | None) -> str:
    if dt is None:
        return ""
    return dt.strftime("%Y-%m-%d %H:%M")


def elide(text: str, limit: int = 60) -> str:
    """긴 경로를 가운데를 줄여 보여준다."""
    if len(text) <= limit:
        return text
    head = limit // 2 - 2
    return f"{text[:head]}...{text[-(limit - head - 3):]}"


def is_dark_mode() -> bool:
    palette = QApplication.palette()
    return palette.color(QPalette.Window).lightness() < 128


def warn(parent: QWidget | None, title: str, text: str,
         detail: str = "") -> None:
    box = QMessageBox(QMessageBox.Warning, title, text, QMessageBox.Ok, parent)
    box.setWindowIcon(app_icon())
    if detail:
        box.setDetailedText(detail)
    box.exec()


def error(parent: QWidget | None, text: str, detail: str = "") -> None:
    box = QMessageBox(QMessageBox.Critical, f"{APP_NAME} - 오류", text,
                      QMessageBox.Ok, parent)
    box.setWindowIcon(app_icon())
    if detail:
        box.setDetailedText(detail)
    box.exec()


def info(parent: QWidget | None, title: str, text: str, detail: str = "") -> None:
    box = QMessageBox(QMessageBox.Information, title, text, QMessageBox.Ok, parent)
    box.setWindowIcon(app_icon())
    if detail:
        box.setDetailedText(detail)
    box.exec()


def confirm(parent: QWidget | None, title: str, text: str,
            ok_text: str = "확인") -> bool:
    box = QMessageBox(QMessageBox.Question, title, text,
                      QMessageBox.NoButton, parent)
    box.setWindowIcon(app_icon())
    ok = box.addButton(ok_text, QMessageBox.AcceptRole)
    box.addButton("취소", QMessageBox.RejectRole)
    box.setDefaultButton(ok)
    box.exec()
    return box.clickedButton() is ok


def open_in_explorer(path: Path) -> None:
    """탐색기에서 해당 항목을 선택한 채로 연다.

    MYZIP_NO_SHELL 환경변수가 설정되어 있으면 아무것도 하지 않는다.
    자동 테스트가 탐색기 창을 잔뜩 띄우는 것을 막기 위한 장치다.
    """
    import os
    import subprocess

    if os.environ.get("MYZIP_NO_SHELL"):
        return

    path = Path(path)
    flags = 0x08000000 if sys.platform == "win32" else 0
    if path.is_dir():
        subprocess.Popen(["explorer", str(path)], creationflags=flags)
    else:
        subprocess.Popen(["explorer", "/select,", str(path)], creationflags=flags)


def shell_open(path: Path) -> None:
    """연결된 프로그램으로 파일을 연다."""
    import os

    try:
        os.startfile(str(path))  # noqa: S606 - Windows 표준 방식
    except OSError as exc:
        raise RuntimeError(f"파일을 열 수 없습니다: {exc}") from exc


APP_STYLE = """
QMainWindow, QDialog { background: palette(window); }

QToolBar {
    border: none;
    padding: 4px 6px;
    spacing: 2px;
}
QToolBar QToolButton {
    padding: 6px 10px;
    border-radius: 6px;
    min-width: 56px;
}
QToolBar QToolButton:hover   { background: rgba(128,128,128,0.18); }
QToolBar QToolButton:pressed { background: rgba(128,128,128,0.30); }
QToolBar QToolButton:disabled { color: palette(mid); }

QTreeView, QTableView, QListView {
    border: 1px solid rgba(128,128,128,0.28);
    border-radius: 6px;
    alternate-background-color: rgba(128,128,128,0.06);
    selection-background-color: palette(highlight);
}
QTreeView::item, QTableView::item { padding: 3px 2px; }

QHeaderView::section {
    padding: 5px 8px;
    border: none;
    border-right: 1px solid rgba(128,128,128,0.22);
    border-bottom: 1px solid rgba(128,128,128,0.28);
    background: rgba(128,128,128,0.10);
}

QLineEdit, QComboBox, QSpinBox {
    padding: 5px 8px;
    border: 1px solid rgba(128,128,128,0.35);
    border-radius: 6px;
    min-height: 20px;
}
QLineEdit:focus, QComboBox:focus, QSpinBox:focus {
    border-color: palette(highlight);
}

QPushButton {
    padding: 6px 16px;
    border: 1px solid rgba(128,128,128,0.35);
    border-radius: 6px;
    min-width: 76px;
    min-height: 22px;
}
QPushButton:hover    { background: rgba(128,128,128,0.14); }
QPushButton:pressed  { background: rgba(128,128,128,0.26); }
QPushButton:default  {
    border-color: palette(highlight);
    font-weight: 600;
}
QPushButton:disabled { color: palette(mid); }

QProgressBar {
    border: 1px solid rgba(128,128,128,0.35);
    border-radius: 6px;
    text-align: center;
    min-height: 20px;
}
QProgressBar::chunk { border-radius: 5px; background: palette(highlight); }

QGroupBox {
    border: 1px solid rgba(128,128,128,0.30);
    border-radius: 8px;
    margin-top: 10px;
    padding-top: 8px;
    font-weight: 600;
}
QGroupBox::title {
    subcontrol-origin: margin;
    left: 10px;
    padding: 0 5px;
}

QStatusBar { border-top: 1px solid rgba(128,128,128,0.25); }
QStatusBar::item { border: none; }
"""
