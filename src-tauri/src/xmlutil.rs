//! Shared XML text helpers for embedded metadata and GROBID TEI.

use regex::Regex;

pub fn strip_tags(s: &str) -> String {
    let re = Regex::new(r"<[^>]*>").unwrap();
    unescape(&re.replace_all(s, ""))
}

pub fn unescape(s: &str) -> String {
    let mut out = s
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'");
    let num = Regex::new(r"&#(x?)([0-9a-fA-F]+);").unwrap();
    out = num
        .replace_all(&out, |c: &regex::Captures| {
            let hex = !c[1].is_empty();
            let code = u32::from_str_radix(&c[2], if hex { 16 } else { 10 }).ok();
            code.and_then(char::from_u32)
                .map(|ch| ch.to_string())
                .unwrap_or_default()
        })
        .into_owned();
    out.replace("&amp;", "&")
}

pub fn clean(s: String) -> Option<String> {
    let ws = Regex::new(r"\s+").unwrap();
    let collapsed = ws.replace_all(s.trim(), " ").trim().to_string();
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}
