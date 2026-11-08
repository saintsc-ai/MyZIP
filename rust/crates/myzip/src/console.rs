//! 콘솔 출력.
//!
//! 이 프로그램은 콘솔 없는(windows subsystem) exe 다. 탐색기에서 실행될 때
//! 검은 창이 번쩍이지 않게 하려는 것이다. 대신 터미널에서 실행하면 부모
//! 콘솔에 붙어서 도움말과 결과를 찍는다.
//!
//! Python 판은 콘솔 인코딩(CP949/CP1252)에 따라 한글을 찍다가 죽은 적이 있다.
//! Rust 표준 출력은 콘솔이면 WriteConsoleW(UTF-16)로, 파이프면 UTF-8 로 쓰므로
//! 인코딩 때문에 실패하지 않는다. 그래도 출력 실패는 모두 무시한다.

use std::io::Write;

#[cfg(windows)]
fn handle_valid(kind: u32) -> bool {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::GetStdHandle;
    // SAFETY: 표준 핸들 조회는 부작용이 없다.
    let handle = unsafe { GetStdHandle(kind) };
    !handle.is_null() && handle != INVALID_HANDLE_VALUE
}

/// 표준 출력이 없으면 부모 프로세스의 콘솔에 붙는다.
pub fn attach() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{
            ATTACH_PARENT_PROCESS, AttachConsole, STD_OUTPUT_HANDLE,
        };
        if !handle_valid(STD_OUTPUT_HANDLE) {
            // SAFETY: 실패해도(부모에게 콘솔이 없으면) 아무 일도 일어나지 않는다.
            unsafe {
                AttachConsole(ATTACH_PARENT_PROCESS);
            }
        }
    }
}

/// 결과를 글자로 받아 줄 곳(콘솔, 파이프, 파일)이 있는지.
/// 없으면 탐색기에서 실행된 것이므로 메시지 상자로 알려야 한다.
pub fn has_output() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::STD_ERROR_HANDLE;
        handle_valid(STD_ERROR_HANDLE)
    }
    #[cfg(not(windows))]
    {
        true
    }
}

pub fn say(message: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{message}");
    let _ = out.flush();
}

pub fn error(message: &str) {
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "{message}");
    let _ = err.flush();
}
