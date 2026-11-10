//! Pure-Rust header title extraction: pdfium text layout + GROBID header CRF.

pub mod assets;
pub mod crf;
pub mod extract;
pub mod features;

use std::path::Path;

use pdfium_render::prelude::Pdfium;

use crf::CrfModel;
use features::{Lexicons, TokenMeta};

pub struct HeaderEngine {
    crf: CrfModel,
    lex: Lexicons,
    pdfium: Pdfium,
}

pub struct TitleResult {
    pub title: Option<String>,
    pub pages: usize,
    pub tokens: usize,
}

impl HeaderEngine {
    /// Build the engine from assets compiled into the binary; only the pdfium
    /// DLL path is external (it is materialized from `assets::PDFIUM_DLL`).
    pub fn from_embedded(pdfium_dll: &Path) -> Result<Self, String> {
        let pdfium = extract::bind_pdfium(pdfium_dll)?;
        let crf = CrfModel::from_bytes(assets::HEADER_MODEL)?;
        let lex = Lexicons::from_bytes(
            assets::LEX_ENGLISH_WF,
            assets::LEX_NAMES_FEMALE,
            assets::LEX_NAMES_MALE,
            assets::LEX_FIRSTNAME_5K,
            assets::LEX_NAMES_FAMILY,
            assets::LEX_LASTNAME_5K,
        );
        Ok(HeaderEngine { crf, lex, pdfium })
    }

    /// `assets` holds pdfium.dll, header-model.wapiti and lexicon/.
    pub fn from_assets(assets: &Path) -> Result<Self, String> {
        let pdfium = extract::bind_pdfium(&assets.join("pdfium.dll"))?;
        let crf = CrfModel::load(&assets.join("header-model.wapiti"))?;
        let lex = Lexicons::load(&assets.join("lexicon"));
        Ok(HeaderEngine { crf, lex, pdfium })
    }

    pub fn new(pdfium_dll: &Path, model: &Path, lexicon_dir: &Path) -> Result<Self, String> {
        let pdfium = extract::bind_pdfium(pdfium_dll)?;
        let crf = CrfModel::load(model)?;
        let lex = Lexicons::load(lexicon_dir);
        Ok(HeaderEngine { crf, lex, pdfium })
    }

    /// First-page DOI candidate (`10.xxxx/...`) from the raw text layer.
    pub fn doi_from_text(&self, pdf: &Path) -> Option<String> {
        let doc = self.pdfium.load_pdf_from_file(pdf, None).ok()?;
        let page = doc.pages().get(0).ok()?;
        let text = page.text().ok()?;
        let raw = text.all();
        let re = regex::Regex::new(r#"(?i)10\.\d{4,9}/[^\s"'<>]+"#).ok()?;
        re.find(&raw).map(|m| {
            m.as_str()
                .trim_end_matches(['.', ',', ')', ']', ';'])
                .to_string()
        })
    }

    pub fn title_from_pdf(&self, pdf: &Path) -> Result<TitleResult, String> {
        let blocks = trim_to_header(extract::extract_page(&self.pdfium, pdf, 0)?);
        let (cols, metas) = features::build(&blocks, &self.lex);
        let labels = self.crf.label(&cols);
        let title = assemble_title(&self.crf, &labels, &metas);
        let title = cjk_fallback(&blocks, title);
        Ok(TitleResult {
            title,
            pages: 1,
            tokens: metas.len(),
        })
    }

    /// Per-token dump for debugging/validation: (text, label).
    pub fn dump(&self, pdf: &Path) -> Result<Vec<(String, String)>, String> {
        let blocks = extract::extract_page(&self.pdfium, pdf, 0)?;
        let (cols, metas) = features::build(&blocks, &self.lex);
        let labels = self.crf.label(&cols);
        Ok(metas
            .iter()
            .zip(labels.iter())
            .map(|(m, &l)| {
                (
                    m.text.clone(),
                    self.crf.labels.get(l as usize).cloned().unwrap_or_default(),
                )
            })
            .collect())
    }

    pub fn lex_stats(&self) -> (usize, usize, usize) {
        self.lex.stats()
    }

    /// Per-token (text, label) dump for debugging.
    pub fn label_dump(&self, pdf: &Path) -> Result<Vec<(String, String)>, String> {
        let blocks = trim_to_header(extract::extract_page(&self.pdfium, pdf, 0)?);
        let (cols, metas) = features::build(&blocks, &self.lex);
        let labels = self.crf.label(&cols);
        Ok(metas
            .iter()
            .zip(labels.iter())
            .map(|(m, &l)| {
                (
                    m.text.clone(),
                    self.crf.labels.get(l as usize).cloned().unwrap_or_default(),
                )
            })
            .collect())
    }

    /// Feature matrix dump in wapiti training format (space separated columns).
    pub fn dump_features(&self, pdf: &Path) -> Result<Vec<String>, String> {
        let blocks = trim_to_header(extract::extract_page(&self.pdfium, pdf, 0)?);
        let (cols, _) = features::build(&blocks, &self.lex);
        Ok(cols
            .iter()
            .map(|row| row.join(" "))
            .collect())
    }

    /// Token layout dump: block/line index, font size, x/width, text.
    pub fn dump_layout(&self, pdf: &Path) -> Result<Vec<String>, String> {        let blocks = extract::extract_page(&self.pdfium, pdf, 0)?;
        let mut out = Vec::new();
        for (bi, block) in blocks.iter().enumerate() {
            for (li, line) in block.lines.iter().enumerate() {
                for tok in &line.tokens {
                    out.push(format!(
                        "b{bi} l{li} x={:.1}-{:.1} fs={:.2} sb={} {} {} {:?}",
                        tok.x,
                        tok.x + tok.width,
                        tok.font_size,
                        tok.space_before as u8,
                        if tok.bold { "B" } else { "-" },
                        if tok.italic { "I" } else { "-" },
                        tok.text
                    ));
                }
            }
        }
        Ok(out)
    }
}

fn base_label(label: &str) -> &str {    label.strip_prefix("I-").unwrap_or(label)
}

/// Approximate GROBID's segmentation-defined header region: drop the publisher
/// running header at the top and stop before the abstract.
fn trim_to_header(blocks: Vec<extract::Block>) -> Vec<extract::Block> {
    let block_text = |b: &extract::Block| -> String {
        b.lines
            .iter()
            .flat_map(|l| l.tokens.iter())
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };

    let mut start = 0usize;
    for (i, b) in blocks.iter().take(8).enumerate() {
        let text = block_text(b);
        if text.contains("journal homepage")
            || text.contains("contents lists available")
            || text.contains("sciencedirect")
        {
            start = i + 1;
        }
    }

    let mut end = blocks.len();
    for (i, b) in blocks.iter().enumerate().skip(start) {
        let text = block_text(b);
        let compact: String = text
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '.')
            .collect();
        if compact.starts_with("abstract")
            || compact.starts_with("articleinfo")
            || compact.starts_with("keywords")
            || compact.starts_with("1introduction")
            || compact.starts_with("摘要")
            || compact.starts_with("关键词")
            || compact.starts_with("中图分类号")
        {
            end = i;
            break;
        }
    }

    if start >= end {
        return blocks;
    }
    blocks[start..end].to_vec()
}

fn assemble_title(crf: &CrfModel, labels: &[u8], metas: &[TokenMeta]) -> Option<String> {
    let mut first_title: Option<String> = None;
    let mut current_base: Option<String> = None;
    let mut cluster: Vec<usize> = Vec::new();

    for (i, meta) in metas.iter().enumerate() {
        let label = crf
            .labels
            .get(labels.get(i).copied().unwrap_or(0) as usize)
            .cloned()
            .unwrap_or_default();
        // GROBID convention: "I-" marks the *start* of an entity; plain labels
        // continue the current cluster.
        let is_start = label.starts_with("I-");
        let base = base_label(&label).to_string();

        let continue_cluster = !is_start && current_base.as_deref() == Some(base.as_str());
        if continue_cluster {
            cluster.push(i);
        } else {
            flush_title(&mut first_title, current_base.as_deref(), &cluster, metas);
            cluster.clear();
            cluster.push(i);
            current_base = Some(base);
        }
        let _ = meta;
    }
    flush_title(&mut first_title, current_base.as_deref(), &cluster, metas);

    first_title.filter(|t| !t.is_empty())
}

fn is_cjk_text(s: &str) -> bool {
    s.chars().next().map(extract::is_cjk).unwrap_or(false)
}

/// The header CRF is trained on Latin layouts and rarely labels Chinese text.
/// For CJK-heavy first pages, fall back to the largest-font line(s) of the
/// header region, which in Chinese journals is the article title.
fn cjk_fallback(blocks: &[extract::Block], title: Option<String>) -> Option<String> {
    let good_title = title
        .as_ref()
        .map(|t| t.chars().filter(|&c| extract::is_cjk(c)).count() >= 2)
        .unwrap_or(false);
    if good_title {
        return title;
    }

    let mut total = 0usize;
    let mut cjk = 0usize;
    for block in blocks {
        for line in &block.lines {
            for tok in &line.tokens {
                total += tok.text.chars().count();
                cjk += tok.text.chars().filter(|&c| extract::is_cjk(c)).count();
            }
        }
    }
    if total == 0 || cjk * 5 < total {
        return title;
    }

    let line_info: Vec<(f32, String, usize)> = blocks
        .iter()
        .flat_map(|b| b.lines.iter())
        .map(|line| {
            let mut fs = 0f32;
            let mut text = String::new();
            let mut len = 0usize;
            let mut prev_cjk = false;
            for tok in &line.tokens {
                fs = fs.max(tok.font_size);
                let tok_cjk = is_cjk_text(&tok.text);
                if !text.is_empty() && tok.space_before && !(prev_cjk && tok_cjk) {
                    text.push(' ');
                }
                text.push_str(&tok.text);
                len += tok.text.chars().count();
                prev_cjk = tok_cjk;
            }
            (fs, text, len)
        })
        .collect();

    let mut best_fs = 0f32;
    for (fs, text, len) in &line_info {
        if is_metadata_block(text) || starts_meta_line(text) || !(6..=80).contains(len) {
            continue;
        }
        if text.chars().filter(|&c| extract::is_cjk(c)).count() >= 3 && *fs > best_fs {
            best_fs = *fs;
        }
    }
    if best_fs <= 0.0 {
        return title;
    }

    let mut parts: Vec<String> = Vec::new();
    let mut started = false;
    for (fs, text, len) in &line_info {
        let qualifies = (*fs - best_fs).abs() <= 0.4
            && (6..=80).contains(len)
            && !is_metadata_block(text)
            && !starts_meta_line(text);
        if !started {
            if qualifies {
                let cleaned = strip_title_prefix(text);
                if !cleaned.is_empty() {
                    parts.push(cleaned);
                    started = true;
                }
            }
            continue;
        }
        if qualifies {
            parts.push(strip_title_prefix(text));
        } else {
            break;
        }
    }
    if parts.is_empty() {
        return title;
    }
    let candidate = clean_title(&parts.join(" "));
    if candidate.chars().count() >= 6 {
        Some(candidate)
    } else {
        title
    }
}

fn starts_meta_line(text: &str) -> bool {
    let t = text.trim();
    ["作者", "单位", "通信作者", "收稿", "摘要", "关键词", "基金"]
        .iter()
        .any(|m| t.starts_with(m))
}

fn is_metadata_block(text: &str) -> bool {
    let t = text.trim();
    t.starts_with('《')
        || t.contains("网络首发")
        || t.contains("[J/OL]")
        || t.contains("https://doi.org")
        || t.contains("ISSN")
        || t.starts_with("出版")
        || t.starts_with("版权")
        || t.starts_with("声明")
        || t.starts_with("引用")
        || t.starts_with("收稿日期")
        || t.starts_with("基金项目")
        || t.starts_with("作者简介")
        || t.starts_with("通信作者")
        || t.starts_with("中图分类号")
        || t.starts_with("文章编号")
        || t.starts_with("DOI")
        || t.starts_with("doi")
}

fn strip_title_prefix(text: &str) -> String {
    let mut s = text.trim();
    for prefix in ["题目：", "题目:", "文章题目：", "文章题目:", "Title:", "Title："] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim();
            break;
        }
    }
    s.to_string()
}

fn flush_title(
    first_title: &mut Option<String>,
    base: Option<&str>,
    cluster: &[usize],
    metas: &[TokenMeta],
) {
    if first_title.is_some() || base != Some("<title>") || cluster.is_empty() {
        return;
    }
    let text = cluster_text(cluster, metas);
    if !text.is_empty() {
        *first_title = Some(clean_title(&text));
    }
}

/// Rebuild the cluster text with layout spacing; a hyphen that ends a line is
/// dropped and the next line joins directly (GROBID dehyphenization).
fn cluster_text(cluster: &[usize], metas: &[TokenMeta]) -> String {
    let mut out = String::new();
    let mut pending_join = false;
    for (k, &idx) in cluster.iter().enumerate() {
        let meta = &metas[idx];
        let prev = if k == 0 { None } else { Some(&metas[cluster[k - 1]]) };
        let next = cluster.get(k + 1).map(|&n| &metas[n]);
        let line_break = prev
            .map(|p| p.block != meta.block || p.line != meta.line)
            .unwrap_or(false);
        let hyphen_at_line_end = meta.text == "-"
            && next
                .map(|n| n.block != meta.block || n.line != meta.line)
                .unwrap_or(false);
        if hyphen_at_line_end {
            pending_join = true;
            continue;
        }
        let need_space = if pending_join || out.is_empty() {
            false
        } else if meta.space_before {
            true
        } else if line_break {
            // Chinese lines wrap without spaces; Latin words need one.
            !(prev.map(|p| is_cjk_text(&p.text)).unwrap_or(false) && is_cjk_text(&meta.text))
        } else {
            false
        };
        if need_space {
            out.push(' ');
        }
        out.push_str(&meta.text);
        pending_join = false;
    }
    out
}

#[allow(dead_code)]
fn should_join_hyphen(metas: &[TokenMeta], i: usize) -> bool {
    if i == 0 {
        return false;
    }
    let prev = &metas[i - 1];
    let cur = &metas[i];
    prev.text.ends_with('-') && (prev.block != cur.block || prev.line != cur.line)
}

fn clean_title(raw: &str) -> String {
    let normalized: String = raw
        .chars()
        .map(|c| match c {
            '\u{2010}' | '\u{2011}' => '-',
            c => c,
        })
        .collect();
    let mut s = normalized
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    // GROBID strips a spurious leading "1" digit from titles.
    if s.len() > 2 && s.starts_with('1') && s.chars().nth(1).map(|c| c == ' ').unwrap_or(false) {
        s = s[1..].trim().to_string();
    }
    s.trim_matches(|c: char| c == '\u{200b}' || c.is_whitespace())
        .to_string()
}
