#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        paper_renamer_lib::run();
    } else {
        paper_renamer_lib::headless::run(&args);
    }
}