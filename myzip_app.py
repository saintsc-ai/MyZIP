#!/usr/bin/env python
"""MyZIP 실행 스크립트.

소스에서 바로 돌릴 때와 PyInstaller 로 묶을 때의 공통 진입점이다.
컨텍스트 메뉴 등록도 이 파일 경로를 가리킨다.
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from myzip.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main())
