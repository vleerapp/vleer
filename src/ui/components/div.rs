use gpui::*;

#[inline(always)]
pub fn flex_col() -> Div {
    div().flex().flex_col()
}

#[inline(always)]
pub fn flex_row() -> Div {
    div().flex().flex_row().items_center()
}
