//! Tauri commands for the GUI mode.

use std::path::{Path, PathBuf};

use crate::{engine, pipeline, rename, settings};

#[tauri::command]
pub async fn init_engine() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| engine::engine().map(|_| ()))
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
        .map(|_| "ready".to_string())
}

#[tauri::command]
pub async fn parse_local(path: String) -> Result<pipeline::LocalParse, String> {
    tauri::async_runtime::spawn_blocking(move || pipeline::parse_local(Path::new(&path)))
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command(rename_all = "snake_case")]
pub async fn verify_crossref(
    title: String,
    doi: Option<String>,
    source: String,
    suggested_name: String,
    cjk: bool,
    ext: String,
) -> Result<pipeline::VerifyOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let local = pipeline::LocalParse {
            title,
            doi,
            source,
            suggested_name,
            warnings: Vec::new(),
            cjk,
            ext,
        };
        pipeline::verify(&local)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))
}

#[tauri::command]
pub async fn get_settings() -> Result<settings::Settings, String> {
    tauri::async_runtime::spawn_blocking(settings::load)
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))
}

#[tauri::command]
pub async fn set_crossref(crossref: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut current = settings::load();
        current.crossref = crossref;
        settings::save(&current)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn make_copy(path: String, title: String) -> Result<rename::CopyOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || {
        rename::make_renamed_copy(Path::new(&path), &title)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn scan_folder(path: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = PathBuf::from(&path);
        if !root.is_dir() {
            return Err(format!("不是目录: {path}"));
        }
        let mut pdfs = Vec::new();
        for entry in walkdir::WalkDir::new(&root).sort_by_file_name() {
            let entry = entry.map_err(|e| format!("目录遍历失败: {e}"))?;
            if entry.file_type().is_file()
                && entry
                    .path()
                    .extension()
                    .map(|e| e.eq_ignore_ascii_case("pdf"))
                    .unwrap_or(false)
            {
                pdfs.push(entry.path().to_string_lossy().to_string());
            }
        }
        Ok(pdfs)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}
