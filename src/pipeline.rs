//! Title extraction pipeline: embedded XMP/Info → Rust header CRF → title choice.
//! Everything here is local; no network access.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use crate::{engine, pdfmeta, rename};

#[derive(Clone, Debug)]
pub struct LocalParse {
    pub title: String,
    pub doi: Option<String>,
    pub source: String,
    pub suggested_name: String,
    pub warnings: Vec<String>,
}

pub fn parse_local(path: &Path) -> Result<LocalParse, String> {
    let bytes = fs::read(path).map_err(|e| format!("读取文件失败: {e}"))?;
    let mut warnings = Vec::new();

    let embedded = pdfmeta::extract(&bytes);
    let header = match header_extract(path) {
        Ok(h) => Some(h),
        Err(e) => {
            warnings.push(e);
            None
        }
    };

    let grobid_title = header
        .as_ref()
        .and_then(|h| h.title.clone())
        .map(|t| strip_header_label(&t));
    let xmp_title = embedded
        .title
        .clone()
        .filter(|t| !is_placeholder_title(t))
        .map(|t| strip_header_label(&t));
    let cjk = has_cjk(grobid_title.as_deref()) || has_cjk(xmp_title.as_deref());

    let (title, source) = match (grobid_title, xmp_title) {
        (Some(g), Some(x)) => {
            if cjk {
                // The CRF is trained on Latin layouts; prefer CJK metadata.
                if has_cjk(Some(&x)) {
                    (x, "XMP".to_string())
                } else {
                    (g, "HEADER".to_string())
                }
            } else if title_similarity(&g, &x) > 0.7 {
                (g, "HEADER+XMP".to_string())
            } else {
                warnings.push("页眉识别与 XMP 标题不一致，采用页眉识别结果".to_string());
                (g, "HEADER".to_string())
            }
        }
        (Some(g), None) => (g, "HEADER".to_string()),
        (None, Some(x)) => (x, "XMP".to_string()),
        (None, None) => {
            return Err("未能提取标题（PDF 无文本层标题且无内嵌元数据）".to_string());
        }
    };

    let doi = header
        .as_ref()
        .and_then(|h| find_doi(&h.first_page_text))
        .or(embedded.doi);

    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_else(|| "pdf".to_string());
    let suggested_name = suggested_name(&title, &ext)?;

    Ok(LocalParse {
        title,
        doi,
        source,
        suggested_name,
        warnings,
    })
}

fn suggested_name(title: &str, ext: &str) -> Result<String, String> {
    let stem = rename::sanitize_title(title);
    if stem.is_empty() {
        return Err("标题清洗后为空，无法生成文件名".to_string());
    }
    Ok(format!("{stem}.{ext}"))
}

/// Publisher PDFs often print an article-state label above the title.
fn strip_header_label(title: &str) -> String {
    let mut text = title.trim();
    for label in ["Journal Pre-proof", "ARTICLE IN PRESS", "Accepted Manuscript"] {
        if let Some(prefix) = text.get(..label.len()) {
            if prefix.eq_ignore_ascii_case(label) {
                text = &text[label.len()..];
                text = text.trim_start_matches(|c: char| {
                    c.is_whitespace() || matches!(c, ':' | '-' | '–' | '—')
                });
                break;
            }
        }
    }
    collapse_ws(text)
}

fn is_placeholder_title(title: &str) -> bool {
    let text = title.trim();
    text.is_empty()
        || ["标题", "无标题", "title", "untitled", "document"]
            .iter()
            .any(|p| text.eq_ignore_ascii_case(p))
}

fn find_doi(text: &str) -> Option<String> {
    static DOI_RE: OnceLock<Option<Regex>> = OnceLock::new();
    let re = DOI_RE
        .get_or_init(|| Regex::new(r#"(?i)10\.\d{4,9}/[^\s"'<>]+"#).ok())
        .as_ref()?;
    re.find(text).map(|m| {
        m.as_str()
            .trim_end_matches(['.', ',', ')', ']', ';'])
            .to_string()
    })
}

struct HeaderInfo {
    title: Option<String>,
    first_page_text: String,
}

fn header_extract(path: &Path) -> Result<HeaderInfo, String> {
    let engine = engine::engine()?;
    let result = engine
        .title_from_pdf(path)
        .map_err(|e| format!("页眉识别失败: {e}"))?;
    Ok(HeaderInfo {
        title: result.title,
        first_page_text: result.first_page_text,
    })
}

fn has_cjk(s: Option<&str>) -> bool {
    s.map(|text| {
        text.chars().any(|c| {
            ('\u{4e00}'..='\u{9fff}').contains(&c)
                || ('\u{3040}'..='\u{30ff}').contains(&c)
                || ('\u{ac00}'..='\u{d7af}').contains(&c)
        })
    })
    .unwrap_or(false)
}

fn collapse_ws(s: &str) -> String {
    static WS: OnceLock<Regex> = OnceLock::new();
    let ws = WS.get_or_init(|| Regex::new(r"\s+").unwrap());
    ws.replace_all(s.trim(), " ").trim().to_string()
}

/// Word-overlap similarity, ported from xDoc (words longer than 2 chars).
fn word_overlap(a: &str, b: &str) -> f64 {
    let words = |s: &str| -> HashSet<String> {
        s.split_whitespace()
            .filter(|w| w.chars().count() > 2)
            .map(|w| w.to_string())
            .collect()
    };
    let wa = words(a);
    let wb = words(b);
    if wa.is_empty() || wb.is_empty() {
        return 0.0;
    }
    let common = wa.intersection(&wb).count();
    common as f64 / wa.len().max(wb.len()) as f64
}

/// Normalized similarity for cross-source title agreement (xDoc titleSimilarity).
fn title_similarity(a: &str, b: &str) -> f64 {
    static NON_WORD: OnceLock<Regex> = OnceLock::new();
    let norm = |s: &str| -> String {
        let lower = s.to_lowercase();
        let re = NON_WORD.get_or_init(|| Regex::new(r"[^a-z0-9\s]").unwrap());
        let stripped = re.replace_all(&lower, "");
        collapse_ws(&stripped)
    };
    let na = norm(a);
    let nb = norm(b);
    if na.is_empty() || nb.is_empty() {
        return 0.0;
    }
    if na == nb {
        return 1.0;
    }
    word_overlap(&na, &nb)
}
