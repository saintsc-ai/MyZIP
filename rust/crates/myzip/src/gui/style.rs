//! 글꼴과 모양.
//!
//! egui 기본 글꼴에는 한글이 없다. Windows 에 늘 있는 맑은 고딕을 읽어 맨 앞에
//! 둔다. 글꼴 파일을 exe 에 넣지 않으므로 exe 가 커지지 않는다.

use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, TextStyle};

/// 한글을 그릴 수 있는 글꼴 후보. 앞에 있는 것부터 쓴다.
const KOREAN_FONTS: &[&str] = &["malgun.ttf", "gulim.ttc", "batang.ttc"];

fn fonts_dir() -> std::path::PathBuf {
    let windir = std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into());
    std::path::PathBuf::from(windir).join("Fonts")
}

pub fn setup(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let dir = fonts_dir();
    if let Some((name, bytes)) = KOREAN_FONTS
        .iter()
        .find_map(|name| std::fs::read(dir.join(name)).ok().map(|b| (*name, b)))
    {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_owned(bytes)));
        // 본문은 한글 글꼴을 먼저, 고정폭은 기본 글꼴 다음에 한글을 쓴다.
        if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
            list.insert(0, name.to_owned());
        }
        if let Some(list) = fonts.families.get_mut(&FontFamily::Monospace) {
            list.push(name.to_owned());
        }
    }
    ctx.set_fonts(fonts);

    ctx.all_styles_mut(|style| {
        use egui::FontId;
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(14.0));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(14.0));
        style
            .text_styles
            .insert(TextStyle::Small, FontId::proportional(12.0));
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::proportional(18.0));
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
        style.spacing.interact_size.y = 26.0;
    });
}
