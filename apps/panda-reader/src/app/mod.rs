//! Application startup and dependency assembly.

mod config;
pub(crate) mod preferences;

use crate::{services::AppServices, ui};
use config::AppConfig;
use gpui_kit::{component::TitleBar, *};
use std::borrow::Cow;

gpui_kit::assets::icon_assets!(
    pub(crate) ExtraIcons,
    [
        PanelLeft,
        Plus,
        RefreshCw,
        Folder,
        Settings,
        Palette,
        Info,
        X,
        List,
        Puzzle,
        Image,
        ImageOff,
        Star,
        Bookmark,
        BookmarkCheck,
        Share,
    ]
);

struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

pub fn run() {
    let AppConfig {
        data_dir,
        database,
        preferences_path,
        provider_settings_path,
        translator_path,
        preferences,
        provider_settings,
        translator_config,
    } = AppConfig::load();
    let services = AppServices::start(
        database,
        provider_settings_path,
        translator_path.clone(),
        data_dir.join("plugins"),
        preferences.library_source.workspace(),
        provider_settings.clone(),
        preferences.detailed_sync_logging,
        preferences.sync_log_retention_days,
    );
    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.text_system()
                .add_fonts(vec![
                    Cow::Borrowed(include_bytes!("../../assets/fonts/SourceSans3-Regular.ttf")),
                    Cow::Borrowed(include_bytes!(
                        "../../assets/fonts/SourceSans3-Semibold.ttf"
                    )),
                    Cow::Borrowed(include_bytes!(
                        "../../assets/fonts/SourceSerif4-Regular.ttf"
                    )),
                    Cow::Borrowed(include_bytes!(
                        "../../assets/fonts/SourceSerif4-Semibold.ttf"
                    )),
                    Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-Regular.ttf")),
                    Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-Medium.ttf")),
                    Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-SemiBold.ttf")),
                    Cow::Borrowed(include_bytes!("../../assets/fonts/NotoSansSC-VF.ttf")),
                ])
                .expect("failed to load bundled fonts");
            ui::theme::apply_theme(&preferences.theme, cx);
            ui::bind_keys(cx);
            let image_client = reqwest_client::ReqwestClient::user_agent("PandaReader/0.1")
                .expect("failed to configure image requests");
            cx.set_http_client(std::sync::Arc::new(image_client));
            let services = services.clone();
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1160.), px(760.)), cx)),
                window_min_size: Some(size(px(820.), px(520.))),
                window_decorations: Some(WindowDecorations::Client),
                app_id: Some("panda-reader".into()),
                icon: Some(std::sync::Arc::new(
                    image::load_from_memory(include_bytes!("../../assets/app-icon.png"))
                        .expect("failed to decode Panda Reader icon")
                        .into_rgba8(),
                )),
                ..TitleBar::window_options()
            };
            gpui_kit::open_window(options, cx, move |window, cx| {
                window.set_window_title("Panda Reader");
                cx.new(|cx| {
                    ui::ReaderWindow::new(
                        window,
                        cx,
                        services.clone(),
                        preferences_path.clone(),
                        preferences.clone(),
                        translator_path.clone(),
                        translator_config.clone(),
                        provider_settings.clone(),
                        data_dir.clone(),
                    )
                })
            })
            .expect("failed to open Panda Reader window");
        });
}
