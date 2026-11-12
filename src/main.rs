#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod dialog;
mod engine;
mod field;
mod header;
mod headless;
mod pdfmeta;
mod pipeline;
mod rename;
mod settings;
mod theme;
mod xmlutil;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(pos) = args.iter().position(|arg| arg == "--gui") {
        args.remove(pos);
        app::run_gui(args.into_iter().map(std::path::PathBuf::from).collect());
    } else if args.is_empty() {
        app::run_gui(Vec::new());
    } else {
        headless::run(&args);
    }
}
