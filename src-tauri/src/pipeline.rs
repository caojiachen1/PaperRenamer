//! Title extraction pipeline: embedded XMP/Info → Rust header CRF → CrossRef
//! cross-validation. No OCR, no external services beyond CrossRef.

use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::{crossref, engine, pdfmeta, rename};

#[derive(Serialize, Clone, Debug)]
pub struct ParseOutcome {
    pub title: String,
    pub doi: Option<String>,
    pub source: String,
    pub suggested_name: String,
    pub warnings: Vec<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct CachedHeader {
    title: Option<String>,
    doi: Option<String>,
}

pub fn parse_pdf(path: &Path, crossref_enabled: bool) -> Result<ParseOutcome, String> {
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

    let (mut title, mut source) = match (grobid_title, xmp_title) {
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

    let mut doi = header.and_then(|h| h.doi).or(embedded.doi);

    if crossref_enabled && !cjk {
        if let Some(d) = &doi {
            if let Some(hit) = crossref::resolve_doi(d) {
                // Guard against a mis-extracted DOI pointing at a different paper.
                if crossref::word_overlap(&title.to_lowercase(), &hit.title.to_lowercase()) >= 0.25
                {
                    title = hit.title;
                    source = "CrossRef".to_string();
                } else {
                    warnings.push("CrossRef 结果与本地标题差异过大，保留本地标题".to_string());
                }
            }
        } else if title.chars().count() > 10 {
            if let Some(hit) = crossref::search_by_title(&title) {
                title = hit.title;
                doi = hit.doi.or(doi);
                source = "CrossRef".to_string();
            }
        }
    } else if cjk {
        warnings.push("检测到中文论文，跳过在线校验（GROBID/CrossRef 对中文支持有限）".to_string());
    }

    let stem = rename::sanitize_title(&title);
    if stem.is_empty() {
        return Err("标题清洗后为空，无法生成文件名".to_string());
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_else(|| "pdf".to_string());

    Ok(ParseOutcome {
        title,
        doi,
        source,
        suggested_name: format!("{stem}.{ext}"),
        warnings,
    })
}

fn header_extract(path: &Path) -> Result<CachedHeader, String> {
    let engine = engine::engine()?;
    let title = engine
        .title_from_pdf(path)
        .map_err(|e| format!("页眉识别失败: {e}"))?
        .title;
    let doi = engine.doi_from_text(path);
    Ok(CachedHeader { title, doi })
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