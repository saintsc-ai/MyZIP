"""창을 띄우고 화면을 캡처한다. 디자인 확인용."""

from __future__ import annotations

import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from PySide6.QtCore import QTimer  # noqa: E402
from PySide6.QtWidgets import QApplication  # noqa: E402

from myzip.core import open_archive  # noqa: E402
from myzip.ui.main_window import MainWindow  # noqa: E402

OUT = Path(os.environ.get("MYZIP_SHOT_DIR")
           or Path(__file__).resolve().parent.parent / "docs" / "images")


def _sample_archive() -> Path:
    """문서용 스크린샷에 쓸 예제 아카이브를 만든다.

    특정 PC 의 임시 폴더에 의존하지 않도록 매번 새로 만든다.
    """
    import shutil
    import tempfile

    from myzip.core import Progress, walk_inputs, writer_for

    base = Path(os.environ.get("MYZIP_TEST_DIR") or tempfile.gettempdir())
    base = base / "myzip-shots"
    src = base / "원본자료"
    if src.exists():
        shutil.rmtree(src)

    (src / "문서" / "하위폴더").mkdir(parents=True)
    (src / "readme.txt").write_text("hello world\n", encoding="utf-8")
    (src / "문서" / "보고서 2026.txt").write_text(
        "한글 내용입니다.\n" * 100, encoding="utf-8")
    (src / "문서" / "하위폴더" / "深い.dat").write_bytes(bytes(range(256)) * 500)
    (src / "big.bin").write_bytes(b"A" * (3 * 1024 * 1024))

    archive = base / "원본자료.zip"
    archive.unlink(missing_ok=True)
    with writer_for(archive, level=6) as writer:
        writer.write(list(walk_inputs([src])), Progress())
    return archive


def load_sync(window: MainWindow, archive: Path, password: str | None = None) -> None:
    """진행 대화상자 없이 아카이브를 창에 채운다."""
    reader = open_archive(archive, password)
    reader.entries
    window.reader = reader
    window.archive_path = archive
    window.password = password
    window._index_entries()
    window._sync_encoding_box()
    window._populate_tree()
    window.navigate("")
    window.setWindowTitle(f"{archive.name} - MyZIP")
    window._update_actions()


def grab(widget, name: str) -> Path:
    QApplication.processEvents()
    OUT.mkdir(parents=True, exist_ok=True)
    path = OUT / f"myzip-{name}.png"
    # QPixmap.save 는 실패해도 예외를 던지지 않고 False 만 돌려준다.
    if not widget.grab().save(str(path)):
        raise SystemExit(f"스크린샷 저장 실패: {path}")
    print(f"{path}  ({widget.width()}x{widget.height()})")
    return path


def main() -> int:
    app = QApplication(sys.argv)

    shots = sys.argv[1:] or ["main", "empty", "compress", "extract", "settings"]
    archive = _sample_archive()

    if "empty" in shots:
        window = MainWindow()
        window.resize(1000, 620)
        window.show()
        QApplication.processEvents()
        grab(window, "empty")
        window.close()

    if "main" in shots:
        window = MainWindow()
        window.resize(1000, 620)
        load_sync(window, archive)
        window.show()
        QApplication.processEvents()
        grab(window, "main")
        # 하위 폴더로 들어간 모습
        window.navigate("원본자료/문서")
        QApplication.processEvents()
        grab(window, "main-subfolder")
        window.close()

    if "compress" in shots:
        from myzip.ui.compress_dialog import CompressDialog

        dialog = CompressDialog([archive.parent / archive.stem])
        dialog.setStyleSheet(__import__(
            "myzip.ui.common", fromlist=["APP_STYLE"]).APP_STYLE)
        dialog.show()
        QApplication.processEvents()
        grab(dialog, "compress")
        dialog.close()

    if "extract" in shots:
        from myzip.ui.common import APP_STYLE
        from myzip.ui.extract_dialog import ExtractDialog

        dialog = ExtractDialog(archive, selection_count=3)
        dialog.setStyleSheet(APP_STYLE)
        dialog.show()
        QApplication.processEvents()
        grab(dialog, "extract")
        dialog.close()

    if "settings" in shots:
        from myzip.ui.common import APP_STYLE
        from myzip.ui.settings_dialog import SettingsDialog

        dialog = SettingsDialog()
        dialog.setStyleSheet(APP_STYLE)
        dialog.show()
        QApplication.processEvents()
        grab(dialog, "settings")
        dialog.close()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
