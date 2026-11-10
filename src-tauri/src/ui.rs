//! Fluent-style dark completion dialog, drawn with Direct2D + DirectWrite.
//! Falls back to a classic MessageBoxW if Direct2D cannot be initialized.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;

use windows::core::{BOOL, HSTRING, PCWSTR};
use windows::Win32::Foundation::{
    COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_RECT_F, D2D_SIZE_U};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1RenderTarget,
    ID2D1SolidColorBrush, ID2D1StrokeStyle, D2D1_CAP_STYLE_FLAT, D2D1_CAP_STYLE_ROUND,
    D2D1_DASH_STYLE_SOLID, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_ELLIPSE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_LINE_JOIN_ROUND,
    D2D1_PRESENT_OPTIONS_NONE, D2D1_RENDER_TARGET_PROPERTIES, D2D1_ROUNDED_RECT,
    D2D1_STROKE_STYLE_PROPERTIES, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_METRICS,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, InvalidateRect, MonitorFromPoint, ScreenToClient,
    UpdateWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    GetDpiForSystem, GetDpiForWindow, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE,
    VK_RETURN, VK_SHIFT, VK_SPACE, VK_TAB,
};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
    GetMessageW, GetWindowLongPtrW, LoadCursorW, MessageBoxW, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage,
    CREATESTRUCTW, GWLP_USERDATA, IDC_ARROW, MB_ICONINFORMATION, MB_OK, MSG, SW_SHOW,
    SW_SHOWNORMAL, SWP_NOACTIVATE, SWP_NOZORDER, WINDOW_EX_STYLE, WM_CLOSE,
    WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_NCCALCSIZE, WM_NCCREATE, WM_NCDESTROY, WM_NCHITTEST,
    WM_PAINT, WNDCLASSW, WS_POPUP, WS_THICKFRAME, CS_HREDRAW, CS_VREDRAW, HTCAPTION, HTCLIENT,
};
use windows_numerics::Vector2;

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

#[derive(Clone, Debug)]
pub struct Summary {
    pub total: usize,
    pub created: u32,
    pub existing: u32,
    pub already_named: u32,
    pub failed: u32,
    pub log_path: Option<PathBuf>,
    pub problems: Vec<String>,
}

impl Summary {
    fn is_empty(&self) -> bool {
        self.total == 0
    }

    fn fallback_text(&self) -> String {
        if self.is_empty() {
            let mut text = "没有可处理的 PDF 文件。".to_string();
            if !self.problems.is_empty() {
                text.push_str("\n\n");
                text.push_str(&self.problems.join("\n"));
            }
            return text;
        }
        let mut text = format!(
            "共 {} 个 PDF：新建副本 {}，已存在 {}，文件名已是标题 {}，失败 {}",
            self.total, self.created, self.existing, self.already_named, self.failed
        );
        if let Some(p) = &self.log_path {
            text.push_str(&format!("\n\n日志: {}", p.display()));
        }
        text
    }
}

pub fn show_summary(summary: Summary) {
    if std::env::var_os("PAPERRENAMER_NO_DIALOG").is_some() {
        return;
    }
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        if show_inner(&summary).is_err() {
            fallback(&summary.fallback_text());
        }
    }
}

unsafe fn fallback(text: &str) {
    let body = wide(text);
    let caption = wide("PaperRenamer");
    MessageBoxW(
        None,
        PCWSTR(body.as_ptr()),
        PCWSTR(caption.as_ptr()),
        MB_OK | MB_ICONINFORMATION,
    );
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn col(rgb: u32, alpha: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((rgb >> 16) & 0xff) as f32 / 255.0,
        g: ((rgb >> 8) & 0xff) as f32 / 255.0,
        b: (rgb & 0xff) as f32 / 255.0,
        a: alpha,
    }
}

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    }
}

fn rounded(r: D2D_RECT_F, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: r,
        radiusX: radius,
        radiusY: radius,
    }
}

fn inside(r: &D2D_RECT_F, x: f32, y: f32) -> bool {
    x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
}

fn v2(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hit {
    None,
    Close,
    OpenLog,
    Done,
}

struct Layout {
    close: D2D_RECT_F,
    open_log: D2D_RECT_F,
    done: D2D_RECT_F,
    divider_y: f32,
}

struct Ui {
    d2d: ID2D1Factory,
    dwt: IDWriteFactory,
    rt: Option<ID2D1HwndRenderTarget>,
    brush: Option<ID2D1SolidColorBrush>,
    stroke: Option<ID2D1StrokeStyle>,
    fmt_title: IDWriteTextFormat,
    fmt_subtitle: IDWriteTextFormat,
    fmt_value: IDWriteTextFormat,
    fmt_label: IDWriteTextFormat,
    fmt_button: IDWriteTextFormat,
    fmt_path: IDWriteTextFormat,
    fmt_body: IDWriteTextFormat,
    summary: Summary,
    empty: bool,
    dip_w: f32,
    dip_h: f32,
    dpi: f32,
    hover: Hit,
    pressed: Hit,
    focus: Hit,
    focus_visible: bool,
    tracking: bool,
}

struct Built {
    d2d: ID2D1Factory,
    dwt: IDWriteFactory,
    stroke: Option<ID2D1StrokeStyle>,
    fmt_title: IDWriteTextFormat,
    fmt_subtitle: IDWriteTextFormat,
    fmt_value: IDWriteTextFormat,
    fmt_label: IDWriteTextFormat,
    fmt_button: IDWriteTextFormat,
    fmt_path: IDWriteTextFormat,
    fmt_body: IDWriteTextFormat,
}

unsafe fn build_resources() -> Result<Built, ()> {
    let d2d: ID2D1Factory =
        D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).map_err(|_| ())?;
    let dwt: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).map_err(|_| ())?;
    let stroke = d2d
        .CreateStrokeStyle(
            &D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_FLAT,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 10.0,
                dashStyle: D2D1_DASH_STYLE_SOLID,
                dashOffset: 0.0,
            },
            None,
        )
        .ok();

    let title = make_format(
        &dwt,
        &["Microsoft YaHei UI", "Segoe UI"],
        19.0,
        DWRITE_FONT_WEIGHT_SEMI_BOLD,
        DWRITE_TEXT_ALIGNMENT_LEADING,
        DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
        DWRITE_WORD_WRAPPING_NO_WRAP,
    )?;
    let subtitle = make_format(
        &dwt,
        &["Microsoft YaHei UI", "Segoe UI"],
        12.5,
        DWRITE_FONT_WEIGHT_NORMAL,
        DWRITE_TEXT_ALIGNMENT_LEADING,
        DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
        DWRITE_WORD_WRAPPING_NO_WRAP,
    )?;
    let value = make_format(
        &dwt,
        &["Segoe UI Variable Display", "Segoe UI", "Microsoft YaHei UI"],
        30.0,
        DWRITE_FONT_WEIGHT_SEMI_BOLD,
        DWRITE_TEXT_ALIGNMENT_CENTER,
        DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
        DWRITE_WORD_WRAPPING_NO_WRAP,
    )?;
    let label = make_format(
        &dwt,
        &["Microsoft YaHei UI", "Segoe UI"],
        12.0,
        DWRITE_FONT_WEIGHT_NORMAL,
        DWRITE_TEXT_ALIGNMENT_CENTER,
        DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
        DWRITE_WORD_WRAPPING_NO_WRAP,
    )?;
    let button = make_format(
        &dwt,
        &["Microsoft YaHei UI", "Segoe UI"],
        13.0,
        DWRITE_FONT_WEIGHT_NORMAL,
        DWRITE_TEXT_ALIGNMENT_CENTER,
        DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
        DWRITE_WORD_WRAPPING_NO_WRAP,
    )?;
    let path = make_format(
        &dwt,
        &["Microsoft YaHei UI", "Segoe UI"],
        12.0,
        DWRITE_FONT_WEIGHT_NORMAL,
        DWRITE_TEXT_ALIGNMENT_LEADING,
        DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
        DWRITE_WORD_WRAPPING_NO_WRAP,
    )?;
    let body = make_format(
        &dwt,
        &["Microsoft YaHei UI", "Segoe UI"],
        12.0,
        DWRITE_FONT_WEIGHT_NORMAL,
        DWRITE_TEXT_ALIGNMENT_LEADING,
        DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
        DWRITE_WORD_WRAPPING_WRAP,
    )?;

    Ok(Built {
        d2d,
        dwt,
        stroke,
        fmt_title: title,
        fmt_subtitle: subtitle,
        fmt_value: value,
        fmt_label: label,
        fmt_button: button,
        fmt_path: path,
        fmt_body: body,
    })
}

unsafe fn make_format(
    dwt: &IDWriteFactory,
    families: &[&str],
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
    align: windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT,
    paragraph: windows::Win32::Graphics::DirectWrite::DWRITE_PARAGRAPH_ALIGNMENT,
    wrap: windows::Win32::Graphics::DirectWrite::DWRITE_WORD_WRAPPING,
) -> Result<IDWriteTextFormat, ()> {
    for family in families {
        if let Ok(format) = dwt.CreateTextFormat(
            &HSTRING::from(*family),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            &HSTRING::from("zh-CN"),
        ) {
            let _ = format.SetTextAlignment(align);
            let _ = format.SetParagraphAlignment(paragraph);
            let _ = format.SetWordWrapping(wrap);
            return Ok(format);
        }
    }
    Err(())
}

unsafe fn show_inner(summary: &Summary) -> Result<(), ()> {
    let built = build_resources()?;
    let empty = summary.is_empty();
    let (dip_w, dip_h) = if empty { (520.0, 300.0) } else { (560.0, 372.0) };
    let dpi = GetDpiForSystem().max(96) as f32;

    let ui = Ui {
        d2d: built.d2d,
        dwt: built.dwt,
        rt: None,
        brush: None,
        stroke: built.stroke,
        fmt_title: built.fmt_title,
        fmt_subtitle: built.fmt_subtitle,
        fmt_value: built.fmt_value,
        fmt_label: built.fmt_label,
        fmt_button: built.fmt_button,
        fmt_path: built.fmt_path,
        fmt_body: built.fmt_body,
        summary: summary.clone(),
        empty,
        dip_w,
        dip_h,
        dpi,
        hover: Hit::None,
        pressed: Hit::None,
        focus: Hit::Done,
        focus_visible: false,
        tracking: false,
    };

    let class_name = wide("PaperRenamerFluentDialog");
    let window_title = wide("PaperRenamer");
    let hinstance = HINSTANCE(
        GetModuleHandleW(None)
            .map_err(|_| ())?
            .0,
    );
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        hInstance: hinstance,
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    if RegisterClassW(&wc) == 0 {
        return Err(());
    }

    let px_w = (dip_w * dpi / 96.0).round() as i32;
    let px_h = (dip_h * dpi / 96.0).round() as i32;
    let mut work = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
    let area = if GetMonitorInfoW(monitor, &mut work).as_bool() {
        work.rcWork
    } else {
        RECT {
            left: 0,
            top: 0,
            right: px_w,
            bottom: px_h,
        }
    };
    let x = area.left + ((area.right - area.left - px_w) / 2).max(0);
    let y = area.top + ((area.bottom - area.top - px_h) / 2).max(0);

    let raw = Box::into_raw(Box::new(ui));
    let hwnd = match CreateWindowExW(
        WINDOW_EX_STYLE(0),
        PCWSTR(class_name.as_ptr()),
        PCWSTR(window_title.as_ptr()),
        WS_POPUP | WS_THICKFRAME,
        x,
        y,
        px_w,
        px_h,
        None,
        None,
        Some(hinstance),
        Some(raw as *const c_void),
    ) {
        Ok(hwnd) => hwnd,
        Err(_) => {
            drop(Box::from_raw(raw));
            return Err(());
        }
    };

    let dark = BOOL(1);
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_USE_IMMERSIVE_DARK_MODE,
        &dark as *const _ as *const c_void,
        std::mem::size_of::<BOOL>() as u32,
    );
    let corner = DWMWCP_ROUND;
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_WINDOW_CORNER_PREFERENCE,
        &corner as *const _ as *const c_void,
        std::mem::size_of_val(&corner) as u32,
    );
    let border = COLORREF(0x00383838);
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_BORDER_COLOR,
        &border as *const _ as *const c_void,
        std::mem::size_of::<COLORREF>() as u32,
    );

    let _ = ShowWindow(hwnd, SW_SHOW);
    let _ = SetForegroundWindow(hwnd);
    let _ = UpdateWindow(hwnd);

    let mut msg = MSG::default();
    loop {
        let result = GetMessageW(&mut msg, None, 0, 0);
        if result.0 <= 0 {
            break;
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    Ok(())
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;

    match msg {
        WM_NCCREATE => {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            LRESULT(1)
        }
        WM_NCCALCSIZE if wparam.0 != 0 => LRESULT(0),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            if state.is_null() {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            let ui = &mut *state;
            let mut paint = PAINTSTRUCT::default();
            let _ = BeginPaint(hwnd, &mut paint);
            ui.dpi = GetDpiForWindow(hwnd).max(96) as f32;
            ui.paint(hwnd);
            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if !state.is_null() {
                let ui = &mut *state;
                let (x, y) = mouse_dip(lparam, ui.dpi);
                let hit = ui.hit_test(x, y);
                if hit != ui.hover {
                    ui.hover = hit;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                if !ui.tracking {
                    ui.tracking = true;
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut track);
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            if !state.is_null() {
                let ui = &mut *state;
                ui.tracking = false;
                if ui.hover != Hit::None || ui.pressed != Hit::None {
                    ui.hover = Hit::None;
                    ui.pressed = Hit::None;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if !state.is_null() {
                let ui = &mut *state;
                let (x, y) = mouse_dip(lparam, ui.dpi);
                let hit = ui.hit_test(x, y);
                if hit != Hit::None {
                    ui.pressed = hit;
                    ui.focus = hit;
                    ui.focus_visible = false;
                    SetCapture(hwnd);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if !state.is_null() {
                let ui = &mut *state;
                let (x, y) = mouse_dip(lparam, ui.dpi);
                let hit = ui.hit_test(x, y);
                let pressed = ui.pressed;
                ui.pressed = Hit::None;
                let _ = ReleaseCapture();
                let _ = InvalidateRect(Some(hwnd), None, false);
                if hit != Hit::None && hit == pressed {
                    ui.activate(hwnd, hit);
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if !state.is_null() {
                let ui = &mut *state;
                let key = wparam.0 as u16;
                if key == VK_TAB.0 {
                    let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
                    ui.focus_visible = true;
                    ui.move_focus(shift);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                } else if key == VK_RETURN.0 || key == VK_SPACE.0 {
                    ui.activate(hwnd, ui.focus);
                } else if key == VK_ESCAPE.0 {
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            if !state.is_null() {
                let ui = &mut *state;
                ui.dpi = GetDpiForWindow(hwnd).max(96) as f32;
                ui.rt = None;
                ui.brush = None;
                let suggested = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_NCHITTEST => {
            if state.is_null() {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            let ui = &*state;
            let x = (lparam.0 as u32 & 0xffff) as i16 as f32;
            let y = ((lparam.0 as u32 >> 16) & 0xffff) as i16 as f32;
            let mut point = POINT {
                x: x as i32,
                y: y as i32,
            };
            let _ = ScreenToClient(hwnd, &mut point);
            let dx = point.x as f32 * 96.0 / ui.dpi;
            let dy = point.y as f32 * 96.0 / ui.dpi;
            let layout = ui.layout();
            let on_control = inside(&layout.close, dx, dy)
                || inside(&layout.done, dx, dy)
                || (!ui.empty && ui.summary.log_path.is_some() && inside(&layout.open_log, dx, dy));
            if !on_control && dy < 82.0 {
                LRESULT(HTCAPTION as isize)
            } else {
                LRESULT(HTCLIENT as isize)
            }
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            if !state.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(state));
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn mouse_dip(lparam: LPARAM, dpi: f32) -> (f32, f32) {
    let x = (lparam.0 as u32 & 0xffff) as i16 as f32;
    let y = ((lparam.0 as u32 >> 16) & 0xffff) as i16 as f32;
    (x * 96.0 / dpi, y * 96.0 / dpi)
}

impl Ui {
    fn layout(&self) -> Layout {
        let done_w = 92.0;
        let done_h = 36.0;
        let done = rect(
            self.dip_w - 32.0 - done_w,
            self.dip_h - 32.0 - done_h,
            self.dip_w - 32.0,
            self.dip_h - 32.0,
        );
        let open_log = rect(done.left - 12.0 - 104.0, done.top, done.left - 12.0, done.bottom);
        Layout {
            close: rect(self.dip_w - 62.0, 24.0, self.dip_w - 32.0, 54.0),
            open_log,
            done,
            divider_y: done.top - 12.0,
        }
    }

    fn hit_test(&self, x: f32, y: f32) -> Hit {
        let layout = self.layout();
        if inside(&layout.close, x, y) {
            return Hit::Close;
        }
        if inside(&layout.done, x, y) {
            return Hit::Done;
        }
        if !self.empty && self.summary.log_path.is_some() && inside(&layout.open_log, x, y) {
            return Hit::OpenLog;
        }
        Hit::None
    }

    fn focusables(&self) -> Vec<Hit> {
        let mut out = Vec::with_capacity(2);
        if !self.empty && self.summary.log_path.is_some() {
            out.push(Hit::OpenLog);
        }
        out.push(Hit::Done);
        out
    }

    fn move_focus(&mut self, backward: bool) {
        let order = self.focusables();
        if order.is_empty() {
            return;
        }
        let index = order.iter().position(|h| *h == self.focus).unwrap_or(0);
        let next = if backward {
            (index + order.len() - 1) % order.len()
        } else {
            (index + 1) % order.len()
        };
        self.focus = order[next];
    }

    unsafe fn activate(&mut self, hwnd: HWND, hit: Hit) {
        match hit {
            Hit::Done | Hit::Close => {
                let _ = DestroyWindow(hwnd);
            }
            Hit::OpenLog => {
                if let Some(path) = &self.summary.log_path {
                    let operation = wide("open");
                    let wide_path: Vec<u16> = path
                        .as_os_str()
                        .encode_wide()
                        .chain(std::iter::once(0))
                        .collect();
                    ShellExecuteW(
                        None,
                        PCWSTR(operation.as_ptr()),
                        PCWSTR(wide_path.as_ptr()),
                        None,
                        None,
                        SW_SHOWNORMAL,
                    );
                }
            }
            Hit::None => {}
        }
    }

    unsafe fn ensure_target(&mut self, hwnd: HWND) -> bool {
        if self.rt.is_some() && self.brush.is_some() {
            return true;
        }
        let mut client = RECT::default();
        if GetClientRect(hwnd, &mut client).is_err() {
            return false;
        }
        let width = (client.right - client.left).max(1) as u32;
        let height = (client.bottom - client.top).max(1) as u32;
        let mut props = D2D1_RENDER_TARGET_PROPERTIES::default();
        props.dpiX = self.dpi;
        props.dpiY = self.dpi;
        let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd,
            pixelSize: D2D_SIZE_U { width, height },
            presentOptions: D2D1_PRESENT_OPTIONS_NONE,
        };
        let Ok(target) = self.d2d.CreateHwndRenderTarget(&props, &hwnd_props) else {
            return false;
        };
        let Ok(brush) = target.CreateSolidColorBrush(&col(0xffffff, 1.0), None) else {
            return false;
        };
        self.rt = Some(target);
        self.brush = Some(brush);
        true
    }

    unsafe fn paint(&mut self, hwnd: HWND) {
        if !self.ensure_target(hwnd) {
            return;
        }
        let rt = self.rt.as_ref().unwrap();
        let brush = self.brush.as_ref().unwrap();
        let _ = rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let _ = rt.BeginDraw();
        rt.Clear(Some(&col(BG, 1.0)));

        let w = self.dip_w;
        let h = self.dip_h;
        let layout = self.layout();

        let frame = rounded(rect(0.5, 0.5, w - 0.5, h - 0.5), 10.0);
        fill_rounded(rt, brush, &frame, col(BG, 1.0));
        stroke_rounded(rt, brush, &frame, col(BORDER, 1.0), 1.0, None);

        let tile = rect(32.0, 28.0, 76.0, 72.0);
        fill_rounded(rt, brush, &rounded(tile, 12.0), col(CARD, 1.0));
        stroke_rounded(
            rt,
            brush,
            &rounded(tile, 12.0),
            col(BORDER, 1.0),
            1.0,
            None,
        );
        if self.empty || self.summary.failed > 0 {
            warning_icon(rt, brush, &tile, self.stroke.as_ref());
        } else {
            document_icon(rt, brush, &tile, self.stroke.as_ref());
        }

        let heading = if self.empty {
            "没有可处理的 PDF"
        } else if self.summary.failed > 0 {
            "重命名完成，部分失败"
        } else {
            "重命名完成"
        };
        draw_text(
            rt,
            brush,
            &self.fmt_title,
            heading,
            rect(92.0, 26.0, w - 76.0, 56.0),
            col(FG, 1.0),
        );
        let subtitle = if self.empty {
            "拖入的文件中没有找到 PDF 文件".to_string()
        } else if self.summary.failed > 0 {
            format!(
                "共 {} 个 PDF · {} 个处理失败，详情见日志",
                self.summary.total, self.summary.failed
            )
        } else {
            format!("共 {} 个 PDF · 原文件保持不变", self.summary.total)
        };
        draw_text(
            rt,
            brush,
            &self.fmt_subtitle,
            &subtitle,
            rect(92.0, 58.0, w - 76.0, 80.0),
            col(SECONDARY, 1.0),
        );

        self.paint_close(rt, brush, &layout);

        if self.empty {
            let card = rect(32.0, 104.0, w - 32.0, 196.0);
            fill_rounded(rt, brush, &rounded(card, 8.0), col(CARD_DEEP, 1.0));
            stroke_rounded(rt, brush, &rounded(card, 8.0), col(BORDER_SOFT, 1.0), 1.0, None);
            let text = if self.summary.problems.is_empty() {
                "所选位置没有找到 PDF 文件".to_string()
            } else {
                self.summary.problems.join("\n")
            };
            draw_text(
                rt,
                brush,
                &self.fmt_body,
                &text,
                rect(46.0, 116.0, w - 46.0, 184.0),
                col(SECONDARY, 1.0),
            );
        } else {
            self.paint_stats(rt, brush, w);
            draw_text(
                rt,
                brush,
                &self.fmt_path,
                "日志",
                rect(32.0, 224.0, 100.0, 242.0),
                col(DIM, 1.0),
            );
            if let Some(path) = &self.summary.log_path {
                let text = path.display().to_string();
                let fitted = fit_middle(&self.dwt, &self.fmt_path, &text, w - 64.0);
                draw_text(
                    rt,
                    brush,
                    &self.fmt_path,
                    &fitted,
                    rect(32.0, 243.0, w - 32.0, 263.0),
                    col(SECONDARY, 1.0),
                );
            }
        }

        line(
            rt,
            brush,
            32.0,
            layout.divider_y,
            w - 32.0,
            layout.divider_y,
            col(BORDER_SOFT, 1.0),
            1.0,
        );

        if !self.empty && self.summary.log_path.is_some() {
            self.paint_button(
                rt,
                brush,
                &layout.open_log,
                "查看日志",
                false,
                Hit::OpenLog,
            );
        }
        self.paint_button(rt, brush, &layout.done, "完成", true, Hit::Done);

        if rt.EndDraw(None, None).is_err() {
            self.rt = None;
            self.brush = None;
        }
    }

    unsafe fn paint_close(
        &self,
        rt: &ID2D1RenderTarget,
        brush: &ID2D1SolidColorBrush,
        layout: &Layout,
    ) {
        let r = layout.close;
        if self.hover == Hit::Close || self.pressed == Hit::Close {
            let color = if self.pressed == Hit::Close {
                col(PRESSED, 1.0)
            } else {
                col(HOVER, 1.0)
            };
            fill_rounded(rt, brush, &rounded(r, 6.0), color);
        }
        let cx = (r.left + r.right) / 2.0;
        let cy = (r.top + r.bottom) / 2.0;
        let color = if self.hover == Hit::Close {
            col(FG, 1.0)
        } else {
            col(DIM, 1.0)
        };
        line(rt, brush, cx - 5.0, cy - 5.0, cx + 5.0, cy + 5.0, color, 1.4);
        line(rt, brush, cx + 5.0, cy - 5.0, cx - 5.0, cy + 5.0, color, 1.4);
    }

    unsafe fn paint_stats(
        &self,
        rt: &ID2D1RenderTarget,
        brush: &ID2D1SolidColorBrush,
        w: f32,
    ) {
        let card = rect(32.0, 104.0, w - 32.0, 204.0);
        fill_rounded(rt, brush, &rounded(card, 8.0), col(CARD, 1.0));
        stroke_rounded(rt, brush, &rounded(card, 8.0), col(BORDER, 1.0), 1.0, None);

        let column = (w - 64.0) / 4.0;
        let labels = ["新建副本", "已存在", "已是标题", "失败"];
        let values = [
            self.summary.created,
            self.summary.existing,
            self.summary.already_named,
            self.summary.failed,
        ];
        for i in 0..4 {
            let left = 32.0 + column * i as f32;
            let right = left + column;
            if i > 0 {
                line(
                    rt,
                    brush,
                    left,
                    122.0,
                    left,
                    186.0,
                    col(BORDER_SOFT, 1.0),
                    1.0,
                );
            }
            draw_text(
                rt,
                brush,
                &self.fmt_value,
                &values[i].to_string(),
                rect(left, 110.0, right, 158.0),
                col(FG, 1.0),
            );
            draw_text(
                rt,
                brush,
                &self.fmt_label,
                labels[i],
                rect(left, 158.0, right, 178.0),
                col(DIM, 1.0),
            );
        }
    }

    unsafe fn paint_button(
        &self,
        rt: &ID2D1RenderTarget,
        brush: &ID2D1SolidColorBrush,
        r: &D2D_RECT_F,
        label: &str,
        primary: bool,
        hit: Hit,
    ) {
        let hovered = self.hover == hit;
        let pressed = self.pressed == hit;
        let (bg, fg) = if primary {
            let bg = if pressed {
                col(0xd6d6d6, 1.0)
            } else if hovered {
                col(0xffffff, 1.0)
            } else {
                col(FG, 1.0)
            };
            (bg, col(0x131313, 1.0))
        } else {
            let bg = if pressed {
                col(PRESSED, 1.0)
            } else if hovered {
                col(HOVER, 1.0)
            } else {
                col(CARD, 1.0)
            };
            (bg, col(FG, 1.0))
        };
        let shape = rounded(*r, 6.0);
        fill_rounded(rt, brush, &shape, bg);
        if !primary {
            stroke_rounded(rt, brush, &shape, col(BORDER_STRONG, 1.0), 1.0, None);
        }
        draw_text(rt, brush, &self.fmt_button, label, *r, fg);
        if self.focus_visible && self.focus == hit {
            let focus = rounded(
                rect(r.left - 2.5, r.top - 2.5, r.right + 2.5, r.bottom + 2.5),
                8.0,
            );
            stroke_rounded(rt, brush, &focus, col(FG, 0.55), 1.4, None);
        }
    }
}

unsafe fn fill_rounded(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    shape: &D2D1_ROUNDED_RECT,
    color: D2D1_COLOR_F,
) {
    brush.SetColor(&color);
    rt.FillRoundedRectangle(shape, brush);
}

unsafe fn stroke_rounded(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    shape: &D2D1_ROUNDED_RECT,
    color: D2D1_COLOR_F,
    width: f32,
    style: Option<&ID2D1StrokeStyle>,
) {
    brush.SetColor(&color);
    rt.DrawRoundedRectangle(shape, brush, width, style);
}

unsafe fn line(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    color: D2D1_COLOR_F,
    width: f32,
) {
    brush.SetColor(&color);
    rt.DrawLine(v2(x1, y1), v2(x2, y2), brush, width, None);
}

unsafe fn draw_text(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    format: &IDWriteTextFormat,
    text: &str,
    area: D2D_RECT_F,
    color: D2D1_COLOR_F,
) {
    if text.is_empty() {
        return;
    }
    brush.SetColor(&color);
    let utf16: Vec<u16> = text.encode_utf16().collect();
    rt.DrawText(
        &utf16,
        format,
        &area,
        brush,
        D2D1_DRAW_TEXT_OPTIONS_CLIP,
        DWRITE_MEASURING_MODE_NATURAL,
    );
}

unsafe fn measure_text(dwt: &IDWriteFactory, format: &IDWriteTextFormat, text: &str) -> f32 {
    let utf16: Vec<u16> = text.encode_utf16().collect();
    match dwt.CreateTextLayout(&utf16, format, 10000.0, 200.0) {
        Ok(layout) => {
            let mut metrics = DWRITE_TEXT_METRICS::default();
            if layout.GetMetrics(&mut metrics).is_ok() {
                metrics.widthIncludingTrailingWhitespace
            } else {
                text.chars().count() as f32 * 7.0
            }
        }
        Err(_) => text.chars().count() as f32 * 7.0,
    }
}

unsafe fn fit_middle(
    dwt: &IDWriteFactory,
    format: &IDWriteTextFormat,
    text: &str,
    max_width: f32,
) -> String {
    if measure_text(dwt, format, text) <= max_width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= 4 {
        return "…".to_string();
    }
    let build = |keep: usize| -> String {
        if keep >= chars.len() {
            return text.to_string();
        }
        let head = (keep * 3 + 2) / 5;
        let tail = keep.saturating_sub(head);
        let mut out: String = chars[..head].iter().collect();
        out.push('…');
        if tail > 0 {
            out.extend(chars[chars.len() - tail..].iter());
        }
        out
    };
    let mut low = 1usize;
    let mut high = chars.len() - 1;
    while low < high {
        let mid = (low + high + 1) / 2;
        if measure_text(dwt, format, &build(mid)) <= max_width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    build(low)
}

unsafe fn document_icon(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    tile: &D2D_RECT_F,
    style: Option<&ID2D1StrokeStyle>,
) {
    let (x, y) = (tile.left, tile.top);
    let ink = col(FG, 1.0);
    stroke_rounded(
        rt,
        brush,
        &rounded(rect(x + 12.0, y + 9.0, x + 32.0, y + 35.0), 3.0),
        ink,
        1.5,
        style,
    );
    line(rt, brush, x + 24.0, y + 9.0, x + 32.0, y + 17.0, ink, 1.2);
    brush.SetColor(&ink);
    rt.DrawLine(
        v2(x + 16.5, y + 23.0),
        v2(x + 21.0, y + 27.5),
        brush,
        2.0,
        style,
    );
    rt.DrawLine(
        v2(x + 21.0, y + 27.5),
        v2(x + 28.5, y + 19.0),
        brush,
        2.0,
        style,
    );
}

unsafe fn warning_icon(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    tile: &D2D_RECT_F,
    style: Option<&ID2D1StrokeStyle>,
) {
    let (x, y) = (tile.left, tile.top);
    let ink = col(FG, 1.0);
    brush.SetColor(&ink);
    rt.DrawEllipse(
        &D2D1_ELLIPSE {
            point: v2(x + 22.0, y + 22.0),
            radiusX: 10.5,
            radiusY: 10.5,
        },
        brush,
        1.5,
        style,
    );
    rt.DrawLine(
        v2(x + 22.0, y + 15.5),
        v2(x + 22.0, y + 23.5),
        brush,
        2.0,
        style,
    );
    rt.FillEllipse(
        &D2D1_ELLIPSE {
            point: v2(x + 22.0, y + 28.5),
            radiusX: 1.4,
            radiusY: 1.4,
        },
        brush,
    );
}
