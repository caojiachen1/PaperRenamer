//! Monochrome Fluent palette ported from the Tauri UI (src/styles.css).

use gpui::{hsla, px, rgb, Hsla, Pixels, SharedString};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

#[derive(Clone)]
pub struct Theme {
    pub mode: Mode,
    pub bg: Hsla,
    pub card: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub fg: Hsla,
    pub fg_secondary: Hsla,
    pub fg_dim: Hsla,
    pub hover: Hsla,
    pub solid_bg: Hsla,
    pub solid_fg: Hsla,
    pub font: SharedString,
    pub mono: SharedString,
    pub font_size: Pixels,
    pub text: Hsla,
    pub text_disabled: Hsla,
    pub cursor: Hsla,
    pub selection: Hsla,
    pub radius: Pixels,
    pub radius_sm: Pixels,
}

impl Theme {
    pub fn new(mode: Mode) -> Self {
        let (bg, card, border, border_strong, fg, fg_secondary, fg_dim, hover, solid_bg, solid_fg) =
            match mode {
                Mode::Dark => (
                    0x1f1f1f, 0x2b2b2b, 0x3d3d3d, 0x525252, 0xf0f0f0, 0xb8b8b8, 0x8a8a8a,
                    0x383838, 0xf0f0f0, 0x1a1a1a,
                ),
                Mode::Light => (
                    0xf3f3f3, 0xffffff, 0xe0e0e0, 0xc8c8c8, 0x1a1a1a, 0x5c5c5c, 0x8a8a8a,
                    0xececec, 0x1a1a1a, 0xffffff,
                ),
            };

        let c = |v: u32| -> Hsla { rgb(v).into() };
        let fg_hsla = c(fg);
        let selection = if mode == Mode::Dark {
            hsla(0.0, 0.0, 1.0, 0.18)
        } else {
            hsla(0.0, 0.0, 0.0, 0.15)
        };

        Self {
            mode,
            bg: c(bg),
            card: c(card),
            border: c(border),
            border_strong: c(border_strong),
            fg: fg_hsla,
            fg_secondary: c(fg_secondary),
            fg_dim: c(fg_dim),
            hover: c(hover),
            solid_bg: c(solid_bg),
            solid_fg: c(solid_fg),
            font: SharedString::from("Microsoft YaHei UI"),
            mono: SharedString::from("Microsoft YaHei UI"),
            font_size: px(13.0),
            text: fg_hsla,
            text_disabled: c(fg_dim),
            cursor: fg_hsla,
            selection,
            radius: px(8.0),
            radius_sm: px(4.0),
        }
    }
}
