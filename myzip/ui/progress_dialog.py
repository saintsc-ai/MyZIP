"""작업 진행 상황 대화상자."""

from __future__ import annotations

import time

from PySide6.QtCore import Qt, QTimer
from PySide6.QtWidgets import (
    QDialog,
    QHBoxLayout,
    QLabel,
    QProgressBar,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from ..core import format_size
from .common import app_icon, elide


class ProgressDialog(QDialog):
    """작업 스레드 하나를 감시하며 진행률과 남은 시간을 보여준다.

    작업이 끝나면 스스로 닫힌다. 취소 버튼은 작업에 취소 신호를 보내고
    실제로 멈출 때까지 기다린다.
    """

    def __init__(self, task, title: str, parent: QWidget | None = None):
        super().__init__(parent)
        self.task = task
        self._started = time.monotonic()
        self._finished = False

        self.setWindowTitle(title)
        self.setWindowIcon(app_icon())
        self.setModal(True)
        self.setMinimumWidth(460)
        # 도움말(?) 버튼과 닫기 버튼을 없애 강제 종료를 막는다.
        self.setWindowFlags(
            (self.windowFlags() | Qt.CustomizeWindowHint)
            & ~Qt.WindowContextHelpButtonHint
            & ~Qt.WindowCloseButtonHint
        )

        self._headline = QLabel(title)
        font = self._headline.font()
        font.setPointSize(font.pointSize() + 1)
        font.setBold(True)
        self._headline.setFont(font)

        self._current = QLabel("준비 중...")
        self._current.setTextFormat(Qt.PlainText)

        self._bar = QProgressBar()
        self._bar.setRange(0, 100)

        self._stats = QLabel(" ")
        self._stats.setStyleSheet("color: palette(mid);")

        self._cancel = QPushButton("취소")
        self._cancel.clicked.connect(self._on_cancel)

        buttons = QHBoxLayout()
        buttons.addStretch(1)
        buttons.addWidget(self._cancel)

        layout = QVBoxLayout(self)
        layout.setContentsMargins(18, 16, 18, 14)
        layout.setSpacing(9)
        layout.addWidget(self._headline)
        layout.addWidget(self._current)
        layout.addWidget(self._bar)
        layout.addWidget(self._stats)
        layout.addLayout(buttons)

        task.progressed.connect(self._on_progress)
        task.finished_ok.connect(self._on_done)
        task.failed.connect(lambda _msg: self._on_done(None))
        task.password_required.connect(lambda _msg: self._on_done(None))
        task.cancelled.connect(lambda: self._on_done(None))

        # 화면 갱신을 초당 10회로 제한해 UI 스레드를 아끼다.
        self._pending: tuple | None = None
        self._timer = QTimer(self)
        self._timer.timeout.connect(self._flush)
        self._timer.start(100)

    # -- 진행 상황 -----------------------------------------------------

    def _on_progress(self, percent: int, current: str, done: int, total: int) -> None:
        self._pending = (percent, current, done, total)

    def _flush(self) -> None:
        if self._pending is None:
            return
        percent, current, done, total = self._pending
        self._pending = None

        self._bar.setValue(percent)
        self._current.setText(elide(current, 62) if current else "...")

        elapsed = time.monotonic() - self._started
        parts = []
        if total:
            parts.append(f"{done:,} / {total:,} 개")
        if percent > 2 and elapsed > 1.0:
            remaining = elapsed * (100 - percent) / percent
            parts.append(f"남은 시간 약 {_duration(remaining)}")
        parts.append(f"경과 {_duration(elapsed)}")
        self._stats.setText("　·　".join(parts))

    # -- 종료 ---------------------------------------------------------

    def _on_cancel(self) -> None:
        self._cancel.setEnabled(False)
        self._cancel.setText("취소하는 중...")
        self._current.setText("작업을 정리하고 있습니다...")
        self.task.cancel()

    def _on_done(self, _result=None) -> None:
        self._finished = True
        self._timer.stop()
        self.accept()

    def closeEvent(self, event) -> None:
        if not self._finished:
            # 작업이 도는 중에는 창을 닫는 대신 취소로 해석한다.
            event.ignore()
            self._on_cancel()
        else:
            event.accept()

    def keyPressEvent(self, event) -> None:
        if event.key() == Qt.Key_Escape:
            self._on_cancel()
            return
        super().keyPressEvent(event)


def _duration(seconds: float) -> str:
    seconds = int(max(0, seconds))
    if seconds < 60:
        return f"{seconds}초"
    minutes, secs = divmod(seconds, 60)
    if minutes < 60:
        return f"{minutes}분 {secs}초"
    hours, minutes = divmod(minutes, 60)
    return f"{hours}시간 {minutes}분"


#: 암호가 필요해 중단되었을 때 결과 자리에 들어가는 표식
PASSWORD_NEEDED = object()


def run_task(task, title: str, parent: QWidget | None = None,
             show_error: bool = True) -> tuple[bool, object]:
    """작업을 돌리고 진행 대화상자를 띄운다.

    Returns:
        (성공 여부, 결과). 실패하거나 취소되면 (False, None).
        암호가 필요해 멈춘 경우에는 (False, PASSWORD_NEEDED) 를 돌려주어
        호출한 쪽이 오류창 대신 암호 입력을 띄울 수 있게 한다.
    """
    from .common import error

    outcome: dict = {"ok": False, "result": None, "error": None}

    task.finished_ok.connect(lambda r: outcome.update(ok=True, result=r))
    task.failed.connect(lambda m: outcome.update(ok=False, error=m))
    task.password_required.connect(
        lambda _m: outcome.update(ok=False, result=PASSWORD_NEEDED)
    )

    dialog = ProgressDialog(task, title, parent)
    task.start()
    dialog.exec()
    task.wait(5000)

    if outcome["error"] and show_error:
        error(parent, outcome["error"])
    return outcome["ok"], outcome["result"]
