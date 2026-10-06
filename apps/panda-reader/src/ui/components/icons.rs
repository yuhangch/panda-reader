use gpui_kit::{Image, ImageFormat, component::Icon};
use std::sync::{Arc, LazyLock};

#[derive(Clone, Copy)]
pub(in crate::ui) enum BundledIcon {
    PanelLeft,
    Plus,
    Refresh,
    Folder,
    Settings,
    Appearance,
    About,
    Close,
    ListFlat,
    Plugins,
    Image,
    ImageOff,
    Star,
    StarFill,
    Bookmark,
    BookmarkCheck,
    Share,
}

pub(in crate::ui) static APP_ICON: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../../../assets/app-icon.png").to_vec(),
    ))
});

pub(in crate::ui) fn bundled_icon(icon: BundledIcon) -> Icon {
    use gpui_kit::assets::IconName;

    match icon {
        BundledIcon::PanelLeft => Icon::new(IconName::PanelLeft),
        BundledIcon::Plus => Icon::new(IconName::Plus),
        BundledIcon::Refresh => Icon::new(IconName::RefreshCw),
        BundledIcon::Folder => Icon::new(IconName::Folder),
        BundledIcon::Settings => Icon::new(IconName::Settings),
        BundledIcon::Appearance => Icon::new(IconName::Palette),
        BundledIcon::About => Icon::new(IconName::Info),
        BundledIcon::Close => Icon::new(IconName::X),
        BundledIcon::ListFlat => Icon::new(IconName::List),
        BundledIcon::Plugins => Icon::new(IconName::Puzzle),
        BundledIcon::Image => Icon::new(IconName::Image),
        BundledIcon::ImageOff => Icon::new(IconName::ImageOff),
        BundledIcon::Star => Icon::new(IconName::Star),
        BundledIcon::StarFill => {
            Icon::default().data(include_bytes!("../../../assets/icons/star-fill.svg"))
        }
        BundledIcon::Bookmark => Icon::new(IconName::Bookmark),
        BundledIcon::BookmarkCheck => Icon::new(IconName::BookmarkCheck),
        BundledIcon::Share => Icon::new(IconName::Share),
    }
}
