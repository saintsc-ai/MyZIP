//! Windows 확장자 연결과 탐색기 컨텍스트 메뉴 등록.
//!
//! Python 판 shell/registry.py 와 키 이름·구조가 같다. 어느 판으로 설치했든
//! 다른 판의 --unregister 로 깨끗이 지울 수 있다.
//!
//! 컨텍스트 메뉴는 두 갈래로 등록한다.
//!
//! * 압축 풀기 계열 -> `SystemFileAssociations\<확장자>\shell`
//!   여기에 넣으면 기본 연결 프로그램이 MyZIP 이 아니어도 메뉴가 나온다.
//! * 압축하기 계열 -> `*\shell` 과 `Directory\shell`
//!   모든 파일과 폴더에 붙는다.
//!
//! 여러 파일을 골라 메뉴를 누르면 탐색기는 파일마다 프로세스를 하나씩 띄운다.
//! 먼저 뜬 프로세스가 나머지 요청을 모아 처리한다 (`instance` 모듈).
//!
//! `MultiSelectModel=Player` 를 두 메뉴 모두에 단다. 이것이 없으면 Windows 는
//! 15개를 넘게 골랐을 때 메뉴를 아예 숨긴다. Python 판은 압축 풀기 메뉴에
//! 이 값을 빠뜨려서 아카이브를 16개 이상 고르면 '압축 풀기'가 사라졌다.
//!
//! Windows 11 에서는 이렇게 등록한 메뉴가 우클릭 -> '추가 옵션 표시'
//! (Shift+F10) 안에 나타난다.

use std::io;
use std::path::{Path, PathBuf};

use winreg::RegKey;
use winreg::enums::{KEY_READ, KEY_WRITE};

use myzip_core::APP_NAME;

use crate::regpath;

pub const CLASSES: &str = r"Software\Classes";
pub const BACKUP_KEY: &str = r"Software\MyZIP\Backup";
pub const ROOT_KEY: &str = r"Software\MyZIP";

/// 확장자 -> (ProgID 접미사, 설명, 아이콘 파일 이름)
pub const FILE_TYPES: &[(&str, &str, &str, &str)] = &[
    (".zip", "zip", "ZIP 압축 파일", "zip"),
    (".tar", "tar", "TAR 아카이브", "tar"),
    (".tgz", "tgz", "TAR+GZIP 압축 파일", "tgz"),
    (".gz", "gz", "GZIP 압축 파일", "tgz"),
    (".bz2", "bz2", "BZIP2 압축 파일", "tgz"),
    (".xz", "xz", "XZ 압축 파일", "tgz"),
    (".tbz2", "tbz2", "TAR+BZIP2 압축 파일", "tgz"),
    (".txz", "txz", "TAR+XZ 압축 파일", "tgz"),
    (".rar", "rar", "RAR 압축 파일", "rar"),
    (".7z", "7z", "7Z 압축 파일", "7z"),
    (".cab", "cab", "CAB 압축 파일", "file"),
    (".arj", "arj", "ARJ 압축 파일", "file"),
    (".lzh", "lzh", "LZH 압축 파일", "file"),
    (".iso", "iso", "ISO 디스크 이미지", "file"),
];

/// 확장자 연결 기본 대상 (자주 쓰는 것만 켜 둔다).
pub const DEFAULT_ASSOCIATIONS: &[&str] = &[".zip", ".tar", ".tgz", ".rar", ".7z"];

const EXTRACT_ITEMS: &[(&str, &str, &str)] = &[
    ("01open", "MyZIP 으로 열기(&O)", "\"%1\""),
    ("02here", "여기에 압축 풀기(&H)", "--extract-here \"%1\""),
    (
        "03folder",
        "압축 풀기 (폴더 생성)(&F)",
        "--extract-to \"%1\"",
    ),
    ("04dialog", "압축 풀기...(&E)", "--extract \"%1\""),
    ("05test", "무결성 검사(&T)", "--test \"%1\""),
];

const COMPRESS_ITEMS: &[(&str, &str, &str, &str)] = &[
    ("01dialog", "압축하기...(&A)", "--compress \"%1\"", "zip"),
    ("02zip", "ZIP 으로 압축(&Z)", "--compress-zip \"%1\"", "zip"),
    (
        "03tgz",
        "TAR.GZ 로 압축(&G)",
        "--compress-tgz \"%1\"",
        "tgz",
    ),
];

fn file_type(
    ext: &str,
) -> Option<&'static (&'static str, &'static str, &'static str, &'static str)> {
    FILE_TYPES.iter().find(|t| t.0 == ext)
}

fn progid(suffix: &str) -> String {
    format!("{APP_NAME}.{suffix}")
}

/// 컨텍스트 메뉴에 넣을 실행 파일.
pub fn executable() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| dunce_canonical(&p))
        .unwrap_or_else(|| PathBuf::from(format!("{APP_NAME}.exe")))
}

fn dunce_canonical(path: &Path) -> Option<PathBuf> {
    let canonical = path.canonicalize().ok()?;
    // \\?\C:\... 꼴은 탐색기가 명령으로 실행하지 못한다.
    let text = canonical.to_string_lossy();
    Some(match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => canonical,
    })
}

fn command(args: &str) -> String {
    format!("\"{}\" {args}", executable().display())
}

/// 아이콘 파일이 있는 폴더. 배포본은 exe 옆에, 개발 중에는 저장소의 resources/ 에 있다.
fn icons_dir() -> Option<PathBuf> {
    let exe = executable();
    let dir = exe.parent()?;
    dir.ancestors()
        .take(6)
        .flat_map(|d| {
            [
                d.join("resources").join("icons"),
                d.join("_internal").join("resources").join("icons"),
            ]
        })
        .find(|d| d.join("app.ico").is_file())
}

fn icon(name: &str) -> String {
    match icons_dir() {
        Some(dir) => format!("{},0", dir.join(format!("{name}.ico")).display()),
        // 아이콘 파일이 없으면 exe 에 박힌 첫 번째 아이콘을 쓴다.
        None => format!("{},0", executable().display()),
    }
}

// ---------------------------------------------------------------- 저수준 유틸

fn set(path: &str, name: &str, value: &str) -> io::Result<()> {
    let (key, _) = regpath::hkcu().create_subkey(regpath::path(path))?;
    key.set_value(name, &value)
}

fn get(path: &str, name: &str) -> Option<String> {
    regpath::hkcu()
        .open_subkey_with_flags(regpath::path(path), KEY_READ)
        .ok()?
        .get_value::<String, _>(name)
        .ok()
}

fn open_writable(path: &str) -> Option<RegKey> {
    regpath::hkcu()
        .open_subkey_with_flags(regpath::path(path), KEY_READ | KEY_WRITE)
        .ok()
}

fn delete_value(path: &str, name: &str) {
    if let Some(key) = open_writable(path) {
        let _ = key.delete_value(name);
    }
}

/// 키와 하위 키를 전부 지운다. 없으면 조용히 넘어간다.
fn delete_tree(path: &str) {
    let _ = regpath::hkcu().delete_subkey_all(regpath::path(path));
}

fn key_exists(path: &str) -> bool {
    regpath::hkcu()
        .open_subkey_with_flags(regpath::path(path), KEY_READ)
        .is_ok()
}

/// 바꾼 연결을 탐색기에 바로 반영한다.
pub fn refresh_shell() {
    notify_shell();
}

/// 탐색기에 연결 정보가 바뀌었다고 알린다. 재부팅 없이 반영된다.
///
/// SHCNF_FLUSHNOWAIT 를 주지 않으면 알림이 처리될 때까지 기다린다. 셸이 없거나
/// 응답하지 않는 환경(서비스 세션, CI 러너)에서는 몇 분씩 멈춘다. 알리기만 하고
/// 바로 돌아온다.
fn notify_shell() {
    if regpath::sandbox().is_some() {
        return;
    }
    use windows_sys::Win32::UI::Shell::{
        SHCNE_ASSOCCHANGED, SHCNF_FLUSHNOWAIT, SHCNF_IDLIST, SHChangeNotify,
    };
    // SAFETY: 포인터 인자는 SHCNE_ASSOCCHANGED 에서 쓰이지 않으므로 null 이어도 된다.
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as i32,
            SHCNF_IDLIST | SHCNF_FLUSHNOWAIT,
            std::ptr::null(),
            std::ptr::null(),
        );
    }
}

// ---------------------------------------------------------------- 확장자 연결

/// 확장자 하나의 현재 연결 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssocStatus {
    pub ext: String,
    /// 우리 ProgID 가 연결되어 있는가
    pub registered: bool,
    /// 사용자가 '기본 앱' 설정으로 다른 앱을 고정했는가
    pub blocked_by_userchoice: bool,
    /// 지금 이 확장자를 가진 ProgID
    pub current_owner: String,
}

fn userchoice_progid(ext: &str) -> Option<String> {
    get(
        &format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\{ext}\UserChoice"),
        "ProgId",
    )
}

/// ProgID(파일 종류 정의)를 만든다. 연결과는 별개다.
pub fn register_filetype(ext: &str) -> io::Result<()> {
    let Some(&(_, suffix, description, icon_name)) = file_type(ext) else {
        return Ok(());
    };
    let base = format!(r"{CLASSES}\{}", progid(suffix));
    set(&base, "", description)?;
    set(&base, "FriendlyTypeName", description)?;
    set(&format!(r"{base}\DefaultIcon"), "", &icon(icon_name))?;
    set(
        &format!(r"{base}\shell\open\command"),
        "",
        &command("\"%1\""),
    )?;
    set(&format!(r"{base}\shell\open"), "FriendlyAppName", APP_NAME)?;
    // ProgID 에도 압축 풀기 메뉴를 달아 둔다.
    install_extract_menu(&format!(r"{base}\shell"))
}

/// 확장자를 MyZIP 에 연결한다.
pub fn associate(ext: &str) -> io::Result<AssocStatus> {
    register_filetype(ext)?;
    let Some(&(_, suffix, _, _)) = file_type(ext) else {
        return Ok(status(ext));
    };
    let id = progid(suffix);
    let ext_key = format!(r"{CLASSES}\{ext}");

    // 기존 연결을 백업해 둔다 (해제할 때 되돌리기 위해)
    if let Some(previous) = get(&ext_key, "").filter(|p| !p.is_empty() && *p != id) {
        set(BACKUP_KEY, ext, &previous)?;
    }
    set(&ext_key, "", &id)?;
    // '연결 프로그램' 목록에 MyZIP 이 뜨도록
    set(&format!(r"{ext_key}\OpenWithProgids"), &id, "")?;
    Ok(status(ext))
}

/// 연결을 해제하고 이전 프로그램으로 되돌린다.
pub fn unassociate(ext: &str) -> io::Result<()> {
    let Some(&(_, suffix, _, _)) = file_type(ext) else {
        return Ok(());
    };
    let id = progid(suffix);
    let ext_key = format!(r"{CLASSES}\{ext}");

    if get(&ext_key, "").as_deref() == Some(id.as_str()) {
        let previous = get(BACKUP_KEY, ext).unwrap_or_default();
        set(&ext_key, "", &previous)?; // 원래 연결이 없었으면 기본값을 비운다
    }
    delete_value(&format!(r"{ext_key}\OpenWithProgids"), &id);
    delete_tree(&format!(r"{CLASSES}\{id}"));
    delete_value(BACKUP_KEY, ext);
    Ok(())
}

/// 확장자 하나의 연결 상태를 조사한다.
pub fn status(ext: &str) -> AssocStatus {
    let id = file_type(ext).map(|t| progid(t.1)).unwrap_or_default();
    if let Some(choice) = userchoice_progid(ext) {
        return AssocStatus {
            ext: ext.into(),
            registered: choice == id,
            blocked_by_userchoice: choice != id,
            current_owner: choice,
        };
    }
    let current = get(&format!(r"{CLASSES}\{ext}"), "").unwrap_or_default();
    AssocStatus {
        ext: ext.into(),
        registered: current == id,
        blocked_by_userchoice: false,
        current_owner: current,
    }
}

pub fn all_status() -> Vec<AssocStatus> {
    FILE_TYPES.iter().map(|t| status(t.0)).collect()
}

// ------------------------------------------------------------ 컨텍스트 메뉴

/// 압축 풀기 계단식 메뉴를 shell_path 아래에 만든다.
fn install_extract_menu(shell_path: &str) -> io::Result<()> {
    let root = format!(r"{shell_path}\{APP_NAME}");
    set(&root, "MUIVerb", APP_NAME)?;
    set(&root, "Icon", &icon("app"))?;
    set(&root, "SubCommands", "")?; // 빈 값 = 아래 shell 키를 하위 메뉴로
    set(&root, "MultiSelectModel", "Player")?;
    for (key, label, args) in EXTRACT_ITEMS {
        let path = format!(r"{root}\shell\{key}");
        set(&path, "MUIVerb", label)?;
        set(&path, "Icon", &icon("app"))?;
        set(&path, "MultiSelectModel", "Player")?;
        set(&format!(r"{path}\command"), "", &command(args))?;
    }
    Ok(())
}

/// 압축하기 계단식 메뉴.
fn install_compress_menu(shell_path: &str) -> io::Result<()> {
    let root = format!(r"{shell_path}\{APP_NAME}.compress");
    set(&root, "MUIVerb", &format!("{APP_NAME} 으로 압축"))?;
    set(&root, "Icon", &icon("zip"))?;
    set(&root, "SubCommands", "")?;
    set(&root, "MultiSelectModel", "Player")?;
    for (key, label, args, icon_name) in COMPRESS_ITEMS {
        let path = format!(r"{root}\shell\{key}");
        set(&path, "MUIVerb", label)?;
        set(&path, "Icon", &icon(icon_name))?;
        set(&path, "MultiSelectModel", "Player")?;
        set(&format!(r"{path}\command"), "", &command(args))?;
    }
    Ok(())
}

/// 탐색기 컨텍스트 메뉴를 등록한다.
pub fn install_context_menu() -> io::Result<()> {
    for (ext, ..) in FILE_TYPES {
        install_extract_menu(&format!(r"{CLASSES}\SystemFileAssociations\{ext}\shell"))?;
    }
    install_compress_menu(&format!(r"{CLASSES}\*\shell"))?;
    install_compress_menu(&format!(r"{CLASSES}\Directory\shell"))?;
    notify_shell();
    Ok(())
}

pub fn uninstall_context_menu() {
    for (ext, ..) in FILE_TYPES {
        delete_tree(&format!(
            r"{CLASSES}\SystemFileAssociations\{ext}\shell\{APP_NAME}"
        ));
    }
    delete_tree(&format!(r"{CLASSES}\*\shell\{APP_NAME}.compress"));
    delete_tree(&format!(r"{CLASSES}\Directory\shell\{APP_NAME}.compress"));
    notify_shell();
}

pub fn context_menu_installed() -> bool {
    key_exists(&format!(r"{CLASSES}\*\shell\{APP_NAME}.compress"))
}

// ------------------------------------------------------------- 앱 등록

fn exe_name() -> String {
    executable()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("{APP_NAME}.exe"))
}

/// '연결 프로그램' 목록과 기본 앱 설정에 MyZIP 이 보이도록 등록한다.
pub fn register_application() -> io::Result<()> {
    let app_key = format!(r"{CLASSES}\Applications\{}", exe_name());
    set(&app_key, "FriendlyAppName", APP_NAME)?;
    set(
        &format!(r"{app_key}\shell\open\command"),
        "",
        &command("\"%1\""),
    )?;
    set(&format!(r"{app_key}\DefaultIcon"), "", &icon("app"))?;
    for (ext, ..) in FILE_TYPES {
        set(&format!(r"{app_key}\SupportedTypes"), ext, "")?;
    }
    Ok(())
}

/// 설치 시 한 번에 처리하는 묶음.
pub fn install_all(extensions: &[&str]) -> io::Result<Vec<AssocStatus>> {
    register_application()?;
    install_context_menu()?;
    let results = extensions
        .iter()
        .map(|ext| associate(ext))
        .collect::<io::Result<Vec<_>>>()?;
    notify_shell();
    Ok(results)
}

/// 제거 시 우리가 만든 모든 흔적을 지운다.
pub fn uninstall_all() -> io::Result<()> {
    uninstall_context_menu();
    for (ext, ..) in FILE_TYPES {
        unassociate(ext)?;
    }
    delete_tree(&format!(r"{CLASSES}\Applications\{}", exe_name()));
    delete_tree(ROOT_KEY);
    notify_shell();
    Ok(())
}
