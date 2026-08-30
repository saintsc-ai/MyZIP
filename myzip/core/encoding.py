"""아카이브 내부 파일명 인코딩 자동 판별.

ZIP 은 UTF-8 플래그(bit 11)가 없으면 파일명 바이트의 인코딩이 명시되지
않는다. 한국에서 만들어진 대부분의 ZIP 은 CP949(EUC-KR 확장), 일본은
CP932, 중국은 GBK 로 기록되어 있어서 그냥 읽으면 이름이 깨진다.
여기서는 원본 바이트를 후보 인코딩으로 디코딩해 보고 점수를 매겨
가장 그럴듯한 것을 고른다.
"""

from __future__ import annotations

import re
import unicodedata

# 우선순위 순서. 한국어 환경이므로 CP949 를 앞에 둔다.
CANDIDATES = ("utf-8", "cp949", "cp932", "gbk", "big5", "cp1252")

DISPLAY_NAMES = {
    "utf-8": "UTF-8",
    "cp949": "한국어 (CP949)",
    "cp932": "일본어 (CP932)",
    "gbk": "중국어 간체 (GBK)",
    "big5": "중국어 번체 (Big5)",
    "cp1252": "서유럽 (CP1252)",
    "cp437": "DOS (CP437)",
}

# 파일명에 나올 리 없는 제어문자
_CONTROL = re.compile(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]")

# CJK / 한글 / 가나 영역
_CJK_RANGES = (
    (0xAC00, 0xD7A3),   # 한글 음절
    (0x1100, 0x11FF),   # 한글 자모
    (0x3130, 0x318F),   # 호환 자모
    (0x4E00, 0x9FFF),   # 한중일 통합 한자
    (0x3040, 0x309F),   # 히라가나
    (0x30A0, 0x30FF),   # 가타카나
    (0xFF01, 0xFF60),   # 전각
)

# 잘못된 인코딩으로 풀었을 때 흔히 나오는 쓰레기 문자들
_MOJIBAKE = set("ÀÁÂÃÄÅÆÇÈÉÊËÌÍÎÏÐÑÒÓÔÕÖ×ØÙÚÛÜÝÞßàáâãäåæçèéêëìíîïðñòóôõö÷øùúûüýþÿ¿½¾¼±¶§¨©ª«¬®¯")


def _is_cjk(ch: str) -> bool:
    cp = ord(ch)
    return any(lo <= cp <= hi for lo, hi in _CJK_RANGES)


def score_decoded(text: str) -> float:
    """디코딩 결과가 '진짜 파일명'처럼 보이는 정도를 점수로 환산한다."""
    if not text:
        return 0.0

    score = 0.0
    for ch in text:
        if _CONTROL.match(ch):
            score -= 50.0
        elif ch in _MOJIBAKE:
            # 깨진 문자열의 전형적인 흔적
            score -= 4.0
        elif _is_cjk(ch):
            score += 3.0
        elif ch.isalnum() or ch in " ._-()[]{}#&+,'!~@$%^=;`":
            score += 1.0
        elif ch in '\\/:':
            score += 0.5
        else:
            cat = unicodedata.category(ch)
            if cat in ("Cn", "Co", "Cs"):     # 미할당 / 사용자영역 / 서로게이트
                score -= 20.0
            elif cat.startswith("C"):
                score -= 10.0
            else:
                score += 0.2
    return score / len(text)


def decode_best(raw: bytes, candidates=CANDIDATES) -> tuple[str, str]:
    """원본 바이트를 가장 그럴듯한 인코딩으로 디코딩한다.

    Returns:
        (디코딩된 문자열, 사용한 인코딩 이름)
    """
    if not raw:
        return "", "utf-8"

    # ASCII 뿐이면 어떤 인코딩이든 결과가 같다.
    if all(b < 0x80 for b in raw):
        return raw.decode("ascii"), "utf-8"

    best_text, best_enc, best_score = None, None, float("-inf")
    for enc in candidates:
        try:
            text = raw.decode(enc)
        except (UnicodeDecodeError, LookupError):
            continue
        s = score_decoded(text)
        # UTF-8 로 성공적으로 풀렸다면 우연일 확률이 매우 낮으므로 가산점
        if enc == "utf-8":
            s += 1.5
        if s > best_score:
            best_text, best_enc, best_score = text, enc, s

    if best_text is None:
        return raw.decode("cp437", errors="replace"), "cp437"
    return best_text, best_enc


def raw_from_cp437(name: str) -> bytes:
    """zipfile 이 cp437 로 잘못 디코딩한 이름에서 원본 바이트를 복원한다.

    Python 의 zipfile 은 UTF-8 플래그가 없는 엔트리를 cp437 로 디코딩한다.
    cp437 은 0x00~0xFF 전체가 문자에 1:1 대응하므로 되돌릴 수 있다.
    """
    try:
        return name.encode("cp437")
    except UnicodeEncodeError:
        return name.encode("utf-8", errors="replace")


def detect_archive_encoding(raw_names: list[bytes]) -> str:
    """아카이브 전체 파일명을 한꺼번에 보고 인코딩 하나를 결정한다.

    파일별로 따로 판별하면 같은 아카이브 안에서 인코딩이 뒤섞일 수 있으므로
    전체 합산 점수로 하나를 고른다.
    """
    non_ascii = [r for r in raw_names if any(b >= 0x80 for b in r)]
    if not non_ascii:
        return "utf-8"

    totals: dict[str, float] = {}
    for enc in CANDIDATES:
        total = 0.0
        ok = True
        for raw in non_ascii:
            try:
                text = raw.decode(enc)
            except (UnicodeDecodeError, LookupError):
                ok = False
                break
            total += score_decoded(text)
        if not ok:
            continue
        if enc == "utf-8":
            total += 1.5 * len(non_ascii)
        totals[enc] = total

    if not totals:
        return "cp437"
    return max(totals, key=totals.__getitem__)
