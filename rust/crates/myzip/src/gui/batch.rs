//! 탐색기 메뉴에서 바로 뜨는 작은 작업 창.
//!
//! 양식(압축하기... / 압축 풀기...) → 진행률 → 결과 순서로 흘러간다. 빠른 동작
//! (여기에 압축 풀기 등)은 양식 없이 바로 진행률부터 시작한다.
//!
//! 여러 작업은 한 스레드가 차례로 처리한다. 암호가 필요하면 스레드가 멈추고
//! 창이 암호를 물어본 뒤 답을 넘겨준다. 한 번 맞춘 암호는 다음 아카이브에도
//! 먼저 시도해 본다 (같은 암호로 묶인 분할 받은 파일들이 흔하다).

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use eframe::egui;
use myzip_core::CancelToken;

use super::Transition;
use super::forms::{CompressForm, ExtractForm, FormResult};
use super::widgets::{
    Notice, PasswordPrompt, PromptResult, Shared, Snapshot, progress_view, tracked_progress,
};
use crate::jobs::{self, Job, Outcome, Summary, file_name};
use crate::notify::{self, Kind};

/// 작업 스레드가 창에 묻는 것.
enum Ask {
    Password { name: String, wrong: bool },
}

struct Runner {
    shared: Shared,
    token: CancelToken,
    started: Instant,
    asks: Receiver<Ask>,
    answers: Sender<Option<String>>,
    done: Receiver<Vec<Outcome>>,
    prompt: Option<PasswordPrompt>,
    cancelling: bool,
    /// 대화상자에서 시작한 압축이면 끝나고 '압축 완료' 창을 보여 준다.
    from_compress_form: bool,
}

enum Stage {
    Compress(CompressForm),
    Extract {
        form: ExtractForm,
        archives: Vec<PathBuf>,
        password: Option<String>,
    },
    Running(Runner),
    /// 끝났고 보여 줄 것이 있다. 버튼에 따라 다음 동작이 다르다.
    Result {
        notice: Notice,
        summary: Summary,
        archive_to_open: Option<PathBuf>,
    },
}

pub struct Batch {
    stage: Stage,
    exit: Arc<std::sync::atomic::AtomicI32>,
}

impl Batch {
    pub fn jobs(
        ctx: &egui::Context,
        jobs: Vec<Job>,
        exit: Arc<std::sync::atomic::AtomicI32>,
    ) -> Self {
        Self {
            stage: Stage::Running(spawn(ctx, jobs, false)),
            exit,
        }
    }

    pub fn compress(sources: Vec<PathBuf>, exit: Arc<std::sync::atomic::AtomicI32>) -> Self {
        Self {
            stage: Stage::Compress(CompressForm::new(sources)),
            exit,
        }
    }

    pub fn extract(
        archives: Vec<PathBuf>,
        password: Option<String>,
        exit: Arc<std::sync::atomic::AtomicI32>,
    ) -> Self {
        Self {
            stage: Stage::Extract {
                form: ExtractForm::new(archives.clone(), 0),
                archives,
                password,
            },
            exit,
        }
    }

    /// 창 크기. 단계마다 다르다.
    pub fn preferred_size(&self) -> egui::Vec2 {
        match &self.stage {
            Stage::Compress(_) => egui::vec2(600.0, 560.0),
            Stage::Extract { .. } => egui::vec2(560.0, 440.0),
            Stage::Running(_) => egui::vec2(500.0, 210.0),
            Stage::Result { .. } => egui::vec2(520.0, 300.0),
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<Transition> {
        let ctx = ui.ctx().clone();
        let mut next: Option<Stage> = None;
        let mut transition = None;

        match &mut self.stage {
            Stage::Compress(form) => match form.ui(ui) {
                FormResult::Submit(job) => {
                    next = Some(Stage::Running(spawn(&ctx, vec![job], true)))
                }
                FormResult::Cancel => transition = Some(Transition::Close),
                FormResult::Pending => {}
            },
            Stage::Extract {
                form,
                archives,
                password,
            } => match form.ui(ui) {
                FormResult::Submit(choice) => {
                    let jobs = archives
                        .iter()
                        .map(|archive| Job::Extract {
                            archive: archive.clone(),
                            password: password.clone(),
                            plan: choice.plan.clone(),
                        })
                        .collect();
                    next = Some(Stage::Running(spawn(&ctx, jobs, false)));
                }
                FormResult::Cancel => transition = Some(Transition::Close),
                FormResult::Pending => {}
            },
            Stage::Running(runner) => {
                if let Some(stage) = runner.ui(ui) {
                    match stage {
                        Finished::Close(code) => {
                            self.exit.store(code, std::sync::atomic::Ordering::SeqCst);
                            transition = Some(Transition::Close);
                        }
                        Finished::Show(stage) => next = Some(*stage),
                    }
                }
            }
            Stage::Result {
                notice,
                summary,
                archive_to_open,
            } => {
                if let Some(button) = notice.ui(ui) {
                    self.exit
                        .store(summary.exit_code(), std::sync::atomic::Ordering::SeqCst);
                    let label = notice.buttons[button].as_str();
                    match label {
                        "폴더 열기" => {
                            if let Some(path) = summary.compressed.first() {
                                notify::open_in_explorer(path);
                            }
                            transition = Some(Transition::Close);
                        }
                        "아카이브 열기" => {
                            transition = archive_to_open
                                .clone()
                                .map(Transition::Browse)
                                .or(Some(Transition::Close));
                        }
                        _ => {
                            if let Some(path) = summary.to_open() {
                                notify::open_in_explorer(path);
                            }
                            transition = Some(Transition::Close);
                        }
                    }
                }
            }
        }

        if let Some(stage) = next {
            self.stage = stage;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(self.preferred_size()));
        }
        transition
    }
}

enum Finished {
    /// 보여 줄 것이 없다. 창을 닫는다.
    Close(i32),
    Show(Box<Stage>),
}

impl Runner {
    fn ui(&mut self, ui: &mut egui::Ui) -> Option<Finished> {
        // 스레드가 끝났나?
        if let Ok(outcomes) = self.done.try_recv() {
            return Some(self.finish(outcomes));
        }
        if self.prompt.is_none()
            && let Ok(Ask::Password { name, wrong }) = self.asks.try_recv()
        {
            self.prompt = Some(PasswordPrompt::new(name, wrong));
        }

        if let Some(prompt) = &mut self.prompt {
            match prompt.ui(ui) {
                PromptResult::Entered(password) => {
                    let _ = self.answers.send(Some(password));
                    self.prompt = None;
                }
                PromptResult::Cancelled => {
                    let _ = self.answers.send(None);
                    self.prompt = None;
                }
                PromptResult::Pending => {}
            }
            return None;
        }

        let snapshot = self.shared.lock().map(|s| s.clone()).unwrap_or_default();
        if progress_view(ui, &snapshot, self.started, self.cancelling) {
            self.cancelling = true;
            self.token.cancel();
        }
        // 진행률이 안 움직여도(7z 가 한참 말이 없을 때) 경과 시간은 흘러가야 한다.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(250));
        None
    }

    fn finish(&self, outcomes: Vec<Outcome>) -> Finished {
        let summary = Summary::new(&outcomes);
        let code = summary.exit_code();

        if self.from_compress_form
            && let (Some(target), true) = (
                summary.compressed.first().cloned(),
                summary.failures.is_empty(),
            )
        {
            let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
            let mut text = format!(
                "{}\n{}\n\n{}",
                file_name(&target),
                myzip_core::format_size(size),
                target
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            );
            for note in &summary.notes {
                text.push_str(&format!("\n\n{note}"));
            }
            let notice = Notice::new(Kind::Info, "압축 완료", text).with_buttons(&[
                "폴더 열기",
                "아카이브 열기",
                "닫기",
            ]);
            return Finished::Show(Box::new(Stage::Result {
                notice,
                summary,
                archive_to_open: Some(target),
            }));
        }

        // 보여 줄 것을 모은다. 실패 > 손상 > 검사 결과 > 알림 순으로 중요하다.
        let mut parts: Vec<String> = Vec::new();
        let mut kind = Kind::Info;
        let mut title = "완료";
        if let Some(message) = summary.failure_report() {
            parts.push(message);
            kind = Kind::Error;
            title = "오류";
        }
        if let Some((test_kind, test_title, message)) = summary.test_report() {
            parts.push(message);
            if kind == Kind::Info {
                kind = test_kind;
                title = test_title;
            }
        }
        parts.extend(summary.notes.iter().cloned());

        if parts.is_empty() {
            // 성공. 알림창으로 붙잡지 않는다. 탐색기에 결과가 생기는 것으로 충분하다.
            if let Some(path) = summary.to_open() {
                notify::open_in_explorer(path);
            }
            return Finished::Close(code);
        }
        if summary.cancelled {
            parts.push("나머지 작업은 취소했습니다.".into());
        }
        let notice = Notice::new(kind, title, parts.join("\n\n")).with_buttons(&["닫기"]);
        Finished::Show(Box::new(Stage::Result {
            notice,
            summary,
            archive_to_open: None,
        }))
    }
}

/// 작업 스레드를 띄운다.
fn spawn(ctx: &egui::Context, jobs: Vec<Job>, from_compress_form: bool) -> Runner {
    let shared: Shared = Arc::new(Mutex::new(Snapshot::default()));
    let token = CancelToken::new();
    let (ask_tx, ask_rx) = mpsc::channel();
    let (answer_tx, answer_rx) = mpsc::channel::<Option<String>>();
    let (done_tx, done_rx) = mpsc::channel();

    let worker_ctx = ctx.clone();
    let worker_shared = shared.clone();
    let worker_token = token.clone();
    thread::spawn(move || {
        let total = jobs.len();
        let mut outcomes = Vec::with_capacity(total);
        let mut last_password: Option<String> = None;

        for (index, job) in jobs.into_iter().enumerate() {
            if worker_token.is_cancelled() {
                outcomes.push(Outcome::Cancelled);
                continue;
            }
            if let Ok(mut s) = worker_shared.lock() {
                *s = Snapshot {
                    headline: job.label(),
                    step: if total > 1 {
                        format!("{} / {} 번째", index + 1, total)
                    } else {
                        String::new()
                    },
                    ..Snapshot::default()
                };
            }
            worker_ctx.request_repaint();

            let mut job = job;
            let mut tried_last = false; // 앞 아카이브의 암호를 이미 시도했나
            let mut asked = false; // 이 아카이브에서 사용자에게 물었나
            let outcome = loop {
                if let Ok(mut s) = worker_shared.lock() {
                    s.started = Some(Instant::now());
                    s.percent = 0.0;
                }
                let mut progress =
                    tracked_progress(&worker_ctx, &worker_shared, worker_token.clone());
                let outcome = jobs::run_job(&job, &mut progress);
                let Outcome::NeedsPassword { subject, wrong } = &outcome else {
                    break outcome;
                };

                // 앞에서 맞춘 암호를 먼저 조용히 시도한다.
                if let (Some(pw), false, false) = (&last_password, tried_last, asked) {
                    tried_last = true;
                    job = job.with_password(pw.clone());
                    continue;
                }
                // '틀렸습니다' 는 사용자가 이 아카이브에 넣은 암호가 틀렸을 때만 말한다.
                // 앞에서 이어받은 암호가 안 맞은 것은 사용자의 잘못이 아니다.
                let show_wrong = asked || (*wrong && !tried_last);
                if ask_tx
                    .send(Ask::Password {
                        name: file_name(subject),
                        wrong: show_wrong,
                    })
                    .is_err()
                {
                    break outcome;
                }
                worker_ctx.request_repaint();
                match answer_rx.recv() {
                    Ok(Some(password)) => {
                        asked = true;
                        last_password = Some(password.clone());
                        job = job.with_password(password);
                    }
                    _ => {
                        break Outcome::Failed {
                            subject: subject.clone(),
                            message: "암호를 입력하지 않아 건너뛰었습니다.".into(),
                        };
                    }
                }
            };
            outcomes.push(outcome);
        }
        let _ = done_tx.send(outcomes);
        worker_ctx.request_repaint();
    });

    Runner {
        shared,
        token,
        started: Instant::now(),
        asks: ask_rx,
        answers: answer_tx,
        done: done_rx,
        prompt: None,
        cancelling: false,
        from_compress_form,
    }
}
