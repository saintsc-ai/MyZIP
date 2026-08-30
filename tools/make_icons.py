"""MyZIP 아이콘 생성기.

외부 이미지 에셋에 의존하지 않도록 QPainter 로 직접 그려서
resources/icons/ 아래에 .ico / .png 를 만든다.
빌드 전에 한 번만 돌리면 되고, 결과물은 저장소에 커밋한다.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

# offscreen 플랫폼은 폰트 데이터베이스가 비어 있어 글자가 두부(□)로 나온다.
# 창을 띄우지는 않지만 실제 플랫폼 플러그인을 써야 시스템 폰트를 쓸 수 있다.
os.environ.pop("QT_QPA_PLATFORM", None)

from PySide6.QtCore import QPointF, QRectF, Qt  # noqa: E402
from PySide6.QtGui import (  # noqa: E402
    QBrush,
    QColor,
    QFont,
    QGuiApplication,
    QImage,
    QLinearGradient,
    QPainter,
    QPainterPath,
    QPen,
)

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "resources" / "icons"

# 포맷별 색상. 탐색기에서 한눈에 구분되도록 확실히 다른 색을 쓴다.
PALETTES = {
    "app":  ("#3B82F6", "#1D4ED8", ""),
    "zip":  ("#F5B94A", "#E08A0B", "ZIP"),
    "tar":  ("#6EC46E", "#2E9E4F", "TAR"),
    "tgz":  ("#4FB8A8", "#1E8A7C", "TGZ"),
    "rar":  ("#B47CE6", "#7C3AC7", "RAR"),
    "7z":   ("#6B8CC7", "#33569E", "7Z"),
    "file": ("#9AA4B2", "#5D6673", ""),
}

ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)


def draw_icon(size: int, light: str, dark: str, label: str) -> QImage:
    """상자 모양 아이콘 하나를 그린다."""
    image = QImage(size, size, QImage.Format_ARGB32)
    image.fill(Qt.transparent)

    p = QPainter(image)
    p.setRenderHint(QPainter.Antialiasing)
    p.setRenderHint(QPainter.TextAntialiasing)

    s = size / 256.0  # 256px 기준으로 그리고 배율만 적용
    p.scale(s, s)

    body = QRectF(26, 30, 204, 196)
    radius = 26.0
    lid_h = 62.0

    # 본체 (아래쪽이 어두운 그라데이션)
    gradient = QLinearGradient(QPointF(0, body.top()), QPointF(0, body.bottom()))
    gradient.setColorAt(0.0, QColor(light))
    gradient.setColorAt(1.0, QColor(dark))
    body_path = QPainterPath()
    body_path.addRoundedRect(body, radius, radius)
    p.fillPath(body_path, QBrush(gradient))

    # 뚜껑: 본체 윗부분을 밝게 덮어 상자 느낌을 준다.
    # 클리핑으로 위쪽 모서리만 둥글게 유지한다.
    p.save()
    p.setClipPath(body_path)
    lid_rect = QRectF(body.left(), body.top(), body.width(), lid_h)
    lid_gradient = QLinearGradient(QPointF(0, lid_rect.top()), QPointF(0, lid_rect.bottom()))
    lid_gradient.setColorAt(0.0, QColor(light).lighter(125))
    lid_gradient.setColorAt(1.0, QColor(light).lighter(105))
    p.fillRect(lid_rect, QBrush(lid_gradient))

    # 뚜껑과 본체 경계선
    p.setPen(QPen(QColor(0, 0, 0, 45), 3))
    p.drawLine(QPointF(body.left(), lid_rect.bottom()),
               QPointF(body.right(), lid_rect.bottom()))
    p.restore()

    # 걸쇠: 뚜껑 경계에 걸쳐 있어야 '잠긴 상자'로 읽힌다.
    p.setPen(Qt.NoPen)
    p.setBrush(QBrush(QColor(255, 255, 255, 240)))
    p.drawRoundedRect(QRectF(104, 22, 48, 86), 12, 12)
    p.setBrush(QBrush(QColor(dark).darker(115)))
    p.drawRoundedRect(QRectF(119, 56, 18, 34), 9, 9)

    # 포맷 이름. 작은 크기에서는 글자가 뭉개지므로 아예 뺀다.
    if label and size >= 48:
        font = QFont("Segoe UI", 58, QFont.Black)
        font.setLetterSpacing(QFont.PercentageSpacing, 94)
        p.setFont(font)
        # 살짝 어두운 그림자를 깔면 밝은 배경색에서도 글자가 읽힌다.
        p.setPen(QPen(QColor(0, 0, 0, 60)))
        p.drawText(QRectF(26, 132, 204, 86).translated(0, 3), Qt.AlignCenter, label)
        p.setPen(QPen(QColor(255, 255, 255, 252)))
        p.drawText(QRectF(26, 132, 204, 86), Qt.AlignCenter, label)

    p.end()
    return image


def _png_bytes(image: QImage) -> bytes:
    """QImage 를 PNG 바이트로."""
    from PySide6.QtCore import QBuffer, QByteArray

    data = QByteArray()
    buf = QBuffer(data)
    buf.open(QBuffer.WriteOnly)
    if not image.save(buf, "PNG"):
        raise SystemExit("PNG 인코딩 실패")
    buf.close()
    return bytes(data)


def write_ico(path: Path, frames: list[QImage]) -> None:
    """여러 해상도를 담은 ICO 를 직접 기록한다.

    Qt 의 ICO 플러그인은 이미지 하나만 받는데, 256px 한 장짜리 아이콘은
    탐색기가 16px 로 줄일 때 뭉개진다. ICO 컨테이너 자체는 단순하므로
    각 크기를 PNG 로 압축해 직접 담는다. (Vista 이후 Windows 는 ICO 안의
    PNG 프레임을 그대로 읽는다.)
    """
    import struct

    blobs = [_png_bytes(img) for img in frames]

    header = struct.pack("<HHH", 0, 1, len(frames))  # reserved, type=icon, count
    offset = len(header) + len(frames) * 16           # 디렉터리 항목 하나당 16바이트

    directory = b""
    for img, blob in zip(frames, blobs):
        w = 0 if img.width() >= 256 else img.width()   # 256 은 0 으로 적는다
        h = 0 if img.height() >= 256 else img.height()
        directory += struct.pack(
            "<BBBBHHII",
            w, h,
            0,      # 색상 팔레트 없음
            0,      # reserved
            1,      # color planes
            32,     # bits per pixel
            len(blob),
            offset,
        )
        offset += len(blob)

    path.write_bytes(header + directory + b"".join(blobs))


def save_ico(name: str, light: str, dark: str, label: str) -> Path:
    """여러 해상도를 담은 .ico 와 미리보기 .png 를 쓴다."""
    OUT.mkdir(parents=True, exist_ok=True)

    ico_path = OUT / f"{name}.ico"
    frames = [draw_icon(size, light, dark, label) for size in ICO_SIZES]
    write_ico(ico_path, frames)

    (OUT / f"{name}.png").parent.mkdir(parents=True, exist_ok=True)
    frames[-1].save(str(OUT / f"{name}.png"), "PNG")
    return ico_path


def main() -> int:
    app = QGuiApplication(sys.argv)  # noqa: F841 - QPainter 에 필요
    for name, (light, dark, label) in PALETTES.items():
        path = save_ico(name, light, dark, label)
        print(f"생성: {path.relative_to(ROOT)}  ({path.stat().st_size:,} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
