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
            let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
            let mut parts = version.split('.').map(|p| p.parse::<u16>().unwrap_or(0));
            let major = parts.next().unwrap_or(0);
            let minor = parts.next().unwrap_or(0);
            let patch = parts.next().unwrap_or(0);

            let icon_str = icon.display().to_string().replace('\\', "\\\\");
            let rc = format!(
                "1 ICON \"{icon_str}\"\n\
                 1 VERSIONINFO\n\
                 FILEVERSION {major},{minor},{patch},0\n\
                 PRODUCTVERSION {major},{minor},{patch},0\n\
                 FILEOS 0x40004\n\
                 FILETYPE 0x1\n\
                 BEGIN\n\
                   BLOCK \"StringFileInfo\"\n\
                   BEGIN\n\
                     BLOCK \"040904B0\"\n\
                     BEGIN\n\
                       VALUE \"FileDescription\", \"PaperRenamer\"\n\
                       VALUE \"FileVersion\", \"{version}\"\n\
                       VALUE \"ProductName\", \"PaperRenamer\"\n\
                       VALUE \"ProductVersion\", \"{version}\"\n\
                     END\n\
                   END\n\
                   BLOCK \"VarFileInfo\"\n\
                   BEGIN\n\
                     VALUE \"Translation\", 0x409, 1200\n\
                   END\n\
                 END\n"
            );
            let rc_path = out.join("app.rc");
            fs::write(&rc_path, rc).expect("write app.rc");
            embed_resource::compile(&rc_path, embed_resource::NONE)
                .manifest_optional()
                .expect("embed icon and version resource");
            println!("cargo:rerun-if-changed={}", icon.display());
        }
    }
}
