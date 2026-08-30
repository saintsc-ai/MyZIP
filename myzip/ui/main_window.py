"""메인 창 - 아카이브 내용 탐색기."""

from __future__ import annotations

import tempfile
from pathlib import Path, PurePosixPath

from PySide6.QtCore import QPoint, Qt, QUrl, Signal
from PySide6.QtGui import QAction, QDesktopServices, QKeySequence
from PySide6.QtWidgets import (
    QAbstractItemView,
    QComboBox,
    QFileDialog,
    QHeaderView,
    QLabel,
    QMainWindow,
    QMenu,
    QMessageBox,
    QSplitter,
    QSizePolicy,
    QToolBar,
    QTreeWidget,
    QTreeWidgetItem,
    QTreeWidgetItemIterator,
    QWidget,
)

from .. import APP_NAME, __version__
from ..config import get, last_dir, put, remember_dir
from ..core import (
    ArchiveEntry,
    ArchiveError,
    PasswordRequired,
    format_size,
    is_archive,
    strip_archive_suffix,
)
from ..core.encoding import CANDIDATES, DISPLAY_NAMES
from .common import (
    APP_STYLE,
    app_icon,
    confirm,
    error,
    format_time,
    icon_for_name,
    info,
    open_in_explorer,
    shell_open,
    warn,
)
from .compress_dialog import CompressDialog
from .extract_dialog import ExtractDialog
from .progress_dialog import PASSWORD_NEEDED, run_task
from .workers import CompressTask, ExtractTask, LoadTask, TestTask

COLUMNS = ("이름", "크기", "압축 크기", "압축률", "종류", "수정한 날짜")


class MainWindow(QMainWindow):
    """반디집 스타일의 아카이브 탐색 창.

    왼쪽에 폴더 트리, 오른쪽에 파일 목록을 두고 아카이브 안을
    실제 폴더처럼 돌아다닐 수 있게 한다.
    """

    closed = Signal()

    def __init__(self):
        super().__init__()
        self.reader = None
        self.archive_path: Path | None = None
        self.password: str | None = None
        self._current_dir = ""            # 아카이브 안의 현재 위치
        self._tree_index: dict[str, list[ArchiveEntry]] = {}
        self._dirs: set[str] = set()
        self._temp_dir: tempfile.TemporaryDirectory | None = None

        self.setWindowTitle(APP_NAME)
        self.setWindowIcon(app_icon())
        self.setAcceptDrops(True)
        self.resize(1000, 620)
        self.setStyleSheet(APP_STYLE)

        self._build_actions()
        self._build_toolbar()
        self._build_body()
        self._build_statusbar()
        self._restore_geometry()
        self._update_actions()

    # ------------------------------------------------------------ 구성

    def _build_actions(self) -> None:
        def act(text, icon, slot, shortcut=None, tip=""):
            a = QAction(app_icon(icon), text, self)
            a.triggered.connect(slot)
            if shortcut:
                a.setShortcut(QKeySequence(shortcut))
            a.setToolTip(tip or text)
            return a

        self.act_open = act("열기", "act-open", self.open_dialog, "Ctrl+O",
                            "압축 파일 열기")
        self.act_new = act("새 압축", "act-new", self.compress_dialog, "Ctrl+N",
                           "파일이나 폴더를 새 압축 파일로 만들기")
        self.act_extract = act("압축 풀기", "act-extract", self.extract_dialog,
                               "Ctrl+E", "위치를 정해서 풀기")
        self.act_extract_here = act("여기에 풀기", "act-extract-here",
                                    self.extract_here, "Ctrl+H",
                                    "압축 파일이 있는 폴더에 바로 풀기")
        self.act_test = act("검사", "act-test", self.test_archive, "Ctrl+T",
                            "아카이브가 손상되지 않았는지 확인")
        self.act_settings = act("설정", "act-settings", self.open_settings, "Ctrl+,",
                                "확장자 연결과 탐색기 메뉴")

        self.act_up = QAction("상위 폴더", self)
        self.act_up.setShortcut(QKeySequence(Qt.Key_Backspace))
        self.act_up.triggered.connect(self.go_up)
        self.addAction(self.act_up)

        self.act_select_all = QAction("모두 선택", self)
        self.act_select_all.setShortcut(QKeySequence.SelectAll)
        self.act_select_all.triggered.connect(lambda: self.list.selectAll())
        self.addAction(self.act_select_all)

    def _build_toolbar(self) -> None:
        bar = QToolBar("주요 기능")
        bar.setMovable(False)
        bar.setIconSize(bar.iconSize() * 1.15)
        bar.setToolButtonStyle(Qt.ToolButtonTextUnderIcon)

        bar.addAction(self.act_new)
        bar.addAction(self.act_open)
        bar.addSeparator()
        bar.addAction(self.act_extract)
        bar.addAction(self.act_extract_here)
        bar.addAction(self.act_test)
        bar.addSeparator()

        spacer = QWidget()
        spacer.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Preferred)
        bar.addWidget(spacer)

        # 파일명 인코딩 선택 (깨진 이름을 살릴 때 쓴다)
        self.encoding_box = QComboBox()
        self.encoding_box.addItem("파일명 인코딩: 자동", "auto")
        for enc in CANDIDATES:
            self.encoding_box.addItem(f"파일명: {DISPLAY_NAMES.get(enc, enc)}", enc)
        self.encoding_box.setMinimumWidth(180)
        self.encoding_box.currentIndexChanged.connect(self._on_encoding_changed)
        self.encoding_box.setToolTip(
            "한글 파일명이 깨져 보이면 여기서 인코딩을 바꿔 보세요."
        )
        bar.addWidget(self.encoding_box)
        bar.addAction(self.act_settings)

        self.addToolBar(bar)

    def _build_body(self) -> None:
        # 왼쪽: 폴더 트리
        self.tree = QTreeWidget()
        self.tree.setHeaderLabel("폴더")
        self.tree.setMinimumWidth(180)
        self.tree.itemSelectionChanged.connect(self._on_tree_selected)

        # 오른쪽: 파일 목록
        self.list = QTreeWidget()
        self.list.setHeaderLabels(COLUMNS)
        self.list.setRootIsDecorated(False)
        self.list.setAlternatingRowColors(True)
        self.list.setSortingEnabled(True)
        self.list.setSelectionMode(QAbstractItemView.ExtendedSelection)
        self.list.setContextMenuPolicy(Qt.CustomContextMenu)
        self.list.customContextMenuRequested.connect(self._show_context_menu)
        self.list.itemDoubleClicked.connect(self._on_double_click)
        self.list.itemSelectionChanged.connect(self._update_statusbar)

        header = self.list.header()
        header.setStretchLastSection(False)
        # 이름 열은 사용자가 조절하고, 마지막 날짜 열이 남는 폭을 먹는다.
        for column, width in enumerate((260, 90, 95, 70, 100)):
            header.setSectionResizeMode(column, QHeaderView.Interactive)
            self.list.setColumnWidth(column, width)
        header.setSectionResizeMode(len(COLUMNS) - 1, QHeaderView.Stretch)
        header.setMinimumSectionSize(56)

        self.splitter = QSplitter(Qt.Horizontal)
        self.splitter.addWidget(self.tree)
        self.splitter.addWidget(self.list)
        self.splitter.setStretchFactor(0, 0)
        self.splitter.setStretchFactor(1, 1)
        self.splitter.setSizes([220, 780])

        self.setCentralWidget(self.splitter)

        # 아무것도 열지 않았을 때 안내
        self._empty_hint = QLabel(
            f"<div style='text-align:center; color:palette(mid);'>"
            f"<h2>{APP_NAME}</h2>"
            "<p>압축 파일을 여기에 끌어다 놓거나<br>"
            "<b>새 압축</b> 으로 파일을 압축하세요.</p></div>",
            self.list,
        )
        self._empty_hint.setTextFormat(Qt.RichText)
        self._empty_hint.setAlignment(Qt.AlignCenter)
        self._empty_hint.setAttribute(Qt.WA_TransparentForMouseEvents)

    def _build_statusbar(self) -> None:
        self._status_left = QLabel()
        self._status_right = QLabel()
        bar = self.statusBar()
        bar.addWidget(self._status_left, 1)
        bar.addPermanentWidget(self._status_right)
        self._update_statusbar()

    # ------------------------------------------------------- 아카이브 열기

    def open_dialog(self) -> None:
        path, _ = QFileDialog.getOpenFileName(
            self, "압축 파일 열기", str(last_dir("extract")),
            "압축 파일 (*.zip *.tar *.tgz *.tar.gz *.tar.bz2 *.tar.xz "
            "*.gz *.bz2 *.xz *.rar *.7z *.cab *.arj *.lzh *.iso);;"
            "모든 파일 (*.*)",
        )
        if path:
            self.open_archive(Path(path))

    def open_archive(self, path: Path, password: str | None = None,
                     encoding: str | None = None) -> bool:
        """아카이브를 열어 목록을 채운다. 성공하면 True."""
        path = Path(path)
        if not path.is_file():
            error(self, f"파일을 찾을 수 없습니다:\n{path}")
            return False

        if not is_archive(path):
            error(self, f"압축 파일이 아니거나 지원하지 않는 형식입니다:\n{path.name}")
            return False

        task = LoadTask(path, password, encoding)
        ok, reader = run_task(task, f"{path.name} 여는 중", self)

        if not ok:
            # RAR/7Z 은 목록조차 암호 없이는 못 읽는 경우가 있다.
            # 그때는 오류로 끝내지 말고 암호를 물어보고 다시 시도한다.
            if reader is PASSWORD_NEEDED:
                entered = self._ask_password(path.name, retry=bool(password))
                if entered is None:
                    return False
                return self.open_archive(path, entered, encoding)
            return False

        # ZIP 처럼 목록은 읽히지만 내용이 암호로 잠긴 경우.
        if reader.has_encrypted and not password:
            entered = self._ask_password(path.name)
            if entered is None:
                reader.close()
                return False
            reader.close()
            return self.open_archive(path, entered, encoding)

        if self.reader is not None:
            self.reader.close()

        self.reader = reader
        self.archive_path = path
        self.password = password
        remember_dir("extract", path)

        self._index_entries()
        self._sync_encoding_box()
        self._populate_tree()
        self.navigate("")
        self.setWindowTitle(f"{path.name} - {APP_NAME}")
        self._update_actions()
        return True

    def _ask_password(self, name: str, retry: bool = False) -> str | None:
        from PySide6.QtWidgets import QInputDialog, QLineEdit

        prompt = (
            f"암호가 맞지 않습니다.\n'{name}' 의 암호를 다시 입력하세요:"
            if retry else
            f"'{name}' 은(는) 암호로 보호되어 있습니다.\n암호를 입력하세요:"
        )
        text, ok = QInputDialog.getText(
            self, f"{APP_NAME} - 암호 필요", prompt, QLineEdit.Password
        )
        return text if ok and text else None

    # -------------------------------------------------------- 목록 구성

    def _index_entries(self) -> None:
        """항목들을 폴더별로 묶어 둔다. 매번 전체를 훑지 않기 위해서다."""
        self._tree_index = {"": []}
        self._dirs = set()

        for entry in self.reader.entries:
            parent = entry.parent
            # 중간 폴더가 아카이브에 명시되어 있지 않을 수도 있으므로
            # 경로를 따라가며 없는 폴더를 만들어 준다.
            parts = PurePosixPath(parent).parts if parent else ()
            accumulated = ""
            for part in parts:
                accumulated = f"{accumulated}/{part}" if accumulated else part
                self._dirs.add(accumulated)
                self._tree_index.setdefault(accumulated, [])

            if entry.is_dir:
                self._dirs.add(entry.path)
                self._tree_index.setdefault(entry.path, [])

            self._tree_index.setdefault(parent, []).append(entry)

        # 명시적으로 기록되지 않은 폴더도 목록에 나타나야 한다.
        for folder in sorted(self._dirs):
            parent = str(PurePosixPath(folder).parent)
            parent = "" if parent == "." else parent
            siblings = self._tree_index.setdefault(parent, [])
            if not any(e.path == folder for e in siblings):
                siblings.append(ArchiveEntry(path=folder, is_dir=True))

    def _populate_tree(self) -> None:
        self.tree.blockSignals(True)
        self.tree.clear()

        root = QTreeWidgetItem([self.archive_path.name])
        root.setIcon(0, app_icon("zip"))
        root.setData(0, Qt.UserRole, "")
        self.tree.addTopLevelItem(root)

        nodes: dict[str, QTreeWidgetItem] = {"": root}
        for folder in sorted(self._dirs):
            parent_path = str(PurePosixPath(folder).parent)
            parent_path = "" if parent_path == "." else parent_path
            parent = nodes.get(parent_path, root)

            node = QTreeWidgetItem([PurePosixPath(folder).name])
            node.setIcon(0, icon_for_name(folder, is_dir=True))
            node.setData(0, Qt.UserRole, folder)
            parent.addChild(node)
            nodes[folder] = node

        root.setExpanded(True)
        self.tree.setCurrentItem(root)
        self.tree.blockSignals(False)

    def navigate(self, folder: str) -> None:
        """아카이브 안의 folder 로 이동한다."""
        self._current_dir = folder
        self.list.setSortingEnabled(False)
        self.list.clear()

        # 상위로 올라가는 항목
        if folder:
            up = _SortableItem(["..", "", "", "", "상위 폴더", ""])
            up.setIcon(0, icon_for_name("..", is_dir=True))
            up.setData(0, Qt.UserRole, None)
            up.sort_first = True
            self.list.addTopLevelItem(up)

        for entry in sorted(self._tree_index.get(folder, []),
                            key=lambda e: (not e.is_dir, e.name.lower())):
            item = _make_item(entry)
            self.list.addTopLevelItem(item)

        self.list.setSortingEnabled(True)
        self._empty_hint.setVisible(self.reader is None)
        self._update_statusbar()

    def _on_tree_selected(self) -> None:
        item = self.tree.currentItem()
        if item is None:
            return
        folder = item.data(0, Qt.UserRole)
        if folder is not None:
            self.navigate(folder)

    def go_up(self) -> None:
        if not self._current_dir:
            return
        parent = str(PurePosixPath(self._current_dir).parent)
        self._select_tree_folder("" if parent == "." else parent)

    def _select_tree_folder(self, folder: str) -> None:
        """왼쪽 트리에서 해당 폴더를 선택한다 (목록도 따라 바뀐다)."""
        iterator = QTreeWidgetItemIterator(self.tree)
        while iterator.value():
            item = iterator.value()
            if item.data(0, Qt.UserRole) == folder:
                parent = item.parent()
                while parent is not None:      # 접혀 있으면 보이지 않는다
                    parent.setExpanded(True)
                    parent = parent.parent()
                self.tree.setCurrentItem(item)
                self.tree.scrollToItem(item)
                return
            iterator += 1
        self.navigate(folder)

    def _on_double_click(self, item: QTreeWidgetItem, _column: int) -> None:
        entry = item.data(0, Qt.UserRole + 1)
        if item.data(0, Qt.UserRole) is None and item.text(0) == "..":
            self.go_up()
            return
        if entry is None:
            return
        if entry.is_dir:
            self._select_tree_folder(entry.path)
        else:
            self._preview_entry(entry)

    # ---------------------------------------------------------- 미리보기

    def _preview_entry(self, entry: ArchiveEntry) -> None:
        """항목을 임시 폴더로 꺼내 연결된 프로그램으로 연다."""
        if entry.size > 400 * 1024 * 1024:
            if not confirm(self, "큰 파일",
                           f"{entry.name} 은(는) {format_size(entry.size)} 입니다.\n"
                           "임시 폴더로 꺼내는 데 시간이 걸릴 수 있습니다.\n\n계속할까요?",
                           "열기"):
                return

        if self._temp_dir is None:
            self._temp_dir = tempfile.TemporaryDirectory(prefix="myzip-view-")

        target = Path(self._temp_dir.name)
        task = ExtractTask(self.archive_path, target, [entry],
                           self.password, reader=self.reader)
        ok, _ = run_task(task, f"{entry.name} 여는 중", self)
        if not ok:
            return

        from ..core.path_safety import safe_join

        opened = safe_join(target, entry.path)
        if not opened.exists():
            error(self, f"항목을 꺼내지 못했습니다: {entry.name}")
            return
        try:
            shell_open(opened)
        except RuntimeError as exc:
            error(self, str(exc))

    # ------------------------------------------------------------ 압축

    def compress_dialog(self, sources: list[Path] | None = None) -> None:
        if not sources:
            paths, _ = QFileDialog.getOpenFileNames(
                self, "압축할 파일 선택", str(last_dir("compress")))
            sources = [Path(p) for p in paths]
            if not sources:
                return

        dialog = CompressDialog(sources, self)
        if dialog.exec() != CompressDialog.Accepted:
            return

        task = CompressTask(dialog.sources, dialog.target_path,
                            dialog.compression_level, dialog.archive_password)
        ok, result = run_task(task, "압축하는 중", self)
        if ok and result:
            self._after_compress(Path(result))

    def _after_compress(self, archive: Path) -> None:
        size = archive.stat().st_size
        box = QMessageBox(self)
        box.setWindowIcon(app_icon())
        box.setIcon(QMessageBox.Information)
        box.setWindowTitle("압축 완료")
        box.setText(f"<b>{archive.name}</b><br>{format_size(size)}")
        box.setInformativeText(str(archive.parent))
        open_folder = box.addButton("폴더 열기", QMessageBox.ActionRole)
        open_archive = box.addButton("아카이브 열기", QMessageBox.ActionRole)
        box.addButton("닫기", QMessageBox.RejectRole)
        box.exec()

        if box.clickedButton() is open_folder:
            open_in_explorer(archive)
        elif box.clickedButton() is open_archive:
            self.open_archive(archive)

    # ------------------------------------------------------------ 해제

    def _selected_entries(self) -> list[ArchiveEntry]:
        """선택된 항목들. 폴더를 고르면 그 안의 모든 항목까지 포함한다."""
        chosen: list[ArchiveEntry] = []
        seen: set[str] = set()

        for item in self.list.selectedItems():
            entry = item.data(0, Qt.UserRole + 1)
            if entry is None:
                continue
            if entry.is_dir:
                prefix = entry.path + "/"
                for candidate in self.reader.entries:
                    if candidate.path == entry.path or candidate.path.startswith(prefix):
                        if candidate.path not in seen:
                            seen.add(candidate.path)
                            chosen.append(candidate)
            elif entry.path not in seen:
                seen.add(entry.path)
                chosen.append(entry)
        return chosen

    def extract_dialog(self) -> None:
        if self.reader is None:
            return
        selection = self._selected_entries()
        dialog = ExtractDialog(self.archive_path, len(selection), self)
        if dialog.exec() != ExtractDialog.Accepted:
            return

        members = selection if dialog.only_selected and selection else None
        self._run_extract(dialog.target_dir, members, dialog.conflict_policy,
                          dialog.strip_root, dialog.open_folder_after)

    def extract_here(self) -> None:
        if self.reader is None or self.archive_path is None:
            return
        target = self.archive_path.parent / strip_archive_suffix(self.archive_path.name)
        self._run_extract(target, None, get("extract/conflict", "rename"),
                          bool(get("extract/strip_redundant", True)),
                          bool(get("extract/open_after", True)))

    def _run_extract(self, target: Path, members, conflict: str,
                     strip_root: bool, open_after: bool,
                     quiet: bool = False) -> None:
        """압축을 푼다.

        quiet 는 컨텍스트 메뉴처럼 한 방에 끝나는 흐름에서 쓴다.
        확인 버튼을 누르라고 사용자를 붙잡아 두지 않는다.
        """
        task = ExtractTask(self.archive_path, target, members, self.password,
                           strip_root=strip_root, reader=self.reader)
        task.conflict_policy = conflict

        ok, _ = run_task(task, "압축을 푸는 중", self)
        if not ok:
            return
        if open_after:
            open_in_explorer(target)
        elif not quiet:
            info(self, "완료", f"압축을 풀었습니다.\n\n{target}")

    # ------------------------------------------------------------ 검사

    def test_archive(self) -> None:
        if self.reader is None:
            return
        task = TestTask(self.reader)
        ok, bad = run_task(task, "무결성 검사 중", self)
        if not ok:
            return
        if bad:
            warn(self, "손상 발견",
                 f"{len(bad)}개 항목에 문제가 있습니다.",
                 "\n".join(str(b) for b in bad[:200]))
        else:
            info(self, "검사 완료", "아카이브에 이상이 없습니다.")

    # ------------------------------------------------------------ 인코딩

    def _sync_encoding_box(self) -> None:
        self.encoding_box.blockSignals(True)
        detected = getattr(self.reader, "encoding", "utf-8")
        # '자동' 항목에 실제로 판별된 인코딩을 괄호로 보여 준다.
        self.encoding_box.setItemText(
            0, f"파일명 인코딩: 자동 ({DISPLAY_NAMES.get(detected, detected)})"
        )
        self.encoding_box.setCurrentIndex(0)
        self.encoding_box.blockSignals(False)

    def _on_encoding_changed(self, index: int) -> None:
        if self.reader is None:
            return
        encoding = self.encoding_box.currentData()
        if encoding == "auto":
            self.reader.set_encoding("")
            self.reader._entries = None
        else:
            self.reader.set_encoding(encoding)
        self._index_entries()
        self._populate_tree()
        self.navigate("")

    # ------------------------------------------------------- 컨텍스트 메뉴

    def _show_context_menu(self, position: QPoint) -> None:
        if self.reader is None:
            return
        menu = QMenu(self)
        selection = self.list.selectedItems()

        if selection:
            menu.addAction("선택 항목 풀기...", self.extract_dialog)
            entry = selection[0].data(0, Qt.UserRole + 1)
            if entry is not None and not entry.is_dir and len(selection) == 1:
                menu.addAction("열기", lambda: self._preview_entry(entry))
                menu.addAction("내용 복사 (텍스트)",
                               lambda: self._copy_text(entry))
            menu.addSeparator()

        menu.addAction("전체 풀기...", self.extract_dialog)
        menu.addAction("여기에 전부 풀기", self.extract_here)
        menu.addSeparator()
        menu.addAction("모두 선택", self.list.selectAll)
        menu.addAction("무결성 검사", self.test_archive)
        menu.exec(self.list.viewport().mapToGlobal(position))

    def _copy_text(self, entry: ArchiveEntry) -> None:
        """텍스트 파일 내용을 클립보드로. 빠르게 확인할 때 쓴다."""
        if entry.size > 4 * 1024 * 1024:
            warn(self, "너무 큼", "4MB 이하 파일만 복사할 수 있습니다.")
            return
        try:
            raw = self.reader.read_bytes(entry)
        except ArchiveError as exc:
            error(self, str(exc))
            return

        from ..core.encoding import decode_best

        text, _ = decode_best(raw)
        from PySide6.QtWidgets import QApplication

        QApplication.clipboard().setText(text)
        self.statusBar().showMessage(f"{entry.name} 내용을 복사했습니다.", 3000)

    # ------------------------------------------------------------ 기타

    def open_settings(self) -> None:
        from .settings_dialog import SettingsDialog

        SettingsDialog(self).exec()

    def _update_actions(self) -> None:
        has = self.reader is not None
        for action in (self.act_extract, self.act_extract_here, self.act_test):
            action.setEnabled(has)
        self.encoding_box.setEnabled(has)
        self._empty_hint.setVisible(not has)
        if not has:
            self._empty_hint.resize(self.list.size())

    def _update_statusbar(self) -> None:
        if self.reader is None:
            self._status_left.setText("열린 압축 파일이 없습니다.")
            self._status_right.setText("")
            return

        entries = [e for e in self.reader.entries if not e.is_dir]
        total = sum(e.size for e in entries)
        packed = sum(e.compressed_size for e in entries)
        ratio = (1 - packed / total) * 100 if total else 0

        selected = [i for i in self.list.selectedItems()
                    if i.data(0, Qt.UserRole + 1) is not None]
        if selected:
            picked = sum(e.size for e in self._selected_entries() if not e.is_dir)
            self._status_left.setText(
                f"{len(selected):,}개 선택　·　{format_size(picked)}"
            )
        else:
            self._status_left.setText(
                f"파일 {len(entries):,}개　·　원본 {format_size(total)}"
            )

        encoding = getattr(self.reader, "encoding", "")
        self._status_right.setText(
            f"압축 후 {format_size(packed)}　·　{ratio:.1f}% 절약　·　"
            f"{DISPLAY_NAMES.get(encoding, encoding)}"
        )

    # -------------------------------------------------------- 드래그&드롭

    def dragEnterEvent(self, event) -> None:
        if event.mimeData().hasUrls():
            event.acceptProposedAction()

    def dragMoveEvent(self, event) -> None:
        if event.mimeData().hasUrls():
            event.acceptProposedAction()

    def dropEvent(self, event) -> None:
        paths = [Path(url.toLocalFile()) for url in event.mimeData().urls()
                 if url.isLocalFile()]
        paths = [p for p in paths if p.exists()]
        if not paths:
            return
        event.acceptProposedAction()

        # 압축 파일 하나만 떨어뜨렸으면 여는 것이 자연스럽다.
        if len(paths) == 1 and paths[0].is_file() and is_archive(paths[0]):
            self.open_archive(paths[0])
            return
        self.compress_dialog(paths)

    # ------------------------------------------------------------ 창 상태

    def _restore_geometry(self) -> None:
        geometry = get("view/geometry", "")
        if geometry:
            from PySide6.QtCore import QByteArray

            self.restoreGeometry(QByteArray.fromBase64(geometry.encode()))
        sizes = get("view/splitter", "")
        if sizes:
            try:
                self.splitter.setSizes([int(x) for x in sizes.split(",")])
            except ValueError:
                pass

    def resizeEvent(self, event) -> None:
        super().resizeEvent(event)
        if self.reader is None:
            self._empty_hint.resize(self.list.size())

    def closeEvent(self, event) -> None:
        put("view/geometry", bytes(self.saveGeometry().toBase64()).decode())
        put("view/splitter", ",".join(str(s) for s in self.splitter.sizes()))

        if self.reader is not None:
            self.reader.close()
        if self._temp_dir is not None:
            self._temp_dir.cleanup()
            self._temp_dir = None

        self.closed.emit()
        event.accept()


def _make_item(entry: ArchiveEntry) -> QTreeWidgetItem:
    """목록 행 하나를 만든다. 정렬이 문자열이 아닌 값 기준이 되도록 데이터도 심는다."""
    if entry.is_dir:
        columns = [entry.name, "", "", "", "폴더", format_time(entry.mtime)]
    else:
        columns = [
            entry.name,
            format_size(entry.size),
            format_size(entry.compressed_size),
            f"{entry.ratio * 100:.0f}%" if entry.size else "",
            _kind(entry.name),
            format_time(entry.mtime),
        ]

    item = _SortableItem(columns)
    item.setIcon(0, icon_for_name(entry.name, entry.is_dir))
    item.setData(0, Qt.UserRole, entry.path)
    item.setData(0, Qt.UserRole + 1, entry)

    # 숫자 열은 실제 값으로 정렬한다 ("9 KB" > "10 MB" 같은 사고 방지)
    item.setData(1, Qt.UserRole + 2, entry.size)
    item.setData(2, Qt.UserRole + 2, entry.compressed_size)
    item.setData(3, Qt.UserRole + 2, entry.ratio)
    item.sort_dir_first = entry.is_dir

    for column in (1, 2, 3):
        item.setTextAlignment(column, Qt.AlignRight | Qt.AlignVCenter)
    if entry.encrypted:
        item.setText(4, item.text(4) + " 🔒")
    return item


class _SortableItem(QTreeWidgetItem):
    """'..' 를 맨 위로, 폴더를 파일보다 위로 올리고 크기는 숫자로 비교한다."""

    sort_dir_first = False
    sort_first = False

    def __lt__(self, other: QTreeWidgetItem) -> bool:
        column = self.treeWidget().sortColumn() if self.treeWidget() else 0

        # 상위 폴더로 가는 항목은 어떤 정렬에서도 첫 줄에 있어야 한다.
        if getattr(self, "sort_first", False) != getattr(other, "sort_first", False):
            return getattr(self, "sort_first", False)

        mine = getattr(self, "sort_dir_first", False)
        theirs = getattr(other, "sort_dir_first", False)
        if mine != theirs:
            return mine  # 폴더가 항상 먼저

        left = self.data(column, Qt.UserRole + 2)
        right = other.data(column, Qt.UserRole + 2)
        if left is not None and right is not None:
            return left < right
        return self.text(column).lower() < other.text(column).lower()


def _kind(name: str) -> str:
    suffix = Path(name).suffix.lower().lstrip(".")
    return f"{suffix.upper()} 파일" if suffix else "파일"
