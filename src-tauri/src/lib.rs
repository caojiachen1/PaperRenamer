pub mod commands;
mod crossref;
mod engine;
pub mod header;
pub mod headless;
mod pdfmeta;
mod pipeline;
mod rename;
mod xmlutil;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::init_engine,
            commands::parse_pdf,
            commands::make_copy,
            commands::scan_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}