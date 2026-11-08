//! MyZIP 실행 파일.
//!
//!     MyZIP.exe <파일>                 아카이브를 창으로 열기
//!     MyZIP.exe --extract-here <파일>  같은 폴더에 바로 풀기
//!     MyZIP.exe --extract-to <파일>    이름 폴더를 만들어 풀기
//!     MyZIP.exe --test <파일>          무결성 검사
//!     MyZIP.exe --compress-zip <대상...>
//!     MyZIP.exe --compress-tgz <대상...>
//!     MyZIP.exe --register / --unregister   셸 통합 설치/제거
//!
//! 콘솔 없는 exe 로 빌드한다. 탐색기 우클릭으로 실행할 때 검은 창이
//! 번쩍이면 안 되기 때문이다. 터미널에서 실행하면 부모 콘솔에 글자를 찍는다.

#![cfg_attr(not(test), windows_subsystem = "windows")]

mod cli;
mod config;
mod console;
mod gui;
mod instance;
mod jobs;
mod notify;
mod registry;
mod regpath;

use cli::Command;
use myzip_core::{APP_NAME, VERSION};
use notify::Kind;

fn main() {
    console::attach();
    std::process::exit(run());
}

fn run() -> i32 {
    let command = match cli::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(message) => {
            notify::show(
                Kind::Error,
                "명령줄 오류",
                &format!("{message}\n\n도움말: MyZIP --help"),
            );
            return 2;
        }
    };

    match command {
        Command::Help => {
            console::say(&cli::help_text());
            0
        }
        Command::Version => {
            console::say(&format!("{APP_NAME} {VERSION}"));
            0
        }
        Command::Register => shell_task(
            "확장자 연결과 탐색기 메뉴를 등록했습니다.",
            || registry::install_all(registry::DEFAULT_ASSOCIATIONS).map(drop),
        ),
        Command::RegisterMenu => shell_task("탐색기 메뉴를 등록했습니다.", || {
            registry::register_application()?;
            registry::install_context_menu()
        }),
        Command::Unregister => shell_task(
            "확장자 연결과 탐색기 메뉴를 제거했습니다.",
            registry::uninstall_all,
        ),
        Command::List(request) => jobs::list(&request),
        Command::Run(request) => {
            // 여러 파일을 골랐을 때 흩어진 프로세스를 하나로 모은다.
            // '열기' 는 파일마다 창이 따로 뜨는 것이 자연스러우므로 모으지 않는다.
            let requests = if request.mode == cli::Mode::Open {
                vec![request]
            } else {
                match instance::gather(request) {
                    Some(requests) => requests,
                    None => return 0, // 이미 떠 있는 창에 넘겼다
                }
            };
            gui::dispatch(requests)
        }
    }
}

/// 설치/제거. 설치 프로그램이 조용히 부르므로 성공은 콘솔에만 찍는다.
fn shell_task(done: &str, task: impl FnOnce() -> std::io::Result<()>) -> i32 {
    match task() {
        Ok(()) => {
            console::say(&format!("{APP_NAME}: {done}"));
            0
        }
        Err(err) => {
            notify::show(
                Kind::Error,
                "레지스트리 오류",
                &format!("레지스트리에 쓰지 못했습니다: {err}"),
            );
            1
        }
    }
}
