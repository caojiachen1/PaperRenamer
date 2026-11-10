//! 单行文本输入框：地址栏编辑、重命名、搜索框都用它。
//! 参考 gpui 0.2.2 examples/input.rs 的实现，按单行 + 水平滚动简化。

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    point, prelude::*, size, App, Bounds, ClipboardItem, Context, Element,
    ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId,
    InspectorElementId, KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Pixels, Point, ShapedLine, Style, TextRun, UnderlineStyle, UTF16Selection,
    Window, fill, hsla, px, SharedString, div,
};

use crate::theme::Theme;

gpui::actions!(
    field,
    [
        Backspace,
        DeleteForward,
        MoveLeft,
        MoveRight,
        SelectLeft,
        SelectRight,
        MoveHome,
        MoveEnd,
        SelectAllText,
        PasteText,
        CutText,
        CopyText,
        Submit,
        Cancel
    ]
);

/// 文本框在 TextInput 上下文里的键位。
pub fn key_bindings() -> Vec<KeyBinding> {
    let c = Some("TextInput");
    vec![
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("delete", DeleteForward, c),
        KeyBinding::new("left", MoveLeft, c),
        KeyBinding::new("right", MoveRight, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("home", MoveHome, c),
        KeyBinding::new("end", MoveEnd, c),
        KeyBinding::new("ctrl-a", SelectAllText, c),
        KeyBinding::new("ctrl-v", PasteText, c),
        KeyBinding::new("ctrl-c", CopyText, c),
        KeyBinding::new("ctrl-x", CutText, c),
        KeyBinding::new("enter", Submit, c),
        KeyBinding::new("kp_enter", Submit, c),
        KeyBinding::new("escape", Cancel, c),
    ]
}

type Callback = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;

/// 单行输入框视图。
pub struct Field {
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    scroll_x: Pixels,
    theme: Theme,
    mono: bool,
    on_submit: Option<Callback>,
    on_cancel: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    on_change: Option<Callback>,
    /// 失焦时视为提交（重命名用）
    commit_on_blur: bool,
}

impl Focusable for Field {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Field {
    pub fn new(cx: &mut Context<Self>, theme: Theme) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: SharedString::default(),
            placeholder: "".into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            scroll_x: px(0.0),
            theme,
            mono: false,
            on_submit: None,
            on_cancel: None,
            on_change: None,
            commit_on_blur: false,
        }
    }

    pub fn set_placeholder(&mut self, p: impl Into<SharedString>) {
        self.placeholder = p.into();
    }

    pub fn set_mono(&mut self, on: bool) {
        self.mono = on;
    }

    pub fn set_commit_on_blur(&mut self, on: bool) {
        self.commit_on_blur = on;
    }

    pub fn on_submit(&mut self, f: impl Fn(SharedString, &mut Window, &mut App) + 'static) {
        self.on_submit = Some(Rc::new(f));
    }
    pub fn on_cancel(&mut self, f: impl Fn(&mut Window, &mut App) + 'static) {
        self.on_cancel = Some(Rc::new(f));
    }
    pub fn on_change(&mut self, f: impl Fn(SharedString, &mut Window, &mut App) + 'static) {
        self.on_change = Some(Rc::new(f));
    }

    /// 供父视图转移焦点。
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    pub fn value(&self) -> SharedString {
        self.content.clone()
    }

    /// 设置内容并全选，便于立刻覆盖输入。
    pub fn text(&self) -> SharedString {
        self.content.clone()
    }

    pub fn set_value_start(&mut self, v: impl Into<SharedString>) {
        self.content = v.into();
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.scroll_x = px(0.0);
    }

    pub fn set_value(&mut self, v: impl Into<SharedString>, select_all: bool) {
        self.content = v.into();
        if select_all {
            self.selected_range = 0..self.content.len();
            self.selection_reversed = false;
        } else {
            let len = self.content.len();
            self.selected_range = len..len;
        }
        self.scroll_x = px(0.0);
        self.last_layout = None;
    }

    /// 只选中文件名主干（不含最后一个扩展名），资源管理器重命名的行为。
    pub fn set_value_stem(&mut self, v: impl Into<SharedString>) {
        let s: SharedString = v.into();
        self.content = s.clone();
        let cut = s.rfind('.').filter(|i| *i > 0).unwrap_or(s.len());
        self.selected_range = 0..cut;
        self.selection_reversed = false;
        self.last_layout = None;
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
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
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .char_indices()
            .filter(|(i, _)| *i < offset)
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .char_indices()
            .find(|(i, _)| *i > offset)
            .map(|(i, _)| i)
            .unwrap_or(self.content.len())
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

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) = (self.last_bounds, self.last_layout.as_ref()) else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        line.closest_index_for_x(position.x - bounds.left() + self.scroll_x)
    }

    /// 回调延到本次更新结束后再跑：Explorer 在这些回调里会回来更新本输入框，
    /// 直接调用会触发 gpui 的"实体重入更新"崩溃。
    fn do_submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cb = self.on_submit.clone();
        let v = self.content.clone();
        if let Some(cb) = cb {
            window.defer(cx, move |window, cx| cb(v, window, cx));
        }
    }

    fn do_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cb = self.on_cancel.clone();
        if let Some(cb) = cb {
            window.defer(cx, move |window, cx| cb(window, cx));
        }
    }

    fn after_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        self.last_layout = None;
        let cb = self.on_change.clone();
        let v = self.content.clone();
        if let Some(cb) = cb {
            window.defer(cx, move |window, cx| cb(v, window, cx));
        }
        cx.notify();
    }
}

impl EntityInputHandler for Field {
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
            range: self.range_to_utf16(&self.selected_range.clone()),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
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
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .unwrap_or_else(|| self.selected_range.clone());
        let mut content = String::new();
        content.push_str(&self.content[..range.start]);
        content.push_str(new_text);
        content.push_str(&self.content[range.end..]);
        let new_end = range.start + new_text.len();
        self.content = content.into();
        self.selected_range = new_end..new_end;
        self.selection_reversed = false;
        self.after_edit(_window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        source: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .unwrap_or_else(|| self.selected_range.clone());
        let mut content = String::new();
        content.push_str(&self.content[..range.start]);
        content.push_str(new_text);
        content.push_str(&self.content[range.end..]);
        self.content = content.into();
        self.selected_range = range.start..range.start + new_text.len();
        self.marked_range = Some(self.selected_range.clone());
        let _ = source;
        self.after_edit(_window, cx);
    }
    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let last_bounds = self.last_bounds?;
        let last_layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(
                bounds.left() + last_layout.x_for_index(range.start) - self.scroll_x,
                last_bounds.top(),
            ),
            point(
                bounds.left() + last_layout.x_for_index(range.end) - self.scroll_x,
                last_bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let line_point = self.last_bounds?.localize(&point)?;
        let last_layout = self.last_layout.as_ref()?;
        let utf8_index = last_layout.index_for_x(point.x - line_point.x + self.scroll_x)?;
        Some(self.offset_to_utf16(utf8_index))
    }
}

impl Render for Field {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("field")
            .key_context("TextInput")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_hidden()
            .cursor_text()
            .line_height(px(20.0))
            .text_size(self.theme.font_size)
            .font_family(if self.mono {
                self.theme.mono.clone()
            } else {
                self.theme.font.clone()
            })
            .text_color(self.theme.text)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_action(cx.listener(|this, _: &MoveLeft, _, cx| {
                let c = this.cursor_offset();
                if this.selection_reversed {
                    this.selected_range.end = this.previous_boundary(c);
                } else {
                    this.selected_range.start = this.previous_boundary(c);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &MoveRight, _, cx| {
                let c = this.cursor_offset();
                if this.selection_reversed {
                    this.selected_range.end = this.next_boundary(c);
                } else {
                    this.selected_range.start = this.next_boundary(c);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SelectLeft, _, cx| {
                let p = this.previous_boundary(this.cursor_offset());
                this.select_to(p, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectRight, _, cx| {
                let n = this.next_boundary(this.cursor_offset());
                this.select_to(n, cx);
            }))
            .on_action(cx.listener(|this, _: &MoveHome, _, cx| this.move_to(0, cx)))
            .on_action(cx.listener(|this, _: &MoveEnd, _, cx| {
                let l = this.content.len();
                this.move_to(l, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectAllText, _, cx| {
                let l = this.content.len();
                this.move_to(0, cx);
                this.select_to(l, cx);
            }))
            .on_action(cx.listener(|this, _: &Backspace, w, cx| {
                if this.selected_range.is_empty() {
                    let p = this.previous_boundary(this.cursor_offset());
                    this.select_to(p, cx);
                }
                this.replace_text_in_range(None, "", w, cx);
            }))
            .on_action(cx.listener(|this, _: &DeleteForward, w, cx| {
                if this.selected_range.is_empty() {
                    let n = this.next_boundary(this.cursor_offset());
                    this.select_to(n, cx);
                }
                this.replace_text_in_range(None, "", w, cx);
            }))
            .on_action(cx.listener(|this, _: &CopyText, _, cx| {
                if !this.selected_range.is_empty() {
                    let s = this.content[this.selected_range.clone()].to_string();
                    cx.write_to_clipboard(ClipboardItem::new_string(s));
                }
            }))
            .on_action(cx.listener(|this, _: &CutText, w, cx| {
                if !this.selected_range.is_empty() {
                    let s = this.content[this.selected_range.clone()].to_string();
                    cx.write_to_clipboard(ClipboardItem::new_string(s));
                    this.replace_text_in_range(None, "", w, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PasteText, w, cx| {
                if let Some(t) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    let t: String = t.chars().filter(|c| *c != '\n' && *c != '\r').collect();
                    this.replace_text_in_range(None, &t, w, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Submit, w, cx| this.do_submit(w, cx)))
            .on_action(cx.listener(|this, _: &Cancel, w, cx| this.do_cancel(w, cx)))
            .child(FieldElement {
                input: cx.entity().clone(),
            })
    }
}

impl Field {
    fn on_mouse_down(&mut self, e: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = true;
        let i = self.index_for_mouse_position(e.position);
        if e.modifiers.shift {
            self.select_to(i, cx);
        } else {
            self.move_to(i, cx)
        }
    }
    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }
    fn on_mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            let i = self.index_for_mouse_position(e.position);
            self.select_to(i, cx);
        }
    }
}

struct FieldElement {
    input: Entity<Field>,
}

struct FieldPrepaint {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
    offset: Pixels,
}

impl IntoElement for FieldElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for FieldElement {
    type RequestLayoutState = ();
    type PrepaintState = FieldPrepaint;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = px(20.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
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
        let empty = content.is_empty();

        let display_text: SharedString = if empty {
            input.placeholder.clone()
        } else {
            content
        };
        let text_color = if empty {
            input.theme.text_disabled
        } else {
            style.color
        };

        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match input.marked_range.as_ref() {
            Some(marked) if !empty => vec![
                TextRun {
                    len: marked.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked.end - marked.start,
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: display_text.len().saturating_sub(marked.end),
                    ..run
                },
            ]
            .into_iter()
            .filter(|r| r.len > 0)
            .collect(),
            _ => vec![run],
        };

        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display_text, font_size, &runs, None);

        // 保持光标可见
        let cursor_pos = line.x_for_index(cursor);
        let available = bounds.size.width;
        let mut offset = px(0.0);
        if cursor_pos > available - px(12.0) {
            offset = cursor_pos - available + px(24.0);
        }
        if cursor_pos < offset {
            offset = cursor_pos.max(px(0.0));
        }
        if line.width < available {
            offset = px(0.0);
        }

        let cursor = if selected_range.is_empty() {
            Some(fill(
                Bounds::new(
                    point(bounds.left() + cursor_pos - offset, bounds.top()),
                    size(px(1.5), bounds.size.height),
                ),
                if empty { hsla(0., 0., 0., 0.) } else { input.theme.cursor },
            ))
        } else {
            None
        };
        let selection = if !selected_range.is_empty() && !empty {
            Some(fill(
                Bounds::from_corners(
                    point(
                        bounds.left() + line.x_for_index(selected_range.start) - offset,
                        bounds.top(),
                    ),
                    point(
                        bounds.left() + line.x_for_index(selected_range.end) - offset,
                        bounds.bottom(),
                    ),
                ),
                input.theme.selection,
            ))
        } else {
            None
        };

        FieldPrepaint {
            line: Some(line),
            cursor,
            selection,
            offset,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if let Some(sel) = prepaint.selection.take() {
            window.paint_quad(sel);
        }
        if let Some(line) = prepaint.line.take() {
            let origin = point(bounds.origin.x - prepaint.offset, bounds.origin.y);
            let _ = line.paint(origin, px(20.0), window, cx);
            self.input.update(cx, |input, _| {
                input.last_layout = Some(line);
                input.last_bounds = Some(bounds);
                input.scroll_x = prepaint.offset;
            });
        }
        if focus_handle.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }
    }
}
