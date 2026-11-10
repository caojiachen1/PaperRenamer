//! GPUI completion dialog for drag-onto-exe batches.

use gpui::prelude::*;
use gpui::{
    div, px, rgb, size, App, AppContext, Application, Bounds, ClickEvent, Context, Div, FocusHandle,
    FontWeight, KeyDownEvent, MouseButton, Render, SharedString, TitlebarOptions, Window,
    WindowBounds, WindowKind, WindowOptions,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const UI_FONT: &str = "Microsoft YaHei UI";
const DISPLAY_FONT: &str = "Segoe UI Variable Display";
const ICON_FONT: &str = "Segoe Fluent Icons";

const BG: u32 = 0x1f1f1f;
const CARD: u32 = 0x262626;
const CARD_DEEP: u32 = 0x232323;
const BORDER: u32 = 0x3a3a3a;
const BORDER_SOFT: u32 = 0x333333;
const BORDER_STRONG: u32 = 0x4a4a4a;
const FG: u32 = 0xf0f0f0;
const SECONDARY: u32 = 0xb8b8b8;
const DIM: u32 = 0x8a8a8a;
const HOVER: u32 = 0x2e2e2e;
const PRESSED: u32 = 0x252525;
const PRIMARY: u32 = 0xf0f0f0;
const PRIMARY_FG: u32 = 0x131313;

const DOC_GLYPH: &str = "\u{e8a5}";
const CHECK_GLYPH: &str = "\u{e73e}";
const WARN_GLYPH: &str = "\u{e7ba}";
const CLOSE_GLYPH: &str = "\u{e8bb}";

const CARD_W: f32 = 560.0;

impl Summary {
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }
}

#[derive(Clone, Debug)]
pub struct Summary {
    pub total: usize,
    pub created: u32,
    pub existing: u32,
    pub already_named: u32,
    pub failed: u32,
    pub log_path: Option<String>,
    pub problems: Vec<String>,
}

struct Dialog {
    summary: Summary,
    focus: FocusHandle,
}

impl Render for Dialog {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(BG))
            .flex()
            .track_focus(&self.focus)
            .key_context("PaperRenamerDialog")
            .on_key_down(|event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                if matches!(event.keystroke.key.as_str(), "escape" | "enter") {
                    window.remove_window();
                    cx.quit();
                }
            })
            .child(render_card(&self.summary))
    }
}

fn render_card(summary: &Summary) -> Div {
    let empty = summary.is_empty();
    let mut card = div()
        .size_full()
        .bg(rgb(BG))
        .font_family(UI_FONT)
        .text_color(rgb(FG))
        .flex()
        .flex_col();

    card = card.child(render_header(summary));
    if empty {
        card = card.child(render_problems(summary));
    } else {
        card = card
            .child(render_stats(summary))
            .child(render_log(summary));
    }
    card.child(div().flex_1()).child(render_footer(summary))
}

fn render_header(summary: &Summary) -> Div {
    let empty = summary.is_empty();
    let (title, subtitle) = if empty {
        (
            "没有可处理的 PDF".to_string(),
            "拖入的文件中没有找到 PDF 文件".to_string(),
        )
    } else if summary.failed > 0 {
        (
            "重命名完成，部分失败".to_string(),
            format!(
                "共 {} 个 PDF · {} 个处理失败，详情见日志",
                summary.total, summary.failed
            ),
        )
    } else {
        (
            "重命名完成".to_string(),
            format!("共 {} 个 PDF · 原文件保持不变", summary.total),
        )
    };

    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(16.0))
        .px(px(32.0))
        .pt(px(26.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(16.0))
                .on_mouse_down(MouseButton::Left, |_, window, _| {
                    window.start_window_move()
                })
                .child(render_icon(empty, summary.failed > 0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .min_w_0()
                        .child(text_line(
                            title,
                            19.0,
                            FontWeight::SEMIBOLD,
                            FG,
                            26.0,
                        ))
                        .child(text_line(
                            subtitle,
                            12.5,
                            FontWeight::NORMAL,
                            SECONDARY,
                            18.0,
                        )),
                ),
        )
        .child(render_close())
}

fn render_icon(empty: bool, failed: bool) -> Div {
    let glyph = if empty || failed {
        WARN_GLYPH
    } else {
        DOC_GLYPH
    };
    let mut tile = div()
        .relative()
        .w(px(44.0))
        .h(px(44.0))
        .flex_shrink_0()
        .rounded(px(12.0))
        .bg(rgb(CARD))
        .border_1()
        .border_color(rgb(BORDER))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .font_family(ICON_FONT)
                .text_size(px(22.0))
                .text_color(rgb(FG))
                .child(glyph),
        );

    if !empty && !failed {
        tile = tile.child(
            div()
                .absolute()
                .right(px(5.0))
                .bottom(px(5.0))
                .w(px(15.0))
                .h(px(15.0))
                .rounded(px(8.0))
                .bg(rgb(CARD))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .font_family(ICON_FONT)
                        .text_size(px(9.0))
                        .text_color(rgb(FG))
                        .child(CHECK_GLYPH),
                ),
        );
    }
    tile
}

fn render_close() -> impl IntoElement {
    div()
        .id("close")
        .w(px(30.0))
        .h(px(30.0))
        .flex_shrink_0()
        .rounded(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .text_color(rgb(DIM))
        .cursor_pointer()
        .hover(|this| this.bg(rgb(HOVER)).text_color(rgb(FG)))
        .active(|this| this.bg(rgb(PRESSED)))
        .on_click(|_: &ClickEvent, window: &mut Window, cx: &mut App| {
            window.remove_window();
            cx.quit();
        })
        .child(
            div()
                .font_family(ICON_FONT)
                .text_size(px(11.0))
                .child(CLOSE_GLYPH),
        )
}

fn render_stats(summary: &Summary) -> Div {
    div()
        .mx(px(32.0))
        .mt(px(24.0))
        .h(px(100.0))
        .rounded(px(8.0))
        .bg(rgb(CARD))
        .border_1()
        .border_color(rgb(BORDER))
        .flex()
        .flex_row()
        .child(stat_cell("新建副本", summary.created))
        .child(separator())
        .child(stat_cell("已存在", summary.existing))
        .child(separator())
        .child(stat_cell("已是标题", summary.already_named))
        .child(separator())
        .child(stat_cell("失败", summary.failed))
}

fn stat_cell(label: &'static str, value: u32) -> Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .pt(px(12.0))
        .child(
            div()
                .font_family(DISPLAY_FONT)
                .text_size(px(30.0))
                .line_height(px(38.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(FG))
                .child(SharedString::from(value.to_string())),
        )
        .child(
            div()
                .text_size(px(12.0))
                .line_height(px(16.0))
                .text_color(rgb(DIM))
                .child(label),
        )
}

fn separator() -> Div {
    div()
        .w(px(1.0))
        .h(px(64.0))
        .mt(px(18.0))
        .bg(rgb(BORDER_SOFT))
}

fn render_log(summary: &Summary) -> Div {
    let path = summary
        .log_path
        .as_deref()
        .map(compact_path)
        .unwrap_or_else(|| "未写入日志".to_string());
    div()
        .w(px(CARD_W - 64.0))
        .mx(px(32.0))
        .mt(px(20.0))
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(
            div()
                .text_size(px(12.0))
                .line_height(px(16.0))
                .text_color(rgb(DIM))
                .child("日志"),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(12.0))
                .line_height(px(16.0))
                .text_color(rgb(SECONDARY))
                .child(SharedString::from(path)),
        )
}

fn render_problems(summary: &Summary) -> Div {
    let text = if summary.problems.is_empty() {
        "所选位置没有找到 PDF 文件".to_string()
    } else {
        summary.problems.join("\n")
    };
    div()
        .w(px(CARD_W - 64.0))
        .mx(px(32.0))
        .mt(px(24.0))
        .h(px(92.0))
        .rounded(px(8.0))
        .bg(rgb(CARD_DEEP))
        .border_1()
        .border_color(rgb(BORDER_SOFT))
        .px(px(14.0))
        .py(px(12.0))
        .overflow_hidden()
        .text_size(px(12.0))
        .line_height(px(18.0))
        .text_color(rgb(SECONDARY))
        .child(SharedString::from(text))
}

fn render_footer(summary: &Summary) -> Div {
    let log = summary.log_path.clone();
    let mut row = div()
        .h(px(76.0))
        .px(px(32.0))
        .border_t_1()
        .border_color(rgb(BORDER_SOFT))
        .flex()
        .flex_row()
        .items_center()
        .justify_end()
        .gap(px(12.0));

    if !summary.is_empty() && log.is_some() {
        row = row.child(secondary_button("查看日志", log));
    }
    row.child(primary_button("完成"))
}

fn primary_button(label: &'static str) -> impl IntoElement {
    div()
        .id("done")
        .w(px(92.0))
        .h(px(36.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(PRIMARY))
        .text_color(rgb(PRIMARY_FG))
        .text_size(px(13.0))
        .cursor_pointer()
        .hover(|this| this.bg(rgb(0xffffff)))
        .active(|this| this.bg(rgb(0xd6d6d6)))
        .on_click(|_: &ClickEvent, window: &mut Window, cx: &mut App| {
            window.remove_window();
            cx.quit();
        })
        .child(label)
}

fn secondary_button(label: &'static str, log: Option<String>) -> impl IntoElement {
    div()
        .id("open-log")
        .w(px(104.0))
        .h(px(36.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(CARD))
        .border_1()
        .border_color(rgb(BORDER_STRONG))
        .text_color(rgb(FG))
        .text_size(px(13.0))
        .cursor_pointer()
        .hover(|this| this.bg(rgb(HOVER)))
        .active(|this| this.bg(rgb(PRESSED)))
        .on_click(move |_: &ClickEvent, _: &mut Window, _: &mut App| {
            if let Some(path) = &log {
                shell_open(path);
            }
        })
        .child(label)
}

fn text_line(
    text: String,
    size: f32,
    weight: FontWeight,
    color: u32,
    line_height: f32,
) -> Div {
    div()
        .text_size(px(size))
        .line_height(px(line_height))
        .font_weight(weight)
        .text_color(rgb(color))
        .child(SharedString::from(text))
}

fn compact_path(path: &str) -> String {
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= 88 {
        return path.to_string();
    }
    let head: String = chars[..22].iter().collect();
    let tail: String = chars[chars.len() - 60..].iter().collect();
    format!("{head}…{tail}")
}

fn apply_dwm(window: &Window) {
    let Ok(handle) = <Window as HasWindowHandle>::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = HWND(win32.hwnd.get() as *mut core::ffi::c_void);
    unsafe {
        let dark: i32 = 1;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const _ as *const core::ffi::c_void,
            4,
        );
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const core::ffi::c_void,
            4,
        );
        let border = COLORREF(0x003a3a3a);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border as *const _ as *const core::ffi::c_void,
            4,
        );
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn shell_open(path: &str) {
    let operation = wide("open");
    let file = wide(path);
    unsafe {
        ShellExecuteW(
            None,
            PCWSTR(operation.as_ptr()),
            PCWSTR(file.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}

pub fn show_summary(summary: Summary) {
    if std::env::var_os("PAPERRENAMER_NO_DIALOG").is_some() {
        return;
    }
    let window_w = CARD_W;
    let window_h = if summary.is_empty() { 300.0 } else { 372.0 };

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(window_w), px(window_h)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("PaperRenamer".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                kind: WindowKind::PopUp,
                is_resizable: false,
                is_minimizable: false,
                is_movable: true,
                focus: true,
                show: true,
                app_id: Some("PaperRenamer".into()),
                ..Default::default()
            },
            move |window, cx| {
                apply_dwm(window);
                let focus = cx.focus_handle();
                window.focus(&focus);
                let summary = summary.clone();
                cx.new(move |_| Dialog {
                    summary,
                    focus: focus.clone(),
                })
            },
        )
        .expect("open dialog window");
        cx.activate(true);
    });
}
