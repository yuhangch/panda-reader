use super::commands::keymap::{
    FocusSearch, ForceRefreshArticle, LoadMoreArticles, NextArticle, OpenSettings, PreviousArticle,
    RefreshFeeds, ShowKeyboardShortcuts, TogglePalette, ToggleSidebar, VimMarkAllRead,
    VimNextArticle, VimNextUnread, VimOpenOriginal, VimPreviousArticle, VimPreviousUnread,
    VimRefresh, VimShowHelp, VimToggleLater, VimToggleRead, VimToggleStar,
};
use super::{
    article_list::ArticleList, article_view::ArticleView, feed_editor::FeedEditor,
    quote_poster::QuotePoster, settings::Settings, sidebar::Sidebar, status::Status,
};
use crate::app::preferences::Preferences;
use crate::services::AppServices;
use crate::ui::commands::{CommandKind, PaletteState};
use crate::ui::components::{APP_ICON, BundledIcon, app_ui_font, bundled_icon};
use crate::ui::i18n;
use crate::ui::settings::SettingsPage;
use crate::updater::{self, UpdateStatus};
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use panda_providers::ProviderSettingsMap;
use panda_translate::TranslatorConfig;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

pub struct ReaderWindow {
    pub(in crate::ui) services: AppServices,
    pub(in crate::ui) focus_handle: FocusHandle,
    pub(in crate::ui) preferences: Preferences,
    pub(in crate::ui) preferences_path: PathBuf,
    pub(in crate::ui) translator_config: TranslatorConfig,
    pub(in crate::ui) translator_path: PathBuf,
    pub(in crate::ui) palette: PaletteState,
    pub(in crate::ui) sidebar: Sidebar,
    pub(in crate::ui) list: ArticleList,
    pub(in crate::ui) reader: ArticleView,
    pub(in crate::ui) settings: Settings,
    pub(in crate::ui) editor: FeedEditor,
    pub(in crate::ui) quote_poster: Option<QuotePoster>,
    pub(in crate::ui) status: Status,
    pub(in crate::ui) title_translation_usage: Vec<panda_core::TranslationUsage>,
    pub(in crate::ui) data_dir: PathBuf,
    pub(in crate::ui) update_status: UpdateStatus,
    pub(in crate::ui) update_events: UnboundedSender<UpdateStatus>,
    pub(in crate::ui) startup_sync_pending: bool,
}

impl ReaderWindow {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        services: AppServices,
        preferences_path: PathBuf,
        preferences: Preferences,
        translator_path: PathBuf,
        translator_config: TranslatorConfig,
        provider_settings: ProviderSettingsMap,
        data_dir: PathBuf,
    ) -> Self {
        let sidebar = Sidebar::new(data_dir.join("feed-icons"));
        let list = ArticleList::new(window, cx, preferences.language);
        let reader = ArticleView::default();
        let settings = Settings::new(
            window,
            cx,
            &preferences,
            &translator_config,
            provider_settings,
            services.clone(),
        );
        let editor = FeedEditor::new(window, cx);
        let status = Status::default();
        let (update_events, mut update_receiver) = tokio::sync::mpsc::unbounded_channel();
        let update_status = std::fs::read_to_string(data_dir.join("update-failure.txt"))
            .map(UpdateStatus::Failed)
            .unwrap_or(UpdateStatus::Idle);
        let mut view = Self {
            services,
            focus_handle: cx.focus_handle(),
            preferences,
            preferences_path,
            translator_config,
            translator_path,
            palette: PaletteState::new(window, cx),

            sidebar,
            list,
            reader,
            settings,
            editor,
            quote_poster: None,
            status,
            title_translation_usage: Vec::new(),
            data_dir: data_dir.clone(),
            update_status,
            update_events: update_events.clone(),
            startup_sync_pending: true,
        };
        view.subscribe_palette_input(window, cx);
        view.load_snapshot(cx);
        view.load_title_translation_usage(cx);
        if view.preferences.check_for_updates {
            updater::start_check(data_dir.clone(), update_events.clone());
            view.settings.check_community_plugins(cx);
        }
        cx.spawn(async move |this, cx| {
            while let Some(status) = update_receiver.recv().await {
                if this
                    .update(cx, |this, cx| {
                        this.update_status = status;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let update_events = update_events.clone();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(6 * 60 * 60))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        if this.preferences.check_for_updates {
                            updater::start_check(data_dir.clone(), update_events.clone());
                            this.settings.check_community_plugins(cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(15 * 60))
                    .await;
                if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        view
    }

    pub(in crate::ui) fn t(&self, english: &'static str) -> &'static str {
        i18n::text(self.preferences.language, english)
    }
}

impl Render for ReaderWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_rem_size(px(self.preferences.ui_font_size));
        let body = if self.settings.open {
            self.settings.render_settings(self, cx).into_any_element()
        } else {
            h_flex()
                .size_full()
                .items_stretch()
                .when(!self.preferences.sidebar_collapsed, |view| {
                    view.child(self.sidebar.render_sidebar(self, cx))
                })
                .child(self.list.render_article_list(self, cx))
                .child(self.reader.render_reader(self, cx))
                .into_any_element()
        };
        div()
            .size_full()
            .relative()
            .track_focus(&self.focus_handle)
            .key_context("Reader")
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font(app_ui_font())
            .on_action(cx.listener(|this, _: &TogglePalette, window, cx| {
                this.toggle_palette(window, cx);
            }))
            .on_action(cx.listener(|this, _: &RefreshFeeds, window, cx| {
                this.handle_chord(CommandKind::Refresh, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ForceRefreshArticle, window, cx| {
                this.handle_chord(CommandKind::ForceRefreshArticle, window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.handle_chord(CommandKind::OpenSettings, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.handle_chord(CommandKind::FocusSearch, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, window, cx| {
                this.handle_chord(CommandKind::ToggleSidebar, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextArticle, window, cx| {
                this.handle_chord(CommandKind::NextArticle, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PreviousArticle, window, cx| {
                this.handle_chord(CommandKind::PreviousArticle, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowKeyboardShortcuts, window, cx| {
                this.handle_chord(CommandKind::ShowKeyboardShortcuts, window, cx);
            }))
            .on_action(cx.listener(|this, _: &LoadMoreArticles, window, cx| {
                this.handle_chord(CommandKind::LoadMoreArticles, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimNextArticle, window, cx| {
                this.handle_vim(CommandKind::NextArticle, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimPreviousArticle, window, cx| {
                this.handle_vim(CommandKind::PreviousArticle, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimNextUnread, window, cx| {
                this.handle_vim(CommandKind::NextUnread, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimPreviousUnread, window, cx| {
                this.handle_vim(CommandKind::PreviousUnread, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimToggleRead, window, cx| {
                this.handle_vim(CommandKind::ToggleRead, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimToggleStar, window, cx| {
                this.handle_vim(CommandKind::ToggleStar, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimToggleLater, window, cx| {
                this.handle_vim(CommandKind::ToggleLater, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimOpenOriginal, window, cx| {
                this.handle_vim(CommandKind::OpenOriginal, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimMarkAllRead, window, cx| {
                this.handle_vim(CommandKind::MarkAllRead, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimRefresh, window, cx| {
                this.handle_vim(CommandKind::Refresh, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimShowHelp, window, cx| {
                this.handle_vim(CommandKind::ShowKeyboardShortcuts, window, cx);
            }))
            .child(
                v_flex()
                    .size_full()
                    .child(self.render_title_bar(window, cx))
                    .child(div().flex_1().min_h_0().child(body))
                    .child(self.status.render_status_bar(self, cx)),
            )
            .when(self.palette.open, |view| {
                view.child(self.palette.render_palette(self, cx))
            })
            .when(self.editor.feed_id.is_some(), |view| {
                view.child(self.editor.render_feed_editor(self, cx))
            })
            .when(self.quote_poster.is_some(), |view| {
                view.child(self.render_quote_poster(cx))
            })
            .when_some(self.reader.image_viewer_url.clone(), |view, url| {
                view.child(self.render_image_viewer(url, cx))
            })
    }
}

impl Focusable for ReaderWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ReaderWindow {
    fn render_image_viewer(&self, url: String, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x000000d8))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.reader.image_viewer_url = None;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .relative()
                    .max_w_full()
                    .max_h_full()
                    .p_5()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(img(url).max_w(px(1400.)).max_h(px(1000.)).max_w_full())
                    .child(
                        Button::new("close-image-viewer")
                            .absolute()
                            .top_2()
                            .right_2()
                            .small()
                            .secondary()
                            .icon(bundled_icon(BundledIcon::Close))
                            .tooltip(self.t("Close"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.reader.image_viewer_url = None;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn render_title_bar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let collapsed = self.preferences.sidebar_collapsed;
        let open = self.settings.open;
        h_flex()
            .h(px(40.))
            .flex_shrink_0()
            .items_center()
            .bg(cx.theme().title_bar)
            .border_b_1()
            .border_color(cx.theme().title_bar_border)
            .child(
                h_flex()
                    .h_full()
                    .w(px(228.))
                    .pl(px(12.))
                    .items_center()
                    .gap_1()
                    .when(self.preferences.show_app_icon, |row| {
                        row.child(img(APP_ICON.clone()).size_5().rounded(cx.theme().radius))
                    })
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .flex()
                            .items_center()
                            .pl_2()
                            .text_sm()
                            .font_semibold()
                            .child("Panda Reader")
                            .window_control_area(WindowControlArea::Drag)
                            .on_mouse_down(MouseButton::Left, |_, window, _| {
                                window.start_window_move();
                            }),
                    )
                    .child(
                        div().occlude().child(
                            Button::new("title-add-feed")
                                .small()
                                .ghost()
                                .icon(bundled_icon(BundledIcon::Plus))
                                .tooltip(self.t("Add feed"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.settings.open = true;
                                    this.settings.page = SettingsPage::General;
                                    this.settings
                                        .add_input
                                        .read(cx)
                                        .focus_handle(cx)
                                        .focus(window, cx);
                                    cx.notify();
                                })),
                        ),
                    )
                    .when(!open, |row| {
                        row.child(
                            div().occlude().child(
                                Button::new("toggle-sidebar")
                                    .small()
                                    .ghost()
                                    .icon(bundled_icon(BundledIcon::PanelLeft))
                                    .tooltip(self.t(if collapsed {
                                        "Expand sidebar"
                                    } else {
                                        "Collapse sidebar"
                                    }))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)),
                                    ),
                            ),
                        )
                    }),
            )
            .child(
                div()
                    .id("title-bar-drag")
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_move();
                    }),
            )
            .child(
                div().occlude().child(
                    Button::new("title-appearance")
                        .small()
                        .ghost()
                        .icon(bundled_icon(BundledIcon::Appearance))
                        .tooltip(self.t("Appearance"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.settings.open = true;
                            this.settings.page = SettingsPage::Appearance;
                            this.settings.theme_picker_open = true;
                            cx.notify();
                        })),
                ),
            )
            .child(
                div().occlude().child(
                    Button::new("title-settings")
                        .small()
                        .ghost()
                        .icon(bundled_icon(BundledIcon::Settings))
                        .tooltip(self.t("Settings"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.settings.open = true;
                            this.settings.theme_picker_open = false;
                            cx.notify();
                        })),
                ),
            )
            .when(!cfg!(target_os = "macos"), |bar| {
                bar.child(
                    div()
                        .id("window-minimize")
                        .occlude()
                        .w(px(34.))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|style| style.bg(cx.theme().secondary_hover))
                        .window_control_area(WindowControlArea::Min)
                        .when(cfg!(target_os = "linux"), |button| {
                            button.on_click(|_, window, _| window.minimize_window())
                        })
                        .child(IconName::WindowMinimize),
                )
                .child(
                    div()
                        .id("window-maximize")
                        .occlude()
                        .w(px(34.))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|style| style.bg(cx.theme().secondary_hover))
                        .window_control_area(WindowControlArea::Max)
                        .when(cfg!(target_os = "linux"), |button| {
                            button.on_click(|_, window, _| window.zoom_window())
                        })
                        .child(if window.is_maximized() {
                            IconName::WindowRestore
                        } else {
                            IconName::WindowMaximize
                        }),
                )
                .child(
                    div()
                        .id("window-close")
                        .occlude()
                        .w(px(34.))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|style| style.bg(cx.theme().danger))
                        .window_control_area(WindowControlArea::Close)
                        .when(cfg!(target_os = "linux"), |button| {
                            button.on_click(|_, window, _| window.remove_window())
                        })
                        .child(IconName::WindowClose),
                )
            })
    }
}
