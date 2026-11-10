//! CrossRef online cross-validation (DOI resolve + title search).
//! Ported from xDoc src/utils/crossrefResolver.ts — title-relevant parts only.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use regex::Regex;
use serde_json::Value;

const MAILTO: &str = "xdoc@app.local";

#[derive(Clone, Debug)]
pub struct CrossRefHit {
    pub title: String,
    pub doi: Option<String>,
}

fn client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(8))
            .user_agent(format!("PaperRenamer/0.1 (mailto:{MAILTO})"))
            .build()
            .expect("reqwest client")
    })
}

fn cache() -> &'static Mutex<HashMap<String, Option<CrossRefHit>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<CrossRefHit>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn is_transient(status: u16) -> bool {
    status == 408 || status == 429 || status >= 500
}

pub fn resolve_doi(doi: &str) -> Option<CrossRefHit> {
    let mut clean = doi.trim().to_lowercase();
    for prefix in ["https://doi.org/", "http://doi.org/", "doi:"] {
        if let Some(rest) = clean.strip_prefix(prefix) {
            clean = rest.to_string();
        }
    }
    let clean = clean.trim_matches(|c: char| c == '.' || c == ')' || c == ']').to_string();
    if clean.is_empty() {
        return None;
    }

    let key = format!("doi:{clean}");
    if let Some(hit) = cache().lock().unwrap().get(&key) {
        return hit.clone();
    }

    let url = format!(
        "https://api.crossref.org/works/{}?mailto={MAILTO}",
        utf8_percent_encode(&clean, NON_ALPHANUMERIC)
    );
    let result = fetch_work(&url);
    let mut guard = cache().lock().unwrap();
    match result {
        Ok(Some(hit)) => {
            guard.insert(key, Some(hit.clone()));
            Some(hit)
        }
        Ok(None) => {
            guard.insert(key, None);
            None
        }
        Err(transient) => {
            if !transient {
                guard.insert(key, None);
            }
            None
        }
    }
}

pub fn search_by_title(title: &str) -> Option<CrossRefHit> {
    let clean = title.trim();
    if clean.chars().count() < 10 {
        return None;
    }
    let key = format!("title:{}", clean.to_lowercase().chars().take(80).collect::<String>());
    if let Some(hit) = cache().lock().unwrap().get(&key) {
        return hit.clone();
    }

    let resp = client()
        .get("https://api.crossref.org/works")
        .query(&[
            ("query.title", clean),
            ("rows", "1"),
            ("mailto", MAILTO),
        ])
        .header("Accept", "application/json")
        .send();

    let resp = match resp {
        Ok(r) => r,
        Err(_) => return None, // network error — keep retryable
    };
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        if !is_transient(status) {
            cache().lock().unwrap().insert(key, None);
        }
        return None;
    }
    let json: Value = match resp.json() {
        Ok(v) => v,
        Err(_) => return None,
    };
    let candidate = &json["message"]["items"][0];
    let candidate_title = candidate["title"][0].as_str().unwrap_or("");
    if word_overlap(&clean.to_lowercase(), &candidate_title.to_lowercase()) < 0.7 {
        cache().lock().unwrap().insert(key, None);
        return None;
    }
    let hit = parse_message(candidate);
    cache().lock().unwrap().insert(key, hit.clone());
    hit
}

/// Ok(Some) = hit, Ok(None) = definitive rejection, Err(transient) = retryable.
fn fetch_work(url: &str) -> Result<Option<CrossRefHit>, bool> {
    let resp = client()
        .get(url)
        .header("Accept", "application/json")
        .send()
        .map_err(|_| true)?;
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        return if is_transient(status) {
            Err(true)
        } else {
            Ok(None)
        };
    }
    let json: Value = resp.json().map_err(|_| true)?;
    Ok(parse_message(&json["message"]))
}

fn parse_message(msg: &Value) -> Option<CrossRefHit> {
    let mut title = strip_markup(msg["title"][0].as_str()?);
    if let Some(sub) = msg["subtitle"][0].as_str() {
        let sub = strip_markup(sub);
        if !sub.is_empty() && !title.to_lowercase().contains(&sub.to_lowercase()) {
            if title.ends_with(':') {
                title = format!("{title} {sub}");
            } else {
                title = format!("{title}: {sub}");
            }
        }
    }
    let title = collapse_ws(&title);
    if title.is_empty() {
        return None;
    }
    Some(CrossRefHit {
        title,
        doi: msg["DOI"].as_str().map(|s| s.to_string()),
    })
}

fn strip_markup(text: &str) -> String {
    let tags = Regex::new(r"<[^>]*>").unwrap();
    tags.replace_all(text, "").replace(['{', '}'], "")
}

fn collapse_ws(s: &str) -> String {
    let ws = Regex::new(r"\s+").unwrap();
    ws.replace_all(s.trim(), " ").trim().to_string()
}

/// Word-overlap similarity, ported from xDoc (words longer than 2 chars).
pub fn word_overlap(a: &str, b: &str) -> f64 {
    let words = |s: &str| -> std::collections::HashSet<String> {
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
pub fn title_similarity(a: &str, b: &str) -> f64 {
    let norm = |s: &str| -> String {
        let lower = s.to_lowercase();
        let stripped = Regex::new(r"[^a-z0-9\s]").unwrap().replace_all(&lower, "");
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