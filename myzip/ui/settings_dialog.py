"""설정 대화상자 - 확장자 연결과 탐색기 통합."""

from __future__ import annotations

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QCheckBox,
    QDialog,
    QDialogButtonBox,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QTabWidget,
    QTreeWidget,
    QTreeWidgetItem,
    QVBoxLayout,
    QWidget,
)

from .. import APP_NAME, __version__
from ..core.sevenzip import find_7z
from ..shell import registry
from .common import app_icon, icon_for_name, info


class SettingsDialog(QDialog):
    """확장자 연결, 컨텍스트 메뉴, 프로그램 정보."""

    def __init__(self, parent: QWidget | None = None):
        super().__init__(parent)
        self.setWindowTitle(f"{APP_NAME} - 설정")
        self.setWindowIcon(app_icon())
        self.resize(560, 560)

        tabs = QTabWidget()
        tabs.addTab(self._build_assoc_tab(), "확장자 연결")
        tabs.addTab(self._build_shell_tab(), "탐색기 통합")
        tabs.addTab(self._build_about_tab(), "정보")

        buttons = QDialogButtonBox(QDialogButtonBox.Close)
        buttons.rejected.connect(self.accept)
        buttons.button(QDialogButtonBox.Close).setText("닫기")

        layout = QVBoxLayout(self)
        layout.setContentsMargins(14, 12, 14, 12)
        layout.addWidget(tabs)
        layout.addWidget(buttons)

        self._refresh_assoc()

    # ------------------------------------------------------- 확장자 연결

    def _build_assoc_tab(self) -> QWidget:
        page = QWidget()

        note = QLabel(
            "체크한 확장자를 더블클릭하면 MyZIP 으로 열립니다.\n"
            "모든 설정은 현재 사용자에게만 적용되며 관리자 권한이 필요 없습니다."
        )
        note.setWordWrap(True)
        note.setStyleSheet("color: palette(mid);")

        self.assoc_tree = QTreeWidget()
        self.assoc_tree.setHeaderLabels(["확장자", "형식", "현재 연결"])
        self.assoc_tree.setRootIsDecorated(False)
        self.assoc_tree.setAlternatingRowColors(True)
        self.assoc_tree.header().setStretchLastSection(True)
        self.assoc_tree.setColumnWidth(0, 110)
        self.assoc_tree.setColumnWidth(1, 180)
        self.assoc_tree.itemChanged.connect(self._on_assoc_toggled)

        select_all = QPushButton("모두 선택")
        select_all.clicked.connect(lambda: self._set_all(True))
        select_none = QPushButton("모두 해제")
        select_none.clicked.connect(lambda: self._set_all(False))

        self._defaults_button = QPushButton("Windows 기본 앱 설정 열기")
        self._defaults_button.clicked.connect(registry.open_default_apps_settings)

        row = QHBoxLayout()
        row.addWidget(select_all)
        row.addWidget(select_none)
        row.addStretch(1)

        self._userchoice_note = QLabel()
        self._userchoice_note.setWordWrap(True)
        self._userchoice_note.setVisible(False)
        self._userchoice_note.setStyleSheet(
            "color: #b45309; background: rgba(245,158,11,0.12);"
            "border-radius: 6px; padding: 8px;"
        )

        layout = QVBoxLayout(page)
        layout.setSpacing(10)
        layout.addWidget(note)
        layout.addWidget(self.assoc_tree, 1)
        layout.addLayout(row)
        layout.addWidget(self._userchoice_note)
        layout.addWidget(self._defaults_button)
        return page

    def _refresh_assoc(self) -> None:
        self.assoc_tree.blockSignals(True)
        self.assoc_tree.clear()

        blocked: list[str] = []
        for ext, (_suffix, description, icon) in registry.FILE_TYPES.items():
            state = registry.status(ext)
            item = QTreeWidgetItem([ext, description, ""])
            item.setIcon(0, icon_for_name(f"x{ext}"))
            item.setFlags(item.flags() | Qt.ItemIsUserCheckable)
            item.setCheckState(0, Qt.Checked if state.registered else Qt.Unchecked)
            item.setData(0, Qt.UserRole, ext)

            if state.blocked_by_userchoice:
                item.setText(2, f"{_pretty(state.current_owner)} (Windows 가 고정)")
                item.setForeground(2, Qt.darkYellow)
                blocked.append(ext)
            elif state.registered:
                item.setText(2, APP_NAME)
            else:
                item.setText(2, _pretty(state.current_owner) or "연결 없음")

            self.assoc_tree.addTopLevelItem(item)

        self.assoc_tree.blockSignals(False)

        if blocked:
            self._userchoice_note.setText(
                "다음 확장자는 Windows '기본 앱' 설정에서 다른 프로그램으로 "
                f"고정되어 있습니다: {', '.join(blocked)}\n"
                "Windows 8 부터 프로그램이 기본 앱을 임의로 바꿀 수 없게 막혀 있어서, "
                "아래 버튼으로 설정을 열어 직접 MyZIP 을 선택해야 합니다."
            )
        self._userchoice_note.setVisible(bool(blocked))

    def _on_assoc_toggled(self, item: QTreeWidgetItem, column: int) -> None:
        if column != 0:
            return
        ext = item.data(0, Qt.UserRole)
        if item.checkState(0) == Qt.Checked:
            registry.associate(ext)
        else:
            registry.unassociate(ext)
        registry.notify_shell()
        self._refresh_assoc()

    def _set_all(self, checked: bool) -> None:
        self.assoc_tree.blockSignals(True)
        for ext in registry.FILE_TYPES:
            if checked:
                registry.associate(ext)
            else:
                registry.unassociate(ext)
        self.assoc_tree.blockSignals(False)
        registry.notify_shell()
        self._refresh_assoc()

    # ------------------------------------------------------ 탐색기 통합

    def _build_shell_tab(self) -> QWidget:
        page = QWidget()

        self.menu_check = QCheckBox("탐색기 마우스 오른쪽 메뉴에 MyZIP 넣기")
        self.menu_check.setChecked(registry.context_menu_installed())
        self.menu_check.toggled.connect(self._on_menu_toggled)

        explain = QLabel(
            "<b>파일/폴더를 우클릭하면</b><br>"
            "· MyZIP 으로 압축 → 압축하기... / ZIP 으로 압축 / TAR.GZ 로 압축<br><br>"
            "<b>압축 파일을 우클릭하면</b><br>"
            "· MyZIP → 열기 / 여기에 압축 풀기 / 폴더 생성 후 풀기 / 무결성 검사"
        )
        explain.setTextFormat(Qt.RichText)
        explain.setWordWrap(True)

        win11 = QLabel(
            "Windows 11 에서는 이 메뉴가 우클릭 후 <b>'추가 옵션 표시'</b> "
            "(또는 Shift+F10) 안에 나타납니다. 1차 메뉴에 바로 넣으려면 "
            "MSIX 패키지와 서명된 COM 확장이 필요합니다."
        )
        win11.setTextFormat(Qt.RichText)
        win11.setWordWrap(True)
        win11.setStyleSheet(
            "color: palette(text); background: rgba(59,130,246,0.12);"
            "border-radius: 6px; padding: 9px;"
        )

        menu_box = QGroupBox("컨텍스트 메뉴")
        menu_layout = QVBoxLayout(menu_box)
        menu_layout.setSpacing(10)
        menu_layout.addWidget(self.menu_check)
        menu_layout.addWidget(explain)
        menu_layout.addWidget(win11)

        # RAR 엔진 상태
        engine_box = QGroupBox("RAR / 7Z 해제 엔진")
        engine_layout = QVBoxLayout(engine_box)

        exe = find_7z()
        if exe:
            status = QLabel(f"✅ 사용 가능<br><code>{exe}</code>")
        else:
            status = QLabel(
                "⚠️ 찾지 못했습니다. RAR 과 7Z 파일은 열 수 없습니다.<br><br>"
                "7-Zip 을 설치하거나, MyZIP 설치 폴더 아래 <code>bin</code> 폴더에 "
                "<code>7z.exe</code> 와 <code>7z.dll</code> 을 넣어 주세요."
            )
        status.setTextFormat(Qt.RichText)
        status.setWordWrap(True)
        engine_layout.addWidget(status)

        rar_note = QLabel(
            "RAR 은 RARLAB 의 독점 형식이라 <b>압축은 만들 수 없고 해제만</b> 됩니다. "
            "unRAR 라이선스가 RAR 압축기 제작에 코드를 쓰는 것을 금지하기 때문이며, "
            "반디집과 7-Zip 도 같은 제약을 받습니다."
        )
        rar_note.setTextFormat(Qt.RichText)
        rar_note.setWordWrap(True)
        rar_note.setStyleSheet("color: palette(mid);")
        engine_layout.addWidget(rar_note)

        layout = QVBoxLayout(page)
        layout.setSpacing(12)
        layout.addWidget(menu_box)
        layout.addWidget(engine_box)
        layout.addStretch(1)
        return page

    def _on_menu_toggled(self, checked: bool) -> None:
        if checked:
            registry.register_application()
            registry.install_context_menu()
        else:
            registry.uninstall_context_menu()

    # ------------------------------------------------------------ 정보

    def _build_about_tab(self) -> QWidget:
        page = QWidget()

        icon = QLabel()
        icon.setPixmap(app_icon().pixmap(72, 72))
        icon.setAlignment(Qt.AlignCenter)

        title = QLabel(f"<h2>{APP_NAME}</h2><p>버전 {__version__}</p>")
        title.setTextFormat(Qt.RichText)
        title.setAlignment(Qt.AlignCenter)

        detail = QLabel(
            "<table cellpadding='4'>"
            "<tr><td><b>압축 가능</b></td>"
            "<td>ZIP, TAR, TAR.GZ(TGZ), TAR.BZ2, TAR.XZ</td></tr>"
            "<tr><td><b>해제 가능</b></td>"
            "<td>위 형식 + RAR, 7Z, CAB, ARJ, LZH, ISO</td></tr>"
            "<tr><td><b>암호</b></td><td>ZIP AES-256 (읽기·쓰기), "
            "ZipCrypto (읽기)</td></tr>"
            "<tr><td><b>파일명 인코딩</b></td>"
            "<td>UTF-8 / CP949 / CP932 / GBK 자동 판별</td></tr>"
            "</table>"
        )
        detail.setTextFormat(Qt.RichText)
        detail.setWordWrap(True)

        remove = QPushButton("모든 연결과 메뉴 제거")
        remove.clicked.connect(self._remove_all)

        layout = QVBoxLayout(page)
        layout.setSpacing(12)
        layout.addSpacing(10)
        layout.addWidget(icon)
        layout.addWidget(title)
        layout.addWidget(detail)
        layout.addStretch(1)
        layout.addWidget(remove)
        return page

    def _remove_all(self) -> None:
        from .common import confirm

        if not confirm(self, "제거 확인",
                       "MyZIP 이 만든 확장자 연결과 탐색기 메뉴를 모두 지웁니다.\n"
                       "프로그램 자체는 지워지지 않습니다.\n\n계속할까요?",
                       "모두 제거"):
            return
        registry.uninstall_all()
        self.menu_check.setChecked(False)
        self._refresh_assoc()
        info(self, "제거 완료", "확장자 연결과 탐색기 메뉴를 모두 제거했습니다.")


def _pretty(progid: str) -> str:
    """ProgID 를 사람이 읽을 만한 이름으로."""
    if not progid:
        return ""
    known = {
        "CompressedFolder": "Windows 탐색기",
        "WinRAR": "WinRAR",
        "7-Zip.zip": "7-Zip",
        "Bandizip.zip": "반디집",
    }
    for key, label in known.items():
        if progid.startswith(key):
            return label
    if progid.startswith("Applications\\"):
        return progid.split("\\", 1)[1]
    return progid
