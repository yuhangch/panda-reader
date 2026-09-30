use gpui_kit::component::input::{Input, InputState};
use gpui_kit::*;

/// Single-line field sized so placeholder glyphs are not clipped when rem ≠ 16.
pub(crate) fn text_input(state: &Entity<InputState>) -> Input {
    let owned = state.clone();
    Input::new(state)
        .h(px(34.))
        .py(px(5.))
        .on_paste(move |item, window, cx| {
            if item.text().is_some_and(|text| !text.is_empty()) {
                return false;
            }
            let Some(text) = read_plain_text() else {
                return false;
            };
            let text = text.replace(['\r', '\n'], "");
            if text.is_empty() {
                return false;
            }
            owned.update(cx, |input, cx| {
                input.replace(text, window, cx);
            });
            true
        })
}

fn read_plain_text() -> Option<String> {
    arboard::Clipboard::new()
        .ok()?
        .get_text()
        .ok()
        .filter(|text| !text.is_empty())
}
