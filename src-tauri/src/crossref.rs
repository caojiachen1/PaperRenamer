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
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(6))
            .user_agent(format!("PaperRenamer/0.1 (mailto:{MAILTO})"))
            .build()
            .expect("reqwest client")
    })
}

/// Consecutive transport failures. Once the network looks dead, skip further
/// lookups so an offline batch does not pay the connect timeout per file.
fn network_failures() -> &'static std::sync::atomic::AtomicU32 {
    static FAILURES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    &FAILURES
}

fn note_transport_error() {
    network_failures().fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

fn note_reachable() {
    network_failures().store(0, std::sync::atomic::Ordering::Relaxed);
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
        "https://api.crossref.org/works/{}",
        utf8_percent_encode(&clean, NON_ALPHANUMERIC)
    );
    let result = match get_json(&url, &[("mailto", MAILTO)]) {
        Ok(Some(json)) => Ok(parse_message(&json["message"])),
        Ok(None) => Ok(None),
        Err(transient) => Err(transient),
    };
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

    let json = match get_json(
        "https://api.crossref.org/works",
        &[
            ("query.title", clean),
            ("rows", "1"),
            ("mailto", MAILTO),
        ],
    ) {
        Ok(Some(json)) => json,
        Ok(None) => {
            cache().lock().unwrap().insert(key, None);
            return None;
        }
        Err(_) => return None, // network error — keep retryable
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

/// Ok(Some) = JSON body, Ok(None) = definitive rejection, Err = retryable.
/// Throttling responses (429/5xx) are retried a couple of times; transport
/// errors are not retried but count towards the offline circuit breaker.
fn get_json(url: &str, query: &[(&str, &str)]) -> Result<Option<Value>, bool> {
    use std::sync::atomic::Ordering;
    const ATTEMPTS: u32 = 3;
    const TRIP_AFTER: u32 = 3;

    if network_failures().load(Ordering::Relaxed) >= TRIP_AFTER {
        return Err(true);
    }
    for attempt in 0..ATTEMPTS {
        let mut req = client().get(url).header("Accept", "application/json");
        if !query.is_empty() {
            req = req.query(query);
        }
        match req.send() {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if resp.status().is_success() {
                    return match resp.json::<Value>() {
                        Ok(v) => {
                            note_reachable();
                            Ok(Some(v))
                        }
                        Err(_) => Err(true),
                    };
                }
                if !is_transient(status) {
                    note_reachable();
                    return Ok(None);
                }
            }
            Err(_) => {
                note_transport_error();
                return Err(true);
            }
        }
        if attempt + 1 < ATTEMPTS {
            std::thread::sleep(Duration::from_millis(300 * u64::from(attempt + 1)));
        }
    }
    Err(true)
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
    static TAGS: OnceLock<Regex> = OnceLock::new();
    let tags = TAGS.get_or_init(|| Regex::new(r"<[^>]*>").unwrap());
    tags.replace_all(text, "").replace(['{', '}'], "")
}

fn collapse_ws(s: &str) -> String {
    static WS: OnceLock<Regex> = OnceLock::new();
    let ws = WS.get_or_init(|| Regex::new(r"\s+").unwrap());
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