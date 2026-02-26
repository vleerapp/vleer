use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    GlobalElementId, IntoElement, KeyBinding, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Render, Rgba, ShapedLine, SharedString,
    Style, TextRun, UTF16Selection, Window, actions, fill, point, prelude::*, px, relative, rgba,
    size,
};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;
use std::time::Instant;
use unicode_segmentation::*;

use crate::ui::{
    components::{div::flex_row, icons::icon::icon},
    global_actions::PlayPause,
    variables::Variables,
};

actions!(
    input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        Paste,
        Cut,
        Copy,
        Enter,
        DeleteToPreviousWord,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight
    ]
);

pub fn bind_input_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, None),
        KeyBinding::new("delete", Delete, None),
        KeyBinding::new("left", Left, None),
        KeyBinding::new("right", Right, None),
        KeyBinding::new("shift-left", SelectLeft, None),
        KeyBinding::new("shift-right", SelectRight, None),
        KeyBinding::new("home", Home, None),
        KeyBinding::new("end", End, None),
        KeyBinding::new("enter", Enter, None),
        KeyBinding::new("secondary-a", SelectAll, None),
        KeyBinding::new("secondary-v", Paste, None),
        KeyBinding::new("secondary-c", Copy, None),
        KeyBinding::new("secondary-x", Cut, None),
        KeyBinding::new("secondary-backspace", DeleteToPreviousWord, None),
    ]);

    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("alt-backspace", DeleteToPreviousWord, None),
        KeyBinding::new("alt-left", WordLeft, None),
        KeyBinding::new("alt-right", WordRight, None),
        KeyBinding::new("alt-shift-left", SelectWordLeft, None),
        KeyBinding::new("alt-shift-right", SelectWordRight, None),
    ]);

    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([
        KeyBinding::new("secondary-left", WordLeft, None),
        KeyBinding::new("secondary-right", WordRight, None),
        KeyBinding::new("secondary-shift-left", SelectWordLeft, None),
        KeyBinding::new("secondary-shift-right", SelectWordRight, None),
    ]);
}

pub struct TextInput {
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    masked: bool,
    icon_path: Option<SharedString>,
    blink_start: Instant,
    scroll_offset: Pixels,
    bg_color: Option<Rgba>,
    text_color: Option<Rgba>,
    centered: bool,
    custom_height: Option<Pixels>,
    validator: Option<Rc<RefCell<dyn Fn(&str) -> bool>>>,
    last_click_time: Option<Instant>,
    last_click_position: Point<Pixels>,
    click_count: u8,
}

impl TextInput {
    pub fn new(cx: &mut Context<Self>, placeholder: impl Into<SharedString>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: "".into(),
            placeholder: placeholder.into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            masked: false,
            icon_path: None,
            blink_start: Instant::now(),
            scroll_offset: px(0.0),
            bg_color: None,
            text_color: None,
            centered: false,
            custom_height: None,
            validator: None,
            last_click_time: None,
            last_click_position: Point::new(px(0.0), px(0.0)),
            click_count: 0,
        }
    }

    pub fn with_icon(mut self, icon_path: impl Into<SharedString>) -> Self {
        self.icon_path = Some(icon_path.into());
        self
    }

    pub fn with_background(mut self, color: impl Into<Rgba>) -> Self {
        self.bg_color = Some(color.into());
        self
    }

    pub fn with_text_color(mut self, color: impl Into<Rgba>) -> Self {
        self.text_color = Some(color.into());
        self
    }

    pub fn centered(mut self) -> Self {
        self.centered = true;
        self
    }

    pub fn with_height(mut self, height: impl Into<Pixels>) -> Self {
        self.custom_height = Some(height.into());
        self
    }

    pub fn with_validator(mut self, validator: impl Fn(&str) -> bool + 'static) -> Self {
        self.validator = Some(Rc::new(RefCell::new(validator)));
        self
    }

    pub fn with_text(mut self, text: impl Into<SharedString>) -> Self {
        let text: SharedString = text.into();
        self.selected_range = 0..text.len();
        self.content = text;
        self.selected_range = self.content.len()..self.content.len();
        self
    }

    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        let text: SharedString = text.into();
        self.content = text;
        self.selected_range = self.content.len()..self.content.len();
        self.marked_range = None;
        cx.notify();
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_word_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.previous_word_boundary(self.selected_range.start), cx);
        }
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_word_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.next_word_boundary(self.selected_range.end), cx);
        }
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_word_boundary(self.cursor_offset()), cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_word_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn select_all_internal(&mut self, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        self.blink_start = Instant::now();
        cx.notify();
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn space(&mut self, _: &PlayPause, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(None, " ", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete_to_previous_word(
        &mut self,
        _: &DeleteToPreviousWord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            let offset = self.cursor_offset();
            let prev_word_boundary = self.previous_word_boundary(offset);
            self.select_to(prev_word_boundary, cx);
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window, cx);

        let click_time = Instant::now();
        let double_click_time = Duration::from_millis(500);
        let click_threshold = px(5.0);

        let is_quick_click = self
            .last_click_time
            .map(|last_time| {
                click_time.duration_since(last_time) < double_click_time
                    && (event.position.x - self.last_click_position.x).abs() < click_threshold
                    && (event.position.y - self.last_click_position.y).abs() < click_threshold
            })
            .unwrap_or(false);

        if is_quick_click {
            self.click_count = (self.click_count + 1).min(3);
        } else {
            self.click_count = 1;
        }

        self.last_click_time = Some(click_time);
        self.last_click_position = event.position;

        self.is_selecting = true;

        if self.click_count >= 3 {
            self.select_all_internal(cx);
        } else if self.click_count == 2 {
            let pos = self.index_for_mouse_position(event.position);
            let word_start = self.previous_word_boundary(pos);
            let word_end = self.next_word_boundary(pos);
            self.selected_range = word_start..word_end;
            self.selection_reversed = false;
            self.blink_start = Instant::now();
            cx.notify();
        } else if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace("\n", " "), window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Submit(self.content.to_string()));
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.blink_start = Instant::now();
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn horizontal_text_offset(&self, bounds_width: Pixels, line_width: Pixels) -> Pixels {
        if self.centered {
            ((bounds_width - line_width).max(px(0.0))) / 2.0
        } else {
            px(0.0)
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        let x_offset = self.horizontal_text_offset(bounds.size.width, line.width);
        line.closest_index_for_x(position.x - bounds.left() + self.scroll_offset - x_offset)
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.blink_start = Instant::now();
        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn previous_word_boundary(&self, offset: usize) -> usize {
        self.content
            .unicode_word_indices()
            .map(|(idx, _)| idx)
            .filter(|&idx| idx < offset)
            .last()
            .unwrap_or(0)
    }

    fn next_word_boundary(&self, offset: usize) -> usize {
        self.content
            .unicode_word_indices()
            .map(|(idx, _)| idx)
            .find(|&idx| idx > offset)
            .unwrap_or(self.content.len())
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }
}

pub enum InputEvent {
    Change(String),
    Submit(String),
}

impl EventEmitter<InputEvent> for TextInput {}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let new_content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .to_string();

        let is_valid = new_content.is_empty()
            || self
                .validator
                .as_ref()
                .map(|v| (v.borrow())(&new_content))
                .unwrap_or(true);

        if !is_valid {
            return;
        }

        self.content = new_content.into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        self.blink_start = Instant::now();
        cx.emit(InputEvent::Change(self.content.to_string()));
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.end)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        self.blink_start = Instant::now();
        cx.emit(InputEvent::Change(self.content.to_string()));
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let last_layout = self.last_layout.as_ref()?;
        let x_offset = self.horizontal_text_offset(bounds.size.width, last_layout.width);
        let origin_x = bounds.left() - self.scroll_offset + x_offset;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(
                origin_x + last_layout.x_for_index(range.start),
                bounds.top(),
            ),
            point(
                origin_x + last_layout.x_for_index(range.end),
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let last_bounds = self.last_bounds.as_ref()?;
        let last_layout = self.last_layout.as_ref()?;
        let x_offset = self.horizontal_text_offset(last_bounds.size.width, last_layout.width);
        let utf8_index = last_layout
            .index_for_x(point.x - last_bounds.left() + self.scroll_offset - x_offset)?;
        Some(self.offset_to_utf16(utf8_index))
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.focus_handle.clone();
        let is_focused = focus.is_focused(window);
        let variables = cx.global::<Variables>();

        if is_focused {
            window.request_animation_frame();
        }

        let active_color = if is_focused || !self.content.is_empty() {
            variables.text
        } else {
            variables.border
        };

        flex_row()
            .items_center()
            .flex_shrink_0()
            .key_context("TextInput")
            .track_focus(&focus)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::space))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_to_previous_word))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::enter))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down_out(|_, window, _| window.blur())
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .bg(self.bg_color.unwrap_or(variables.element))
            .size_full()
            .h(self.custom_height.unwrap_or(px(variables.padding_32)))
            .px(px(variables.padding_8))
            .gap(px(variables.padding_8))
            .when_some(self.icon_path.clone(), |this, path| {
                this.child(icon(path).text_color(active_color))
            })
            .child(TextElement {
                input: cx.entity(),
                is_focused,
                centered: self.centered,
                custom_text_color: self.text_color,
            })
    }
}

struct TextElement {
    input: Entity<TextInput>,
    is_focused: bool,
    centered: bool,
    custom_text_color: Option<Rgba>,
}

struct PrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
    scroll_offset: Pixels,
    x_offset: Pixels,
    y_offset: Pixels,
}

impl IntoElement for TextElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let variables = cx.global::<Variables>();

        let content = if input.masked {
            "*".repeat(input.content.chars().count()).into()
        } else {
            input.content.clone()
        };
        let selected_range = input.selected_range.clone();
        let cursor_index = input.cursor_offset();
        let style = window.text_style();

        let default_text_color = self.custom_text_color.unwrap_or(variables.text);
        let (display_text, text_color) = if content.is_empty() {
            (
                input.placeholder.clone(),
                if self.is_focused {
                    variables.text
                } else {
                    variables.border
                },
            )
        } else {
            (content, default_text_color)
        };

        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: text_color.into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = vec![run];
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display_text, font_size, &runs, None);

        let line_height = window.line_height();
        let y_offset = (bounds.size.height - line_height) / 2.0;

        let mut scroll_offset = input.scroll_offset;
        let cursor_x = line.x_for_index(cursor_index);
        let width = bounds.size.width;
        let x_offset = if self.centered {
            ((width - line.width).max(px(0.0))) / 2.0
        } else {
            px(0.0)
        };

        let cursor_width = px(2.0);

        if cursor_x < scroll_offset {
            scroll_offset = cursor_x;
        } else if cursor_x + cursor_width > scroll_offset + width {
            scroll_offset = cursor_x + cursor_width - width;
        }

        let max_scroll = (line.width + cursor_width - width).max(px(0.));
        scroll_offset = scroll_offset.min(max_scroll);
        scroll_offset = scroll_offset.max(px(0.));

        let cursor_pos = cursor_x - scroll_offset + x_offset;

        let (selection, cursor) = if selected_range.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(bounds.left() + cursor_pos, bounds.top() + y_offset),
                        size(px(1.5), line_height),
                    ),
                    variables.text,
                )),
            )
        } else {
            let start_x = line.x_for_index(selected_range.start) - scroll_offset + x_offset;
            let end_x = line.x_for_index(selected_range.end) - scroll_offset + x_offset;
            (
                Some(fill(
                    Bounds::from_corners(
                        point(bounds.left() + start_x, bounds.top() + y_offset),
                        point(bounds.left() + end_x, bounds.bottom() - y_offset),
                    ),
                    rgba(0xA058FF4f),
                )),
                None,
            )
        };

        PrepaintState {
            line: Some(line),
            cursor,
            selection,
            scroll_offset,
            x_offset,
            y_offset,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let input_entity = self.input.read(cx);
        let focus_handle = input_entity.focus_handle.clone();
        let blink_start = input_entity.blink_start;

        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if self.is_focused {
                if let Some(selection) = prepaint.selection.take() {
                    window.paint_quad(selection);
                }
            }

            if let Some(line) = prepaint.line.as_ref() {
                let origin = point(
                    bounds.left() - prepaint.scroll_offset + prepaint.x_offset,
                    bounds.top() + prepaint.y_offset,
                );
                line.paint(
                    origin,
                    window.line_height(),
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                )
                .unwrap();
            }

            if focus_handle.is_focused(window) {
                let elapsed = blink_start.elapsed().as_millis();
                let blink_on = (elapsed / 500) % 2 == 0;

                if blink_on {
                    if let Some(cursor) = prepaint.cursor.take() {
                        window.paint_quad(cursor);
                    }
                }
            }
        });

        let line = prepaint.line.take();
        let scroll_offset = prepaint.scroll_offset;

        self.input.update(cx, |input, _cx| {
            if let Some(l) = line {
                input.last_layout = Some(l);
            }
            input.last_bounds = Some(bounds);
            input.scroll_offset = scroll_offset;
        });
    }
}
