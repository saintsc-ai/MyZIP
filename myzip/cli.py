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


def make_output_safe() -> None:
    """콘솔 인코딩 때문에 프로그램이 죽지 않게 한다.

    한글 메시지를 찍을 때 stdout 이 UTF-8 이 아니면 UnicodeEncodeError 가
    난다. 한국어 Windows 콘솔(CP949)에서는 멀쩡하지만, 출력이 파이프나
    파일로 넘어가거나 다른 언어의 Windows 에서 돌리면 CP1252 가 잡혀
    한글을 인코딩하지 못한다.

    게다가 이 프로그램은 콘솔 없는(windowed) 실행 파일로 배포된다.
    그런 빌드에서 처리되지 않은 예외가 나면 PyInstaller 가 모달 오류창을
    띄우는데, 아무도 누르지 않으므로 프로세스가 영원히 멈춘다.
    """
    for name in ("stdout", "stderr"):
        stream = getattr(sys, name, None)
        if stream is None:
            continue
        try:
            # 콘솔에 직접 찍을 때는 콘솔의 인코딩을 그대로 둔다.
            # UTF-8 로 바꿔 버리면 CP949 콘솔에서 오히려 한글이 깨진다.
            # 파이프나 파일로 넘어가는 경우에만 UTF-8 로 맞춘다.
            if stream.isatty():
                stream.reconfigure(errors="replace")
            else:
                stream.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, ValueError, OSError):
            pass  # 재설정할 수 없는 스트림이면 say() 가 알아서 막아 준다


def say(message: str) -> None:
    """콘솔 메시지 출력. 콘솔이 없거나 인코딩이 달라도 죽지 않는다.

    windowed 빌드에서는 sys.stdout 이 아예 None 일 수 있다.
    """
    stream = getattr(sys, "stdout", None)
    if stream is None:
        return
    try:
        stream.write(message + "\n")
        stream.flush()
    except (UnicodeEncodeError, OSError, ValueError):
        pass  # 알림 메시지일 뿐이다. 못 찍는다고 작업을 실패시킬 이유가 없다


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
        say(f"{APP_NAME}: 확장자 연결과 탐색기 메뉴를 등록했습니다.")
        return 0
    if args.register_menu:
        registry.register_application()
        registry.install_context_menu()
        say(f"{APP_NAME}: 탐색기 메뉴를 등록했습니다.")
        return 0
    if args.unregister:
        registry.uninstall_all()
        say(f"{APP_NAME}: 확장자 연결과 탐색기 메뉴를 제거했습니다.")
        return 0
    return -1  # 해당 없음


def main(argv: list[str] | None = None) -> int:
    make_output_safe()

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
