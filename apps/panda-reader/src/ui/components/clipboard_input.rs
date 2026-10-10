use gpui_kit::component::input::{Input, InputState};
use gpui_kit::*;

/// Single-line field sized so placeholder glyphs are not clipped when rem ≠ 16.
pub(crate) fn text_input(state: &Entity<InputState>) -> Input {
    Input::new(state)
        .h(px(34.))
        .py(px(5.))
}
