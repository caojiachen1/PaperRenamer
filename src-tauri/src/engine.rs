//! Title engine bootstrap: assets are embedded in the binary, so the only
//! runtime materialization is the pdfium DLL next to the user profile.

use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::header::{assets, HeaderEngine};

static ENGINE: OnceLock<Result<HeaderEngine, String>> = OnceLock::new();

pub fn engine() -> Result<&'static HeaderEngine, String> {
    ENGINE.get_or_init(init_inner).as_ref().map_err(|e| e.clone())
}

pub fn app_data_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata).join("PaperRenamer");
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("PaperRenamer-data")
}

fn runtime_dir() -> PathBuf {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local).join("PaperRenamer").join("runtime");
    }
    std::env::temp_dir().join("PaperRenamer").join("runtime")
}

/// Extract the embedded pdfium DLL, reusing the previous copy when identical.
fn materialize_pdfium() -> Result<PathBuf, String> {
    let dir = runtime_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("创建运行时目录失败: {e}"))?;
    let path = dir.join("pdfium.dll");
    let expected = assets::PDFIUM_DLL;

    let up_to_date = match fs::read(&path) {
        Ok(existing) => existing == expected,
        Err(_) => false,
    };
    if up_to_date {
        return Ok(path);
    }

    let tmp = dir.join(format!("pdfium-{}.tmp", std::process::id()));
    fs::write(&tmp, expected).map_err(|e| format!("写入 pdfium.dll 失败: {e}"))?;
    match fs::rename(&tmp, &path) {
        Ok(()) => Ok(path),
        Err(_) if path.is_file() => {
            let _ = fs::remove_file(&tmp);
            Ok(path)
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(format!("部署 pdfium.dll 失败: {e}"))
        }
    }
}

fn init_inner() -> Result<HeaderEngine, String> {
    let dll = materialize_pdfium()?;
    HeaderEngine::from_embedded(&dll)
}
