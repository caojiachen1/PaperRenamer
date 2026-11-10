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
    let assets = manifest.join("assets");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());

    pack(assets.join("pdfium.dll"), &out, "pdfium.dll", "PDFIUM_DLL_LEN");
    pack(
        assets.join("header-model.wapiti"),
        &out,
        "header-model.wapiti",
        "HEADER_MODEL_LEN",
    );
    pack(
        assets.join("lexicon/wordforms/english.wf"),
        &out,
        "english.wf",
        "ENGLISH_WF_LEN",
    );

    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(windows)]
    {
        let icon = manifest.join("icons/icon.ico");
        if icon.is_file() {
            let rc_path = out.join("app.rc");
            let icon_str = icon.display().to_string().replace('\\', "\\\\");
            fs::write(&rc_path, format!("1 ICON \"{icon_str}\"\n")).expect("write app.rc");
            embed_resource::compile(&rc_path, embed_resource::NONE)
                .manifest_optional()
                .expect("embed application icon");
            println!("cargo:rerun-if-changed={}", icon.display());
        }
    }
}
