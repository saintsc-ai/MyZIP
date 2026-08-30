"""압축 풀기 대화상자."""

from __future__ import annotations

from pathlib import Path

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QButtonGroup,
    QCheckBox,
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFileDialog,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPushButton,
    QRadioButton,
    QVBoxLayout,
    QWidget,
)

from .. import APP_NAME
from ..config import get, last_dir, put, remember_dir
from ..core import strip_archive_suffix
from .common import app_icon, warn

CONFLICT_CHOICES = (
    ("overwrite", "덮어쓰기"),
    ("skip", "건너뛰기"),
    ("rename", "다른 이름으로 저장"),
)


class ExtractDialog(QDialog):
    """어디에, 어떻게 풀지 정한다."""

    def __init__(self, archive: Path, selection_count: int = 0,
                 parent: QWidget | None = None):
        super().__init__(parent)
        self.archive = Path(archive)
        self.selection_count = selection_count

        self.setWindowTitle(f"{APP_NAME} - 압축 풀기")
        self.setWindowIcon(app_icon())
        self.setMinimumWidth(540)

        self._build()
        self._restore()
        self._update_preview()

    def _build(self) -> None:
        header = QLabel(f"<b>{self.archive.name}</b>")
        header.setTextFormat(Qt.RichText)

        # 위치
        self.dest = QLineEdit()
        self.dest.textChanged.connect(self._update_preview)
        browse = QPushButton("찾아보기...")
        browse.setMaximumWidth(96)
        browse.clicked.connect(self._browse)

        dest_row = QHBoxLayout()
        dest_row.setSpacing(6)
        dest_row.addWidget(self.dest, 1)
        dest_row.addWidget(browse)

        # 폴더 구성 방식
        self.mode_here = QRadioButton("선택한 폴더에 그대로 풀기")
        self.mode_sub = QRadioButton("아카이브 이름으로 폴더를 만들어 풀기")
        self.mode_group = QButtonGroup(self)
        self.mode_group.addButton(self.mode_here, 0)
        self.mode_group.addButton(self.mode_sub, 1)
        self.mode_group.idToggled.connect(lambda *_: self._update_preview())

        self.strip = QCheckBox("아카이브 안이 폴더 하나뿐이면 중복 폴더 만들지 않기")
        self.strip.setToolTip(
            "예: 자료.zip 안에 '자료' 폴더 하나만 있을 때\n"
            "자료/자료/... 처럼 두 겹이 되는 것을 막습니다."
        )
        self.strip.toggled.connect(self._update_preview)

        self._preview = QLabel()
        self._preview.setWordWrap(True)
        self._preview.setStyleSheet("color: palette(mid);")

        location_box = QGroupBox("풀어낼 위치")
        location_layout = QVBoxLayout(location_box)
        location_layout.setSpacing(8)
        location_layout.addLayout(dest_row)
        location_layout.addWidget(self.mode_here)
        location_layout.addWidget(self.mode_sub)
        location_layout.addWidget(self.strip)
        location_layout.addWidget(self._preview)

        # 기타
        self.conflict = QComboBox()
        for key, label in CONFLICT_CHOICES:
            self.conflict.addItem(label, key)

        conflict_row = QHBoxLayout()
        conflict_row.addWidget(QLabel("같은 파일이 있으면"))
        conflict_row.addWidget(self.conflict, 1)

        self.open_after = QCheckBox("작업이 끝나면 폴더 열기")

        self.selected_only = QCheckBox(
            f"선택한 {self.selection_count:,}개 항목만 풀기"
        )
        self.selected_only.setChecked(self.selection_count > 0)
        self.selected_only.setVisible(self.selection_count > 0)

        other_box = QGroupBox("옵션")
        other_layout = QVBoxLayout(other_box)
        other_layout.setSpacing(8)
        other_layout.addLayout(conflict_row)
        other_layout.addWidget(self.open_after)
        other_layout.addWidget(self.selected_only)

        buttons = QDialogButtonBox()
        ok = buttons.addButton("압축 풀기", QDialogButtonBox.AcceptRole)
        ok.setDefault(True)
        buttons.addButton("취소", QDialogButtonBox.RejectRole)
        buttons.accepted.connect(self._on_accept)
        buttons.rejected.connect(self.reject)

        layout = QVBoxLayout(self)
        layout.setContentsMargins(16, 14, 16, 14)
        layout.setSpacing(12)
        layout.addWidget(header)
        layout.addWidget(location_box)
        layout.addWidget(other_box)
        layout.addWidget(buttons)

    # -- 상태 ---------------------------------------------------------

    def _browse(self) -> None:
        start = self.dest.text().strip() or str(last_dir("extract"))
        path = QFileDialog.getExistingDirectory(self, "풀어낼 폴더 선택", start)
        if path:
            self.dest.setText(path)

    def _update_preview(self) -> None:
        target = self.target_dir
        note = f"→ {target}"
        if self.mode_sub.isChecked() and self.strip.isChecked():
            note += "\n   (아카이브 최상위가 폴더 하나뿐이면 그 폴더는 생략됩니다)"
        self._preview.setText(note)

    def _restore(self) -> None:
        base = get("extract/last_dir", "") or str(self.archive.parent)
        self.dest.setText(base)

        mode = get("extract/mode", "subfolder")
        (self.mode_here if mode == "here" else self.mode_sub).setChecked(True)

        self.strip.setChecked(bool(get("extract/strip_redundant", True)))
        self.open_after.setChecked(bool(get("extract/open_after", True)))

        conflict = get("extract/conflict", "rename")
        index = self.conflict.findData(conflict)
        self.conflict.setCurrentIndex(index if index >= 0 else 0)

    def _save(self) -> None:
        put("extract/mode", "here" if self.mode_here.isChecked() else "subfolder")
        put("extract/strip_redundant", self.strip.isChecked())
        put("extract/open_after", self.open_after.isChecked())
        put("extract/conflict", self.conflict.currentData())
        remember_dir("extract", Path(self.dest.text().strip()))

    def _on_accept(self) -> None:
        base = Path(self.dest.text().strip())
        if not base.parent.exists() and not base.exists():
            warn(self, "폴더 없음", f"폴더를 찾을 수 없습니다:\n{base}")
            return
        self._save()
        self.accept()

    # -- 결과 ---------------------------------------------------------

    @property
    def target_dir(self) -> Path:
        base = Path(self.dest.text().strip() or ".")
        if self.mode_sub.isChecked():
            return base / strip_archive_suffix(self.archive.name)
        return base

    @property
    def conflict_policy(self) -> str:
        return self.conflict.currentData()

    @property
    def strip_root(self) -> bool:
        return self.mode_sub.isChecked() and self.strip.isChecked()

    @property
    def only_selected(self) -> bool:
        return self.selected_only.isVisible() and self.selected_only.isChecked()

    @property
    def open_folder_after(self) -> bool:
        return self.open_after.isChecked()
