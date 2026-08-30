"""Windows 확장자 연결과 탐색기 컨텍스트 메뉴 등록.

전부 HKEY_CURRENT_USER 아래에만 쓰므로 관리자 권한이 필요 없고,
제거할 때도 우리가 만든 키만 지우면 되어 깨끗하다.

컨텍스트 메뉴는 두 갈래로 등록한다.

* 압축 풀기 계열 -> ``SystemFileAssociations\\<확장자>\\shell``
  여기에 넣으면 기본 연결 프로그램이 MyZIP 이 아니어도 메뉴가 나온다.
  (다른 압축 프로그램을 기본으로 쓰는 사람도 우리 메뉴를 볼 수 있다.)
* 압축하기 계열 -> ``*\\shell`` 과 ``Directory\\shell``
  모든 파일과 폴더에 붙는다.

Windows 11 에서는 이렇게 등록한 메뉴가 우클릭 -> '추가 옵션 표시'
(Shift+F10) 안에 나타난다. 1차 메뉴에 넣으려면 MSIX 패키지와
IExplorerCommand COM 구현이 필요하다.
"""

from __future__ import annotations

import sys
import winreg
from dataclasses import dataclass
from pathlib import Path

from .. import APP_NAME

CLASSES = r"Software\Classes"
BACKUP_KEY = r"Software\MyZIP\Backup"
SETTINGS_KEY = r"Software\MyZIP"

# 확장자 -> (ProgID 접미사, 설명, 아이콘 파일 이름)
FILE_TYPES: dict[str, tuple[str, str, str]] = {
    ".zip":     ("zip", "ZIP 압축 파일", "zip"),
    ".tar":     ("tar", "TAR 아카이브", "tar"),
    ".tgz":     ("tgz", "TAR+GZIP 압축 파일", "tgz"),
    ".gz":      ("gz", "GZIP 압축 파일", "tgz"),
    ".bz2":     ("bz2", "BZIP2 압축 파일", "tgz"),
    ".xz":      ("xz", "XZ 압축 파일", "tgz"),
    ".tbz2":    ("tbz2", "TAR+BZIP2 압축 파일", "tgz"),
    ".txz":     ("txz", "TAR+XZ 압축 파일", "tgz"),
    ".rar":     ("rar", "RAR 압축 파일", "rar"),
    ".7z":      ("7z", "7Z 압축 파일", "7z"),
    ".cab":     ("cab", "CAB 압축 파일", "file"),
    ".arj":     ("arj", "ARJ 압축 파일", "file"),
    ".lzh":     ("lzh", "LZH 압축 파일", "file"),
    ".iso":     ("iso", "ISO 디스크 이미지", "file"),
}

#: 확장자 연결 기본 대상 (자주 쓰는 것만 켜 둔다)
DEFAULT_ASSOCIATIONS = (".zip", ".tar", ".tgz", ".rar", ".7z")


def _progid(ext: str) -> str:
    suffix = FILE_TYPES[ext][0]
    return f"{APP_NAME}.{suffix}"


def executable() -> str:
    """컨텍스트 메뉴에 넣을 실행 명령의 경로."""
    if getattr(sys, "frozen", False):
        return str(Path(sys.executable).resolve())
    # 소스에서 돌릴 때는 pythonw.exe 로 스크립트를 실행한다.
    launcher = Path(sys.executable).with_name("pythonw.exe")
    if not launcher.exists():
        launcher = Path(sys.executable)
    script = Path(__file__).resolve().parent.parent.parent / "myzip_app.py"
    return f'"{launcher}" "{script}"'


def _command(args: str) -> str:
    """레지스트리에 넣을 완전한 명령 문자열."""
    exe = executable()
    if exe.startswith('"'):
        return f"{exe} {args}"          # 이미 인용된 python 런처 형태
    return f'"{exe}" {args}'


def icons_dir() -> Path:
    """레지스트리에 적을 아이콘 폴더.

    PyInstaller onedir 빌드에서는 자원이 실행 파일 옆 _internal/ 에 풀린다.
    레지스트리에는 실제로 존재하는 절대 경로를 적어야 탐색기가 아이콘을
    그릴 수 있다.
    """
    from ..core.sevenzip import bundle_dir

    return bundle_dir() / "resources" / "icons"


def _icon(name: str) -> str:
    return f"{icons_dir() / (name + '.ico')},0"


# ---------------------------------------------------------------- 저수준 유틸


def _set(path: str, name: str | None, value: str) -> None:
    """HKCU 아래 키를 만들고 값을 쓴다."""
    with winreg.CreateKeyEx(winreg.HKEY_CURRENT_USER, path, 0, winreg.KEY_WRITE) as key:
        winreg.SetValueEx(key, name or "", 0, winreg.REG_SZ, value)


def _get(path: str, name: str | None = None) -> str | None:
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, path) as key:
            value, _ = winreg.QueryValueEx(key, name or "")
            return value
    except OSError:
        return None


def _delete_tree(path: str) -> None:
    """키와 하위 키를 전부 지운다. 없으면 조용히 넘어간다."""
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, path, 0,
                            winreg.KEY_READ | winreg.KEY_WRITE) as key:
            while True:
                try:
                    child = winreg.EnumKey(key, 0)
                except OSError:
                    break
                _delete_tree(f"{path}\\{child}")
        winreg.DeleteKey(winreg.HKEY_CURRENT_USER, path)
    except OSError:
        pass


def _key_exists(root: int, path: str) -> bool:
    try:
        winreg.OpenKey(root, path).Close()
        return True
    except OSError:
        return False


def notify_shell() -> None:
    """탐색기에 연결 정보가 바뀌었다고 알린다. 재부팅 없이 반영된다."""
    try:
        import ctypes

        SHCNE_ASSOCCHANGED = 0x08000000
        SHCNF_IDLIST = 0x0000
        ctypes.windll.shell32.SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None)
    except Exception:
        pass  # 알림 실패는 치명적이지 않다 (로그아웃하면 반영된다)


# ---------------------------------------------------------------- 확장자 연결


@dataclass
class AssocStatus:
    """확장자 하나의 현재 연결 상태."""

    ext: str
    registered: bool          # 우리 ProgID 가 연결되어 있는가
    blocked_by_userchoice: bool   # 사용자가 다른 앱을 기본으로 고정했는가
    current_owner: str = ""       # 지금 이 확장자를 가진 ProgID


def _userchoice_progid(ext: str) -> str | None:
    """Windows 8 이후 '기본 앱' 설정이 고정한 ProgID."""
    path = (r"Software\Microsoft\Windows\CurrentVersion\Explorer"
            rf"\FileExts\{ext}\UserChoice")
    return _get(path, "ProgId")


def register_filetype(ext: str) -> None:
    """ProgID(파일 종류 정의)를 만든다. 연결과는 별개다."""
    if ext not in FILE_TYPES:
        return
    _, description, icon = FILE_TYPES[ext]
    progid = _progid(ext)
    base = rf"{CLASSES}\{progid}"

    _set(base, None, description)
    _set(base, "FriendlyTypeName", description)
    _set(rf"{base}\DefaultIcon", None, _icon(icon))
    _set(rf"{base}\shell\open\command", None, _command('"%1"'))
    _set(rf"{base}\shell\open", "FriendlyAppName", APP_NAME)

    # ProgID 에도 압축 풀기 메뉴를 달아 둔다.
    _install_extract_menu(rf"{base}\shell")


def associate(ext: str) -> AssocStatus:
    """확장자를 MyZIP 에 연결한다."""
    register_filetype(ext)
    progid = _progid(ext)
    ext_key = rf"{CLASSES}\{ext}"

    # 기존 연결을 백업해 둔다 (해제할 때 되돌리기 위해)
    previous = _get(ext_key)
    if previous and previous != progid:
        _set(BACKUP_KEY, ext, previous)

    _set(ext_key, None, progid)
    # '연결 프로그램' 목록에 MyZIP 이 뜨도록
    _set(rf"{ext_key}\OpenWithProgids", progid, "")

    return status(ext)


def unassociate(ext: str) -> None:
    """연결을 해제하고 이전 프로그램으로 되돌린다."""
    progid = _progid(ext)
    ext_key = rf"{CLASSES}\{ext}"

    if _get(ext_key) == progid:
        previous = _get(BACKUP_KEY, ext)
        if previous:
            _set(ext_key, None, previous)
        else:
            # 원래 연결이 없었으면 기본값을 비운다
            _set(ext_key, None, "")

    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, rf"{ext_key}\OpenWithProgids",
                            0, winreg.KEY_WRITE) as key:
            winreg.DeleteValue(key, progid)
    except OSError:
        pass

    _delete_tree(rf"{CLASSES}\{progid}")
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, BACKUP_KEY, 0,
                            winreg.KEY_WRITE) as key:
            winreg.DeleteValue(key, ext)
    except OSError:
        pass


def status(ext: str) -> AssocStatus:
    """확장자 하나의 연결 상태를 조사한다."""
    progid = _progid(ext)
    current = _get(rf"{CLASSES}\{ext}") or ""
    choice = _userchoice_progid(ext)

    if choice:
        return AssocStatus(
            ext=ext,
            registered=choice == progid,
            blocked_by_userchoice=choice != progid,
            current_owner=choice,
        )
    return AssocStatus(
        ext=ext,
        registered=current == progid,
        blocked_by_userchoice=False,
        current_owner=current,
    )


def all_status() -> list[AssocStatus]:
    return [status(ext) for ext in FILE_TYPES]


# ------------------------------------------------------------ 컨텍스트 메뉴


def _install_extract_menu(shell_path: str) -> None:
    """압축 풀기 계단식 메뉴를 shell_path 아래에 만든다."""
    root = rf"{shell_path}\{APP_NAME}"
    _set(root, "MUIVerb", APP_NAME)
    _set(root, "Icon", _icon("app"))
    _set(root, "SubCommands", "")          # 빈 값 = 아래 shell 키를 하위 메뉴로

    items = (
        ("01open",     "MyZIP 으로 열기(&O)",        '"%1"',                  "app"),
        ("02here",     "여기에 압축 풀기(&H)",        '--extract-here "%1"',   "app"),
        ("03folder",   "압축 풀기 (폴더 생성)(&F)",   '--extract-to "%1"',     "app"),
        ("04dialog",   "압축 풀기...(&E)",           '--extract "%1"',        "app"),
        ("05test",     "무결성 검사(&T)",            '--test "%1"',           "app"),
    )
    for key, label, args, icon in items:
        path = rf"{root}\shell\{key}"
        _set(path, "MUIVerb", label)
        _set(path, "Icon", _icon(icon))
        _set(rf"{path}\command", None, _command(args))


def _install_compress_menu(shell_path: str) -> None:
    """압축하기 계단식 메뉴."""
    root = rf"{shell_path}\{APP_NAME}.compress"
    _set(root, "MUIVerb", f"{APP_NAME} 으로 압축")
    _set(root, "Icon", _icon("zip"))
    _set(root, "SubCommands", "")
    # 여러 개를 선택했을 때 한 번만 실행되도록 (하나의 아카이브로 묶기 위해)
    _set(root, "MultiSelectModel", "Player")

    items = (
        ("01dialog", "압축하기...(&A)",           '--compress "%1"',       "zip"),
        ("02zip",    "ZIP 으로 압축(&Z)",         '--compress-zip "%1"',   "zip"),
        ("03tgz",    "TAR.GZ 로 압축(&G)",        '--compress-tgz "%1"',   "tgz"),
    )
    for key, label, args, icon in items:
        path = rf"{root}\shell\{key}"
        _set(path, "MUIVerb", label)
        _set(path, "Icon", _icon(icon))
        _set(path, "MultiSelectModel", "Player")
        _set(rf"{path}\command", None, _command(args))


def install_context_menu() -> None:
    """탐색기 컨텍스트 메뉴를 등록한다."""
    # 1) 아카이브 확장자에는 '압축 풀기' 메뉴를 붙인다.
    #    SystemFileAssociations 를 쓰면 기본 연결 프로그램과 무관하게 나온다.
    for ext in FILE_TYPES:
        _install_extract_menu(rf"{CLASSES}\SystemFileAssociations\{ext}\shell")

    # 2) 모든 파일과 폴더에는 '압축하기' 메뉴를 붙인다.
    _install_compress_menu(rf"{CLASSES}\*\shell")
    _install_compress_menu(rf"{CLASSES}\Directory\shell")

    notify_shell()


def uninstall_context_menu() -> None:
    for ext in FILE_TYPES:
        _delete_tree(rf"{CLASSES}\SystemFileAssociations\{ext}\shell\{APP_NAME}")
    _delete_tree(rf"{CLASSES}\*\shell\{APP_NAME}.compress")
    _delete_tree(rf"{CLASSES}\Directory\shell\{APP_NAME}.compress")
    notify_shell()


def context_menu_installed() -> bool:
    return _key_exists(winreg.HKEY_CURRENT_USER,
                       rf"{CLASSES}\*\shell\{APP_NAME}.compress")


# ------------------------------------------------------------- 앱 등록


def register_application() -> None:
    """'연결 프로그램' 목록과 기본 앱 설정에 MyZIP 이 보이도록 등록한다."""
    exe = executable()
    exe_name = Path(exe.strip('"').split('" "')[0]).name

    app_key = rf"{CLASSES}\Applications\{exe_name}"
    _set(app_key, "FriendlyAppName", APP_NAME)
    _set(rf"{app_key}\shell\open\command", None, _command('"%1"'))
    _set(rf"{app_key}\DefaultIcon", None, _icon("app"))
    for ext in FILE_TYPES:
        _set(rf"{app_key}\SupportedTypes", ext, "")


def install_all(extensions=DEFAULT_ASSOCIATIONS) -> list[AssocStatus]:
    """설치 시 한 번에 처리하는 묶음."""
    register_application()
    install_context_menu()
    results = [associate(ext) for ext in extensions]
    notify_shell()
    return results


def uninstall_all() -> None:
    """제거 시 우리가 만든 모든 흔적을 지운다."""
    uninstall_context_menu()
    for ext in list(FILE_TYPES):
        unassociate(ext)
    exe_name = Path(executable().strip('"').split('" "')[0]).name
    _delete_tree(rf"{CLASSES}\Applications\{exe_name}")
    _delete_tree(SETTINGS_KEY)
    notify_shell()


def open_default_apps_settings() -> None:
    """Windows '기본 앱' 설정 화면을 연다.

    UserChoice 가 걸려 있으면 프로그램이 기본 앱을 바꿀 수 없다.
    (Windows 8 부터 의도적으로 막아 두었다.) 사용자가 직접 골라야 한다.
    """
    import subprocess

    subprocess.Popen(["cmd", "/c", "start", "", "ms-settings:defaultapps"],
                     creationflags=0x08000000)
