"""툴바 동작 아이콘 생성기.

포맷 아이콘(상자)을 툴바에 그대로 쓰면 버튼이 전부 똑같아 보인다.
동작마다 알아볼 수 있는 글리프를 따로 그린다.
밝은 테마와 어두운 테마 양쪽에서 읽히도록 채도가 있는 색을 쓰고
흰 테두리를 두르지 않는다.
"""

from __future__ import annotations

import os
import struct
import sys
from pathlib import Path

os.environ.pop("QT_QPA_PLATFORM", None)

from PySide6.QtCore import QBuffer, QByteArray, QPointF, QRectF, Qt  # noqa: E402
from PySide6.QtGui import (  # noqa: E402
    QBrush,
    QColor,
    QGuiApplication,
    QImage,
    QPainter,
    QPainterPath,
    QPen,
)

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "resources" / "icons"
SIZES = (16, 20, 24, 32, 48, 64)

BLUE = "#3B82F6"
DARK_BLUE = "#1D4ED8"
AMBER = "#F59E0B"
GREEN = "#22A559"
GRAY = "#6B7280"
RED = "#DC2626"


def _box(p: QPainter, color: str, rect: QRectF, radius: float = 8.0) -> None:
    """압축 상자 실루엣."""
    p.setPen(Qt.NoPen)
    p.setBrush(QBrush(QColor(color)))
    p.drawRoundedRect(rect, radius, radius)
    # 뚜껑 경계
    p.setPen(QPen(QColor(0, 0, 0, 55), 3))
    y = rect.top() + rect.height() * 0.30
    p.drawLine(QPointF(rect.left(), y), QPointF(rect.right(), y))
    # 걸쇠
    p.setPen(Qt.NoPen)
    p.setBrush(QBrush(QColor(255, 255, 255, 235)))
    w = rect.width() * 0.17
    p.drawRoundedRect(
        QRectF(rect.center().x() - w / 2, rect.top() - 2, w, rect.height() * 0.42),
        3, 3,
    )


def _arrow(p: QPainter, color: str, x: float, top: float, bottom: float,
           width: float) -> None:
    """아래를 향하는 굵은 화살표."""
    shaft = width * 0.34
    head = width * 0.5
    head_h = (bottom - top) * 0.45

    path = QPainterPath()
    path.moveTo(x - shaft / 2, top)
    path.lineTo(x + shaft / 2, top)
    path.lineTo(x + shaft / 2, bottom - head_h)
    path.lineTo(x + head, bottom - head_h)
    path.lineTo(x, bottom)
    path.lineTo(x - head, bottom - head_h)
    path.lineTo(x - shaft / 2, bottom - head_h)
    path.closeSubpath()

    p.setPen(Qt.NoPen)
    p.setBrush(QBrush(QColor(color)))
    p.drawPath(path)


def _plus(p: QPainter, color: str, cx: float, cy: float, size: float) -> None:
    p.setPen(Qt.NoPen)
    p.setBrush(QBrush(QColor(255, 255, 255)))
    p.drawEllipse(QPointF(cx, cy), size * 0.62, size * 0.62)
    p.setBrush(QBrush(QColor(color)))
    p.drawEllipse(QPointF(cx, cy), size * 0.52, size * 0.52)
    p.setPen(QPen(QColor(255, 255, 255), size * 0.20, Qt.SolidLine, Qt.RoundCap))
    p.drawLine(QPointF(cx - size * 0.28, cy), QPointF(cx + size * 0.28, cy))
    p.drawLine(QPointF(cx, cy - size * 0.28), QPointF(cx, cy + size * 0.28))


def _folder(p: QPainter, color: str, rect: QRectF) -> None:
    p.setPen(Qt.NoPen)
    # 뒤쪽 탭
    p.setBrush(QBrush(QColor(color).darker(115)))
    p.drawRoundedRect(
        QRectF(rect.left(), rect.top(), rect.width() * 0.52, rect.height() * 0.3),
        4, 4,
    )
    p.setBrush(QBrush(QColor(color)))
    p.drawRoundedRect(
        QRectF(rect.left(), rect.top() + rect.height() * 0.16,
               rect.width(), rect.height() * 0.84),
        7, 7,
    )


def draw_new(p: QPainter) -> None:
    _box(p, BLUE, QRectF(14, 20, 74, 66))
    _plus(p, GREEN, 78, 78, 22)


def draw_open(p: QPainter) -> None:
    _folder(p, AMBER, QRectF(10, 22, 84, 62))
    # 열린 앞면
    p.setPen(Qt.NoPen)
    p.setBrush(QBrush(QColor(AMBER).lighter(122)))
    path = QPainterPath()
    path.moveTo(14, 86)
    path.lineTo(30, 50)
    path.lineTo(102, 50)
    path.lineTo(86, 86)
    path.closeSubpath()
    p.drawPath(path)


def draw_extract(p: QPainter) -> None:
    _box(p, BLUE, QRectF(10, 12, 62, 56))
    _arrow(p, GREEN, 68, 50, 92, 30)


def draw_extract_here(p: QPainter) -> None:
    _box(p, BLUE, QRectF(14, 8, 58, 50))
    _arrow(p, GREEN, 50, 52, 78, 30)
    # 바닥선 = '여기'
    p.setPen(QPen(QColor(GREEN), 8, Qt.SolidLine, Qt.RoundCap))
    p.drawLine(QPointF(24, 90), QPointF(76, 90))


def draw_test(p: QPainter) -> None:
    # 방패
    path = QPainterPath()
    path.moveTo(50, 8)
    path.lineTo(88, 24)
    path.lineTo(88, 54)
    path.cubicTo(88, 76, 70, 90, 50, 96)
    path.cubicTo(30, 90, 12, 76, 12, 54)
    path.lineTo(12, 24)
    path.closeSubpath()
    p.setPen(Qt.NoPen)
    p.setBrush(QBrush(QColor(BLUE)))
    p.drawPath(path)
    # 체크
    p.setPen(QPen(QColor(255, 255, 255), 11, Qt.SolidLine, Qt.RoundCap, Qt.RoundJoin))
    p.drawLine(QPointF(30, 52), QPointF(44, 66))
    p.drawLine(QPointF(44, 66), QPointF(72, 36))


def draw_settings(p: QPainter) -> None:
    """톱니바퀴.

    별 모양 다각형으로 그리면 톱니가 뾰족해져 작은 크기에서 뭉개진다.
    몸통 원에 사각 톱니를 얹는 방식이 훨씬 또렷하다.
    """
    cx = cy = 50.0
    teeth = 8

    p.setPen(Qt.NoPen)
    p.setBrush(QBrush(QColor(GRAY)))

    # 톱니: 중심을 기준으로 회전시키며 둥근 사각형을 찍는다
    p.save()
    p.translate(cx, cy)
    for _ in range(teeth):
        p.drawRoundedRect(QRectF(-9, -47, 18, 26), 4, 4)
        p.rotate(360.0 / teeth)
    p.restore()

    # 몸통
    p.drawEllipse(QPointF(cx, cy), 32, 32)

    # 가운데 구멍 (배경이 비쳐야 하므로 지우기 합성)
    p.save()
    p.setCompositionMode(QPainter.CompositionMode_Clear)
    p.setBrush(QBrush(QColor(0, 0, 0)))
    p.drawEllipse(QPointF(cx, cy), 14, 14)
    p.restore()


GLYPHS = {
    "act-new": draw_new,
    "act-open": draw_open,
    "act-extract": draw_extract,
    "act-extract-here": draw_extract_here,
    "act-test": draw_test,
    "act-settings": draw_settings,
}


def render(draw, size: int) -> QImage:
    image = QImage(size, size, QImage.Format_ARGB32)
    image.fill(Qt.transparent)
    p = QPainter(image)
    p.setRenderHint(QPainter.Antialiasing)
    p.scale(size / 100.0, size / 100.0)   # 100x100 좌표계로 그린다
    draw(p)
    p.end()
    return image


def png_bytes(image: QImage) -> bytes:
    data = QByteArray()
    buf = QBuffer(data)
    buf.open(QBuffer.WriteOnly)
    image.save(buf, "PNG")
    buf.close()
    return bytes(data)


def write_ico(path: Path, frames: list[QImage]) -> None:
    blobs = [png_bytes(f) for f in frames]
    header = struct.pack("<HHH", 0, 1, len(frames))
    offset = len(header) + len(frames) * 16
    directory = b""
    for img, blob in zip(frames, blobs):
        w = 0 if img.width() >= 256 else img.width()
        h = 0 if img.height() >= 256 else img.height()
        directory += struct.pack("<BBBBHHII", w, h, 0, 0, 1, 32, len(blob), offset)
        offset += len(blob)
    path.write_bytes(header + directory + b"".join(blobs))


def main() -> int:
    app = QGuiApplication(sys.argv)  # noqa: F841
    OUT.mkdir(parents=True, exist_ok=True)
    for name, draw in GLYPHS.items():
        frames = [render(draw, s) for s in SIZES]
        write_ico(OUT / f"{name}.ico", frames)
        frames[-1].save(str(OUT / f"{name}.png"), "PNG")
        print(f"생성: {name}.ico")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
