//! 창(GUI). egui / eframe.
//!
//! 창은 두 가지다.
//!
//! * 작업 창 (`batch`): 탐색기 메뉴에서 바로 뜬다. 양식 → 진행률 → 결과.
//! * 메인 창 (`browser`): 아카이브 안을 폴더처럼 돌아다닌다.
//!
//! 작업 창에서 '아카이브 열기' 를 누르면 같은 창이 메인 창으로 바뀐다.
//! 한 프로세스에 창은 하나다 (winit 의 이벤트 루프는 프로세스에 하나뿐이다).

mod batch;
mod browser;
mod forms;
mod settings;
mod style;
mod task;
mod widgets;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use eframe::egui;
use myzip_core::APP_NAME;

use crate::cli::{Mode, Request};
use crate::console;
use crate::jobs::{self, Job};
use crate::notify::{self, Kind};

/// 창이 무엇으로 시작하나.
pub enum Start {
    Jobs(Vec<Job>),
    CompressForm(Vec<PathBuf>),
    ExtractForm(Vec<PathBuf>, Option<String>),
    Browse(Option<PathBuf>, Option<String>),
}

/// 작업 창이 다음에 할 일.
pub enum Transition {
    Close,
    Browse(PathBuf),
}

// 프로세스에 하나만 만드는 값이라 크기 차이는 상관없다.
#[allow(clippy::large_enum_variant)]
enum Screen {
    Batch(batch::Batch),
    Browser(Box<browser::Browser>),
}

struct App {
    screen: Screen,
    /// 마지막으로 본 창 크기. 닫을 때 저장한다.
    size: Option<egui::Vec2>,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let transition = match &mut self.screen {
            Screen::Batch(batch) => {
                let mut transition = None;
                egui::CentralPanel::default().show(ui, |ui| transition = batch.ui(ui));
                transition
            }
            Screen::Browser(browser) => {
                if let Some(rect) = ctx.input(|i| i.viewport().inner_rect) {
                    self.size = Some(rect.size());
                }
                browser.ui(ui);
                None
            }
        };
        match transition {
            Some(Transition::Close) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Some(Transition::Browse(path)) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(browser_size()));
                ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(true));
                self.screen = Screen::Browser(Box::new(browser::Browser::new(Some(path), None)));
            }
            None => {}
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Screen::Browser(browser) = &mut self.screen {
            browser.cleanup();
            if let Some(size) = self.size {
                let _ = crate::config::set(
                    "view/size",
                    &format!("{},{}", size.x as i32, size.y as i32),
                );
            }
        }
    }
}

/// 메인 창 크기. 지난번에 쓰던 크기를 기억한다.
fn browser_size() -> egui::Vec2 {
    let saved = crate::config::get("view/size");
    let mut parts = saved
        .split(',')
        .filter_map(|p| p.trim().parse::<f32>().ok());
    match (parts.next(), parts.next()) {
        (Some(w), Some(h)) if w >= 640.0 && h >= 400.0 => egui::vec2(w, h),
        _ => egui::vec2(1000.0, 620.0),
    }
}

/// 창을 띄워 시작한다. 종료 코드를 돌려준다.
pub fn run(start: Start) -> i32 {
    let exit = Arc::new(AtomicI32::new(0));
    // 창을 띄우지 못하면(원격 세션에 OpenGL 이 없는 등) 창 없이라도 처리해야 한다.
    let fallback = match &start {
        Start::Jobs(jobs) => Some(jobs.clone()),
        _ => None,
    };

    let (size, resizable) = match &start {
        Start::Browse(..) => (browser_size(), true),
        Start::CompressForm(_) => (egui::vec2(600.0, 560.0), true),
        Start::ExtractForm(..) => (egui::vec2(560.0, 440.0), true),
        Start::Jobs(_) => (egui::vec2(500.0, 210.0), true),
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_inner_size(size)
        .with_min_inner_size([420.0, 180.0])
        .with_resizable(resizable)
        .with_drag_and_drop(true);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(widgets::APP_PNG) {
        viewport = viewport.with_icon(Arc::new(icon));
    }
    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        system_font_fallback: false,
        ..Default::default()
    };

    let app_exit = exit.clone();
    let result = eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            style::setup(&cc.egui_ctx);
            let screen = match start {
                Start::Jobs(jobs) => {
                    Screen::Batch(batch::Batch::jobs(&cc.egui_ctx, jobs, app_exit.clone()))
                }
                Start::CompressForm(sources) => {
                    Screen::Batch(batch::Batch::compress(sources, app_exit.clone()))
                }
                Start::ExtractForm(archives, password) => {
                    Screen::Batch(batch::Batch::extract(archives, password, app_exit.clone()))
                }
                Start::Browse(archive, password) => {
                    Screen::Browser(Box::new(browser::Browser::new(archive, password)))
                }
            };
            Ok(Box::new(App { screen, size: None }))
        }),
    );

    match result {
        Ok(()) => exit.load(Ordering::SeqCst),
        Err(err) => match fallback {
            Some(jobs) => run_headless(jobs),
            None => {
                notify::show(Kind::Error, "창을 띄우지 못했습니다", &err.to_string());
                1
            }
        },
    }
}

fn run_headless(jobs: Vec<Job>) -> i32 {
    let outcomes: Vec<jobs::Outcome> = jobs
        .iter()
        .map(|j| jobs::run_job(j, &mut myzip_core::Progress::new()))
        .collect();
    let summary = jobs::Summary::new(&outcomes);
    if let Some(message) = summary.failure_report() {
        notify::show(Kind::Error, "오류", &message);
    }
    if let Some((kind, title, message)) = summary.test_report() {
        notify::show(kind, title, &message);
    }
    summary.exit_code()
}

/// 창이 필요한 동작인지.
pub fn needs_window(request: &Request) -> bool {
    matches!(
        request.mode,
        Mode::Open | Mode::ExtractDialog | Mode::CompressDialog
    ) || request.paths.is_empty()
}

/// 같은 동작을 새 프로세스로 다시 띄운다 (한 프로세스에는 창이 하나뿐이라서).
fn relaunch(request: &Request) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut cmd = std::process::Command::new(exe);
    let flag = match request.mode {
        Mode::Open => None,
        Mode::ExtractDialog => Some("--extract"),
        Mode::CompressDialog => Some("--compress"),
        other => Some(match other {
            Mode::ExtractHere => "--extract-here",
            Mode::ExtractTo => "--extract-to",
            Mode::Test => "--test",
            Mode::CompressZip => "--compress-zip",
            _ => "--compress-tgz",
        }),
    };
    cmd.args(flag);
    if let Some(password) = &request.password {
        cmd.arg("--password").arg(password);
    }
    cmd.arg("--").args(&request.paths);
    let _ = cmd.spawn();
}

/// 모인 요청들을 처리한다.
pub fn dispatch(requests: Vec<Request>) -> i32 {
    // 테스트와 자동화: 창을 띄우지 않는다.
    if std::env::var_os(notify::ENV_NO_SHELL).is_some() {
        return jobs::execute(requests);
    }

    let (windowed, quick): (Vec<Request>, Vec<Request>) =
        requests.into_iter().partition(needs_window);
    if windowed.is_empty() {
        // 출력을 파이프나 파일로 돌렸으면(스크립트) 결과를 글자로 돌려준다.
        if console::is_redirected() {
            return jobs::execute(quick);
        }
        return run(Start::Jobs(jobs::plan(quick)));
    }

    // 한 번의 클릭에서 동작이 섞여 들어오는 일은 거의 없다. 그래도 오면 빠른 것은
    // 창 없이 먼저 처리한다.
    let mut code = 0;
    if !quick.is_empty() {
        code = jobs::execute(quick);
    }

    let first_mode = windowed[0].mode;
    let password = windowed[0].password.clone();
    let mut paths: Vec<PathBuf> = Vec::new();
    for request in &windowed {
        if request.mode == first_mode {
            paths.extend(request.paths.iter().cloned());
        } else {
            relaunch(request);
        }
    }
    paths.dedup();

    let start = match first_mode {
        Mode::CompressDialog => Start::CompressForm(paths),
        Mode::ExtractDialog if !paths.is_empty() => Start::ExtractForm(paths, password),
        _ => {
            // 열기: 첫 번째는 이 창에서, 나머지는 각자의 창에서.
            let mut iter = paths.into_iter();
            let first = iter.next();
            for rest in iter {
                relaunch(&Request {
                    mode: Mode::Open,
                    paths: vec![rest],
                    password: password.clone(),
                });
            }
            Start::Browse(first, password)
        }
    };
    code.max(run(start))
}
