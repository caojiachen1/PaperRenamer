//! Title extraction pipeline, split into two phases so the slow online check
//! can run concurrently with other files:
//!   phase 1 (local): embedded XMP/Info → Rust header CRF → title choice
//!   phase 2 (online): CrossRef cross-validation against the local result

use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;

use crate::{crossref, engine, pdfmeta, rename};

/// Everything that can be decided without network access.
#[derive(Serialize, Clone, Debug)]
pub struct LocalParse {
    pub title: String,
    pub doi: Option<String>,
    pub source: String,
    pub suggested_name: String,
    pub warnings: Vec<String>,
    pub cjk: bool,
    pub ext: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct VerifyOutcome {
    pub title: String,
    pub doi: Option<String>,
    pub source: String,
    pub suggested_name: String,
    pub warning: Option<String>,
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

    let grobid_title = header.as_ref().and_then(|h| h.title.clone());
    let xmp_title = embedded.title.clone();
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
            } else if crossref::title_similarity(&g, &x) > 0.7 {
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

    if cjk {
        warnings.push("检测到中文论文，跳过在线校验（GROBID/CrossRef 对中文支持有限）".to_string());
    }

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
        cjk,
        ext,
    })
}

/// CrossRef verification of a local result. Returns the local values unchanged
/// when the online lookup is inconclusive.
pub fn verify(local: &LocalParse) -> VerifyOutcome {
    let mut out = VerifyOutcome {
        title: local.title.clone(),
        doi: local.doi.clone(),
        source: local.source.clone(),
        suggested_name: local.suggested_name.clone(),
        warning: None,
    };
    if local.cjk {
        return out;
    }

    let mut changed = false;
    if let Some(d) = &local.doi {
        if let Some(hit) = crossref::resolve_doi(d) {
            // Guard against a mis-extracted DOI pointing at a different paper.
            if crossref::word_overlap(&out.title.to_lowercase(), &hit.title.to_lowercase()) >= 0.25
            {
                out.title = hit.title;
                out.source = "CrossRef".to_string();
                changed = true;
            } else {
                out.warning = Some("CrossRef 结果与本地标题差异过大，保留本地标题".to_string());
            }
        }
    } else if out.title.chars().count() > 10 {
        if let Some(hit) = crossref::search_by_title(&out.title) {
            out.title = hit.title;
            out.doi = hit.doi.or(out.doi);
            out.source = "CrossRef".to_string();
            changed = true;
        }
    }

    if changed {
        match suggested_name(&out.title, &local.ext) {
            Ok(name) => out.suggested_name = name,
            Err(_) => {
                out.title = local.title.clone();
                out.source = local.source.clone();
            }
        }
    }
    out
}

fn suggested_name(title: &str, ext: &str) -> Result<String, String> {
    let stem = rename::sanitize_title(title);
    if stem.is_empty() {
        return Err("标题清洗后为空，无法生成文件名".to_string());
    }
    Ok(format!("{stem}.{ext}"))
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
