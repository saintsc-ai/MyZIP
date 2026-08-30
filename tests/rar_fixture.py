"""테스트용 최소 RAR 4.x 아카이브 생성기.

RAR '압축'은 만들 수 없지만(라이선스·독점 알고리즘), 무압축 저장(method 0x30)
방식의 RAR 컨테이너는 문서화된 헤더 구조만으로 조립할 수 있다.
해제 경로를 진짜 RAR 파일로 검증하기 위한 픽스처 전용 코드다.

RAR 4.x 블록 구조
    marker      52 61 72 21 1A 07 00
    main head   HEAD_CRC(2) TYPE(1)=0x73 FLAGS(2) SIZE(2) RESERVED1(2) RESERVED2(4)
    file head   HEAD_CRC(2) TYPE(1)=0x74 FLAGS(2) SIZE(2) PACK_SIZE(4) UNP_SIZE(4)
                HOST_OS(1) FILE_CRC(4) FTIME(4) UNP_VER(1) METHOD(1)
                NAME_SIZE(2) ATTR(4) NAME(NAME_SIZE)
    file data   PACK_SIZE 바이트
    end block   TYPE(1)=0x7B

HEAD_CRC 는 TYPE 부터 헤더 끝까지의 CRC32 하위 16비트다.
"""

from __future__ import annotations

import struct
import zlib
from datetime import datetime
from pathlib import Path

MARKER = bytes([0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x00])

TYPE_MAIN = 0x73
TYPE_FILE = 0x74
TYPE_END = 0x7B

FLAG_LONG_BLOCK = 0x8000     # PACK_SIZE(ADD_SIZE) 필드가 있음
METHOD_STORE = 0x30
HOST_WIN32 = 2
UNP_VER_20 = 20

ATTR_FILE = 0x20             # FILE_ATTRIBUTE_ARCHIVE
ATTR_DIR = 0x10              # FILE_ATTRIBUTE_DIRECTORY
FLAG_DIRECTORY = 0x00E0      # 디렉터리는 사전 크기 비트가 모두 1


def _dos_time(dt: datetime) -> int:
    return (
        ((dt.year - 1980) << 25) | (dt.month << 21) | (dt.day << 16)
        | (dt.hour << 11) | (dt.minute << 5) | (dt.second // 2)
    )


def _with_crc(body: bytes) -> bytes:
    """TYPE 부터 시작하는 헤더 본문 앞에 HEAD_CRC 를 붙인다."""
    return struct.pack("<H", zlib.crc32(body) & 0xFFFF) + body


def _main_header() -> bytes:
    body = struct.pack(
        "<BHHHI",
        TYPE_MAIN,
        0x0000,   # FLAGS
        13,       # HEAD_SIZE (CRC 2 + 본문 11)
        0,        # RESERVED1
        0,        # RESERVED2
    )
    return _with_crc(body)


def _file_header(name: str, data: bytes, is_dir: bool = False) -> bytes:
    encoded = name.replace("/", "\\").encode("cp949")
    head_size = 32 + len(encoded)

    flags = FLAG_LONG_BLOCK
    attr = ATTR_FILE
    if is_dir:
        flags |= FLAG_DIRECTORY
        attr = ATTR_DIR

    body = struct.pack(
        "<BHHIIBIIBBHI",
        TYPE_FILE,
        flags,
        head_size,
        len(data),                 # PACK_SIZE
        len(data),                 # UNP_SIZE
        HOST_WIN32,
        zlib.crc32(data) & 0xFFFFFFFF,
        _dos_time(datetime(2026, 8, 31, 12, 30, 0)),
        UNP_VER_20,
        METHOD_STORE,
        len(encoded),
        attr,
    ) + encoded

    return _with_crc(body) + data


def _end_header() -> bytes:
    body = struct.pack("<BHH", TYPE_END, 0x4000, 7)
    return _with_crc(body)


def build_rar(path: Path, entries: dict[str, bytes],
              directories: tuple[str, ...] = ()) -> Path:
    """무압축 RAR 4.x 아카이브를 만든다.

    Args:
        entries: 아카이브 내부 경로 -> 내용
        directories: 명시적으로 넣을 폴더 경로들
    """
    blocks = [MARKER, _main_header()]
    for folder in directories:
        blocks.append(_file_header(folder, b"", is_dir=True))
    for name, data in entries.items():
        blocks.append(_file_header(name, data))
    blocks.append(_end_header())

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"".join(blocks))
    return path


if __name__ == "__main__":
    import sys
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from paths import work_dir

    target = work_dir("myzip-rar-tests") / "조립.rar"
    build_rar(
        target,
        {
            "샘플/읽어보기.txt": "한글 내용 테스트\n".encode("utf-8") * 40,
            "샘플/안쪽폴더/이진.dat": bytes(range(256)) * 300,
        },
        directories=("샘플", "샘플/안쪽폴더"),
    )
    print(f"만듦: {target}  ({target.stat().st_size:,} bytes)")
