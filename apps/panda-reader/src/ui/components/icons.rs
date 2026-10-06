use gpui_kit::{Image, ImageFormat, component::Icon};
use std::sync::{Arc, LazyLock};

pub(in crate::ui) static APP_ICON: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../../../assets/app-icon.png").to_vec(),
    ))
});

pub(in crate::ui) fn tty_icon(name: &str) -> Icon {
    let bytes: &'static [u8] = match name {
        "panel-left" => include_bytes!("../../../assets/icons/panel-left.svg"),
        "plus" => include_bytes!("../../../assets/icons/plus.svg"),
        "ellipsis" => include_bytes!("../../../assets/icons/ellipsis.svg"),
        "refresh" => include_bytes!("../../../assets/icons/refresh.svg"),
        "folder" => include_bytes!("../../../assets/icons/folder-closed.svg"),
        "settings" => include_bytes!("../../../assets/icons/settings.svg"),
        "appearance" => include_bytes!("../../../assets/icons/appearance.svg"),
        "about" => include_bytes!("../../../assets/icons/about.svg"),
        "close" => include_bytes!("../../../assets/icons/close.svg"),
        "list-flat" => include_bytes!("../../../assets/icons/list-flat.svg"),
        "image" => include_bytes!("../../../assets/icons/image.svg"),
        "image-off" => include_bytes!("../../../assets/icons/image-off.svg"),
        "star" => include_bytes!("../../../assets/icons/star.svg"),
        "star-fill" => include_bytes!("../../../assets/icons/star-fill.svg"),
        "bookmark" => include_bytes!("../../../assets/icons/bookmark.svg"),
        "bookmark-check" => include_bytes!("../../../assets/icons/bookmark-check.svg"),
        "share" => include_bytes!("../../../assets/icons/share.svg"),
        _ => unreachable!("unknown tty7 icon"),
    };
    Icon::default().data(bytes)
}
