//! Embedded PDF metadata (Info dictionary + XMP packet) via lopdf. No OCR.

use regex::Regex;

use crate::xmlutil::{clean, strip_tags, unescape};

#[derive(Default, Debug)]
pub struct EmbeddedMeta {
    pub title: Option<String>,
    pub doi: Option<String>,
}

pub fn extract(bytes: &[u8]) -> EmbeddedMeta {
    let mut out = EmbeddedMeta::default();
    let doc = match lopdf::Document::load_mem(bytes) {
        Ok(d) => d,
        Err(_) => return out,
    };

    if let Ok(info) = doc
        .trailer
        .get(b"Info")
        .and_then(|o| o.as_reference())
        .and_then(|id| doc.get_object(id))
        .and_then(|o| o.as_dict())
    {
        if let Ok(raw) = info.get(b"Title").and_then(|o| o.as_str()) {
            out.title = clean(decode_pdf_string(raw));
        }
    }

    if let Ok(stream) = doc
        .catalog()
        .and_then(|c| c.get(b"Metadata"))
        .and_then(|o| o.as_reference())
        .and_then(|id| doc.get_object(id))
        .and_then(|o| o.as_stream())
    {
        if let Ok(content) = stream.decompressed_content() {
            if let Ok(xmp) = String::from_utf8(content) {
                let (title, doi) = parse_xmp(&xmp);
                // XMP is generally richer and more current than the Info dict.
                if title.is_some() {
                    out.title = title;
                }
                if doi.is_some() {
                    out.doi = doi;
                }
            }
        }
    }

    out
}

fn decode_pdf_string(raw: &[u8]) -> String {
    if raw.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = raw[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    let raw = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(raw);
    match std::str::from_utf8(raw) {
        Ok(s) => s.to_string(),
        Err(_) => raw.iter().map(|&b| b as char).collect(),
    }
}

fn parse_xmp(xmp: &str) -> (Option<String>, Option<String>) {    let li_title = Regex::new(
        r"<(?:[\w.-]+:)?title[^>]*>\s*<(?:[\w.-]+:)?Alt[^>]*>\s*<(?:[\w.-]+:)?li[^>]*>([\s\S]*?)</(?:[\w.-]+:)?li>",
    )
    .unwrap();
    let plain_title =
        Regex::new(r"<(?:[\w.-]+:)?title[^>]*>([^<]+)</(?:[\w.-]+:)?title>").unwrap();
    let doi_re = Regex::new(r"<(?:[\w.-]+:)?doi[^>]*>\s*([^<\s]+)\s*</").unwrap();
    let dc_id_doi = Regex::new(r#"(?i)doi:\s*(10\.\d{4,9}/[^\s<>"']+)"#).unwrap();

    let title = li_title
        .captures(xmp)
        .and_then(|c| c.get(1))
        .map(|m| strip_tags(m.as_str()))
        .or_else(|| {
            plain_title
                .captures(xmp)
                .and_then(|c| c.get(1))
                .map(|m| strip_tags(m.as_str()))
        })
        .and_then(clean);

    let doi = doi_re
        .captures(xmp)
        .and_then(|c| c.get(1))
        .map(|m| unescape(m.as_str()))
        .or_else(|| {
            dc_id_doi
                .captures(xmp)
                .and_then(|c| c.get(1))
                .map(|m| unescape(m.as_str()))
        })
        .and_then(clean);

    (title, doi)
}
