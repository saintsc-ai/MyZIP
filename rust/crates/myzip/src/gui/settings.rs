//! 설정 창: 확장자 연결, 탐색기 통합, 정보. Python 판 ui/settings_dialog.py 와 같다.

use std::path::PathBuf;

use eframe::egui::{self, RichText};
use myzip_core::{APP_NAME, VERSION, sevenzip};

use super::widgets::{Icons, Notice, muted};
use crate::notify::{self, Kind};
use crate::registry::{self, AssocStatus, FILE_TYPES};

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tab {
    Associations,
    Shell,
    About,
}

pub struct Settings {
    tab: Tab,
    statuses: Vec<AssocStatus>,
    menu_installed: bool,
    engine: Option<PathBuf>,
    notice: Option<(Notice, bool)>,
}

/// "CompressedFolder" 같은 ProgID 를 사람이 알아보는 이름으로.
fn pretty(progid: &str) -> String {
    match progid {
        "" => String::new(),
        "CompressedFolder" => "Windows 압축 폴더".into(),
        "ArchiveFolder" => "Windows 보관 폴더".into(),
        id if id.starts_with(&format!("{APP_NAME}.")) => APP_NAME.into(),
        id if id.starts_with("7-Zip.") => "7-Zip".into(),
        id if id.to_lowercase().contains("bandizip") => "반디집".into(),
        id if id.starts_with("WinRAR") => "WinRAR".into(),
        id => id.into(),
    }
}

impl Settings {
    pub fn new() -> Self {
        Self {
            tab: Tab::Associations,
            statuses: registry::all_status(),
            menu_installed: registry::context_menu_installed(),
            engine: sevenzip::find_7z(true),
            notice: None,
        }
    }

    fn refresh(&mut self) {
        self.statuses = registry::all_status();
        self.menu_installed = registry::context_menu_installed();
    }

    fn report(&mut self, result: std::io::Result<()>) {
        if let Err(err) = result {
            self.notice = Some((
                Notice::new(
                    Kind::Error,
                    "레지스트리 오류",
                    format!("레지스트리에 쓰지 못했습니다: {err}"),
                ),
                false,
            ));
        }
        registry::refresh_shell();
        self.refresh();
    }

    /// 닫혔으면 true.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> bool {
        if let Some((notice, is_confirm)) = &self.notice {
            let is_confirm = *is_confirm;
            if let Some(button) = notice.ui(ui) {
                self.notice = None;
                if is_confirm && button == 0 {
                    let result = registry::uninstall_all();
                    self.report(result);
                    self.notice = Some((
                        Notice::new(
                            Kind::Info,
                            "제거 완료",
                            "확장자 연결과 탐색기 메뉴를 모두 제거했습니다.",
                        ),
                        false,
                    ));
                }
            }
            return false;
        }

        ui.label(
            RichText::new(format!("{APP_NAME} 설정"))
                .strong()
                .size(16.0),
        );
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Associations, "확장자 연결");
            ui.selectable_value(&mut self.tab, Tab::Shell, "탐색기 통합");
            ui.selectable_value(&mut self.tab, Tab::About, "정보");
        });
        ui.separator();

        egui::ScrollArea::vertical()
            .max_height(400.0)
            .auto_shrink([false, true])
            .show(ui, |ui| match self.tab {
                Tab::Associations => self.associations(ui),
                Tab::Shell => self.shell(ui),
                Tab::About => self.about(ui),
            });

        ui.separator();
        let mut closed = false;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("닫기").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                closed = true;
            }
        });
        closed
    }

    fn associations(&mut self, ui: &mut egui::Ui) {
        ui.label(muted(
            "체크한 확장자를 더블클릭하면 MyZIP 으로 열립니다.\n\
             모든 설정은 현재 사용자에게만 적용되며 관리자 권한이 필요 없습니다.",
        ));
        ui.add_space(4.0);

        let mut toggled: Option<(String, bool)> = None;
        egui::Grid::new("assoc")
            .striped(true)
            .num_columns(3)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                ui.label(RichText::new("확장자").strong());
                ui.label(RichText::new("형식").strong());
                ui.label(RichText::new("현재 연결").strong());
                ui.end_row();
                for (state, (ext, _, description, _)) in self.statuses.iter().zip(FILE_TYPES) {
                    let mut checked = state.registered;
                    if ui.checkbox(&mut checked, *ext).changed() {
                        toggled = Some(((*ext).to_owned(), checked));
                    }
                    ui.label(*description);
                    if state.blocked_by_userchoice {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            format!("{} (Windows 가 고정)", pretty(&state.current_owner)),
                        );
                    } else if state.registered {
                        ui.label(APP_NAME);
                    } else {
                        let owner = pretty(&state.current_owner);
                        ui.label(if owner.is_empty() {
                            "연결 없음".to_owned()
                        } else {
                            owner
                        });
                    }
                    ui.end_row();
                }
            });
        if let Some((ext, on)) = toggled {
            let result = if on {
                registry::associate(&ext).map(drop)
            } else {
                registry::unassociate(&ext)
            };
            self.report(result);
        }

        ui.horizontal(|ui| {
            if ui.button("모두 선택").clicked() {
                let result = FILE_TYPES
                    .iter()
                    .try_for_each(|(ext, ..)| registry::associate(ext).map(drop));
                self.report(result);
            }
            if ui.button("모두 해제").clicked() {
                let result = FILE_TYPES
                    .iter()
                    .try_for_each(|(ext, ..)| registry::unassociate(ext));
                self.report(result);
            }
        });

        let blocked: Vec<&str> = self
            .statuses
            .iter()
            .filter(|s| s.blocked_by_userchoice)
            .map(|s| s.ext.as_str())
            .collect();
        if !blocked.is_empty() {
            ui.add_space(4.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!(
                        "다음 확장자는 Windows '기본 앱' 설정에서 다른 프로그램으로 고정되어 있습니다: {}\n\
                         Windows 8 부터 프로그램이 기본 앱을 임의로 바꿀 수 없게 막혀 있어서, \
                         아래 버튼으로 설정을 열어 직접 MyZIP 을 선택해야 합니다.",
                        blocked.join(", ")
                    ),
                );
            });
        }
        if ui.button("Windows 기본 앱 설정 열기").clicked() {
            notify::shell_open("ms-settings:defaultapps".as_ref());
        }
    }

    fn shell(&mut self, ui: &mut egui::Ui) {
        let mut installed = self.menu_installed;
        if ui
            .checkbox(&mut installed, "탐색기 마우스 오른쪽 메뉴에 MyZIP 넣기")
            .changed()
        {
            let result = if installed {
                registry::register_application().and_then(|()| registry::install_context_menu())
            } else {
                registry::uninstall_context_menu();
                Ok(())
            };
            self.report(result);
        }
        ui.add_space(4.0);
        ui.label(RichText::new("파일/폴더를 우클릭하면").strong());
        ui.label("· MyZIP 으로 압축 → 압축하기... / ZIP 으로 압축 / TAR.GZ 로 압축");
        ui.label(RichText::new("압축 파일을 우클릭하면").strong());
        ui.label("· MyZIP → 열기 / 여기에 압축 풀기 / 압축 풀기 (폴더 생성) / 압축 풀기... / 무결성 검사");
        ui.label(muted("여러 개를 골라도 됩니다. 압축은 하나로 묶고, 압축 풀기는 각각 자기 이름의 폴더로 풉니다."));
        ui.add_space(4.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(
                "Windows 11 에서는 이 메뉴가 우클릭 후 '추가 옵션 표시' (또는 Shift+F10) 안에 나타납니다. \
                 1차 메뉴에 바로 넣으려면 MSIX 패키지와 서명된 COM 확장이 필요합니다.",
            );
        });

        ui.add_space(10.0);
        ui.label(RichText::new("RAR / 7Z 해제 엔진").strong());
        match &self.engine {
            Some(exe) => {
                ui.label(format!("✔ 사용 가능\n{}", exe.display()));
            }
            None => {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "⚠ 찾지 못했습니다. RAR 과 7Z 파일은 열 수 없습니다.\n\
                     7-Zip 을 설치하거나, MyZIP 설치 폴더 아래 bin 폴더에 7z.exe 와 7z.dll 을 넣어 주세요.",
                );
            }
        }
        ui.label(muted(
            "RAR 은 RARLAB 의 독점 형식이라 압축은 만들 수 없고 해제만 됩니다. \
             unRAR 라이선스가 RAR 압축기 제작에 코드를 쓰는 것을 금지하기 때문이며, \
             반디집과 7-Zip 도 같은 제약을 받습니다.",
        ));
    }

    fn about(&mut self, ui: &mut egui::Ui) {
        let icons = Icons::get(ui.ctx());
        ui.vertical_centered(|ui| {
            ui.add(egui::Image::new((icons.app.id(), egui::vec2(72.0, 72.0))));
            ui.label(RichText::new(APP_NAME).strong().size(20.0));
            ui.label(format!("버전 {VERSION}"));
        });
        ui.add_space(8.0);
        egui::Grid::new("about")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                for (key, value) in [
                    ("압축 가능", "ZIP, TAR, TAR.GZ(TGZ), TAR.BZ2, TAR.XZ"),
                    ("해제 가능", "위 형식 + RAR, 7Z, CAB, ARJ, LZH, ISO"),
                    ("암호", "ZIP AES-256 (읽기·쓰기), ZipCrypto (읽기)"),
                    (
                        "파일명 인코딩",
                        "UTF-8 / CP949 / CP932 / GBK / Big5 자동 판별",
                    ),
                ] {
                    ui.label(RichText::new(key).strong());
                    ui.label(value);
                    ui.end_row();
                }
            });
        ui.add_space(12.0);
        if ui.button("모든 연결과 메뉴 제거").clicked() {
            self.notice = Some((
                Notice::new(
                    Kind::Warning,
                    "제거 확인",
                    "MyZIP 이 만든 확장자 연결과 탐색기 메뉴를 모두 지웁니다.\n프로그램 자체는 지워지지 않습니다.\n\n계속할까요?",
                )
                .with_buttons(&["모두 제거", "취소"]),
                true,
            ));
        }
    }
}
