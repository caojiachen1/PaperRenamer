//! Drag-onto-exe mode: batch parse + renamed copies, log file, summary dialog.
//!
//! Local extraction (pdfium is not thread-safe) runs sequentially, then the
//! network cross-check and file copies run on a small worker pool.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::pipeline::{self, LocalParse};
use crate::{engine, rename};

struct Item {
    pdf: PathBuf,
    display: String,
    local: Result<LocalParse, String>,
}

struct Processed {
    lines: Vec<String>,
    created: u32,
    existing: u32,
    already_named: u32,
    failed: u32,
}

pub fn run(args: &[String]) {
    let started = Instant::now();
    let timing = std::env::var_os("PAPERRENAMER_TIMING").is_some();
    let online = if std::env::var_os("PAPERRENAMER_OFFLINE").is_some() {
        false
    } else if std::env::var_os("PAPERRENAMER_ONLINE").is_some() {
        true
    } else {
        crate::settings::load().crossref
    };
    let (pdfs, problems) = rename::collect_pdfs(args);

    let mut lines: Vec<String> = problems.iter().map(|p| format!("[SKIP] {p}")).collect();
    if let Err(e) = engine::engine() {
        lines.push(format!("[WARN] 标题引擎不可用，仅尝试内嵌元数据: {e}"));
    }

    let mut items: Vec<Item> = Vec::with_capacity(pdfs.len());
    for pdf in &pdfs {
        items.push(Item {
            display: pdf.display().to_string(),
            local: pipeline::parse_local(pdf),
            pdf: pdf.clone(),
        });
    }
    let local_done = Instant::now();

    let results: Vec<OnceLock<Processed>> = (0..items.len()).map(|_| OnceLock::new()).collect();
    for (i, item) in items.iter().enumerate() {
        if let Err(e) = &item.local {
            let _ = results[i].set(Processed {
                lines: vec![format!("[FAIL] {}: {e}", item.display)],
                created: 0,
                existing: 0,
                already_named: 0,
                failed: 1,
            });
        }
    }

    let workers = std::env::var("PAPERRENAMER_WORKERS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .clamp(2, 4)
        })
        .clamp(1, 16)
        .min(items.len().max(1));
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= items.len() {
                    break;
                }
                if results[i].get().is_some() {
                    continue;
                }
                let item = &items[i];
                let local = item.local.as_ref().expect("local parse succeeded");
                let _ = results[i].set(process_item(item, local, online));
            });
        }
    });
    let processed_done = Instant::now();

    let (mut created, mut existing, mut already_named, mut failed) = (0u32, 0u32, 0u32, 0u32);
    for result in results {
        let p = result.into_inner().expect("every item processed");
        lines.extend(p.lines);
        created += p.created;
        existing += p.existing;
        already_named += p.already_named;
        failed += p.failed;
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
    if timing {
        eprintln!(
            "[TIMING] local {:.0}ms, verify+copy {:.0}ms, total {:.0}ms, workers {workers}",
            (local_done - started).as_secs_f64() * 1000.0,
            (processed_done - local_done).as_secs_f64() * 1000.0,
            started.elapsed().as_secs_f64() * 1000.0
        );
    }

    crate::ui::show_summary(crate::ui::Summary {
        total: pdfs.len(),
        created,
        existing,
        already_named,
        failed,
        log_path,
        problems,
    });
}

fn process_item(item: &Item, local: &LocalParse, online: bool) -> Processed {
    let mut lines = Vec::new();
    let mut title = local.title.clone();
    let mut source = local.source.clone();

    for w in &local.warnings {
        lines.push(format!("[WARN] {}: {w}", item.display));
    }
    if online && !local.cjk {
        let verified = pipeline::verify(local);
        title = verified.title;
        source = verified.source;
        if let Some(w) = verified.warning {
            lines.push(format!("[WARN] {}: {w}", item.display));
        }
    }

    let mut processed = Processed {
        lines,
        created: 0,
        existing: 0,
        already_named: 0,
        failed: 0,
    };
    match rename::make_renamed_copy(&item.pdf, &title) {
        Ok(copy) => {
            let tag = match &copy {
                rename::CopyOutcome::Created { .. } => {
                    processed.created = 1;
                    "OK"
                }
                rename::CopyOutcome::IdenticalExists { .. } => {
                    processed.existing = 1;
                    "EXIST"
                }
                rename::CopyOutcome::AlreadyNamed { .. } => {
                    processed.already_named = 1;
                    "NAMED"
                }
            };
            processed.lines.push(format!(
                "[{tag}] {} -> {} (来源: {source})",
                item.display,
                copy.new_path()
            ));
        }
        Err(e) => {
            processed.failed = 1;
            processed.lines.push(format!("[FAIL] {}: {e}", item.display));
        }
    }
    processed
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

