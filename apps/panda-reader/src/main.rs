#![recursion_limit = "512"]
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod backend;
mod i18n;
mod reader_body;
mod theme;
mod ui;

use directories::ProjectDirs;
use gpui_kit::component::TitleBar;
use gpui_kit::*;
use std::borrow::Cow;
use std::path::PathBuf;

fn data_dir() -> PathBuf {
    ProjectDirs::from("com", "PandaReader", "PandaReader")
        .map(|dirs| dirs.data_local_dir().to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().join("data"))
}

fn main() {
    let data_dir = data_dir();
    let database = data_dir.join("panda-reader.sqlite3");
    let preferences_path = data_dir.join("preferences.json");
    let provider_settings_path = data_dir.join("providers.json");
    let legacy_credentials_path = data_dir.join("miniflux.json");
    let translator_path = data_dir.join("translator.json");
    let mut preferences = theme::Preferences::load(&preferences_path);
    let has_saved_source = std::fs::read(&preferences_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|value| value.get("library_source").is_some());
    let mut provider_settings = panda_providers::load_settings(&provider_settings_path)
        .unwrap_or_else(|error| {
            eprintln!("could not read provider settings: {error}");
            panda_providers::ProviderSettingsMap::new()
        });
    if let Ok(Some(legacy)) = panda_miniflux::Connection::load(&legacy_credentials_path) {
        provider_settings
            .entry(panda_providers::ProviderKind::Miniflux)
            .or_insert_with(|| panda_providers::ProviderSettings {
                endpoint: legacy.endpoint,
                username: String::new(),
                secret: legacy.token,
            });
        if !has_saved_source {
            preferences.library_source = theme::LibrarySource::Miniflux;
        }
        match panda_providers::save_settings(&provider_settings_path, &provider_settings) {
            Ok(()) => {
                if let Err(error) = std::fs::remove_file(&legacy_credentials_path)
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    eprintln!("could not remove migrated Miniflux settings: {error}");
                }
            }
            Err(error) => eprintln!("could not migrate Miniflux settings: {error}"),
        }
    }
    let translator_config = panda_translate::TranslatorConfig::load(&translator_path)
        .unwrap_or_else(|error| {
            eprintln!("could not read translator settings: {error}");
            panda_translate::TranslatorConfig::default()
        });
    let backend = backend::Backend::start(
        database,
        provider_settings_path,
        translator_path.clone(),
        preferences.library_source.workspace(),
        provider_settings.clone(),
    );
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.text_system()
                .add_fonts(vec![
                    Cow::Borrowed(include_bytes!("../assets/fonts/SourceSans3-Regular.ttf")),
                    Cow::Borrowed(include_bytes!("../assets/fonts/SourceSans3-Semibold.ttf")),
                    Cow::Borrowed(include_bytes!("../assets/fonts/SourceSerif4-Regular.ttf")),
                    Cow::Borrowed(include_bytes!("../assets/fonts/SourceSerif4-Semibold.ttf")),
                    Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Regular.ttf")),
                    Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Medium.ttf")),
                    Cow::Borrowed(include_bytes!("../assets/fonts/Inter-SemiBold.ttf")),
                    Cow::Borrowed(include_bytes!("../assets/fonts/NotoSansSC-VF.ttf")),
                ])
                .expect("failed to load bundled fonts");
            theme::apply_theme(&preferences.theme, cx);
            ui::bind_keys(cx);
            let image_client = reqwest_client::ReqwestClient::user_agent("PandaReader/0.1")
                .expect("failed to configure image requests");
            cx.set_http_client(std::sync::Arc::new(image_client));
            let backend = backend.clone();
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1160.), px(760.)), cx)),
                window_min_size: Some(size(px(820.), px(520.))),
                window_decorations: Some(WindowDecorations::Client),
                icon: Some(std::sync::Arc::new(
                    image::load_from_memory(include_bytes!("../assets/app-icon.png"))
                        .expect("failed to decode Panda Reader icon")
                        .into_rgba8(),
                )),
                ..TitleBar::window_options()
            };
            gpui_kit::open_window(options, cx, move |window, cx| {
                window.set_window_title("Panda Reader");
                cx.new(|cx| {
                    ui::ReaderView::new(
                        window,
                        cx,
                        backend.clone(),
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
