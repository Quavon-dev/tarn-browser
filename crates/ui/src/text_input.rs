// Adapted from GPUI's `examples/input.rs` (Zed Industries, Apache-2.0).

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    GlobalElementId, KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, ShapedLine, SharedString,
    Style, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div, fill, point, prelude::*, px, relative, size,
};

use engine::menu::Item;

use crate::theme::Theme;
use unicode_segmentation::*;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        DeleteToBeginning,
        DeleteToEnd,
        DeleteWordLeft,
        DeleteWordRight,
        Left,
        Right,
        WordLeft,
        WordRight,
        SelectLeft,
        SelectRight,
        SelectWordLeft,
        SelectWordRight,
        SelectToBeginning,
        SelectToEnd,
        SelectAll,
        Home,
        End,
        ShowCharacterPalette,
        Paste,
        Cut,
        Copy,
        Submit,
        Cancel,
        Up,
        Down,
    ]
);

pub enum TextInputEvent {
    Changed,
    Submit,
    Cancel,
    Up,
    Down,
    /// The context menu closed; a pop-up that closes on losing activation checks it again here.
    MenuClosed,
}

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("TextInput");
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("left", Left, ctx),
        KeyBinding::new("right", Right, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("secondary-a", SelectAll, ctx),
        KeyBinding::new("secondary-v", Paste, ctx),
        KeyBinding::new("secondary-c", Copy, ctx),
        KeyBinding::new("secondary-x", Cut, ctx),
        KeyBinding::new("home", Home, ctx),
        KeyBinding::new("end", End, ctx),
        KeyBinding::new("shift-home", SelectToBeginning, ctx),
        KeyBinding::new("shift-end", SelectToEnd, ctx),
        KeyBinding::new("cmd-left", Home, ctx),
        KeyBinding::new("cmd-right", End, ctx),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, ctx),
        KeyBinding::new("enter", Submit, ctx),
        KeyBinding::new("escape", Cancel, ctx),
        KeyBinding::new("up", Up, ctx),
        KeyBinding::new("down", Down, ctx),
    ]);
    cx.bind_keys(editing_keys(ctx));
}

/// The system's own text editing keys: ⌘ for line ends, ⌥ for words and the Emacs keys.
#[cfg(target_os = "macos")]
fn editing_keys(ctx: Option<&str>) -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("shift-cmd-left", SelectToBeginning, ctx),
        KeyBinding::new("shift-cmd-right", SelectToEnd, ctx),
        KeyBinding::new("cmd-backspace", DeleteToBeginning, ctx),
        KeyBinding::new("cmd-delete", DeleteToEnd, ctx),
        KeyBinding::new("alt-left", WordLeft, ctx),
        KeyBinding::new("alt-right", WordRight, ctx),
        KeyBinding::new("alt-shift-left", SelectWordLeft, ctx),
        KeyBinding::new("alt-shift-right", SelectWordRight, ctx),
        KeyBinding::new("alt-backspace", DeleteWordLeft, ctx),
        KeyBinding::new("alt-delete", DeleteWordRight, ctx),
        KeyBinding::new("ctrl-a", Home, ctx),
        KeyBinding::new("ctrl-e", End, ctx),
        KeyBinding::new("ctrl-k", DeleteToEnd, ctx),
    ]
}

/// The system's own text editing keys: Ctrl for words.
#[cfg(not(target_os = "macos"))]
fn editing_keys(ctx: Option<&str>) -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("secondary-left", WordLeft, ctx),
        KeyBinding::new("secondary-right", WordRight, ctx),
        KeyBinding::new("secondary-shift-left", SelectWordLeft, ctx),
        KeyBinding::new("secondary-shift-right", SelectWordRight, ctx),
        KeyBinding::new("secondary-backspace", DeleteWordLeft, ctx),
        KeyBinding::new("secondary-delete", DeleteWordRight, ctx),
    ]
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
    inserted: bool,
    paste_and_go: bool,
    menu_open: bool,
}

/// An entry of a field's context menu.
#[derive(Clone, Copy)]
enum Edit {
    Cut,
    Copy,
    Paste,
    PasteAndGo,
    SelectAll,
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl TextInput {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        TextInput {
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
            inserted: false,
            paste_and_go: false,
            menu_open: false,
        }
    }

    /// Offers Paste and Go in the field's context menu, for the address field.
    pub fn with_paste_and_go(mut self) -> Self {
        self.paste_and_go = true;
        self
    }

    /// Replaces the content with the clipboard's text and submits it, as pasting and pressing
    /// Enter would.
    pub fn paste_and_go(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // An IME composition would otherwise be the range the paste replaces.
        self.marked_range = None;
        self.selected_range = 0..self.content.len();
        self.paste(&Paste, window, cx);
        cx.emit(TextInputEvent::Submit);
    }

    /// Shows bullets instead of the text, for passwords.
    pub fn masked(mut self) -> Self {
        self.masked = true;
        self
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>) {
        self.placeholder = placeholder.into();
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    /// Replaces the content and selects all of it.
    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.reset();
        self.content = text.into();
        self.selected_range = 0..self.content.len();
        cx.notify();
    }

    /// Whether the last edit typed text rather than removing or pasting it.
    pub fn inserted(&self) -> bool {
        self.inserted
    }

    /// Whether the caret sits at the end with nothing selected and no IME composition open.
    pub fn caret_at_end(&self) -> bool {
        self.marked_range.is_none() && self.selected_range == (self.content.len()..self.content.len())
    }

    /// Appends `suffix` selected, so typing replaces it and Backspace removes it.
    pub fn append_selected(&mut self, suffix: &str, cx: &mut Context<Self>) {
        let start = self.content.len();
        self.content = format!("{}{suffix}", self.content).into();
        self.selected_range = start..self.content.len();
        self.selection_reversed = false;
        cx.notify();
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextInputEvent::Submit);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextInputEvent::Cancel);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextInputEvent::Up);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextInputEvent::Down);
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

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(word_left(&self.content, self.selected_range.start), cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(word_right(&self.content, self.selected_range.end), cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(word_left(&self.content, self.cursor_offset()), cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(word_right(&self.content, self.cursor_offset()), cx);
    }

    fn select_to_beginning(&mut self, _: &SelectToBeginning, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    fn select_to_end(&mut self, _: &SelectToEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.content.len(), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(word_left(&self.content, self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete_word_right(&mut self, _: &DeleteWordRight, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(word_right(&self.content, self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete_to_beginning(&mut self, _: &DeleteToBeginning, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_through(0, window, cx)
    }

    fn delete_to_end(&mut self, _: &DeleteToEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_through(self.content.len(), window, cx)
    }

    /// Deletes the selection together with everything between it and `offset`.
    fn delete_through(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = extend(&self.selected_range, offset);
        self.selection_reversed = false;
        self.replace_text_in_range(None, "", window, cx)
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = true;

        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    /// Cut, Copy, Paste and Select All, by mouse. The menu runs the system's modal tracking
    /// loop, so it opens from a task, outside this update.
    fn on_right_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        window.focus(&self.focus_handle);
        let at = self.index_for_mouse_position(event.position);
        if !(self.selected_range.start..=self.selected_range.end).contains(&at) {
            self.move_to(at, cx);
        }
        let Some(host) = engine::Host::from_window(window) else { return };
        let selected = !self.selected_range.is_empty() && !self.masked;
        let clipboard = cx.read_from_clipboard().and_then(|item| item.text()).is_some_and(|t| !t.trim().is_empty());
        let mut items = vec![
            Item::action("Cut", Edit::Cut).enabled(selected),
            Item::action("Copy", Edit::Copy).enabled(selected),
            Item::action("Paste", Edit::Paste).enabled(clipboard),
        ];
        if self.paste_and_go {
            items.push(Item::action("Paste and Go", Edit::PasteAndGo).enabled(clipboard));
        }
        items.extend([Item::Separator, Item::action("Select All", Edit::SelectAll).enabled(!self.content.is_empty())]);
        self.menu_open = true;
        cx.spawn_in(window, async move |this, cx| {
            let chosen = Rc::new(Cell::new(None));
            let pick = chosen.clone();
            engine::menu::show(&host, items, None, move |edit| pick.set(Some(edit)));
            this.update_in(cx, |this, window, cx| {
                this.menu_open = false;
                cx.emit(TextInputEvent::MenuClosed);
                match chosen.get() {
                    Some(Edit::Cut) => this.cut(&Cut, window, cx),
                    Some(Edit::Copy) => this.copy(&Copy, window, cx),
                    Some(Edit::Paste) => this.paste(&Paste, window, cx),
                    Some(Edit::PasteAndGo) => this.paste_and_go(window, cx),
                    Some(Edit::SelectAll) => this.select_all(&SelectAll, window, cx),
                    None => {}
                }
            })
            .ok();
        })
        .detach();
    }

    /// Whether the field's context menu is open: the native menu takes the window's activation,
    /// so a pop-up that closes when it loses activation must stay open meanwhile.
    pub fn menu_open(&self) -> bool {
        self.menu_open
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn show_character_palette(&mut self, _: &ShowCharacterPalette, window: &mut Window, _: &mut Context<Self>) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let line: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
            self.replace_text_in_range(None, &line, window, cx);
            self.inserted = false;
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() && !self.masked {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected_range.clone()].to_string()));
        }
    }
    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() && !self.masked {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected_range.clone()].to_string()));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed { self.selected_range.start } else { self.selected_range.end }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        if self.masked {
            return self.content.len();
        }

        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref()) else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        line.closest_index_for_x(position.x - bounds.left())
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
        self.content.grapheme_indices(true).rev().find_map(|(idx, _)| (idx < offset).then_some(idx)).unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content.grapheme_indices(true).find_map(|(idx, _)| (idx > offset).then_some(idx)).unwrap_or(self.content.len())
    }

    pub fn reset(&mut self) {
        self.content = "".into();
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.last_layout = None;
        self.last_bounds = None;
        self.is_selecting = false;
        self.inserted = false;
    }
}

/// `range` grown to reach `offset`, so an inline completion is removed along with the typed text.
fn extend(range: &Range<usize>, offset: usize) -> Range<usize> {
    range.start.min(offset)..range.end.max(offset)
}

fn is_word(segment: &str) -> bool {
    segment.chars().any(char::is_alphanumeric)
}

/// The start of the word before `offset`, past any spaces or punctuation in between.
fn word_left(text: &str, offset: usize) -> usize {
    text[..offset].split_word_bound_indices().rev().find(|(_, segment)| is_word(segment)).map_or(0, |(ix, _)| ix)
}

/// The end of the word after `offset`, past any spaces or punctuation in between.
fn word_right(text: &str, offset: usize) -> usize {
    text[offset..].split_word_bound_indices().find(|(_, segment)| is_word(segment)).map_or(text.len(), |(ix, segment)| offset + ix + segment.len())
}

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
        if self.masked {
            return Some("•".repeat(self.content[range].chars().count()));
        }
        Some(self.content[range].to_string())
    }

    fn selected_text_range(&mut self, _ignore_disabled_input: bool, _window: &mut Window, _cx: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.range_to_utf16(&self.selected_range), reversed: self.selection_reversed })
    }

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(&mut self, range_utf16: Option<Range<usize>>, new_text: &str, _: &mut Window, cx: &mut Context<Self>) {
        let range =
            range_utf16.as_ref().map(|range_utf16| self.range_from_utf16(range_utf16)).or(self.marked_range.clone()).unwrap_or(self.selected_range.clone());

        self.content = (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..]).into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        self.inserted = !new_text.is_empty();
        cx.emit(TextInputEvent::Changed);
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
        if self.masked {
            return self.replace_text_in_range(range_utf16, new_text, _window, cx);
        }
        let range =
            range_utf16.as_ref().map(|range_utf16| self.range_from_utf16(range_utf16)).or(self.marked_range.clone()).unwrap_or(self.selected_range.clone());

        self.content = (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..]).into();
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

        cx.notify();
    }

    fn bounds_for_range(&mut self, range_utf16: Range<usize>, bounds: Bounds<Pixels>, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let last_layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(bounds.left() + last_layout.x_for_index(range.start), bounds.top()),
            point(bounds.left() + last_layout.x_for_index(range.end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(&mut self, point: gpui::Point<Pixels>, _window: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        let line_point = self.last_bounds?.localize(&point)?;
        let last_layout = self.last_layout.as_ref()?;

        if last_layout.text != self.content {
            return None;
        }
        let utf8_index = last_layout.index_for_x(point.x - line_point.x)?;
        Some(self.offset_to_utf16(utf8_index))
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
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

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
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
        style.size.height = window.line_height().into();
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
        let content = input.content.clone();
        let selected_range = input.selected_range.clone();
        let cursor = input.cursor_offset();
        let style = window.text_style();
        let masked = input.masked;
        let to_display = |offset: usize| {
            if masked { content[..offset].chars().count() * '•'.len_utf8() } else { offset }
        };
        let (cursor, selected_range) = (to_display(cursor), to_display(selected_range.start)..to_display(selected_range.end));

        let (display_text, text_color) = if content.is_empty() {
            (input.placeholder.clone(), Theme::faint().into())
        } else if masked {
            ("•".repeat(content.chars().count()).into(), style.color)
        } else {
            (content.clone(), style.color)
        };

        let run = TextRun { len: display_text.len(), font: style.font(), color: text_color, background_color: None, underline: None, strikethrough: None };
        let runs = if let Some(marked_range) = input.marked_range.as_ref().filter(|_| !masked) {
            vec![
                TextRun { len: marked_range.start, ..run.clone() },
                TextRun {
                    len: marked_range.end - marked_range.start,
                    underline: Some(UnderlineStyle { color: Some(run.color), thickness: px(1.0), wavy: false }),
                    ..run.clone()
                },
                TextRun { len: display_text.len() - marked_range.end, ..run },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![run]
        };

        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window.text_system().shape_line(display_text, font_size, &runs, None);

        let cursor_pos = line.x_for_index(cursor);
        let (selection, cursor) = if selected_range.is_empty() {
            (None, Some(fill(Bounds::new(point(bounds.left() + cursor_pos, bounds.top()), size(px(2.), bounds.bottom() - bounds.top())), Theme::accent())))
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(bounds.left() + line.x_for_index(selected_range.start), bounds.top()),
                        point(bounds.left() + line.x_for_index(selected_range.end), bounds.bottom()),
                    ),
                    Theme::accent_soft(),
                )),
                None,
            )
        };
        PrepaintState { line: Some(line), cursor, selection }
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
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(&focus_handle, ElementInputHandler::new(bounds, self.input.clone()), cx);
        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection)
        }
        let Some(line) = prepaint.line.take() else {
            return;
        };
        line.paint(bounds.origin, window.line_height(), window, cx).ok();

        if focus_handle.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        self.input.update(cx, |input, _cx| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .key_context("TextInput")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_to_beginning))
            .on_action(cx.listener(Self::delete_to_end))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_to_beginning))
            .on_action(cx.listener(Self::select_to_end))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .w_full()
            .line_height(px(22.))
            .text_color(Theme::text())
            .child(TextElement { input: cx.entity() })
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{extend, word_left, word_right};

    #[test]
    fn delete_to_beginning_takes_everything_before_the_caret() {
        assert_eq!(extend(&(5..5), 0), 0..5);
    }

    #[test]
    fn delete_to_beginning_takes_a_selected_completion_too() {
        assert_eq!(extend(&(3..11), 0), 0..11);
    }

    #[test]
    fn delete_to_end_takes_everything_after_the_selection() {
        assert_eq!(extend(&(2..4), 9), 2..9);
    }

    #[test]
    fn word_left_stops_at_the_start_of_each_word() {
        let url = "https://example.com/some-path";
        assert_eq!(word_left(url, url.len()), 25);
        assert_eq!(word_left(url, 25), 20);
        assert_eq!(word_left(url, 20), 8, "a host is one word");
        assert_eq!(word_left(url, 8), 0);
        assert_eq!(word_left(url, 3), 0);
        assert_eq!(word_left(url, 0), 0);
    }

    #[test]
    fn word_right_stops_at_the_end_of_each_word() {
        let query = "  rust   gpui ";
        assert_eq!(word_right(query, 0), 6);
        assert_eq!(word_right(query, 6), 13);
        assert_eq!(word_right(query, 13), query.len());
    }

    #[test]
    fn words_hold_non_latin_letters() {
        let text = "grüße Αθήνα";
        assert_eq!(word_left(text, text.len()), "grüße ".len());
        assert_eq!(word_right(text, 0), "grüße".len());
    }
}
