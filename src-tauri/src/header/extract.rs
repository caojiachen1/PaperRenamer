//! PDF page text extraction via pdfium: characters -> lines -> blocks.
//!
//! Mirrors the granularity of pdfalto's XML that GROBID consumes: word tokens
//! with font name/size, bold/italic flags, block and line structure.

use std::path::Path;

use pdfium_render::prelude::*;

#[derive(Clone, Debug)]
pub struct Token {
    pub text: String,
    pub x: f32,
    pub width: f32,
    pub font: String,
    pub font_size: f32,
    pub bold: bool,
    pub italic: bool,
    /// Whether the source layout had a visible gap before this token.
    pub space_before: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Line {
    pub tokens: Vec<Token>,
    pub left: f32,
    pub top: f32,
    pub bottom: f32,
    pub font_size: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Block {
    pub lines: Vec<Line>,
}

struct Ch {
    c: char,
    x0: f32,
    x1: f32,
    cy: f32,
    top: f32,
    bottom: f32,
    fs: f32,
    font: String,
    bold: bool,
    italic: bool,
    /// The preceding character in the PDF text stream was a space glyph.
    space_before: bool,
}

pub fn bind_pdfium(dll: &Path) -> Result<Pdfium, String> {
    if !dll.is_file() {
        return Err(format!("找不到 pdfium.dll: {}", dll.display()));
    }
    let bindings = Pdfium::bind_to_library(dll).map_err(|e| format!("加载 pdfium 失败: {e}"))?;
    Ok(Pdfium::new(bindings))
}

pub fn extract_page(pdfium: &Pdfium, pdf: &Path, page_index: i32) -> Result<Vec<Block>, String> {
    Ok(extract_page_with_raw(pdfium, pdf, page_index)?.0)
}

/// Same as [extract_page], but also returns the raw text stream of the page so
/// callers can reuse the single document open for DOI detection.
pub fn extract_page_with_raw(
    pdfium: &Pdfium,
    pdf: &Path,
    page_index: i32,
) -> Result<(Vec<Block>, String), String> {
    let doc = pdfium
        .load_pdf_from_file(pdf, None)
        .map_err(|e| format!("打开 PDF 失败: {e}"))?;
    let page = doc
        .pages()
        .get(page_index)
        .map_err(|e| format!("读取页面失败: {e}"))?;
    let text = page.text().map_err(|e| format!("抽取文本层失败: {e}"))?;
    let raw = text.all();

    let mut chars: Vec<Ch> = Vec::with_capacity(text.len() as usize);
    let mut pending_space = true;
    for ch in text.chars().iter() {
        let Some(c) = ch.unicode_char() else { continue };
        if c.is_control() && c != ' ' {
            continue;
        }
        let bounds = match ch.loose_bounds() {
            Ok(b) => b,
            Err(_) => continue,
        };
        let fs = ch.scaled_font_size().value;
        let font_name = ch.font_name();
        let lower = font_name.to_ascii_lowercase();
        let item = Ch {
            c,
            x0: bounds.left().value,
            x1: bounds.right().value,
            cy: (bounds.top().value + bounds.bottom().value) / 2.0,
            top: bounds.top().value,
            bottom: bounds.bottom().value,
            fs: if fs > 0.0 { fs } else { 1.0 },
            bold: lower.contains("bold") || ch.font_is_bold_reenforced(),
            italic: lower.contains("italic") || ch.font_is_italic(),
            font: strip_subset_prefix(&font_name).to_ascii_lowercase(),
            space_before: pending_space,
        };
        if c == ' ' || c == '\u{a0}' || c == '\t' {
            pending_space = true;
        } else {
            pending_space = false;
            chars.push(item);
        }
    }

    // Reading order: top of page first (PDF y grows upwards), then left to right.
    chars.sort_by(|a, b| {
        b.cy.partial_cmp(&a.cy)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x0.partial_cmp(&b.x0).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut lines: Vec<Vec<Ch>> = Vec::new();
    let mut line_cy = 0f32;
    let mut line_fs = 0f32;
    for ch in chars {
        if lines.is_empty() {
            line_cy = ch.cy;
            line_fs = ch.fs;
            lines.push(vec![ch]);
            continue;
        }
        let tol = 0.45 * line_fs.max(ch.fs);
        if (ch.cy - line_cy).abs() <= tol {
            line_cy = (line_cy * 0.8) + (ch.cy * 0.2);
            lines.last_mut().unwrap().push(ch);
        } else {
            line_cy = ch.cy;
            line_fs = ch.fs;
            lines.push(vec![ch]);
        }
    }

    let mut out_lines: Vec<Line> = Vec::new();
    for mut chars in lines {
        chars.sort_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap_or(std::cmp::Ordering::Equal));
        let mut tokens: Vec<Token> = Vec::new();
        let mut cur: Option<Token> = None;
        for ch in &chars {
            let space_before = ch.space_before;
            let cjk = is_cjk(ch.c);
            if is_delimiter(ch.c) || cjk {
                if let Some(t) = cur.take() {
                    tokens.push(t);
                }
                // Punctuation delimiters and CJK characters become standalone
                // tokens, mirroring GROBID's default/CJK analyzers.
                if !ch.c.is_whitespace() && ch.c != '\u{a0}' && ch.c != '\u{200c}' {
                    tokens.push(Token {
                        text: ch.c.to_string(),
                        x: ch.x0,
                        width: ch.x1 - ch.x0,
                        font: ch.font.clone(),
                        font_size: ch.fs,
                        bold: ch.bold,
                        italic: ch.italic,
                        space_before,
                    });
                }
                continue;
            }
            let new_token = cur.is_none() || space_before;
            match cur.as_mut() {
                Some(t) if !new_token => {
                    t.text.push(ch.c);
                    t.width = ch.x1 - t.x;
                }
                _ => {
                    if let Some(t) = cur.take() {
                        tokens.push(t);
                    }
                    cur = Some(Token {
                        text: ch.c.to_string(),
                        x: ch.x0,
                        width: ch.x1 - ch.x0,
                        font: ch.font.clone(),
                        font_size: ch.fs,
                        bold: ch.bold,
                        italic: ch.italic,
                        space_before,
                    });
                }
            }
        }
        if let Some(t) = cur.take() {
            tokens.push(t);
        }
        if tokens.is_empty() {
            continue;
        }
        let left = tokens.first().map(|t| t.x).unwrap_or(0.0);
        let top = chars.iter().map(|c| c.top).fold(f32::MIN, f32::max);
        let bottom = chars.iter().map(|c| c.bottom).fold(f32::MAX, f32::min);
        let font_size = median(&chars.iter().map(|c| c.fs).collect::<Vec<_>>());
        out_lines.push(Line {
            tokens,
            left,
            top,
            bottom,
            font_size,
        });
    }

    // Re-sort lines by visual position: out_lines is already in reading order
    // from the char sort, but char-grouping can interleave; sort defensively.
    out_lines.sort_by(|a, b| {
        b.top
            .partial_cmp(&a.top)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok((group_blocks(out_lines), raw))
}

fn group_blocks(lines: Vec<Line>) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    for line in lines {
        let new_block = match blocks.last().and_then(|b| b.lines.last()) {
            None => true,
            Some(prev) => {
                let gap = prev.bottom - line.top;
                let fs = prev.font_size.max(line.font_size);
                gap > 0.8 * fs
                    || (line.font_size - prev.font_size).abs() > 0.6
                    || (line.left - prev.left).abs() > 2.0 * line.font_size
            }
        };
        if new_block {
            blocks.push(Block { lines: vec![line] });
        } else {
            blocks.last_mut().unwrap().lines.push(line);
        }
    }
    blocks
}

fn median(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[v.len() / 2]
}

fn strip_subset_prefix(name: &str) -> String {
    match name.split_once('+') {
        Some((prefix, rest)) if prefix.len() == 6 && prefix.chars().all(|c| c.is_ascii_uppercase()) => {
            rest.to_string()
        }
        _ => name.to_string(),
    }
}

/// GROBID's TextUtilities.delimiters: whitespace controls plus the punctuation
/// set that the default analyzer splits into standalone tokens.
pub(crate) fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{3400}'..='\u{4dbf}'
        | '\u{4e00}'..='\u{9fff}'
        | '\u{f900}'..='\u{faff}'
        | '\u{3040}'..='\u{30ff}'
        | '\u{ac00}'..='\u{d7af}'
    )
}

fn is_delimiter(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r'
            | '\t'
            | '\u{0c}'
            | ' '
            | '\u{a0}'
            | '\u{200c}'
            | '('
            | '（'
            | '['
            | '•'
            | '*'
            | ','
            | ':'
            | ';'
            | '?'
            | '.'
            | '!'
            | '/'
            | ')'
            | '）'
            | '-'
            | '−'
            | '–'
            | '‐'
            | '«'
            | '»'
            | '„'
            | '"'
            | '“'
            | '”'
            | '‘'
            | '’'
            | '\''
            | '`'
            | '$'
            | '#'
            | '@'
            | ']'
            | '◆'
            | '♥'
            | '♣'
            | '♠'
            | '。'
            | '、'
            | '，'
            | '・'
    )
}
