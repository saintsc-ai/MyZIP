//! 사용자에게 알리기: 콘솔이 있으면 글자로, 없으면(탐색기에서 실행) 메시지 상자로.

use std::path::Path;
use std::process::Command;

use myzip_core::APP_NAME;

use crate::console;

/// 탐색기 창 열기와 메시지 상자를 끄는 환경 변수. 자동 테스트용이다.
pub const ENV_NO_SHELL: &str = "MYZIP_NO_SHELL";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Info,
    Warning,
    Error,
}

fn quiet() -> bool {
    std::env::var_os(ENV_NO_SHELL).is_some()
}

/// 결과 하나를 알린다.
pub fn show(kind: Kind, title: &str, message: &str) {
    if console::has_output() || quiet() {
        let line = format!("{title}: {message}");
        match kind {
            Kind::Info => console::say(&line),
            Kind::Warning | Kind::Error => console::error(&line),
        }
        return;
    }
    message_box(kind, title, message);
}

#[cfg(windows)]
fn message_box(kind: Kind, title: &str, message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
        MessageBoxW,
    };
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let caption = wide(&format!("{APP_NAME} - {title}"));
    let text = wide(message);
    let icon = match kind {
        Kind::Info => MB_ICONINFORMATION,
        Kind::Warning => MB_ICONWARNING,
        Kind::Error => MB_ICONERROR,
    };
    // 창이 없는 프로세스라 다른 창 뒤에 숨기 쉽다. 맨 앞으로 띄운다.
    // SAFETY: 두 문자열 모두 null 로 끝나고 호출이 끝날 때까지 살아 있다.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | icon | MB_SETFOREGROUND | MB_TOPMOST,
        );
    }
}

#[cfg(not(windows))]
fn message_box(_kind: Kind, title: &str, message: &str) {
    console::error(&format!("{title}: {message}"));
}

/// 연결된 프로그램으로 연다 (파일, 또는 ms-settings: 같은 주소).
pub fn shell_open(target: &std::ffi::OsStr) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let file: Vec<u16> = target.encode_wide().chain(Some(0)).collect();
    let verb: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
    // SAFETY: 문자열은 모두 null 로 끝나고 호출이 끝날 때까지 살아 있다.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // 32 보다 크면 성공이다 (Win32 의 오래된 약속).
    result as usize > 32
}

/// 탐색기에서 폴더를 연다. 파일이면 그 파일을 선택한 채로 연다.
pub fn open_in_explorer(path: &Path) {
    if quiet() {
        return;
    }
    let mut cmd = Command::new("explorer");
    if path.is_dir() {
        cmd.arg(path);
    } else {
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path);
        cmd.arg(arg);
    }
    let _ = cmd.spawn();
}
