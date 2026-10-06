//! Command palette overlay (tty7 Actions, single-tab).

use super::catalog::{self as commands, CommandItem, CommandKind};
use super::keymap::{PaletteConfirm, PaletteDismiss, PaletteMoveDown, PaletteMoveUp};
use crate::app::preferences::Language;
use crate::ui::components::text_input;
use crate::ui::i18n;
use crate::ui::window::ReaderWindow;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, h_flex,
    input::{InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

pub(in crate::ui) struct PaletteState {
    _subscriptions: Vec<Subscription>,
    pub open: bool,
    pub query: String,
    pub selected: usize,
    pub input: Entity<InputState>,
}

impl PaletteState {
    pub fn new(window: &mut Window, cx: &mut Context<ReaderWindow>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(i18n::text(Language::English, "Type a command…"))
        });
        Self {
            open: false,
            query: String::new(),
            selected: 0,
            input,
            _subscriptions: Vec::new(),
        }
    }

    pub fn items(&self, language: Language) -> Vec<&'static CommandItem> {
        commands::filtered_commands(&self.query, language)
    }

    pub fn clamp_selected(&mut self, len: usize) {
        if len == 0 {
            self.selected = 0;
        } else if self.selected >= len {
            self.selected = len - 1;
        }
    }
}

impl ReaderWindow {
    pub(in crate::ui) fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.open {
            self.close_palette(window, cx);
        } else {
            self.open_palette(window, cx);
        }
    }

    pub(in crate::ui) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette.open = true;
        self.palette.query.clear();
        self.palette.selected = 0;
        let _ = self.palette.input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus_handle(cx).focus(window, cx);
        });
        cx.notify();
    }

    pub(in crate::ui) fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette.open = false;
        self.palette.query.clear();
        self.palette.selected = 0;
        let _ = self.palette.input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    pub(in crate::ui) fn palette_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.palette.items(self.preferences.language).len();
        if len == 0 {
            return;
        }
        let next = self.palette.selected as isize + delta;
        self.palette.selected = next.rem_euclid(len as isize) as usize;
        cx.notify();
    }

    pub(in crate::ui) fn palette_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.palette.items(self.preferences.language);
        let Some(item) = items.get(self.palette.selected).copied() else {
            return;
        };
        let kind = item.kind;
        self.close_palette(window, cx);
        if kind != CommandKind::TogglePalette {
            self.run_command(kind, window, cx);
        }
    }

    pub(in crate::ui) fn subscribe_palette_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.palette.input.clone();
        self.palette._subscriptions.push(cx.subscribe_in(
            &input,
            window,
            move |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.palette.query = this.palette.input.read(cx).value().to_string();
                    let len = this.palette.items(this.preferences.language).len();
                    this.palette.clamp_selected(len);
                    cx.notify();
                }
            },
        ));
    }
}

impl PaletteState {
    pub(in crate::ui) fn render_palette(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let language = owner.preferences.language;
        let items = self.items(language);
        let selected = self.selected.min(items.len().saturating_sub(1));
        let scrim = cx.theme().background.opacity(0.55);
        let card_bg = cx.theme().popover;
        let border = cx.theme().border;
        let accent = cx.theme().accent;
        let muted = cx.theme().muted_foreground;
        let fg = cx.theme().foreground;

        let mut rows = v_flex().w_full();
        let mut last_group = None;
        let empty_query = self.query.trim().is_empty();
        for (index, item) in items.iter().enumerate() {
            if empty_query && last_group != Some(item.group) {
                last_group = Some(item.group);
                rows = rows.child(
                    div()
                        .px_3()
                        .pt_2()
                        .pb_1()
                        .text_xs()
                        .font_semibold()
                        .text_color(muted)
                        .child(item.group.title(language)),
                );
            }
            let is_selected = index == selected;
            let kind = item.kind;
            let title = item.localized_title(language);
            let chord = item.chord;
            rows = rows.child(
                h_flex()
                    .id(("palette-row", index))
                    .w_full()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .cursor_pointer()
                    .bg(if is_selected {
                        cx.theme().list_active
                    } else {
                        card_bg
                    })
                    .hover(|s| s.bg(cx.theme().list_active))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(fg)
                            .child(title),
                    )
                    .when_some(chord, |row, chord| {
                        row.child(div().text_xs().text_color(muted).child(chord))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_palette(window, cx);
                        if kind != CommandKind::TogglePalette {
                            this.run_command(kind, window, cx);
                        }
                    })),
            );
        }
        if items.is_empty() {
            rows = rows.child(
                div()
                    .px_3()
                    .py_6()
                    .text_sm()
                    .text_color(muted)
                    .child(owner.t("No matching commands")),
            );
        }
        let list = div()
            .id("palette-command-list")
            .w_full()
            .max_h(px(360.))
            .overflow_y_scroll()
            .child(rows);

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_start()
            .justify_center()
            .pt(px(96.))
            .bg(scrim)
            .key_context("Palette")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close_palette(window, cx)),
            )
            .on_action(cx.listener(|this, _: &PaletteDismiss, window, cx| {
                this.close_palette(window, cx);
            }))
            .on_action(cx.listener(|this, _: &PaletteMoveUp, _, cx| {
                this.palette_move(-1, cx);
            }))
            .on_action(cx.listener(|this, _: &PaletteMoveDown, _, cx| {
                this.palette_move(1, cx);
            }))
            .on_action(cx.listener(|this, _: &PaletteConfirm, window, cx| {
                this.palette_confirm(window, cx);
            }))
            .child(
                v_flex()
                    .w(px(540.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border)
                    .bg(card_bg)
                    .shadow_lg()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .px_3()
                            .pt_3()
                            .pb_2()
                            .border_b_1()
                            .border_color(border)
                            .child(text_input(&self.input)),
                    )
                    .child(div().px_0().pt_1().pb_2().child(list))
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .px_3()
                            .py_2()
                            .border_t_1()
                            .border_color(border)
                            .text_xs()
                            .text_color(muted)
                            .child(owner.t("↑↓ navigate · Enter run · Esc close"))
                            .child(div().text_color(accent).child(format!(
                                "{}/{}",
                                if items.is_empty() { 0 } else { selected + 1 },
                                items.len()
                            ))),
                    ),
            )
    }
}
