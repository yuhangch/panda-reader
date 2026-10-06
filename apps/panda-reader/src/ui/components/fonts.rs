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

pub(in crate::ui) fn article_font() -> Font {
    let mut face = font("Source Serif 4");
    face.fallbacks = Some(chinese_sans_fallbacks());
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
