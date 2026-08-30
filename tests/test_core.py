"""코어 압축/해제 동작 검증."""

from __future__ import annotations

import shutil
import struct
import sys
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from paths import work_dir  # noqa: E402
from myzip.core import (  # noqa: E402
    Progress,
    format_of,
    open_archive,
    walk_inputs,
    writer_for,
)
from myzip.core.encoding import detect_archive_encoding  # noqa: E402
from myzip.core.path_safety import safe_join  # noqa: E402

WORK = work_dir("myzip-tests")
PASSED: list[str] = []
FAILED: list[str] = []


def check(name: str, ok: bool, detail: str = "") -> None:
    (PASSED if ok else FAILED).append(name)
    mark = "PASS" if ok else "FAIL"
    print(f"[{mark}] {name}" + (f"  -- {detail}" if detail and not ok else ""))


def make_source() -> Path:
    """한글 이름과 폴더 구조를 포함한 원본 트리를 만든다."""
    src = WORK / "원본자료"
    if src.exists():
        shutil.rmtree(src)
    (src / "문서" / "하위폴더").mkdir(parents=True)
    (src / "readme.txt").write_text("hello world\n", encoding="utf-8")
    (src / "문서" / "보고서 2026.txt").write_text("한글 내용입니다.\n" * 100, encoding="utf-8")
    (src / "문서" / "하위폴더" / "深い.dat").write_bytes(bytes(range(256)) * 500)
    (src / "big.bin").write_bytes(b"A" * (3 * 1024 * 1024))
    return src


def tree_snapshot(root: Path) -> dict[str, int]:
    """상대경로 -> 크기 (폴더는 -1)."""
    out: dict[str, int] = {}
    for p in sorted(root.rglob("*")):
        rel = p.relative_to(root).as_posix()
        out[rel] = -1 if p.is_dir() else p.stat().st_size
    return out


def roundtrip(src: Path, archive_name: str, password: str | None = None) -> None:
    archive = WORK / archive_name
    archive.unlink(missing_ok=True)

    items = list(walk_inputs([src]))
    with writer_for(archive, level=6, password=password) as w:
        w.write(items, Progress())

    check(f"{archive_name} 생성", archive.exists() and archive.stat().st_size > 0)

    dest = WORK / f"out-{archive_name.replace('.', '_')}"
    if dest.exists():
        shutil.rmtree(dest)

    with open_archive(archive, password) as reader:
        entries = reader.entries
        reader.extract(dest, progress=Progress())

    before = tree_snapshot(src)
    after = tree_snapshot(dest / src.name)
    check(f"{archive_name} 왕복 일치", before == after,
          f"\n  before={before}\n  after ={after}")

    names = {e.path for e in entries}
    check(f"{archive_name} 한글 파일명 보존",
          any("보고서 2026.txt" in n for n in names), str(sorted(names)))


def test_cp949_zip() -> None:
    """CP949 로 기록된 구형 ZIP 의 파일명을 제대로 읽는지."""
    archive = WORK / "legacy_cp949.zip"
    archive.unlink(missing_ok=True)

    korean = ["한글파일.txt", "문서/보고서.hwp", "사진/여름 휴가.jpg"]

    # zipfile 은 non-ASCII 이름을 만나면 UTF-8 플래그를 강제로 켜 버린다.
    # 진짜 구형 아카이브를 흉내내려면 같은 길이의 ASCII 이름으로 쓴 다음
    # 파일 안의 이름 바이트를 CP949 바이트로 바꿔치기해야 한다.
    mapping: dict[bytes, bytes] = {}
    with zipfile.ZipFile(archive, "w") as zf:
        for i, name in enumerate(korean):
            raw = name.encode("cp949")
            placeholder = f"N{i:03d}".ljust(len(raw), "x")[: len(raw)]
            assert len(placeholder.encode("ascii")) == len(raw)
            mapping[placeholder.encode("ascii")] = raw
            zf.writestr(placeholder, b"data")

    blob = archive.read_bytes()
    for placeholder, raw in mapping.items():
        blob = blob.replace(placeholder, raw)
    archive.write_bytes(blob)

    # 바꿔치기가 제대로 됐는지 확인: UTF-8 플래그가 꺼져 있어야 한다.
    with zipfile.ZipFile(archive) as zf:
        flags = [i.flag_bits & 0x800 for i in zf.infolist()]
    check("CP949 픽스처가 구형 ZIP 인지", not any(flags), f"flags={flags}")

    with open_archive(archive) as reader:
        paths = [e.path for e in reader.entries]

    check("CP949 ZIP 파일명 복원", paths == korean, f"got={paths}")
    check("CP949 자동 판별", reader.encoding == "cp949", f"got={reader.encoding}")


def test_zipslip() -> None:
    """경로 탈출을 시도하는 ZIP 이 상위 폴더를 오염시키지 않는지."""
    archive = WORK / "evil.zip"
    archive.unlink(missing_ok=True)
    with zipfile.ZipFile(archive, "w") as zf:
        zf.writestr("../../pwned.txt", b"x")
        zf.writestr("normal.txt", b"ok")

    dest = WORK / "slip" / "inner"
    if (WORK / "slip").exists():
        shutil.rmtree(WORK / "slip")
    dest.mkdir(parents=True)

    with open_archive(archive) as reader:
        reader.extract(dest, progress=Progress())

    escaped = (WORK / "pwned.txt").exists() or (WORK / "slip" / "pwned.txt").exists()
    check("Zip Slip 차단", not escaped and (dest / "pwned.txt").exists())


def test_format_sniff() -> None:
    """확장자가 거짓일 때도 매직 바이트로 알아보는지."""
    archive = WORK / "원본자료.zip"
    fake = WORK / "actually_a_zip.tar"
    shutil.copy(archive, fake)
    check("매직 바이트 판별", format_of(fake) == "zip", f"got={format_of(fake)}")


def test_strip_root() -> None:
    """최상위 폴더 하나뿐이면 벗겨내는 옵션."""
    archive = WORK / "원본자료.zip"
    dest = WORK / "stripped"
    if dest.exists():
        shutil.rmtree(dest)
    with open_archive(archive) as reader:
        reader.extract(dest, progress=Progress(), strip_root=True)
    check("중복 폴더 벗기기", (dest / "readme.txt").exists(),
          str(sorted(p.name for p in dest.iterdir())))


def test_password() -> None:
    """AES 암호 ZIP 이 암호 없이는 안 열리는지."""
    from myzip.core import PasswordRequired

    archive = WORK / "secret.zip"
    src = WORK / "원본자료" / "readme.txt"
    archive.unlink(missing_ok=True)
    with writer_for(archive, level=6, password="비밀1234") as w:
        w.write([(src, "readme.txt")], Progress())

    with open_archive(archive, "비밀1234") as reader:
        ok = reader.read_bytes(reader.entries[0]) == src.read_bytes()
    check("암호 ZIP 정상 해제", ok)

    blocked = False
    try:
        with open_archive(archive, "틀린암호") as reader:
            reader.read_bytes(reader.entries[0])
    except Exception:
        blocked = True
    check("틀린 암호 거부", blocked)


def test_cancel() -> None:
    """진행 중 취소가 실제로 멈추는지."""
    from myzip.core import OperationCancelled

    archive = WORK / "원본자료.zip"
    dest = WORK / "cancelled"
    if dest.exists():
        shutil.rmtree(dest)

    progress = Progress()
    progress.callback = lambda p: p.cancel() if p.done_bytes > 0 else None

    stopped = False
    try:
        with open_archive(archive) as reader:
            reader.extract(dest, progress=progress)
    except OperationCancelled:
        stopped = True
    check("작업 취소 동작", stopped)


def test_single_compressed_file() -> None:
    """tar 로 감싸지 않은 단일 .gz/.bz2/.xz 도 열려야 한다."""
    import bz2
    import gzip
    import lzma

    from myzip.core.tar_handler import SingleFileReader

    payload = ("단일 파일 압축 내용\n" * 300).encode("utf-8")
    cases = (("보고서.txt.gz", gzip), ("로그.txt.bz2", bz2), ("덤프.sql.xz", lzma))

    for name, module in cases:
        archive = WORK / name
        with module.open(archive, "wb") as fh:
            fh.write(payload)

        with open_archive(archive) as reader:
            entries = reader.entries
            dest = WORK / f"single-{archive.suffix.lstrip('.')}"
            if dest.exists():
                shutil.rmtree(dest)
            reader.extract(dest, progress=Progress())
            used_single = isinstance(reader, SingleFileReader)

        inner = dest / entries[0].path
        check(f"{name} 단일 파일로 인식", used_single)
        check(f"{name} 내부 이름 복원",
              entries[0].path == name[: name.rindex(".")], entries[0].path)
        check(f"{name} 내용 일치",
              inner.exists() and inner.read_bytes() == payload)


def main() -> int:
    WORK.mkdir(parents=True, exist_ok=True)
    src = make_source()

    for name in ("원본자료.zip", "원본자료.tar", "원본자료.tgz",
                 "원본자료.tar.bz2", "원본자료.tar.xz"):
        roundtrip(src, name)

    test_cp949_zip()
    test_zipslip()
    test_format_sniff()
    test_strip_root()
    test_password()
    test_cancel()
    test_single_compressed_file()

    print()
    print(f"통과 {len(PASSED)} / 실패 {len(FAILED)}")
    if FAILED:
        print("실패 항목:", ", ".join(FAILED))
    return 1 if FAILED else 0


if __name__ == "__main__":
    raise SystemExit(main())
