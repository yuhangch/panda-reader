use super::text_input;
use super::*;
use crate::theme::Preset;

impl ReaderView {
    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match self.settings_page {
            SettingsPage::General => self.render_general_settings(cx).into_any_element(),
            SettingsPage::Appearance => self.render_appearance_settings(cx).into_any_element(),
            SettingsPage::Reading => self.render_reading_settings(cx).into_any_element(),
            SettingsPage::About => self.render_about_settings(cx).into_any_element(),
        };
        h_flex()
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().background)
            .child(
                v_flex()
                    .w(px(220.))
                    .h_full()
                    .font(sidebar_font())
                    .bg(cx.theme().sidebar)
                    .border_r_1()
                    .border_color(cx.theme().sidebar_border)
                    .child(
                        h_flex()
                            .id("settings-back")
                            .items_center()
                            .gap_2()
                            .mx_3()
                            .mt_3()
                            .mb_5()
                            .px_3()
                            .py_2()
                            .rounded(cx.theme().radius)
                            .cursor_pointer()
                            .hover(|style| style.bg(cx.theme().sidebar_accent))
                            .child(IconName::ArrowLeft)
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .child(self.t("Back to Reader")),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings_open = false;
                                this.theme_picker_open = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .px_5()
                            .pb_2()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.t("SETTINGS")),
                    )
                    .child(
                        v_flex()
                            .gap_0p5()
                            .px_2()
                            .child(self.settings_nav_row(
                                "settings-general",
                                "General",
                                tty_icon("settings"),
                                SettingsPage::General,
                                cx,
                            ))
                            .child(self.settings_nav_row(
                                "settings-appearance",
                                "Appearance",
                                tty_icon("appearance"),
                                SettingsPage::Appearance,
                                cx,
                            ))
                            .child(self.settings_nav_row(
                                "settings-reading",
                                "Reading",
                                Icon::new(IconName::BookOpen),
                                SettingsPage::Reading,
                                cx,
                            ))
                            .child(self.settings_nav_row(
                                "settings-about",
                                "About",
                                tty_icon("about"),
                                SettingsPage::About,
                                cx,
                            )),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .px_5()
                            .py_4()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("Panda Reader · {}", self.t("Development build"))),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .id("settings-scroll")
                            .flex_1()
                            .w_full()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(
                                h_flex().w_full().justify_center().child(
                                    div()
                                        .w(px(640.))
                                        .max_w_full()
                                        .px_5()
                                        .pt_5()
                                        .pb_8()
                                        .child(page),
                                ),
                            ),
                    ),
            )
            .when(self.theme_picker_open, |view| {
                view.child(self.render_theme_picker(cx))
            })
    }

    fn settings_nav_row(
        &self,
        id: &'static str,
        label: &'static str,
        icon: Icon,
        page: SettingsPage,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.settings_page == page;
        h_flex()
            .id(id)
            .w_full()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(cx.theme().radius)
            .cursor_pointer()
            .bg(if selected {
                cx.theme().sidebar_accent
            } else {
                cx.theme().sidebar
            })
            .hover(|style| style.bg(cx.theme().sidebar_accent))
            .child(icon.small())
            .child(div().text_sm().child(self.t(label)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.settings_page = page;
                this.theme_picker_open = false;
                cx.notify();
            }))
    }

    fn settings_heading(&self, title: &'static str) -> impl IntoElement {
        div().pb_5().text_xl().font_semibold().child(self.t(title))
    }

    fn render_general_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_provider = self.library_source.provider();
        let active_provider_settings =
            selected_provider.and_then(|provider| self.provider_settings.get(&provider));
        let feed_query = self.added_feeds_search.trim().to_lowercase();
        let mut visible_feed_count = 0;
        let mut feeds = v_flex().gap_2().pt_4();
        for feed in self.feeds.iter().filter(|feed| {
            feed_query.is_empty()
                || feed.title.to_lowercase().contains(&feed_query)
                || feed.feed_url.to_lowercase().contains(&feed_query)
                || feed
                    .folder
                    .as_deref()
                    .is_some_and(|folder| folder.to_lowercase().contains(&feed_query))
        }) {
            visible_feed_count += 1;
            let id = feed.id;
            feeds = feeds.child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(div().text_sm().child(feed.title.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(i18n::format(
                                        self.preferences.language,
                                        "{} unread",
                                        feed.unread,
                                    )),
                            ),
                    )
                    .child(
                        Button::new(("settings-remove-feed", id as u64))
                            .small()
                            .ghost()
                            .label(self.t("Remove"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.pending_remove_feed = Some(id);
                                cx.notify();
                            })),
                    ),
            );
            if self.pending_remove_feed == Some(id) {
                feeds = feeds.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(self.t("Remove this feed and its cached articles from the selected source?")),
                        )
                        .child(
                            Button::new("confirm-remove-feed").small()
                                .secondary()
                                .label(self.t("Confirm removal"))
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.remove_feed(id, cx)),
                                ),
                        )
                        .child(
                            Button::new("cancel-remove-feed").small()
                                .ghost()
                                .label(self.t("Cancel"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.pending_remove_feed = None;
                                    cx.notify();
                                })),
                        ),
                );
            }
        }
        if visible_feed_count == 0 {
            feeds = feeds.child(
                div()
                    .py_3()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("No feeds found")),
            );
        }
        v_flex()
            .child(self.settings_heading("General"))
            .child(div().pb_1().font_semibold().child(self.t("Language")))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(self.t("Choose the language used throughout the app.")))
            .child(
                div()
                    .pt_3()
                    .w(px(260.))
                    .child(
                        Select::new(&self.language_select)
                            .small()
                            .placeholder(self.t("Language")),
                    ),
            )
            .child(div().my_6().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(self.t("Subscription source")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t(
                        "Choose local subscriptions or connect a provider. Each source keeps its own feeds and reading state.",
                    )),
            )
            .child(div().pt_4().pb_2().text_sm().child(self.t("Mode / Provider")))
            .child(
                div().w(px(260.)).child(
                    Select::new(&self.upstream_select)
                        .small()
                        .placeholder(self.t("Mode / Provider")),
                ),
            )
            .when(self.library_source.provider().is_some(), |view| {
                view.when_some(active_provider_settings, |view, settings| {
                    view.child(
                        div()
                            .pt_3()
                            .text_sm()
                            .text_color(cx.theme().primary)
                            .child(i18n::format(
                                self.preferences.language,
                                "Connected: {}",
                                &settings.endpoint,
                            )),
                    )
                })
                .child(div().pt_4().pb_2().text_sm().child(self.t("Server URL")))
                .child(text_input(&self.provider_url_input))
                .when(self.library_source == LibrarySource::FreshRss, |view| view
                    .child(div().pt_4().pb_2().text_sm().child(self.t("Username")))
                    .child(text_input(&self.provider_username_input)))
                .child(div().pt_4().pb_2().text_sm().child(self.t(if self.library_source == LibrarySource::Miniflux { "API Token" } else { "API Password" })))
                .child(text_input(&self.provider_secret_input).mask_toggle())
                .child(
                    h_flex()
                        .gap_2()
                        .pt_4()
                        .child(
                            Button::new("connect-miniflux").small()
                                .primary()
                                .label(self.t(if active_provider_settings.is_some() {
                                    "Update connection"
                                } else {
                                    "Connect and sync"
                                }))
                                .loading(self.is_connecting)
                                .on_click(cx.listener(|this, _, _, cx| this.connect_provider(cx))),
                        )
                        .when(active_provider_settings.is_some(), |row| {
                            row.child(
                                Button::new("disconnect-provider").small()
                                    .ghost()
                                    .label(self.t("Disconnect"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.disconnect_provider(cx)
                                    })),
                            )
                        }),
                )
            })
            .when(self.library_source == LibrarySource::Local, |view| {
                view.child(
                    div()
                        .pt_4()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.t("Feeds and articles stay on this device and refresh directly from their RSS sources.")),
                )
            })
            .child(div().my_6().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(self.t("Feed management")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(i18n::format(
                        self.preferences.language,
                        "{} feeds in this source. Additions and removals apply only here.",
                        self.feeds.len(),
                    )),
            )
            .child(div().pt_4().child(text_input(&self.add_input)))
            .child(
                h_flex()
                    .gap_2()
                    .pt_2()
                    .child(
                        Button::new("settings-add-feed").small()
                            .primary()
                            .icon(tty_icon("plus"))
                            .label(self.t("Add feed"))
                            .on_click(cx.listener(|this, _, window, cx| this.add_feed(window, cx))),
                    )
                    .child(
                        Button::new("settings-refresh").small()
                            .ghost()
                            .icon(tty_icon("refresh"))
                            .label(self.t("Sync now"))
                            .loading(self.is_refreshing)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .pt_3()
                    .child(
                        Button::new("settings-import").small()
                            .ghost()
                            .label(self.t("Import OPML"))
                            .on_click(cx.listener(|this, _, _, cx| this.import_opml(cx))),
                    )
                    .child(
                        Button::new("settings-export").small()
                            .ghost()
                            .label(self.t("Export OPML"))
                            .on_click(cx.listener(|this, _, _, cx| this.export_opml(cx))),
                    ),
            )
            .child(div().my_6().border_t_1().border_color(cx.theme().border))
            .child(div().font_semibold().child(self.t("Added feeds")))
            .child(div().pt_3().child(text_input(&self.added_feeds_search_input)))
            .child(feeds)
            .child(div().my_6().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(self.t("Reset settings")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t(
                        "Restore interface and translation settings, including saved translation keys, to defaults. Feeds, articles and Provider connections are kept.",
                    )),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .pt_3()
                    .when(!self.pending_reset_settings, |row| {
                        row.child(
                            Button::new("reset-settings")
                                .small()
                                .ghost()
                                .label(self.t("Reset settings"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.pending_reset_settings = true;
                                    cx.notify();
                                })),
                        )
                    })
                    .when(self.pending_reset_settings, |row| {
                        row.child(
                            div()
                                .flex_1()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(self.t("Reset app settings now?")),
                        )
                        .child(
                            Button::new("confirm-reset-settings")
                                .small()
                                .secondary()
                                .label(self.t("Restore defaults"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.reset_settings(window, cx)
                                })),
                        )
                        .child(
                            Button::new("cancel-reset-settings")
                                .small()
                                .ghost()
                                .label(self.t("Cancel"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.pending_reset_settings = false;
                                    cx.notify();
                                })),
                        )
                    }),
            )
    }

    fn render_appearance_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = theme::preset(&self.preferences.theme);
        v_flex()
            .child(self.settings_heading("Appearance"))
            .child(div().pb_1().font_semibold().child(self.t("Theme")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("Choose a color theme for reading.")),
            )
            .child(
                h_flex()
                    .id("current-theme")
                    .items_center()
                    .gap_4()
                    .mt_5()
                    .p_3()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted)
                    .cursor_pointer()
                    .child(Self::theme_preview(current))
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(self.t(if current.dark {
                                        "Dark theme"
                                    } else {
                                        "Light theme"
                                    })),
                            )
                            .child(div().font_semibold().child(self.t(current.name))),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.t("Change theme  ›")),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.theme_picker_open = true;
                        cx.notify();
                    })),
            )
            .child(div().my_7().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(self.t("Interface size")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("Adjust text and controls in the sidebar, list and settings.")),
            )
            .child(self.font_size_controls(cx))
            .child(div().my_7().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(self.t("Branding")))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .py_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child(self.t("Show panda icon")))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(self.t("Display the panda in the top-left corner.")),
                            ),
                    )
                    .child(
                        Switch::new("settings-show-panda-icon")
                            .checked(self.preferences.show_app_icon)
                            .accessibility_label(self.t("Show panda icon"))
                            .on_change(cx.listener(|this, checked, _, cx| {
                                this.preferences.show_app_icon = *checked;
                                this.save_preferences();
                                cx.notify();
                            })),
                    ),
            )
    }

    fn render_reading_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let collapsed = self.preferences.sidebar_collapsed;
        let shortcuts = [
            ("j", "Next article"),
            ("k", "Previous article"),
            ("Shift+J", "Next unread"),
            ("Shift+K", "Previous unread"),
            ("u", "Toggle read"),
            ("s", "Toggle star"),
            ("l", "Toggle Read Later"),
            ("r", "Refresh feeds"),
            ("o", "Open original"),
            ("x", "Mark all as read"),
        ]
        .into_iter()
        .fold(v_flex().gap_1().pt_2(), |list, (keys, label)| {
            list.child(self.keyboard_shortcut_row(keys, label, cx))
        });
        v_flex()
            .child(self.settings_heading("Reading"))
            .child(div().pb_2().font_semibold().child(self.t("Layout")))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .py_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child(self.t("Collapse feed sidebar")))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(self.t("Leave more room for articles.")),
                            ),
                    )
                    .child(
                        Button::new("settings-sidebar").small()
                            .secondary()
                            .label(self.t(if collapsed { "Collapsed" } else { "Expanded" }))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                    ),
            )
            .child(div().my_5().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(self.t("Keyboard")))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .py_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child(self.t("Vim navigation")))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(self.t(
                                        "Use j/k, s, u, l and other single keys when not typing.",
                                    )),
                            ),
                    )
                    .child(
                        Button::new("settings-vim-nav").small()
                            .secondary()
                            .label(self.t(if self.preferences.vim_navigation {
                                "On"
                            } else {
                                "Off"
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.preferences.vim_navigation = !this.preferences.vim_navigation;
                                this.save_preferences();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .pt_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("Open the Command Palette with Ctrl/Cmd+K anytime.")),
            )
            .child(shortcuts)
            .child(div().my_5().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(self.t("Full text")))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .py_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child(self.t("Auto extract full text")))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(self.t(
                                        "Fetch the original page when opening an article without full content.",
                                    )),
                            ),
                    )
                    .child(
                        Button::new("settings-auto-extract").small()
                            .secondary()
                            .label(self.t(if self.preferences.auto_extract_full_text {
                                "On"
                            } else {
                                "Off"
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.preferences.auto_extract_full_text =
                                    !this.preferences.auto_extract_full_text;
                                this.save_preferences();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .py_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child(self.t("Content extractor")))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(self.t(
                                        "Library used when extracting full text. Re-extract to compare.",
                                    )),
                            ),
                    )
                    .child(
                        Button::new("settings-content-extractor").small()
                            .secondary()
                            .label(self.t(self.preferences.content_extractor.label()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.preferences.content_extractor =
                                    this.preferences.content_extractor.next();
                                this.save_preferences();
                                cx.notify();
                            })),
                    ),
            )
            .child(div().my_5().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(self.t("Translation")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t(
                        "Interface language and article translation language can differ.",
                    )),
            )
            .child(div().pt_4().pb_2().text_sm().child(self.t("Article language")))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("Translate article bodies into this language.")),
            )
            .child(
                div().pt_3().w(px(260.)).child(
                    Select::new(&self.translation_language_select)
                        .small()
                        .placeholder(self.t("Article language")),
                ),
            )
            .child(div().pt_4().pb_2().text_sm().child(self.t("Display mode")))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t(
                        "Immersive shows original and translation together, like Immersive Translate.",
                    )),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .pt_3()
                    .child(div().text_sm().child(self.t(self.preferences.translation_layout.label())))
                    .child(
                        Button::new("settings-translation-layout").small()
                            .secondary()
                            .label(self.t("Switch"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_translation_layout(cx)
                            })),
                    ),
            )
            .child(div().pt_4().pb_2().text_sm().child(self.t("Provider")))
            .child(
                div().w(px(260.)).child(
                    Select::new(&self.provider_select)
                        .small()
                        .placeholder(self.t("Provider")),
                ),
            )
            .when(self.translator_config.provider == Provider::Azure, |view| {
                view.child(div().pt_4().pb_2().text_sm().child(self.t("Azure API key")))
                    .child(text_input(&self.azure_key_input).mask_toggle())
                    .child(div().pt_4().pb_2().text_sm().child(self.t("Azure region")))
                    .child(text_input(&self.azure_region_input))
            })
            .when(self.translator_config.provider == Provider::Volcengine, |view| {
                view.child(
                    div()
                        .pt_4()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.t(
                            "Use Access Key ID / Secret from Volcengine IAM. Service: translate.",
                        )),
                )
                .child(div().pt_4().pb_2().text_sm().child(self.t("Access Key ID")))
                .child(text_input(&self.volcengine_ak_input))
                .child(div().pt_4().pb_2().text_sm().child(self.t("Secret Access Key")))
                .child(text_input(&self.volcengine_sk_input).mask_toggle())
            })
            .when(self.translator_config.provider.is_ready(), |view| {
                view.child(
                    div().pt_4().child(
                        Button::new("settings-save-translator").small()
                            .secondary()
                            .label(self.t("Save translation settings"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.persist_translator_settings(cx)
                            })),
                    ),
                )
            })
            .when(!self.translator_config.provider.is_ready(), |view| {
                view.child(
                    div()
                        .pt_4()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.t("Coming soon")),
                )
            })
            .child(div().my_5().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(self.t("Interface text")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("Article and interface text follow the interface size.")),
            )
            .child(self.font_size_controls(cx))
    }

    fn keyboard_shortcut_row(
        &self,
        keys: &'static str,
        label: &'static str,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .items_center()
            .justify_between()
            .gap_4()
            .py_1()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t(label)),
            )
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(cx.theme().muted)
                    .text_xs()
                    .font_semibold()
                    .child(keys),
            )
    }

    fn font_size_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .items_center()
            .gap_3()
            .pt_4()
            .child(
                Button::new("font-smaller")
                    .small()
                    .secondary()
                    .icon(IconName::Minus)
                    .tooltip(self.t("Smaller"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_ui_font_size((this.preferences.ui_font_size - 1.).max(13.), cx)
                    })),
            )
            .child(
                div()
                    .w(px(62.))
                    .text_center()
                    .text_sm()
                    .child(format!("{} px", self.preferences.ui_font_size as u32)),
            )
            .child(
                Button::new("font-larger")
                    .small()
                    .secondary()
                    .icon(tty_icon("plus"))
                    .tooltip(self.t("Larger"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_ui_font_size((this.preferences.ui_font_size + 1.).min(18.), cx)
                    })),
            )
    }

    fn render_about_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .child(self.settings_heading("About"))
            .child(div().pb_2().font_semibold().child("Panda Reader"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "Version {} · Rust / GPUI Kit",
                        env!("CARGO_PKG_VERSION")
                    )),
            )
            .child(
                h_flex()
                    .gap_4()
                    .pt_2()
                    .text_sm()
                    .text_color(cx.theme().primary)
                    .child(
                        div()
                            .id("about-github-link")
                            .cursor_pointer()
                            .child("Panda Reader on GitHub")
                            .on_click(|_, _, cx| cx.open_url("https://github.com/yuhang/panda-reader")),
                    )
            )
            .child(div().my_6().border_t_1().border_color(cx.theme().border))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("Feeds, articles and settings are stored on this device.")),
            )
            .child(div().my_5().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(self.t("Papr reference")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("Papr informed our feed parsing, article extraction, and store design. Panda Reader's implementation is independent and does not include papr-core.")),
            )
            .child(
                h_flex()
                    .gap_4()
                    .pt_2()
                    .text_sm()
                    .text_color(cx.theme().primary)
                    .child(
                        div()
                            .id("about-papr-link")
                            .cursor_pointer()
                            .child("Papr on GitHub")
                            .on_click(|_, _, cx| cx.open_url("https://github.com/l0ng-ai/papr")),
                    ),
            )
            .child(div().my_5().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(self.t("TTY7 reference")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("TTY7 inspired our packaging and theme design.")),
            )
    }

    fn render_theme_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = v_flex().gap_3().p_4();
        for preset in PRESETS {
            let id = preset.id;
            let selected = self.preferences.theme == id;
            list = list.child(
                v_flex()
                    .id(format!("theme-preset-{id}"))
                    .gap_2()
                    .cursor_pointer()
                    .child(Self::theme_preview(*preset))
                    .child(
                        h_flex()
                            .justify_between()
                            .text_sm()
                            .child(self.t(preset.name))
                            .when(selected, |row| row.child(IconName::Check)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.choose_theme(id, cx))),
            );
        }
        v_flex()
            .w(px(276.))
            .h_full()
            .border_l_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .pt_4()
                    .child(div().font_semibold().child(self.t("Theme")))
                    .child(
                        Button::new("close-theme-picker")
                            .small()
                            .ghost()
                            .icon(tty_icon("close"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.theme_picker_open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .px_4()
                    .pb_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("Choose a reading palette")),
            )
            .child(
                div()
                    .id("theme-picker-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(list),
            )
    }

    fn theme_preview(preset: Preset) -> impl IntoElement {
        let line = |width| {
            div()
                .w(px(width))
                .h(px(3.))
                .rounded_sm()
                .bg(rgb(preset.foreground))
        };
        v_flex()
            .w(px(148.))
            .h(px(78.))
            .gap_2()
            .p_3()
            .rounded(px(3.))
            .border_1()
            .border_color(rgb(preset.foreground))
            .bg(rgb(preset.background))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().w(px(7.)).h(px(3.)).bg(rgb(preset.accent)))
                    .child(line(60.)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(div().w(px(34.)).h(px(3.)).bg(rgb(preset.accent)))
                    .child(line(48.)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(line(24.))
                    .child(div().w(px(50.)).h(px(3.)).bg(rgb(preset.accent))),
            )
    }
}
