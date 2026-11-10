use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use flate2::write::DeflateEncoder;
use flate2::Compression;

fn pack(src: PathBuf, out: &PathBuf, name: &str, env_len: &str) {
    let data = fs::read(&src).unwrap_or_else(|e| panic!("read {}: {e}", src.display()));
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::best());
    enc.write_all(&data).expect("deflate");
    let packed = enc.finish().expect("deflate finish");
    fs::write(out.join(format!("{name}.deflate")), packed).expect("write packed asset");
    println!("cargo:rustc-env={env_len}={}", data.len());
    println!("cargo:rerun-if-changed={}", src.display());
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());

    pack(manifest.join("assets/pdfium.dll"), &out, "pdfium.dll", "PDFIUM_DLL_LEN");
    pack(
        manifest.join("assets/header-model.wapiti"),
        &out,
        "header-model.wapiti",
        "HEADER_MODEL_LEN",
    );
    pack(
        manifest.join("assets/lexicon/wordforms/english.wf"),
        &out,
        "english.wf",
        "ENGLISH_WF_LEN",
    );

    tauri_build::build()
}
