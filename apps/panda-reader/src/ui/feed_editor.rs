use crate::services::Command;
use crate::ui::components::text_input;
use crate::ui::window::ReaderWindow;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::InputState,
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use tokio::sync::oneshot;

pub(in crate::ui) struct FeedEditor {
    pub(in crate::ui) title_input: Entity<InputState>,
    pub(in crate::ui) folder_input: Entity<InputState>,
    pub(in crate::ui) url_input: Entity<InputState>,
    pub(in crate::ui) feed_id: Option<i64>,
    pub(in crate::ui) auto_translate_titles: bool,
}

impl ReaderWindow {
    pub(in crate::ui) fn update_feed_fields(
        &mut self,
        id: i64,
        title: String,
        folder: Option<String>,
        feed_url: String,
        auto_translate_titles: bool,
        cx: &mut Context<Self>,
    ) {
        let (reply, response) = oneshot::channel();
        self.services.send(Command::UpdateFeed {
            id,
            title,
            folder,
            feed_url,
            auto_translate_titles,
            reply,
        });
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.editor.feed_id = None;
                        this.set_flash(this.t("Feed updated"), cx);
                        this.load_snapshot(cx);
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn begin_edit_feed(
        &mut self,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(feed) = self
            .sidebar
            .feeds
            .iter()
            .find(|feed| feed.id == id)
            .cloned()
        else {
            return;
        };
        self.editor.feed_id = Some(id);
        self.editor.auto_translate_titles = feed.auto_translate_titles;
        let _ = self.editor.title_input.update(cx, |input, cx| {
            input.set_value(feed.title.as_str(), window, cx);
            input.focus_handle(cx).focus(window, cx);
        });
        let _ = self.editor.folder_input.update(cx, |input, cx| {
            input.set_value(feed.folder.as_deref().unwrap_or(""), window, cx);
        });
        let _ = self.editor.url_input.update(cx, |input, cx| {
            input.set_value(feed.feed_url.as_str(), window, cx);
        });
        cx.notify();
    }

    pub(in crate::ui) fn save_feed_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.editor.feed_id else {
            return;
        };
        let auto_translate_titles = self.editor.auto_translate_titles;
        if self.settings.library_source != crate::app::preferences::LibrarySource::Local {
            self.set_feed_auto_translate_titles(id, auto_translate_titles, cx);
            return;
        }
        let title = self.editor.title_input.read(cx).value().to_string();
        let folder = {
            let value = self.editor.folder_input.read(cx).value().to_string();
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        };
        let feed_url = self.editor.url_input.read(cx).value().to_string();
        let _ = window;
        self.update_feed_fields(id, title, folder, feed_url, auto_translate_titles, cx);
    }
}

impl FeedEditor {
    pub(in crate::ui) fn new(window: &mut Window, cx: &mut Context<ReaderWindow>) -> Self {
        let title_input = cx.new(|cx| InputState::new(window, cx).placeholder("Feed title"));
        let folder_input = cx.new(|cx| InputState::new(window, cx).placeholder("Folder"));
        let url_input = cx.new(|cx| InputState::new(window, cx).placeholder("https://…"));
        Self {
            title_input,
            folder_input,
            url_input,
            feed_id: None,
            auto_translate_titles: false,
        }
    }
}

impl FeedEditor {
    pub(in crate::ui) fn render_feed_editor(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let scrim = cx.theme().background.opacity(0.55);
        let can_edit_feed_fields =
            owner.settings.library_source == crate::app::preferences::LibrarySource::Local;
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(scrim)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.editor.feed_id = None;
                    cx.notify();
                }),
            )
            .child(
                v_flex()
                    .w(px(440.))
                    .gap_3()
                    .p_5()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().popover)
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(div().text_lg().font_semibold().child(owner.t("Edit feed")))
                    .when(can_edit_feed_fields, |view| {
                        view.child(div().text_sm().child(owner.t("Title")))
                            .child(text_input(&self.title_input))
                            .child(div().text_sm().child(owner.t("Folder")))
                            .child(text_input(&self.folder_input))
                            .child(div().text_sm().child(owner.t("Feed URL")))
                            .child(text_input(&self.url_input))
                    })
                    .when(!can_edit_feed_fields, |view| {
                        view.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(owner.t(
                                "Feed details are managed by the selected subscription provider.",
                            )),
                        )
                    })
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .py_2()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().text_sm().child(owner.t("Translate titles")))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(owner.t(
                                                "Automatically translate titles from this feed.",
                                            )),
                                    ),
                            )
                            .child(
                                Switch::new("feed-editor-auto-translate-titles")
                                    .checked(self.auto_translate_titles)
                                    .accessibility_label(owner.t("Translate titles"))
                                    .on_change(cx.listener(|this, checked, _, cx| {
                                        this.editor.auto_translate_titles = *checked;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .pt_2()
                            .child(
                                Button::new("feed-edit-cancel")
                                    .small()
                                    .secondary()
                                    .label(owner.t("Cancel"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.editor.feed_id = None;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("feed-edit-save")
                                    .small()
                                    .primary()
                                    .label(owner.t("Save"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_feed_editor(window, cx);
                                    })),
                            ),
                    ),
            )
    }
}
