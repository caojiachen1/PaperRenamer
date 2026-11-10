//! Main GPUI window: title list, settings, background pipeline.

use std::collections::HashSet;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::prelude::*;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Shell::ExtractIconExW;
use windows::Win32::UI::WindowsAndMessaging::{
    SendMessageW, HICON, ICON_BIG, ICON_SMALL, WM_SETICON,
};
use gpui::{
    div, px, rgb, size, App, AppContext, Application, Bounds, ClickEvent, Context, Div, ElementId,
    Entity, FileDropEvent, FontWeight, IntoElement, MouseButton, ParentElement, PathPromptOptions,
    Render, SharedString, Stateful, Styled, TitlebarOptions, Window, WindowAppearance, WindowBounds,
    WindowKind, WindowOptions,
};

use crate::field::{self, Field};
use crate::settings::{self, Settings};
use crate::theme::{Mode, Theme};
use crate::{engine, pipeline, rename};

const ICON_FONT: &str = "Segoe Fluent Icons";
const GLYPH_CHECK: &str = "\u{e73e}";
const GLYPH_DOC: &str = "\u{e8a5}";
const GLYPH_CLOSE: &str = "\u{e8bb}";
const GLYPH_MIN: &str = "\u{e921}";
const GLYPH_MAX: &str = "\u{e922}";
const GLYPH_RESTORE: &str = "\u{e923}";
const CLOSE_HOVER: u32 = 0xc42b1c;
const CLOSE_PRESSED: u32 = 0xa3231a;

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowStatus {
    Queued,
    Parsing,
    Ready,
    Failed,
    Copied,
}

struct Row {
    id: u64,
    path: PathBuf,
    name: String,
    status: RowStatus,
    field: Entity<Field>,
    title: String,
    source: Option<String>,
    doi: Option<String>,
    suggested: SharedString,
    warnings: Vec<String>,
    cjk: bool,
    ext: String,
    verified: bool,
    verifying: bool,
    error: Option<String>,
    new_path: Option<PathBuf>,
    copy_label: Option<&'static str>,
    copy_error: Option<String>,
    copying: bool,
}

enum EngineState {
    Loading,
    Ready,
    Error(String),
}

struct LocalJob {
    id: u64,
    path: PathBuf,
}

struct VerifyJob {
    id: u64,
    local: pipeline::LocalParse,
}

struct CopyJob {
    id: u64,
    path: PathBuf,
    title: String,
}

enum UiEvent {
    EngineReady,
    EngineError(String),
    LocalParsed {
        id: u64,
        result: Result<pipeline::LocalParse, String>,
    },
    Verified {
        id: u64,
        outcome: pipeline::VerifyOutcome,
    },
    Copied {
        id: u64,
        result: Result<rename::CopyOutcome, String>,
    },
    Scanned(Vec<PathBuf>),
}

pub struct AppView {
    theme: Theme,
    rows: Vec<Row>,
    known: HashSet<PathBuf>,
    next_id: u64,
    engine: EngineState,
    crossref: bool,
    show_dialog: bool,
    drag_over: bool,
    dropped_paths: Vec<PathBuf>,
    copying_all: bool,
    copy_pending: usize,
    scanning: usize,
    rx: Receiver<UiEvent>,
    ui_tx: Sender<UiEvent>,
    local_tx: Sender<LocalJob>,
    verify_tx: Sender<VerifyJob>,
    copy_tx: Sender<CopyJob>,
}

pub fn run_gui(initial: Vec<PathBuf>) {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys(field::key_bindings());
        let bounds = Bounds::centered(None, size(px(1080.0), px(700.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Maximized(bounds)),
                window_min_size: Some(size(px(860.0), px(520.0))),
                titlebar: Some(TitlebarOptions {
                    title: Some("PaperRenamer — 论文 PDF 重命名".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                kind: WindowKind::Normal,
                is_resizable: true,
                is_minimizable: true,
                focus: true,
                show: true,
                app_id: Some("PaperRenamer".into()),
                ..Default::default()
            },
            move |window, cx| cx.new(|cx| AppView::new(window, cx, initial)),
        )
        .expect("open main window");
        cx.activate(true);
    });
}

impl AppView {
    fn new(window: &mut Window, cx: &mut Context<Self>, initial: Vec<PathBuf>) -> Self {
        let mode = match window.appearance() {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Mode::Dark,
            _ => Mode::Light,
        };
        let theme = Theme::new(mode);
        apply_window_icon(window);
        let saved = settings::load();

        let (tx, rx) = mpsc::channel::<UiEvent>();
        let ui_tx = tx.clone();
        let (local_tx, local_rx) = mpsc::channel::<LocalJob>();
        let (verify_tx, verify_rx) = mpsc::channel::<VerifyJob>();
        let (copy_tx, copy_rx) = mpsc::channel::<CopyJob>();

        {
            let tx = tx.clone();
            std::thread::spawn(move || {
                while let Ok(job) = local_rx.recv() {
                    let result = pipeline::parse_local(&job.path);
                    let _ = tx.send(UiEvent::LocalParsed {
                        id: job.id,
                        result,
                    });
                }
            });
        }

        {
            let tx = tx.clone();
            std::thread::spawn(move || match engine::engine() {
                Ok(_) => {
                    let _ = tx.send(UiEvent::EngineReady);
                }
                Err(e) => {
                    let _ = tx.send(UiEvent::EngineError(e));
                }
            });
        }

        let verify_rx = Arc::new(Mutex::new(verify_rx));
        for _ in 0..4 {
            let rx = verify_rx.clone();
            let tx = tx.clone();
            std::thread::spawn(move || loop {
                let job = {
                    let guard = rx.lock().unwrap();
                    guard.recv()
                };
                let Ok(job) = job else { break };
                let outcome = pipeline::verify(&job.local);
                let _ = tx.send(UiEvent::Verified {
                    id: job.id,
                    outcome,
                });
            });
        }

        let copy_rx = Arc::new(Mutex::new(copy_rx));
        for _ in 0..4 {
            let rx = copy_rx.clone();
            let tx = tx.clone();
            std::thread::spawn(move || loop {
                let job = {
                    let guard = rx.lock().unwrap();
                    guard.recv()
                };
                let Ok(job) = job else { break };
                let result = rename::make_renamed_copy(&job.path, &job.title);
                let _ = tx.send(UiEvent::Copied {
                    id: job.id,
                    result,
                });
            });
        }

        let mut this = Self {
            theme,
            rows: Vec::new(),
            known: HashSet::new(),
            next_id: 1,
            engine: EngineState::Loading,
            crossref: saved.crossref,
            show_dialog: saved.show_dialog,
            drag_over: false,
            dropped_paths: Vec::new(),
            copying_all: false,
            copy_pending: 0,
            scanning: 0,
            rx,
            ui_tx,
            local_tx,
            verify_tx,
            copy_tx,
        };
        this.start_watcher(cx);
        if !initial.is_empty() {
            this.add_paths(initial, cx);
        }
        this
    }

    fn start_watcher(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |view, cx| {
            loop {
                match cx.update(|app| app.background_executor().timer(Duration::from_millis(40))) {
                    Ok(timer) => timer.await,
                    Err(_) => break,
                }
                let alive = view
                    .update(cx, |this: &mut AppView, cx| {
                        this.drain_events(cx);
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }

    fn drain_events(&mut self, cx: &mut Context<Self>) {
        loop {
            let event = match self.rx.try_recv() {
                Ok(event) => event,
                Err(_) => break,
            };
            self.handle_event(event, cx);
        }
    }

    fn handle_event(&mut self, event: UiEvent, cx: &mut Context<Self>) {
        match event {
            UiEvent::EngineReady => self.engine = EngineState::Ready,
            UiEvent::EngineError(e) => self.engine = EngineState::Error(e),
            UiEvent::Scanned(paths) => {
                self.scanning = self.scanning.saturating_sub(1);
                self.push_rows(paths, cx);
            }
            UiEvent::LocalParsed { id, result } => {
                if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
                    match result {
                        Ok(local) => {
                            row.status = RowStatus::Ready;
                            row.title = local.title.clone();
                            row.source = Some(local.source.clone());
                            row.doi = local.doi.clone();
                            row.suggested = SharedString::from(local.suggested_name.clone());
                            row.warnings = local.warnings.clone();
                            row.cjk = local.cjk;
                            row.ext = local.ext.clone();
                            row.error = None;
                            let field = row.field.clone();
                            field.update(cx, |field, cx| {
                                field.set_value_start(local.title.clone());
                                cx.notify();
                            });
                            if self.crossref && !local.cjk {
                                self.enqueue_verify(id, local);
                            }
                        }
                        Err(e) => {
                            row.status = RowStatus::Failed;
                            row.error = Some(e);
                        }
                    }
                }
                self.pump_local();
            }
            UiEvent::Verified { id, outcome } => {
                if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
                    row.verifying = false;
                    row.verified = true;
                    let current = row.field.read(cx).text();
                    let user_edited = current != row.title;
                    if !user_edited {
                        row.source = Some(outcome.source.clone());
                        row.doi = outcome.doi.clone();
                        row.suggested = SharedString::from(outcome.suggested_name.clone());
                        row.title = outcome.title.clone();
                        if let Some(warning) = outcome.warning {
                            row.warnings.push(warning);
                        }
                        let field = row.field.clone();
                        field.update(cx, |field, cx| {
                            field.set_value_start(outcome.title.clone());
                            cx.notify();
                        });
                    }
                }
            }
            UiEvent::Copied { id, result } => {
                if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
                    row.copying = false;
                    match result {
                        Ok(copy) => {
                            row.status = RowStatus::Copied;
                            row.new_path = Some(PathBuf::from(copy.new_path()));
                            row.copy_label = Some(match copy {
                                rename::CopyOutcome::Created { .. } => "已生成",
                                rename::CopyOutcome::IdenticalExists { .. } => "已存在相同副本",
                                rename::CopyOutcome::AlreadyNamed { .. } => "文件名已是标题",
                            });
                            row.copy_error = None;
                        }
                        Err(e) => row.copy_error = Some(e),
                    }
                }
                self.copy_pending = self.copy_pending.saturating_sub(1);
                if self.copy_pending == 0 {
                    self.copying_all = false;
                }
            }
        }
    }

    fn pump_local(&mut self) {
        if self
            .rows
            .iter()
            .any(|row| row.status == RowStatus::Parsing)
        {
            return;
        }
        if let Some(row) = self
            .rows
            .iter_mut()
            .find(|row| row.status == RowStatus::Queued)
        {
            row.status = RowStatus::Parsing;
            let _ = self.local_tx.send(LocalJob {
                id: row.id,
                path: row.path.clone(),
            });
        }
    }

    fn enqueue_verify(&mut self, id: u64, local: pipeline::LocalParse) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            if row.verifying || row.verified {
                return;
            }
            row.verifying = true;
        }
        let _ = self.verify_tx.send(VerifyJob { id, local });
    }

    fn add_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        for path in paths {
            if path.is_dir() {
                dirs.push(path);
            } else if is_pdf(&path) {
                files.push(path);
            }
        }
        self.push_rows(files, cx);
        for dir in dirs {
            self.scanning += 1;
            let tx = self.ui_tx.clone();
            std::thread::spawn(move || {
                let found = scan_dir(&dir);
                let _ = tx.send(UiEvent::Scanned(found));
            });
        }
    }

    fn push_rows(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        for path in paths {
            if !self.known.insert(path.clone()) {
                continue;
            }
            let id = self.next_id;
            self.next_id += 1;
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string());
            let theme = self.theme.clone();
            let field = cx.new(|cx| {
                let mut field = Field::new(cx, theme);
                field.set_value("", false);
                field
            });
            self.rows.push(Row {
                id,
                path,
                name,
                status: RowStatus::Queued,
                field,
                title: String::new(),
                source: None,
                doi: None,
                suggested: SharedString::default(),
                warnings: Vec::new(),
                cjk: false,
                ext: "pdf".to_string(),
                verified: false,
                verifying: false,
                error: None,
                new_path: None,
                copy_label: None,
                copy_error: None,
                copying: false,
            });
        }
        self.pump_local();
    }

    fn remove_row(&mut self, id: u64) {
        if let Some(index) = self.rows.iter().position(|row| row.id == id) {
            let row = self.rows.remove(index);
            self.known.remove(&row.path);
        }
    }

    fn toggle_crossref(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.crossref = enabled;
        self.save_settings();
        if enabled {
            let jobs: Vec<(u64, pipeline::LocalParse)> = self
                .rows
                .iter()
                .filter(|row| {
                    row.status == RowStatus::Ready && !row.verified && !row.cjk && !row.verifying
                })
                .filter_map(|row| {
                    Some((
                        row.id,
                        pipeline::LocalParse {
                            title: row.title.clone(),
                            doi: row.doi.clone(),
                            source: row.source.clone().unwrap_or_default(),
                            suggested_name: row.suggested.to_string(),
                            warnings: Vec::new(),
                            cjk: row.cjk,
                            ext: row.ext.clone(),
                        },
                    ))
                })
                .collect();
            for (id, local) in jobs {
                self.enqueue_verify(id, local);
            }
        }
        cx.notify();
    }

    fn toggle_show_dialog(&mut self, enabled: bool) {
        self.show_dialog = enabled;
        self.save_settings();
    }

    fn save_settings(&self) {
        let _ = settings::save(&Settings {
            crossref: self.crossref,
            show_dialog: self.show_dialog,
        });
    }

    fn copy_row(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter_mut().find(|row| row.id == id) else {
            return;
        };
        let title = row.field.read(cx).text().trim().to_string();
        if title.is_empty() {
            return;
        }
        row.copy_error = None;
        row.copying = true;
        self.copy_pending += 1;
        let _ = self.copy_tx.send(CopyJob {
            id,
            path: row.path.clone(),
            title,
        });
    }

    fn copy_all(&mut self, cx: &mut Context<Self>) {
        if self.copying_all {
            return;
        }
        let ids: Vec<u64> = self
            .rows
            .iter()
            .filter(|row| row.status == RowStatus::Ready)
            .map(|row| row.id)
            .collect();
        if ids.is_empty() {
            return;
        }
        self.copying_all = true;
        for id in ids {
            self.copy_row(id, cx);
        }
    }

    fn on_file_drop(&mut self, event: &FileDropEvent, _window: &mut Window, cx: &mut Context<Self>) {
        match event {
            FileDropEvent::Entered { paths, .. } => {
                self.drag_over = true;
                self.dropped_paths = paths.paths().to_vec();
            }
            FileDropEvent::Pending { .. } => self.drag_over = true,
            FileDropEvent::Submit { .. } => {
                self.drag_over = false;
                let paths = std::mem::take(&mut self.dropped_paths);
                if !paths.is_empty() {
                    self.add_paths(paths, cx);
                }
            }
            FileDropEvent::Exited => {
                self.drag_over = false;
                self.dropped_paths.clear();
            }
        }
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> Div {
        let theme = self.theme.clone();
        let has_rows = !self.rows.is_empty();
        let ready = self.count(Some(RowStatus::Ready));
        let mut files_btn = button(&theme, "pick-files", "选择文件", false, false);
        files_btn = files_btn.on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
            let rx = cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: true,
                prompt: Some("选择 PDF".into()),
            });
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(Some(paths))) = rx.await {
                    let _ = this.update(cx, |this, cx| this.add_paths(paths, cx));
                }
            })
            .detach();
        }));
        let mut dir_btn = button(&theme, "pick-dir", "选择文件夹", false, false);
        dir_btn = dir_btn.on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
            let rx = cx.prompt_for_paths(PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("选择文件夹".into()),
            });
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(Some(paths))) = rx.await {
                    let _ = this.update(cx, |this, cx| this.add_paths(paths, cx));
                }
            })
            .detach();
        }));
        let mut clear_btn = button(&theme, "clear", "清空", false, false);
        if has_rows {
            clear_btn = clear_btn.on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                this.rows.clear();
                this.known.clear();
                this.copying_all = false;
                this.copy_pending = 0;
                cx.notify();
            }));
        } else {
            clear_btn = clear_btn.opacity(0.45);
        }

        let copy_label = if self.copying_all {
            "生成中…".to_string()
        } else if ready > 0 {
            format!("全部生成副本（{ready}）")
        } else {
            "全部生成副本".to_string()
        };
        let mut copy_btn = button(&theme, "copy-all", copy_label, true, false);
        let enabled = ready > 0 && !self.copying_all;
        if enabled {
            copy_btn = copy_btn.on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                this.copy_all(cx);
            }));
        } else if !self.copying_all {
            copy_btn = copy_btn.opacity(0.45);
        }

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(20.0))
            .pt(px(4.0))
            .pb(px(12.0))
            .child(files_btn)
            .child(dir_btn)
            .child(clear_btn)
            .child(checkbox(
                &theme,
                "crossref",
                "CrossRef 在线校验",
                self.crossref,
            ).on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                let next = !this.crossref;
                this.toggle_crossref(next, cx);
            })))
            .child(checkbox(
                &theme,
                "show-dialog",
                "拖拽完成后弹窗",
                self.show_dialog,
            ).on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                let next = !this.show_dialog;
                this.toggle_show_dialog(next);
                cx.notify();
            })))
            .child(div().flex_1())
            .child(copy_btn)
    }

    fn render_topbar(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let theme = self.theme.clone();
        let (badge_label, badge_solid, badge_dashed) = match &self.engine {
            EngineState::Loading => ("引擎初始化中…", false, false),
            EngineState::Ready => ("标题引擎就绪", true, false),
            EngineState::Error(_) => ("引擎不可用 · 仅内嵌元数据", false, true),
        };
        let mut badge = div()
            .px(px(12.0))
            .py(px(4.0))
            .rounded(px(999.0))
            .border_1()
            .border_color(theme.border_strong)
            .text_size(px(12.0))
            .text_color(theme.fg_secondary)
            .child(badge_label);
        if badge_solid {
            badge = badge
                .bg(theme.solid_bg)
                .text_color(theme.solid_fg)
                .border_color(theme.solid_bg);
        }
        if badge_dashed {
            badge = badge.border_dashed();
        }

        div()
            .h(px(40.0))
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .child(
                div()
                    .id("title-block")
                    .h_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .pl(px(12.0))
                    .window_control_area(gpui::WindowControlArea::Drag)
                    .child(
                        div()
                            .font_family(ICON_FONT)
                            .text_size(px(18.0))
                            .text_color(theme.fg)
                            .child(GLYPH_DOC),
                    )
                    .child(
                        div()
                            .text_size(px(17.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("PaperRenamer"),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme.fg_secondary)
                            .child("论文 PDF 标题重命名"),
                    ),
            )
            .child(
                div()
                    .id("title-drag")
                    .flex_1()
                    .h_full()
                    .window_control_area(gpui::WindowControlArea::Drag),
            )
            .child(badge)
            .child(div().w(px(10.0)))
            .child(
                caption_button(&theme, "win-min", GLYPH_MIN)
                    .window_control_area(gpui::WindowControlArea::Min),
            )
            .child(
                caption_button(
                    &theme,
                    "win-max",
                    if window.is_maximized() {
                        GLYPH_RESTORE
                    } else {
                        GLYPH_MAX
                    },
                )
                .window_control_area(gpui::WindowControlArea::Max),
            )
            .child(
                close_caption_button(&theme, "win-close", GLYPH_CLOSE).on_click(
                    |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        window.remove_window();
                        cx.quit();
                    },
                ),
            )
    }

    fn render_empty(&self) -> Div {
        let theme = &self.theme;
        div()
            .flex_1()
            .mx(px(20.0))
            .mb(px(12.0))
            .border_dashed()
            .border_1()
            .border_color(if self.drag_over {
                theme.fg
            } else {
                theme.border_strong
            })
            .rounded(theme.radius)
            .bg(theme.card)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.0))
            .child(
                div()
                    .font_family(ICON_FONT)
                    .text_size(px(44.0))
                    .text_color(theme.fg_dim)
                    .child(GLYPH_DOC),
            )
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("拖拽 PDF 文件或文件夹到窗口"),
            )
            .child(
                div()
                    .max_w(px(520.0))
                    .text_size(px(12.0))
                    .text_color(theme.fg_secondary)
                    .text_center()
                    .line_height(px(19.0))
                    .child("自动解析标题（版面识别 + XMP + CrossRef 交叉验证），确认后在原目录生成重命名副本"),
            )
            .child(
                div()
                    .max_w(px(520.0))
                    .text_size(px(12.0))
                    .text_color(theme.fg_dim)
                    .text_center()
                    .line_height(px(19.0))
                    .child("也可以直接把 PDF 拖到 PaperRenamer.exe 图标上，静默批量生成副本"),
            )
    }

    fn render_list(&self, cx: &mut Context<Self>) -> Div {
        let theme = &self.theme;
        let header = div()
            .flex()
            .flex_row()
            .gap(px(12.0))
            .px(px(14.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(theme.border)
            .text_size(px(11.0))
            .text_color(theme.fg_dim)
            .child(div().w(px(200.0)).child("原文件"))
            .child(div().flex_1().min_w_0().child("解析标题（可编辑）"))
            .child(div().w(px(110.0)).child("来源"))
            .child(div().w(px(230.0)).child("状态 / 新文件名"))
            .child(div().w(px(168.0)).child("操作"));

        let mut body = div()
            .id("rows")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .bg(theme.card)
            .flex()
            .flex_col();
        for row in &self.rows {
            body = body.child(self.render_row(row, cx));
        }

        div()
            .flex_1()
            .min_h_0()
            .mx(px(20.0))
            .mb(px(12.0))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(2.0))
                    .pb(px(6.0))
                    .text_size(px(11.0))
                    .text_color(theme.fg_dim)
                    .child(if self.scanning > 0 {
                        "正在扫描文件夹…"
                    } else {
                        "继续拖拽 PDF 到窗口可追加文件"
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .overflow_hidden()
                    .bg(theme.card)
                    .child(header)
                    .child(body),
            )
    }

    fn render_row(&self, row: &Row, cx: &mut Context<Self>) -> Div {
        let theme = &self.theme;
        let id = row.id;
        let mut actions = div()
            .w(px(168.0))
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .gap(px(6.0));

        if row.status == RowStatus::Ready {
            let btn = small_button(theme, ("gen", id), "生成副本").on_click(
                cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.copy_row(id, cx);
                    cx.notify();
                }),
            );
            actions = actions.child(btn);
        }
        if row.status == RowStatus::Copied {
            let edited = row.field.read(cx).text() != row.title;
            if edited {
                let btn = small_button(theme, ("regen", id), "重新生成").on_click(
                    cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.copy_row(id, cx);
                        cx.notify();
                    }),
                );
                actions = actions.child(btn);
            }
            let reveal = row.new_path.clone().unwrap_or_else(|| row.path.clone());
            let btn = small_button(theme, ("open", id), "打开目录").on_click(
                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    cx.reveal_path(&reveal);
                },
            );
            actions = actions.child(btn);
        }
        if row.status == RowStatus::Failed {
            let btn = small_button(theme, ("retry", id), "重试").on_click(
                cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    if let Some(row) = this.rows.iter_mut().find(|row| row.id == id) {
                        row.status = RowStatus::Queued;
                        row.error = None;
                    }
                    this.pump_local();
                    cx.notify();
                }),
            );
            actions = actions.child(btn);
        }
        let remove = div()
            .id(("del", id))
            .h(px(24.0))
            .px(px(6.0))
            .flex()
            .items_center()
            .rounded(theme.radius_sm)
            .text_size(px(12.0))
            .text_color(theme.fg_dim)
            .cursor_pointer()
            .hover(|this| this.bg(theme.hover).text_color(theme.fg))
            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.remove_row(id);
                cx.notify();
            }))
            .child("\u{2715}");
        actions = actions.child(remove);

        let title_cell = match row.status {
            RowStatus::Ready | RowStatus::Copied => div()
                .min_h_0()
                .h(px(26.0))
                .rounded(theme.radius_sm)
                .border_1()
                .border_color(theme.border)
                .bg(theme.bg)
                .px(px(4.0))
                .flex()
                .items_center()
                .child(row.field.clone()),
            RowStatus::Parsing => div()
                .text_size(px(12.0))
                .text_color(theme.fg_dim)
                .child("解析中…"),
            RowStatus::Failed => div()
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(row.error.clone().unwrap_or_else(|| "失败".to_string())),
            RowStatus::Queued => div()
                .text_size(px(12.0))
                .text_color(theme.fg_dim)
                .child("等待中"),
        };

        let source = div()
            .w(px(110.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .child(match &row.source {
                Some(source) => badge_outline(theme, source),
                None => div(),
            })
            .child(if row.verifying {
                div()
                    .text_size(px(11.0))
                    .text_color(theme.fg_dim)
                    .child("核对中…")
            } else {
                div()
            });

        let status = div()
            .w(px(230.0))
            .min_w_0()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(3.0))
            .child(match row.status {
                RowStatus::Ready => badge_outline(theme, "就绪"),
                RowStatus::Copied => badge_solid(theme, row.copy_label.unwrap_or("已生成")),
                RowStatus::Failed => badge_solid(theme, "失败"),
                RowStatus::Parsing => badge_solid(theme, "解析中"),
                RowStatus::Queued => div().text_size(px(11.0)).text_color(theme.fg_dim).child("等待"),
            })
            .child({
                let preview = match row.status {
                    RowStatus::Ready => row.suggested.to_string(),
                    RowStatus::Copied => row
                        .new_path
                        .as_ref()
                        .and_then(|path| path.file_name())
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                if preview.is_empty() {
                    div()
                } else {
                    div()
                        .w(px(200.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(11.0))
                        .text_color(theme.fg_secondary)
                        .child(SharedString::from(preview))
                }
            })
            .child(if let Some(error) = &row.copy_error {
                div()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(SharedString::from(error.clone()))
            } else {
                div()
            });

        let mut title_col = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(title_cell);
        for warning in &row.warnings {
            title_col = title_col.child(
                div()
                    .text_size(px(11.0))
                    .text_color(theme.fg_dim)
                    .child(SharedString::from(warning.clone())),
            );
        }

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .px(px(14.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(theme.border)
            .hover(|this| this.bg(theme.hover))
            .child(
                div()
                    .w(px(200.0))
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(12.0))
                    .text_color(theme.fg_secondary)
                    .child(SharedString::from(row.name.clone())),
            )
            .child(title_col)
            .child(source)
            .child(status)
            .child(actions)
    }

    fn render_statusbar(&self) -> Div {
        let theme = &self.theme;
        let total = self.rows.len();
        let ready = self.count(Some(RowStatus::Ready));
        let copied = self.count(Some(RowStatus::Copied));
        let failed = self.count(Some(RowStatus::Failed));
        div()
            .flex()
            .flex_row()
            .justify_between()
            .px(px(20.0))
            .py(px(8.0))
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.card)
            .text_size(px(12.0))
            .text_color(theme.fg_secondary)
            .child(SharedString::from(format!(
                "共 {total} 个 · 就绪 {ready} · 已生成 {copied} · 失败 {failed}"
            )))
            .child(
                div()
                    .text_color(theme.fg_dim)
                    .child("副本生成在原 PDF 同目录，不改动原文件"),
            )
    }

    fn count(&self, status: Option<RowStatus>) -> usize {
        match status {
            Some(status) => self.rows.iter().filter(|row| row.status == status).count(),
            None => self.rows.len(),
        }
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme.clone();
        let empty = self.rows.is_empty();
        let mut root = div()
            .id("root")
            .size_full()
            .bg(theme.bg)
            .text_color(theme.fg)
            .font_family(theme.font.clone())
            .text_size(theme.font_size)
            .flex()
            .flex_col()
            .on_drop(cx.listener(Self::on_file_drop));

        root = root
            .child(self.render_topbar(window, cx))
            .child(self.render_toolbar(cx))
            .child(if empty {
                self.render_empty().into_any_element()
            } else {
                self.render_list(cx).into_any_element()
            })
            .child(self.render_statusbar());
        if self.drag_over {
            root = root.child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .w_full()
                    .h_full()
                    .border_2()
                    .border_color(theme.fg),
            );
        }
        root
    }
}

fn hwnd_of(window: &Window) -> Option<HWND> {
    let handle = <Window as HasWindowHandle>::window_handle(window).ok()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return None;
    };
    Some(HWND(win32.hwnd.get() as *mut core::ffi::c_void))
}

fn apply_window_icon(window: &Window) {
    let Some(hwnd) = hwnd_of(window) else {
        return;
    };
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let wide: Vec<u16> = exe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let mut large = HICON::default();
        let mut small = HICON::default();
        let count = ExtractIconExW(
            PCWSTR(wide.as_ptr()),
            0,
            Some(&mut large),
            Some(&mut small),
            1,
        );
        if count > 0 {
            if !large.is_invalid() {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_BIG as usize)),
                    Some(LPARAM(large.0 as isize)),
                );
            }
            if !small.is_invalid() {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_SMALL as usize)),
                    Some(LPARAM(small.0 as isize)),
                );
            }
        }
    }
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .map(|ext| ext.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
}

fn scan_dir(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root).sort_by_file_name() {
        let Ok(entry) = entry else { continue };
        if entry.file_type().is_file() && is_pdf(entry.path()) {
            out.push(entry.path().to_path_buf());
        }
    }
    out
}

fn button(
    theme: &Theme,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    primary: bool,
    small: bool,
) -> Stateful<Div> {
    let base = div()
        .id(id)
        .h(px(if small { 24.0 } else { 30.0 }))
        .px(px(if small { 10.0 } else { 14.0 }))
        .flex()
        .items_center()
        .justify_center()
        .rounded(theme.radius_sm)
        .border_1()
        .cursor_pointer()
        .text_size(px(if small { 12.0 } else { 13.0 }))
        .child(label.into());
    if primary {
        base.bg(theme.solid_bg)
            .text_color(theme.solid_fg)
            .border_color(theme.solid_bg)
            .hover(|this| this.opacity(0.85))
    } else {
        base.bg(theme.card)
            .text_color(theme.fg)
            .border_color(theme.border_strong)
            .hover(|this| this.bg(theme.hover))
    }
}

fn small_button(theme: &Theme, id: impl Into<ElementId>, label: &'static str) -> Stateful<Div> {
    button(theme, id, label, false, true)
}

fn checkbox(
    theme: &Theme,
    id: impl Into<ElementId>,
    label: &'static str,
    checked: bool,
) -> Stateful<Div> {
    let box_el = div()
        .w(px(15.0))
        .h(px(15.0))
        .rounded(px(3.0))
        .border_1()
        .border_color(theme.border_strong)
        .bg(if checked { theme.solid_bg } else { theme.card })
        .flex()
        .items_center()
        .justify_center()
        .child(if checked {
            div()
                .font_family(ICON_FONT)
                .text_size(px(10.0))
                .text_color(theme.solid_fg)
                .child(GLYPH_CHECK)
        } else {
            div()
        });
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .ml(px(8.0))
        .px(px(4.0))
        .py(px(2.0))
        .rounded(theme.radius_sm)
        .cursor_pointer()
        .text_size(px(13.0))
        .text_color(theme.fg_secondary)
        .hover(|this| this.bg(theme.hover).text_color(theme.fg))
        .child(box_el)
        .child(label)
}

fn badge_outline(theme: &Theme, label: &str) -> Div {
    div()
        .px(px(9.0))
        .py(px(2.0))
        .rounded(px(999.0))
        .border_1()
        .border_color(theme.border_strong)
        .text_size(px(11.0))
        .text_color(theme.fg_secondary)
        .child(SharedString::from(label.to_string()))
}

fn badge_solid(theme: &Theme, label: &str) -> Div {
    div()
        .px(px(9.0))
        .py(px(2.0))
        .rounded(px(999.0))
        .bg(theme.solid_bg)
        .text_color(theme.solid_fg)
        .text_size(px(11.0))
        .child(SharedString::from(label.to_string()))
}

fn close_caption_button(
    theme: &Theme,
    id: impl Into<ElementId>,
    glyph: &'static str,
) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(46.0))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_color(theme.fg_secondary)
        .hover(|this| this.bg(rgb(CLOSE_HOVER)).text_color(rgb(0xffffff)))
        .active(|this| this.bg(rgb(CLOSE_PRESSED)).text_color(rgb(0xffffff)))
        .child(
            div()
                .font_family(ICON_FONT)
                .text_size(px(10.0))
                .child(glyph),
        )
}

fn caption_button(theme: &Theme, id: impl Into<ElementId>, glyph: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(46.0))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_color(theme.fg_secondary)
        .hover(|this| this.bg(theme.hover).text_color(theme.fg))
        .child(
            div()
                .font_family(ICON_FONT)
                .text_size(px(10.0))
                .child(glyph),
        )
}
