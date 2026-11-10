//! Drag-onto-exe mode: batch parse + renamed copies, log file, summary dialog.

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{engine, pipeline, rename};

pub fn run(args: &[String]) {
    let (pdfs, problems) = rename::collect_pdfs(args);

    let mut lines: Vec<String> = Vec::new();
    for p in &problems {
        lines.push(format!("[SKIP] {p}"));
    }

    if let Err(e) = engine::engine() {
        lines.push(format!("[WARN] 标题引擎不可用，仅尝试内嵌元数据: {e}"));
    }

    let mut created = 0u32;
    let mut existing = 0u32;
    let mut already_named = 0u32;
    let mut failed = 0u32;

    for pdf in &pdfs {
        let display = pdf.display();
        match pipeline::parse_pdf(pdf, true) {
            Ok(outcome) => {
                for w in &outcome.warnings {
                    lines.push(format!("[WARN] {display}: {w}"));
                }
                match rename::make_renamed_copy(pdf, &outcome.title) {
                    Ok(copy) => {
                        let tag = match &copy {
                            rename::CopyOutcome::Created { .. } => {
                                created += 1;
                                "OK"
                            }
                            rename::CopyOutcome::IdenticalExists { .. } => {
                                existing += 1;
                                "EXIST"
                            }
                            rename::CopyOutcome::AlreadyNamed { .. } => {
                                already_named += 1;
                                "NAMED"
                            }
                        };
                        lines.push(format!(
                            "[{tag}] {display} -> {} (来源: {})",
                            copy.new_path(),
                            outcome.source
                        ));
                    }
                    Err(e) => {
                        failed += 1;
                        lines.push(format!("[FAIL] {display}: {e}"));
                    }
                }
            }
            Err(e) => {
                failed += 1;
                lines.push(format!("[FAIL] {display}: {e}"));
            }
        }
    }

    let summary = format!(
        "共 {} 个 PDF：新建副本 {created}，已存在 {existing}，文件名已是标题 {already_named}，失败 {failed}",
        pdfs.len()
    );
    lines.push(summary.clone());

    let log_path = write_log(&lines);
    for line in &lines {
        eprintln!("{line}");
    }

    let mut text = summary;
    if let Some(lp) = &log_path {
        text.push_str(&format!("\n\n日志: {}", lp.display()));
    }
    if pdfs.is_empty() {
        text = format!("没有可处理的 PDF 文件。\n\n{}", problems.join("\n"));
    }
    message_box(&text, failed > 0 && created == 0 && existing == 0);
}

fn write_log(lines: &[String]) -> Option<std::path::PathBuf> {
    let dir = engine::app_data_dir().join("logs");
    fs::create_dir_all(&dir).ok()?;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("rename-{secs}.log"));
    fs::write(&path, lines.join("\n")).ok()?;
    Some(path)
}

fn message_box(text: &str, error: bool) {
    if std::env::var_os("PAPERRENAMER_NO_DIALOG").is_some() {
        return;
    }
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK,
    };
    let to_wide = |s: &str| -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    };
    let body = to_wide(text);
    let caption = to_wide("PaperRenamer");
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            body.as_ptr(),
            caption.as_ptr(),
            MB_OK | if error { MB_ICONERROR } else { MB_ICONINFORMATION },
        );
    }
}