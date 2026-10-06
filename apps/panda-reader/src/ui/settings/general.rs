use super::Settings;
use crate::app::preferences::LibrarySource;
use crate::ui::components::{text_input, tty_icon};
use crate::ui::i18n;
use crate::ui::window::ReaderWindow;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    select::Select,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

impl Settings {
    pub(in crate::ui) fn render_general_settings(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let selected_provider = self.library_source.provider();
        let active_provider_settings =
            selected_provider.and_then(|provider| self.provider_settings.get(&provider));
        let feed_query = self.added_feeds_search.trim().to_lowercase();
        let mut visible_feed_count = 0;
        let mut feeds = v_flex().gap_2().pt_3();
        for feed in owner.sidebar.feeds.iter().filter(|feed| {
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
            let row = h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .gap_3()
                .py_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_sm().child(feed.title.clone()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(i18n::format(
                                    owner.preferences.language,
                                    "{} unread",
                                    feed.unread,
                                )),
                        ),
                )
                .child(
                    h_flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new(("settings-edit-feed", id as u64))
                                .small()
                                .ghost()
                                .label(owner.t("Edit"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.begin_edit_feed(id, window, cx);
                                })),
                        )
                        .child(
                            Button::new(("settings-remove-feed", id as u64))
                                .small()
                                .ghost()
                                .label(owner.t("Remove"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.sidebar.pending_remove_feed = Some(id);
                                    cx.notify();
                                })),
                        ),
                );
            feeds = feeds.child(row);
            if owner.sidebar.pending_remove_feed == Some(id) {
                feeds = feeds.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(owner.t("Remove this feed and its cached articles from the selected source?")),
                        )
                        .child(
                            Button::new("confirm-remove-feed").small()
                                .secondary()
                                .label(owner.t("Confirm removal"))
                                .on_click(cx.listener(move |this, _, _, cx| this.remove_feed(id, cx))),
                        )
                        .child(
                            Button::new("cancel-remove-feed").small()
                                .ghost()
                                .label(owner.t("Cancel"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.sidebar.pending_remove_feed = None;
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
                    .child(owner.t("No feeds found")),
            );
        }
        v_flex()
            .child(self.settings_heading(owner, "General"))
            .child(div().pb_1().font_semibold().child(owner.t("Language")))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(owner.t("Choose the language used throughout the app.")))
            .child(
                div()
                    .pt_3()
                    .w(px(260.))
                    .child(
                        Select::new(&self.language_select)
                            .small()
                            .placeholder(owner.t("Language")),
                    ),
            )
            .child(div().my_6().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(owner.t("Subscription source")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(owner.t(
                        "Choose local subscriptions or connect a provider. Each source keeps its own feeds and reading state.",
                    )),
            )
            .child(div().pt_4().pb_2().text_sm().child(owner.t("Mode / Provider")))
            .child(
                div().w(px(260.)).child(
                    Select::new(&self.upstream_select)
                        .small()
                        .placeholder(owner.t("Mode / Provider")),
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
                                owner.preferences.language,
                                "Connected: {}",
                                &settings.endpoint,
                            )),
                    )
                })
                .child(div().pt_4().pb_2().text_sm().child(owner.t("Server URL")))
                .child(text_input(&self.provider_url_input))
                .when(self.library_source == LibrarySource::FreshRss, |view| view
                    .child(div().pt_4().pb_2().text_sm().child(owner.t("Username")))
                    .child(text_input(&self.provider_username_input)))
                .child(div().pt_4().pb_2().text_sm().child(owner.t(if self.library_source == LibrarySource::Miniflux { "API Token" } else { "API Password" })))
                .child(text_input(&self.provider_secret_input).mask_toggle())
                .child(
                    h_flex()
                        .gap_2()
                        .pt_4()
                        .child(
                            Button::new("connect-miniflux").small()
                                .primary()
                                .label(owner.t(if active_provider_settings.is_some() {
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
                                    .label(owner.t("Disconnect"))
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
                        .child(owner.t("Feeds and articles stay on this device and refresh directly from their RSS sources.")),
                )
            })
            .child(div().my_6().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(owner.t("Feed management")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(i18n::format(
                        owner.preferences.language,
                        "{} feeds in this source. Additions and removals apply only here.",
                        owner.sidebar.feeds.len(),
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
                            .label(owner.t("Add feed"))
                            .on_click(cx.listener(|this, _, window, cx| this.add_feed(window, cx))),
                    )
                    .child(
                        Button::new("settings-refresh").small()
                            .ghost()
                            .icon(tty_icon("refresh"))
                            .label(owner.t("Sync now"))
                            .loading(owner.sidebar.is_refreshing)
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
                            .label(owner.t("Import OPML"))
                            .on_click(cx.listener(|this, _, _, cx| this.import_opml(cx))),
                    )
                    .child(
                        Button::new("settings-export").small()
                            .ghost()
                            .label(owner.t("Export OPML"))
                            .on_click(cx.listener(|this, _, _, cx| this.export_opml(cx))),
                    ),
            )
            .child(div().my_6().border_t_1().border_color(cx.theme().border))
            .child(div().font_semibold().child(owner.t("Added feeds")))
            .child(
                div()
                    .pt_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(owner.t(
                        "Use Edit on a feed to change its details and title translation.",
                    )),
            )
            .child(div().pt_3().child(text_input(&self.added_feeds_search_input)))
            .child(if visible_feed_count == 0 {
                div()
                    .pt_3()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(owner.t("No feeds found"))
                    .into_any_element()
            } else {
                feeds.into_any_element()
            })
            .child(div().my_6().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(owner.t("Reset settings")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(owner.t(
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
                                .label(owner.t("Reset settings"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.settings.pending_reset_settings = true;
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
                                .child(owner.t("Reset app settings now?")),
                        )
                        .child(
                            Button::new("confirm-reset-settings")
                                .small()
                                .secondary()
                                .label(owner.t("Restore defaults"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.reset_settings(window, cx)
                                })),
                        )
                        .child(
                            Button::new("cancel-reset-settings")
                                .small()
                                .ghost()
                                .label(owner.t("Cancel"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.settings.pending_reset_settings = false;
                                    cx.notify();
                                })),
                        )
                    }),
            )
    }
}
