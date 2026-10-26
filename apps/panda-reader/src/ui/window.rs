use super::commands::keymap::{
    AddFeed, FocusSearch, ForceRefreshArticle, LoadMoreArticles, NextArticle, OpenSettings,
    PreviousArticle, RefreshFeeds, ShowKeyboardShortcuts, TogglePalette, ToggleSidebar,
    VimMarkAllRead, VimNextArticle, VimNextUnread, VimOpenOriginal, VimPreviousArticle,
    VimPreviousUnread, VimRefresh, VimShowHelp, VimToggleLater, VimToggleRead, VimToggleStar,
};
use super::{
    article_list::ArticleList,
    article_view::ArticleView,
    feed_editor::FeedEditor,
    quote_poster::QuotePoster,
    settings::Settings,
    sidebar::Sidebar,
    status::{Status, StatusBar},
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
use panda_core::Scope;
use panda_providers::ProviderSettingsMap;
use panda_translate::TranslatorConfig;
use std::collections::HashMap;
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
    pub(in crate::ui) sidebar: Entity<Sidebar>,
    pub(in crate::ui) pane_subscriptions: Vec<Subscription>,
    pub(in crate::ui) list: Entity<ArticleList>,
    pub(in crate::ui) reader: Entity<ArticleView>,
    pub(in crate::ui) settings: Settings,
    pub(in crate::ui) editor: FeedEditor,
    pub(in crate::ui) quote_poster: Option<QuotePoster>,
    pub(in crate::ui) status: Status,
    pub(in crate::ui) status_view: Entity<StatusBar>,
    pub(in crate::ui) title_translation_usage: Vec<panda_core::TranslationUsage>,
    pub(in crate::ui) data_dir: PathBuf,
    pub(in crate::ui) update_status: UpdateStatus,
    pub(in crate::ui) update_events: UnboundedSender<UpdateStatus>,
    pub(in crate::ui) startup_sync_pending: bool,
    pub(in crate::ui) mark_revisions: HashMap<(i64, panda_core::MarkField), u64>,
    pub(in crate::ui) next_mark_revision: u64,
}

impl ReaderWindow {
    pub(in crate::ui) fn update_article_reader<R>(
        &self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut ArticleView) -> R,
    ) -> R {
        self.reader.update(cx, |reader, reader_cx| {
            let result = update(reader);
            reader_cx.notify();
            result
        })
    }

    pub(in crate::ui) fn read_article_reader<R>(
        &self,
        cx: &App,
        read: impl FnOnce(&ArticleView) -> R,
    ) -> R {
        read(self.reader.read(cx))
    }

    pub(in crate::ui) fn update_article_list<R>(
        &self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut ArticleList, &mut Context<ArticleList>) -> R,
    ) -> R {
        self.list.update(cx, |list, list_cx| {
            let result = update(list, list_cx);
            list_cx.notify();
            result
        })
    }

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
        let sidebar = cx.new(|_| Sidebar::new(data_dir.join("feed-icons"), preferences.language));
        let list = cx.new(|cx| ArticleList::new(window, cx, preferences.language));
        let reader = cx.new(|_| ArticleView::new(preferences.clone(), translator_config.clone()));
        let settings = Settings::new(
            window,
            cx,
            &preferences,
            &translator_config,
            provider_settings,
            services.clone(),
            data_dir.clone(),
        );
        let editor = FeedEditor::new(window, cx);
        let status = Status::default();
        let status_view = cx.new(|_| StatusBar::default());
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
            pane_subscriptions: Vec::new(),
            list,
            reader,
            settings,
            editor,
            quote_poster: None,
            status,
            status_view,
            title_translation_usage: Vec::new(),
            data_dir: data_dir.clone(),
            update_status,
            update_events: update_events.clone(),
            startup_sync_pending: true,
            mark_revisions: HashMap::new(),
            next_mark_revision: 0,
        };
        let sidebar_events = cx.subscribe(&view.sidebar, |this, _, event, cx| match event {
            super::sidebar::SidebarEvent::SelectScope(scope) => {
                this.select_scope(scope.clone(), cx)
            }
            super::sidebar::SidebarEvent::Refresh { force } => this.refresh_with_force(*force, cx),
        });
        view.pane_subscriptions.push(sidebar_events);
        let sidebar_menu_events =
            cx.subscribe_in(&view.sidebar, window, |this, _, event, window, cx| {
                super::context_menus::handle_pane_menu_event(this, event, window, cx);
            });
        view.pane_subscriptions.push(sidebar_menu_events);
        let list_events = cx.subscribe(&view.list, |this, _, event, cx| match event {
            super::article_list::ArticleListEvent::SearchChanged => {
                let revision = this.list.read(cx).search_revision;
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(200))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        if revision == this.list.read(cx).search_revision {
                            this.load_snapshot(cx);
                        }
                    });
                })
                .detach();
            }
            super::article_list::ArticleListEvent::SelectArticle(id) => this.open_article(*id, cx),
            super::article_list::ArticleListEvent::MarkAllRead => this.mark_all_read(cx),
            super::article_list::ArticleListEvent::LoadMoreArticles => this.load_more_articles(cx),
        });
        view.pane_subscriptions.push(list_events);
        let list_menu_events = cx.subscribe_in(&view.list, window, |this, _, event, window, cx| {
            super::context_menus::handle_pane_menu_event(this, event, window, cx);
        });
        view.pane_subscriptions.push(list_menu_events);
        let status_events = cx.subscribe(&view.status_view, |this, _, event, cx| match event {
            super::status::StatusBarEvent::OpenAbout => {
                this.settings.open = true;
                this.settings.select_page(SettingsPage::About);
                cx.notify();
            }
        });
        view.pane_subscriptions.push(status_events);
        let reader_events = cx.subscribe(&view.reader, |this, _, event, cx| match event {
            super::article_view::ArticleViewEvent::Mark {
                article_id,
                field,
                value,
            } => this.mark(*article_id, *field, *value, cx),
            super::article_view::ArticleViewEvent::ToggleHideImages => this.toggle_hide_images(cx),
            super::article_view::ArticleViewEvent::ExtractFullText(article_id) => {
                this.extract(*article_id, true, cx)
            }
            super::article_view::ArticleViewEvent::ToggleTranslation(article_id) => {
                this.toggle_or_translate(*article_id, cx)
            }
            super::article_view::ArticleViewEvent::ToggleTranslationLayout => {
                this.toggle_translation_layout(cx)
            }
            super::article_view::ArticleViewEvent::ShareArticle => this.share_article(cx),
            super::article_view::ArticleViewEvent::SelectFeed(feed_id) => {
                this.select_scope_preserving_article(Scope::Feed(*feed_id), cx)
            }
            super::article_view::ArticleViewEvent::SaveReadingProgress => {
                this.schedule_reading_progress_save(cx)
            }
            super::article_view::ArticleViewEvent::OpenImage(index) => {
                this.update_article_reader(cx, |reader| {
                    reader.image_viewer_url = reader.image_urls.get(*index).cloned();
                });
                cx.notify();
            }
            super::article_view::ArticleViewEvent::CopyImage(url) => {
                this.copy_article_image(url.clone(), cx)
            }
        });
        view.pane_subscriptions.push(reader_events);
        let reader_menu_events =
            cx.subscribe_in(&view.reader, window, |this, _, event, window, cx| {
                super::context_menus::handle_pane_menu_event(this, event, window, cx);
            });
        view.pane_subscriptions.push(reader_menu_events);
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
        self.sync_status_bar(cx);
        window.set_rem_size(px(self.preferences.ui_font_size));
        let reader_presentation_matches = self
            .reader
            .read(cx)
            .presentation_matches(&self.preferences, &self.translator_config);
        if !reader_presentation_matches {
            let preferences = self.preferences.clone();
            let translator_config = self.translator_config.clone();
            self.reader.update(cx, |reader, reader_cx| {
                reader.preferences = preferences;
                reader.translator_config = translator_config;
                reader_cx.notify();
            });
        }
        let sidebar_scope = self.list.read(cx).scope.clone();
        let sidebar_language = self.preferences.language;
        let sidebar_source = self.settings.library_source;
        let sidebar_folder_order = self.preferences.folder_order.clone();
        let sidebar_collapsed_folders = self.preferences.collapsed_folders.clone();
        let sidebar_folder_icons = self.preferences.folder_icons.clone();
        let sidebar_presentation_changed = {
            let sidebar = self.sidebar.read(cx);
            sidebar.selected_scope != sidebar_scope
                || sidebar.language != sidebar_language
                || sidebar.library_source != sidebar_source
                || sidebar.folder_order != sidebar_folder_order
                || sidebar.collapsed_folders != sidebar_collapsed_folders
                || sidebar.folder_icons != sidebar_folder_icons
        };
        if sidebar_presentation_changed {
            self.sidebar.update(cx, |sidebar, sidebar_cx| {
                sidebar.selected_scope = sidebar_scope;
                sidebar.language = sidebar_language;
                sidebar.library_source = sidebar_source;
                sidebar.folder_order = sidebar_folder_order;
                sidebar.collapsed_folders = sidebar_collapsed_folders;
                sidebar.folder_icons = sidebar_folder_icons;
                sidebar_cx.notify();
            });
        }
        let body = if self.settings.open {
            self.settings.render_settings(self, cx).into_any_element()
        } else {
            let target_language = self
                .preferences
                .translation_language
                .translator_code()
                .to_owned();
            let list_presentation_changed = {
                let list = self.list.read(cx);
                list.language != self.preferences.language
                    || list.target_language != target_language
            };
            if list_presentation_changed {
                self.update_article_list(cx, |list, _| {
                    list.language = self.preferences.language;
                    list.target_language = target_language;
                });
            }
            h_flex()
                .size_full()
                .items_stretch()
                .when(!self.preferences.sidebar_collapsed, |view| {
                    view.child(self.sidebar.clone())
                })
                .child(self.list.clone())
                .child(self.reader.clone())
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
            .on_action(cx.listener(|this, _: &AddFeed, window, cx| {
                this.handle_chord(CommandKind::AddFeed, window, cx);
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
                    .child(self.status_view.clone()),
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
            .when_some(
                self.reader.read(cx).image_viewer_url.clone(),
                |view, url| view.child(self.render_image_viewer(url, cx)),
            )
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
                    this.reader.update(cx, |reader, cx| {
                        reader.image_viewer_url = None;
                        cx.notify();
                    });
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
                                this.reader.update(cx, |reader, cx| {
                                    reader.image_viewer_url = None;
                                    cx.notify();
                                });
                                cx.notify();
                            })),
                    ),
            )
    }

    fn render_title_bar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let collapsed = self.preferences.sidebar_collapsed;
        let open = self.settings.open;
        let title_bar_left_padding = if cfg!(target_os = "macos") {
            // Leave room for the native traffic lights provided by GPUI's title bar.
            px(76.)
        } else {
            px(12.)
        };
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
                    .pl(if cfg!(target_os = "macos") {
                        px(0.)
                    } else {
                        px(12.)
                    })
                    .items_center()
                    .gap_1()
                    .when(cfg!(target_os = "macos"), |row| {
                        row.child(
                            div()
                                .w(title_bar_left_padding)
                                .h_full()
                                .flex_shrink_0()
                                .window_control_area(WindowControlArea::Drag)
                                .on_mouse_down(MouseButton::Left, |_, window, _| {
                                    window.start_window_move();
                                }),
                        )
                    })
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
                                    this.settings.select_page(SettingsPage::General);
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
                            this.settings.select_page(SettingsPage::Appearance);
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
