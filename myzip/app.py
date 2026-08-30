"""애플리케이션 부트스트랩과 동작 라우팅."""

from __future__ import annotations

import json
import sys
from pathlib import Path

from PySide6.QtCore import QTimer, Qt
from PySide6.QtNetwork import QLocalServer, QLocalSocket
from PySide6.QtWidgets import QApplication

from . import APP_NAME, __version__
from .cli import COLLECT_DELAY, IPC_NAME


def _make_app() -> QApplication:
    QApplication.setAttribute(Qt.AA_DontUseNativeMenuBar, False)
    app = QApplication(sys.argv)
    app.setApplicationName(APP_NAME)
    app.setApplicationVersion(__version__)
    app.setOrganizationName("MyZIP")
    app.setQuitOnLastWindowClosed(True)

    from .ui.common import app_icon

    app.setWindowIcon(app_icon())

    # 작업 표시줄에서 python.exe 가 아니라 MyZIP 으로 묶이게 한다.
    if sys.platform == "win32":
        try:
            import ctypes

            ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID(
                "MyZIP.Archiver.1"
            )
        except Exception:
            pass

    return app


class Session:
    """한 번의 실행에서 열리는 창들을 관리한다.

    탐색기에서 파일을 여러 개 선택해 '압축하기' 를 누르면 Windows 가
    프로세스를 여러 번 띄울 수 있다. 이때 먼저 뜬 인스턴스가 서버가 되어
    나머지가 보낸 경로를 모은 뒤, 잠깐 기다렸다가 한 번에 처리한다.
    """

    def __init__(self, app: QApplication):
        self.app = app
        self.windows: list = []
        self._server: QLocalServer | None = None
        self._pending_compress: list[Path] = []
        self._collect_timer = QTimer()
        self._collect_timer.setSingleShot(True)
        self._collect_timer.timeout.connect(self._flush_compress)

    # ------------------------------------------------------------ IPC

    def try_claim_server(self) -> bool:
        """서버 자리를 잡는다. 이미 다른 인스턴스가 있으면 False."""
        socket = QLocalSocket()
        socket.connectToServer(IPC_NAME)
        if socket.waitForConnected(120):
            socket.disconnectFromServer()
            return False

        # 비정상 종료로 남은 소켓 파일을 치운다.
        QLocalServer.removeServer(IPC_NAME)

        self._server = QLocalServer()
        self._server.setSocketOptions(QLocalServer.UserAccessOption)
        if not self._server.listen(IPC_NAME):
            self._server = None
            return False

        self._server.newConnection.connect(self._on_connection)
        return True

    def send_to_running(self, mode: str, paths: list[Path],
                        password: str | None) -> bool:
        """이미 떠 있는 인스턴스에 작업을 넘긴다. 성공하면 True."""
        socket = QLocalSocket()
        socket.connectToServer(IPC_NAME)
        if not socket.waitForConnected(400):
            return False

        payload = json.dumps({
            "mode": mode,
            "paths": [str(p) for p in paths],
            "password": password,
        }).encode("utf-8")

        socket.write(payload)
        socket.flush()
        sent = socket.waitForBytesWritten(1000)
        socket.disconnectFromServer()
        return sent

    def _on_connection(self) -> None:
        socket = self._server.nextPendingConnection()
        if socket is None:
            return

        def read() -> None:
            data = bytes(socket.readAll())
            if not data:
                return
            try:
                message = json.loads(data.decode("utf-8"))
            except (ValueError, UnicodeDecodeError):
                return
            self.handle(
                message.get("mode", "open"),
                [Path(p) for p in message.get("paths", [])],
                message.get("password"),
                from_ipc=True,
            )
            socket.deleteLater()

        socket.readyRead.connect(read)
        socket.disconnected.connect(socket.deleteLater)

    # ------------------------------------------------------- 동작 라우팅

    def handle(self, mode: str, paths: list[Path], password: str | None,
               from_ipc: bool = False) -> None:
        paths = [p for p in paths if p.exists()]

        if mode in ("compress", "compress-zip", "compress-tgz"):
            # 여러 프로세스로 흩어져 들어온 경로를 모은다.
            self._pending_compress.extend(paths)
            self._pending_mode = mode
            self._collect_timer.start(COLLECT_DELAY)
            return

        if not paths:
            self._window().show()
            return

        for path in paths:
            self._do_archive_action(mode, path, password)

    def _flush_compress(self) -> None:
        paths, self._pending_compress = self._pending_compress, []
        # 중복 제거 (순서는 유지)
        seen: set[str] = set()
        unique = []
        for path in paths:
            key = str(path.resolve()).lower()
            if key not in seen:
                seen.add(key)
                unique.append(path)

        if not unique:
            self._finish_oneshot()
            return

        mode = getattr(self, "_pending_mode", "compress")
        window = self._window()

        if mode == "compress":
            # 대화상자를 띄우는 흐름이므로 창을 보여 준다.
            window.show()
            window.compress_dialog(unique)
        else:
            self._quick_compress(window, unique,
                                 ".zip" if mode == "compress-zip" else ".tar.gz")

        window.close()
        self._finish_oneshot()

    def _quick_compress(self, window, sources: list[Path], extension: str) -> None:
        """대화상자 없이 바로 압축한다 (컨텍스트 메뉴의 빠른 항목).

        성공했을 때 알림창을 띄우지 않는다. 탐색기에 파일이 생기는 것
        자체가 결과이고, 매번 확인을 누르게 하면 성가시기 때문이다.
        실패했을 때만 오류를 보여 준다.
        """
        from .config import get
        from .core.path_safety import unique_path
        from .ui.progress_dialog import run_task
        from .ui.workers import CompressTask

        first = sources[0]
        if len(sources) == 1:
            stem = first.name if first.is_dir() else first.stem
        else:
            stem = first.parent.name or "압축파일"

        target = unique_path(first.parent / f"{stem}{extension}")
        task = CompressTask(sources, target, int(get("compress/level", 6)))
        run_task(task, "압축하는 중", window)

    def _do_archive_action(self, mode: str, path: Path,
                           password: str | None) -> None:
        """컨텍스트 메뉴에서 온 한 방짜리 동작.

        메인 창은 진행 대화상자의 부모 노릇만 하고 화면에는 띄우지 않는다.
        '여기에 압축 풀기' 를 눌렀는데 탐색기 창이 덩달아 뜨면 성가시다.
        """
        from .config import get
        from .core import strip_archive_suffix

        window = self._window()

        if mode == "open":
            window.show()
            window.raise_()
            window.activateWindow()
            window.open_archive(path, password)
            return

        opened = window.open_archive(path, password)
        if opened:
            if mode == "extract-here":
                window._run_extract(
                    path.parent, None, get("extract/conflict", "rename"),
                    False, bool(get("extract/open_after", True)), quiet=True,
                )
            elif mode == "extract-to":
                window._run_extract(
                    path.parent / strip_archive_suffix(path.name), None,
                    get("extract/conflict", "rename"),
                    bool(get("extract/strip_redundant", True)),
                    bool(get("extract/open_after", True)), quiet=True,
                )
            elif mode == "extract":
                window.extract_dialog()
            elif mode == "test":
                window.test_archive()

        window.close()
        self._finish_oneshot()

    def _finish_oneshot(self) -> None:
        """창을 하나도 띄우지 않는 동작이 끝났으면 앱을 내린다."""
        if not any(w.isVisible() for w in self.windows):
            self.app.quit()

    # ------------------------------------------------------------ 창 관리

    def _window(self):
        """비어 있는 창이 있으면 재활용하고, 없으면 새로 만든다."""
        from .ui.main_window import MainWindow

        for window in self.windows:
            if window.reader is None and window.isVisible():
                return window

        window = MainWindow()
        window.closed.connect(lambda: self._forget(window))
        self.windows.append(window)
        return window

    def _forget(self, window) -> None:
        if window in self.windows:
            self.windows.remove(window)

    def _close_if_idle(self, window) -> None:
        """한 방에 끝나는 작업(여기에 풀기 등)이면 창을 남기지 않는다."""
        if window.reader is None:
            window.close()


def run_gui(mode: str, paths: list[Path], password: str | None = None) -> int:
    """GUI 를 띄우고 요청한 동작을 수행한다."""
    app = _make_app()
    session = Session(app)

    if not session.try_claim_server():
        # 이미 다른 인스턴스가 돌고 있다. 그쪽에 넘기고 조용히 빠진다.
        if session.send_to_running(mode, paths, password):
            return 0
        # 넘기지 못했으면 그냥 우리가 처리한다.

    QTimer.singleShot(0, lambda: session.handle(mode, paths, password))
    return app.exec()
