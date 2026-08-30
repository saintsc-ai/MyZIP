"""python -m myzip 로 실행할 때의 진입점."""

from .cli import main

if __name__ == "__main__":
    raise SystemExit(main())
