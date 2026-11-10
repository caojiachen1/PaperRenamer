//! Persisted user preferences shared by the GUI and drag-onto-exe mode.

use std::fs;

use serde::{Deserialize, Serialize};

use crate::engine;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub crossref: bool,
    pub show_dialog: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            crossref: false,
            show_dialog: true,
        }
    }
}

pub fn load() -> Settings {
    fs::read_to_string(engine::app_data_dir().join("settings.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(settings: &Settings) -> Result<(), String> {
    let dir = engine::app_data_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("创建设置目录失败: {e}"))?;
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(dir.join("settings.json"), text).map_err(|e| format!("写入设置失败: {e}"))
}
