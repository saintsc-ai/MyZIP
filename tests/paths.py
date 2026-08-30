"""테스트가 쓸 작업 폴더.

경로를 특정 드라이브에 박아 두면 다른 PC 나 CI 에서 돌지 않는다.
기본값은 시스템 임시 폴더이고, MYZIP_TEST_DIR 로 바꿀 수 있다.
"""

from __future__ import annotations

import os
import tempfile
from pathlib import Path


def work_dir(name: str) -> Path:
    """테스트별 작업 폴더를 만들어 돌려준다."""
    base = os.environ.get("MYZIP_TEST_DIR") or tempfile.gettempdir()
    path = Path(base) / name
    path.mkdir(parents=True, exist_ok=True)
    return path
