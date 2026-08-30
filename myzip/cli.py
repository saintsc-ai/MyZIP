"""명령줄 진입점.

탐색기 컨텍스트 메뉴가 호출하는 얼굴이기도 하다.

    myzip.exe <파일>                 아카이브 열기
    myzip.exe --extract-here <파일>  같은 폴더에 바로 풀기
    myzip.exe --extract-to <파일>    이름 폴더를 만들어 풀기
    myzip.exe --extract <파일>       대화상자로 위치를 정해 풀기
    myzip.exe --test <파일>          무결성 검사
    myzip.exe --compress <대상...>   압축 대화상자
    myzip.exe --compress-zip <대상...>
    myzip.exe --compress-tgz <대상...>
    myzip.exe --register / --unregister   셸 통합 설치/제거

여러 파일을 선택하고 우클릭하면 Windows 가 명령을 여러 번 실행할 수도 있다.
그런 경우 하나로 합쳐 아카이브 한 개를 만들도록 QLocalServer 로
먼저 뜬 인스턴스에 경로를 넘긴다.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from . import APP_NAME, __version__

#: 여러 인스턴스가 서로를 찾을 때 쓰는 이름
IPC_NAME = "MyZIP.SingleInstance.v1"

#: 흩어져 들어온 경로를 모으고 기다리는 시간 (밀리초)
COLLECT_DELAY = 500


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="myzip",
        description=f"{APP_NAME} {__version__} - 압축 프로그램",
        add_help=True,
    )
    parser.add_argument("paths", nargs="*", help="아카이브 또는 압축할 대상")

    action = parser.add_mutually_exclusive_group()
    action.add_argument("--extract-here", action="store_true",
                        help="아카이브가 있는 폴더에 바로 풀기")
    action.add_argument("--extract-to", action="store_true",
                        help="아카이브 이름의 폴더를 만들어 풀기")
    action.add_argument("--extract", action="store_true",
                        help="압축 풀기 대화상자 열기")
    action.add_argument("--test", action="store_true",
                        help="무결성 검사")
    action.add_argument("--compress", action="store_true",
                        help="압축 대화상자 열기")
    action.add_argument("--compress-zip", action="store_true",
                        help="바로 ZIP 으로 압축")
    action.add_argument("--compress-tgz", action="store_true",
                        help="바로 TAR.GZ 로 압축")
    action.add_argument("--register", action="store_true",
                        help="확장자 연결과 탐색기 메뉴 설치")
    action.add_argument("--register-menu", action="store_true",
                        help="탐색기 메뉴만 설치 (확장자 연결은 건드리지 않음)")
    action.add_argument("--unregister", action="store_true",
                        help="확장자 연결과 탐색기 메뉴 제거")

    parser.add_argument("--password", "-p", default=None, help="아카이브 암호")
    parser.add_argument("--version", "-V", action="version",
                        version=f"{APP_NAME} {__version__}")
    return parser


def mode_of(args: argparse.Namespace) -> str:
    """인자에서 동작 이름 하나를 뽑아낸다."""
    for name in ("extract_here", "extract_to", "extract", "test",
                 "compress", "compress_zip", "compress_tgz"):
        if getattr(args, name):
            return name.replace("_", "-")
    return "open"


def run_headless(args: argparse.Namespace) -> int:
    """GUI 없이 끝나는 명령들 (설치/제거)."""
    from .shell import registry

    if args.register:
        registry.install_all()
        print(f"{APP_NAME}: 확장자 연결과 탐색기 메뉴를 등록했습니다.")
        return 0
    if args.register_menu:
        registry.register_application()
        registry.install_context_menu()
        print(f"{APP_NAME}: 탐색기 메뉴를 등록했습니다.")
        return 0
    if args.unregister:
        registry.uninstall_all()
        print(f"{APP_NAME}: 확장자 연결과 탐색기 메뉴를 제거했습니다.")
        return 0
    return -1  # 해당 없음


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    args = build_parser().parse_args(argv)

    headless = run_headless(args)
    if headless >= 0:
        return headless

    paths = [Path(p) for p in args.paths]
    from .app import run_gui

    return run_gui(mode_of(args), paths, args.password)


if __name__ == "__main__":
    raise SystemExit(main())
