"""압축하기 대화상자."""

from __future__ import annotations

from pathlib import Path
from typing import Sequence

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QCheckBox,
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFileDialog,
    QFormLayout,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QPushButton,
    QSlider,
    QVBoxLayout,
    QWidget,
)

from .. import APP_NAME
from ..config import get, put, remember_dir
from ..core import WRITABLE_FORMATS, default_extension, format_size
from .common import app_icon, icon_for_name, warn

# 압축 강도 슬라이더 눈금
LEVELS = [
    (0, "저장 (압축 안 함)", "가장 빠름. 이미 압축된 파일에 적합"),
    (1, "가장 빠르게", "속도 우선"),
    (3, "빠르게", ""),
    (6, "표준", "속도와 크기의 균형 (권장)"),
    (9, "최대 압축", "가장 작지만 오래 걸림"),
]


class CompressDialog(QDialog):
    """압축할 대상, 저장 위치, 포맷, 강도, 암호를 정한다."""

    def __init__(self, sources: Sequence[Path], parent: QWidget | None = None):
        super().__init__(parent)
        self.sources = [Path(s) for s in sources]

        self.setWindowTitle(f"{APP_NAME} - 압축하기")
        self.setWindowIcon(app_icon("zip"))
        self.setMinimumWidth(560)

        self._build()
        self._fill_sources()
        self._restore()
        self._suggest_target()
        self._update_enabled()

    # -- 화면 구성 -----------------------------------------------------

    def _build(self) -> None:
        # 대상 목록
        self.list = QListWidget()
        self.list.setAlternatingRowColors(True)
        self.list.setMaximumHeight(150)
        self.list.setSelectionMode(QListWidget.ExtendedSelection)

        add_files = QPushButton("파일 추가...")
        add_files.clicked.connect(self._add_files)
        add_dir = QPushButton("폴더 추가...")
        add_dir.clicked.connect(self._add_folder)
        self._remove = QPushButton("제외")
        self._remove.clicked.connect(self._remove_selected)

        source_buttons = QVBoxLayout()
        source_buttons.setSpacing(6)
        source_buttons.addWidget(add_files)
        source_buttons.addWidget(add_dir)
        source_buttons.addWidget(self._remove)
        source_buttons.addStretch(1)

        source_row = QHBoxLayout()
        source_row.addWidget(self.list, 1)
        source_row.addLayout(source_buttons)

        source_box = QGroupBox("압축할 대상")
        source_layout = QVBoxLayout(source_box)
        source_layout.addLayout(source_row)
        self._summary = QLabel()
        self._summary.setStyleSheet("color: palette(mid);")
        source_layout.addWidget(self._summary)

        # 저장 위치
        self.target = QLineEdit()
        browse = QPushButton("찾아보기...")
        browse.setMaximumWidth(96)
        browse.clicked.connect(self._browse_target)

        target_row = QHBoxLayout()
        target_row.setSpacing(6)
        target_row.addWidget(self.target, 1)
        target_row.addWidget(browse)

        # 포맷
        self.format = QComboBox()
        for key, label, _cls in WRITABLE_FORMATS:
            self.format.addItem(label, key)
        self.format.currentIndexChanged.connect(self._on_format_changed)

        # 강도
        self.level = QSlider(Qt.Horizontal)
        self.level.setRange(0, len(LEVELS) - 1)
        self.level.setPageStep(1)
        self.level.setTickPosition(QSlider.TicksBelow)
        self.level.setTickInterval(1)
        self.level.valueChanged.connect(self._on_level_changed)

        self._level_label = QLabel()
        self._level_hint = QLabel()
        self._level_hint.setStyleSheet("color: palette(mid);")

        level_box = QVBoxLayout()
        level_box.setSpacing(2)
        level_box.addWidget(self.level)
        level_box.addWidget(self._level_label)
        level_box.addWidget(self._level_hint)

        # 암호
        self.password = QLineEdit()
        self.password.setEchoMode(QLineEdit.Password)
        self.password.setPlaceholderText("비워 두면 암호를 걸지 않습니다")
        self.password.textChanged.connect(self._update_enabled)

        self.password2 = QLineEdit()
        self.password2.setEchoMode(QLineEdit.Password)
        self.password2.setPlaceholderText("확인을 위해 한 번 더")
        self.password2.textChanged.connect(self._update_enabled)

        self.show_password = QCheckBox("암호 보기")
        self.show_password.toggled.connect(self._toggle_echo)

        self._password_note = QLabel()
        self._password_note.setWordWrap(True)
        self._password_note.setStyleSheet("color: palette(mid);")

        options = QFormLayout()
        options.setLabelAlignment(Qt.AlignRight | Qt.AlignVCenter)
        options.setSpacing(9)
        options.addRow("저장 위치", target_row)
        options.addRow("압축 형식", self.format)
        options.addRow("압축 강도", level_box)
        options.addRow("암호", self.password)
        options.addRow("암호 확인", self.password2)
        options.addRow("", self.show_password)
        options.addRow("", self._password_note)

        option_box = QGroupBox("설정")
        option_layout = QVBoxLayout(option_box)
        option_layout.addLayout(options)

        self.buttons = QDialogButtonBox()
        self._ok = self.buttons.addButton("압축 시작", QDialogButtonBox.AcceptRole)
        self._ok.setDefault(True)
        self.buttons.addButton("취소", QDialogButtonBox.RejectRole)
        self.buttons.accepted.connect(self._on_accept)
        self.buttons.rejected.connect(self.reject)

        layout = QVBoxLayout(self)
        layout.setContentsMargins(16, 14, 16, 14)
        layout.setSpacing(12)
        layout.addWidget(source_box)
        layout.addWidget(option_box)
        layout.addWidget(self.buttons)

    # -- 대상 목록 -----------------------------------------------------

    def _fill_sources(self) -> None:
        self.list.clear()
        for path in self.sources:
            item = QListWidgetItem(icon_for_name(path.name, path.is_dir()), str(path))
            item.setData(Qt.UserRole, str(path))
            self.list.addItem(item)
        self._update_summary()

    def _update_summary(self) -> None:
        files = folders = 0
        total = 0
        for path in self.sources:
            if path.is_dir():
                folders += 1
                for sub in path.rglob("*"):
                    if sub.is_file():
                        files += 1
                        try:
                            total += sub.stat().st_size
                        except OSError:
                            pass
            elif path.is_file():
                files += 1
                try:
                    total += path.stat().st_size
                except OSError:
                    pass

        bits = []
        if folders:
            bits.append(f"폴더 {folders:,}개")
        bits.append(f"파일 {files:,}개")
        bits.append(f"총 {format_size(total)}")
        self._summary.setText("　·　".join(bits))

    def _add_files(self) -> None:
        paths, _ = QFileDialog.getOpenFileNames(self, "압축할 파일 선택",
                                                str(self._start_dir()))
        self._append([Path(p) for p in paths])

    def _add_folder(self) -> None:
        path = QFileDialog.getExistingDirectory(self, "압축할 폴더 선택",
                                                str(self._start_dir()))
        if path:
            self._append([Path(path)])

    def _append(self, paths: list[Path]) -> None:
        known = {str(p) for p in self.sources}
        added = [p for p in paths if str(p) not in known]
        if not added:
            return
        self.sources.extend(added)
        self._fill_sources()
        if not self.target.text().strip():
            self._suggest_target()
        self._update_enabled()

    def _remove_selected(self) -> None:
        chosen = {item.data(Qt.UserRole) for item in self.list.selectedItems()}
        if not chosen:
            return
        self.sources = [p for p in self.sources if str(p) not in chosen]
        self._fill_sources()
        self._update_enabled()

    def _start_dir(self) -> Path:
        if self.sources:
            first = self.sources[0]
            return first if first.is_dir() else first.parent
        from ..config import last_dir

        return last_dir("compress")

    # -- 저장 위치 -----------------------------------------------------

    def _suggest_target(self) -> None:
        """대상에서 아카이브 이름을 추측한다.

        하나만 골랐으면 그 이름, 여러 개면 상위 폴더 이름을 쓴다.
        (반디집도 같은 규칙을 쓴다.)
        """
        if not self.sources:
            return

        first = self.sources[0]
        if len(self.sources) == 1:
            stem = first.name if first.is_dir() else first.stem
        else:
            stem = first.parent.name or "압축파일"

        ext = default_extension(self.format.currentData())
        self.target.setText(str(first.parent / f"{stem}{ext}"))

    def _browse_target(self) -> None:
        ext = default_extension(self.format.currentData())
        current = self.target.text().strip() or str(self._start_dir())
        path, _ = QFileDialog.getSaveFileName(
            self, "저장할 위치", current,
            f"압축 파일 (*{ext});;모든 파일 (*.*)",
        )
        if path:
            self.target.setText(path)

    def _on_format_changed(self) -> None:
        """포맷을 바꾸면 확장자를 갈아 끼우고 지원 여부를 반영한다."""
        text = self.target.text().strip()
        new_ext = default_extension(self.format.currentData())
        if text:
            path = Path(text)
            name = path.name
            for known in (".tar.gz", ".tar.bz2", ".tar.xz"):
                if name.lower().endswith(known):
                    name = name[: -len(known)]
                    break
            else:
                name = path.stem
            self.target.setText(str(path.parent / f"{name}{new_ext}"))
        self._update_enabled()

    # -- 강도 / 암호 ---------------------------------------------------

    def _on_level_changed(self, index: int) -> None:
        _value, label, hint = LEVELS[index]
        self._level_label.setText(label)
        self._level_hint.setText(hint)

    def _toggle_echo(self, shown: bool) -> None:
        mode = QLineEdit.Normal if shown else QLineEdit.Password
        self.password.setEchoMode(mode)
        self.password2.setEchoMode(mode)

    def _update_enabled(self) -> None:
        fmt = self.format.currentData()
        zip_selected = fmt == "zip"

        # 암호는 ZIP 에서만 지원한다. TAR 계열은 규격 자체에 암호가 없다.
        for widget in (self.password, self.password2, self.show_password):
            widget.setEnabled(zip_selected)
        if not zip_selected:
            self.password.clear()
            self.password2.clear()
            self._password_note.setText(
                "TAR 계열은 형식 자체에 암호 기능이 없습니다. "
                "암호를 걸려면 ZIP 을 선택하세요."
            )
        elif self.password.text():
            self._password_note.setText("AES-256 으로 암호화합니다.")
        else:
            self._password_note.setText("")

        mismatch = (zip_selected and self.password.text()
                    and self.password.text() != self.password2.text())
        if mismatch:
            self._password_note.setText("두 암호가 서로 다릅니다.")

        self._remove.setEnabled(bool(self.list.selectedItems()))
        self._ok.setEnabled(
            bool(self.sources) and bool(self.target.text().strip()) and not mismatch
        )

    # -- 확인 ---------------------------------------------------------

    def _on_accept(self) -> None:
        target = Path(self.target.text().strip())

        if not target.parent.exists():
            warn(self, "저장 위치 없음",
                 f"폴더가 존재하지 않습니다:\n{target.parent}")
            return

        if target.exists():
            from .common import confirm

            if not confirm(self, "덮어쓰기 확인",
                           f"이미 파일이 있습니다.\n\n{target.name}\n\n덮어쓸까요?",
                           "덮어쓰기"):
                return

        # 자기 자신을 압축 대상에 넣으면 무한히 커진다.
        resolved = target.resolve()
        for source in self.sources:
            try:
                if source.resolve() == resolved:
                    warn(self, "잘못된 대상",
                         "저장할 파일이 압축 대상에 포함되어 있습니다.\n"
                         "다른 이름이나 위치를 선택하세요.")
                    return
            except OSError:
                continue

        self._save()
        self.accept()

    def _restore(self) -> None:
        fmt = get("compress/format", "zip")
        index = self.format.findData(fmt)
        if index >= 0:
            self.format.setCurrentIndex(index)

        level = get("compress/level", 6)
        slider = next((i for i, (v, _, _) in enumerate(LEVELS) if v == level), 3)
        self.level.setValue(slider)
        self._on_level_changed(slider)

    def _save(self) -> None:
        put("compress/format", self.format.currentData())
        put("compress/level", LEVELS[self.level.value()][0])
        remember_dir("compress", Path(self.target.text().strip()))

    # -- 결과 ---------------------------------------------------------

    @property
    def target_path(self) -> Path:
        return Path(self.target.text().strip())

    @property
    def compression_level(self) -> int:
        return LEVELS[self.level.value()][0]

    @property
    def archive_password(self) -> str | None:
        text = self.password.text()
        return text if text and self.format.currentData() == "zip" else None
