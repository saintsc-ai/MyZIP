//! 메인 창의 백그라운드 작업 하나.
//!
//! 압축 풀기는 수십 초가 걸릴 수 있으므로 반드시 별도 스레드에서 돌린다.
//! 창은 진행률만 읽어서 그린다.

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use eframe::egui;
use myzip_core::{CancelToken, Progress};

use super::widgets::{Shared, Snapshot, progress_view, tracked_progress};

pub struct Task<T> {
    shared: Shared,
    token: CancelToken,
    started: Instant,
    rx: Receiver<T>,
    cancelling: bool,
}

impl<T: Send + 'static> Task<T> {
    pub fn spawn<F>(ctx: &egui::Context, headline: impl Into<String>, work: F) -> Self
    where
        F: FnOnce(&mut Progress) -> T + Send + 'static,
    {
        let shared: Shared = Arc::new(Mutex::new(Snapshot {
            headline: headline.into(),
            ..Snapshot::default()
        }));
        let token = CancelToken::new();
        let (tx, rx) = mpsc::channel();
        let mut progress = tracked_progress(ctx, &shared, token.clone());
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = work(&mut progress);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        Self {
            shared,
            token,
            started: Instant::now(),
            rx,
            cancelling: false,
        }
    }

    /// 끝났으면 결과.
    pub fn poll(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }

    /// 진행률 화면을 그린다.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.shared.lock().map(|s| s.clone()).unwrap_or_default();
        if progress_view(ui, &snapshot, self.started, self.cancelling) {
            self.cancelling = true;
            self.token.cancel();
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(250));
    }
}
