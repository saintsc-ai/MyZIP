"""압축 해제 시 경로 탈출(Zip Slip) 방지 및 경로 유틸."""

from __future__ import annotations

import re
from pathlib import Path, PurePosixPath

# Windows 에서 파일명에 쓸 수 없는 문자
_ILLEGAL = re.compile(r'[<>:"|?*\x00-\x1f]')

# Windows 예약 장치명
_RESERVED = {
    "CON", "PRN", "AUX", "NUL",
    *(f"COM{i}" for i in range(1, 10)),
    *(f"LPT{i}" for i in range(1, 10)),
}


def sanitize_component(name: str) -> str:
    """경로 조각 하나를 Windows 에서 안전한 이름으로 바꾼다."""
    name = _ILLEGAL.sub("_", name).rstrip(" .")
    if not name:
        return "_"
    stem = name.split(".", 1)[0].upper()
    if stem in _RESERVED:
        name = "_" + name
    return name


def safe_join(base: Path, member: str) -> Path:
    """아카이브 내부 경로를 base 아래로만 떨어지도록 붙인다.

    상위 이동, 절대경로, 드라이브 문자, UNC 경로를 모두 무력화한다.
    """
    base = base.resolve()
    normalized = member.replace("\\", "/")

    parts: list[str] = []
    for part in PurePosixPath(normalized).parts:
        if part in ("", ".", "/"):
            continue
        if part == "..":
            # 상위로 올라가려는 시도는 통째로 버린다
            continue
        if len(part) == 2 and part[1] == ":":   # C: 같은 드라이브 지정
            continue
        parts.append(sanitize_component(part))

    if not parts:
        parts = ["_"]

    result = base.joinpath(*parts)

    # 심볼릭 링크 등으로 여전히 벗어날 수 있으므로 최종 확인
    try:
        resolved = result.resolve()
    except OSError:
        resolved = result
    if base != resolved and base not in resolved.parents:
        raise ValueError(f"안전하지 않은 경로입니다: {member}")
    return result


def unique_path(path: Path) -> Path:
    """이미 있는 경로면 (2), (3) 을 붙여 비어있는 경로를 찾는다."""
    if not path.exists():
        return path
    stem, suffix, parent = path.stem, path.suffix, path.parent
    for i in range(2, 10000):
        candidate = parent / f"{stem} ({i}){suffix}"
        if not candidate.exists():
            return candidate
    raise OSError(f"사용 가능한 이름을 찾지 못했습니다: {path}")


def format_size(nbytes: float) -> str:
    """사람이 읽는 크기 문자열."""
    if nbytes < 1024:
        return f"{int(nbytes)} B"
    for unit in ("KB", "MB", "GB", "TB"):
        nbytes /= 1024.0
        if nbytes < 1024 or unit == "TB":
            return f"{nbytes:,.1f} {unit}"
    return f"{nbytes:,.1f} TB"
