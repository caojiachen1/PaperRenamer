//! Windows-safe filename sanitization + non-destructive renamed copy.
//! Sanitization ported from xDoc paper_rename (src-tauri/src/commands/paper.rs).

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "status")]
pub enum CopyOutcome {
    Created { new_path: String },
    IdenticalExists { new_path: String },
    AlreadyNamed { new_path: String },
}

impl CopyOutcome {
    pub fn new_path(&self) -> &str {
        match self {
            Self::Created { new_path }
            | Self::IdenticalExists { new_path }
            | Self::AlreadyNamed { new_path } => new_path,
        }
    }
}

pub fn sanitize_title(title: &str) -> String {
    static WS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let ws = WS.get_or_init(|| regex::Regex::new(r"\s+").unwrap());
    let collapsed = ws.replace_all(title.trim(), " ");
    let mut sanitized: String = collapsed
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    sanitized = sanitized.trim().trim_end_matches(['.', ' ']).to_string();
    // Keep component length sane on Windows (255-char limit per component).
    sanitized = sanitized.chars().take(180).collect();
    sanitized = sanitized.trim_end_matches(['.', ' ']).to_string();

    let upper = sanitized.to_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"];
    let is_reserved = reserved.contains(&upper.as_str())
        || (upper.len() >= 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit());
    if is_reserved {
        sanitized = format!("_{sanitized}");
    }
    sanitized
}

pub fn make_renamed_copy(src: &Path, title: &str) -> Result<CopyOutcome, String> {
    if !src.is_file() {
        return Err(format!("文件不存在: {}", src.display()));
    }
    let stem = sanitize_title(title);
    if stem.is_empty() {
        return Err("标题清洗后为空，无法生成文件名".to_string());
    }
    let parent = src.parent().unwrap_or(Path::new("."));
    let ext = src
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    let base_name = if ext.is_empty() {
        stem.clone()
    } else {
        format!("{stem}.{ext}")
    };
    let dest = parent.join(&base_name);

    if dest == src {
        return Ok(CopyOutcome::AlreadyNamed {
            new_path: src.to_string_lossy().to_string(),
        });
    }

    let final_path = if dest.exists() {
        if same_content(src, &dest)? {
            return Ok(CopyOutcome::IdenticalExists {
                new_path: dest.to_string_lossy().to_string(),
            });
        }
        let mut counter = 1u32;
        loop {
            let candidate = if ext.is_empty() {
                parent.join(format!("{stem}_{counter}"))
            } else {
                parent.join(format!("{stem}_{counter}.{ext}"))
            };
            if !candidate.exists() {
                break candidate;
            }
            if same_content(src, &candidate)? {
                return Ok(CopyOutcome::IdenticalExists {
                    new_path: candidate.to_string_lossy().to_string(),
                });
            }
            counter += 1;
        }
    } else {
        dest
    };

    fs::copy(src, &final_path).map_err(|e| format!("复制失败: {e}"))?;
    Ok(CopyOutcome::Created {
        new_path: final_path.to_string_lossy().to_string(),
    })
}

fn same_content(a: &Path, b: &Path) -> Result<bool, String> {
    let ma = fs::metadata(a).map_err(|e| e.to_string())?;
    let mb = fs::metadata(b).map_err(|e| e.to_string())?;
    if ma.len() != mb.len() {
        return Ok(false);
    }
    let ha = file_hash(a)?;
    let hb = file_hash(b)?;
    Ok(ha == hb)
}

fn file_hash(p: &Path) -> Result<String, String> {
    let bytes = fs::read(p).map_err(|e| e.to_string())?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

pub fn collect_pdfs(args: &[String]) -> (Vec<PathBuf>, Vec<String>) {
    let mut pdfs = Vec::new();
    let mut problems = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for arg in args {
        let p = absolutize(PathBuf::from(arg));
        if !p.exists() {
            problems.push(format!("路径不存在: {arg}"));
            continue;
        }
        if p.is_dir() {
            for entry in walkdir::WalkDir::new(&p).sort_by_file_name() {
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        problems.push(format!("目录遍历失败: {e}"));
                        continue;
                    }
                };
                if entry.file_type().is_file() && is_pdf(entry.path()) {
                    push_unique(&mut pdfs, &mut seen, entry.into_path());
                }
            }
        } else if is_pdf(&p) {
            push_unique(&mut pdfs, &mut seen, p);
        } else {
            problems.push(format!("不是 PDF，已跳过: {arg}"));
        }
    }
    (pdfs, problems)
}

fn is_pdf(p: &Path) -> bool {
    p.extension()
        .map(|e| e.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
}

/// Paths coming from Explorer or the shell may be relative; normalize them so
/// later working-directory changes cannot break file access.
fn absolutize(p: PathBuf) -> PathBuf {
    if p.is_absolute() {
        return p;
    }
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(&p),
        Err(_) => p,
    }
}

fn push_unique(pdfs: &mut Vec<PathBuf>, seen: &mut std::collections::HashSet<PathBuf>, p: PathBuf) {
    let key = fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
    if seen.insert(key) {
        pdfs.push(p);
    }
}