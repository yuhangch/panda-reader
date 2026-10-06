use gpui_kit::*;

pub(crate) fn sidebar_font() -> Font {
    let mut face = font("Inter");
    face.fallbacks = Some(chinese_sans_fallbacks());
    face
}

pub(in crate::ui) fn app_ui_font() -> Font {
    let mut face = font("Source Sans 3");
    face.fallbacks = Some(chinese_sans_fallbacks());
    face
}

pub(in crate::ui) fn article_font_family(serif: bool) -> Font {
    let mut face = font(if serif {
        "Source Serif 4"
    } else {
        "Source Sans 3"
    });
    face.fallbacks = Some(if serif {
        FontFallbacks::from_fonts(vec![
            "Noto Serif SC".into(),
            "Songti SC".into(),
            "SimSun".into(),
            "Noto Sans SC".into(),
        ])
    } else {
        chinese_sans_fallbacks()
    });
    face
}

fn chinese_sans_fallbacks() -> FontFallbacks {
    FontFallbacks::from_fonts(vec![
        "Noto Sans SC".into(),
        "Microsoft YaHei UI".into(),
        "PingFang SC".into(),
        "Segoe UI".into(),
    ])
}
