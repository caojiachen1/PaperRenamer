//! Assets compiled into the binary: pdfium, header CRF model, lexicons.
//! Large assets are deflated at build time and inflated on first use.

pub const PDFIUM_DLL_LEN: usize = const_parse(env!("PDFIUM_DLL_LEN"));

const fn const_parse(s: &str) -> usize {
    let b = s.as_bytes();
    let mut n = 0usize;
    let mut i = 0;
    while i < b.len() {
        n = n * 10 + (b[i] - b'0') as usize;
        i += 1;
    }
    n
}

pub const PDFIUM_DLL_DEFLATE: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/pdfium.dll.deflate"));
pub const HEADER_MODEL_DEFLATE: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/header-model.wapiti.deflate"));
pub const LEX_ENGLISH_WF_DEFLATE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/english.wf.deflate"));

pub const LEX_NAMES_FEMALE: &[u8] = include_bytes!("../../assets/lexicon/names/names.female");
pub const LEX_NAMES_MALE: &[u8] = include_bytes!("../../assets/lexicon/names/names.male");
pub const LEX_FIRSTNAME_5K: &[u8] = include_bytes!("../../assets/lexicon/names/firstname.5k");
pub const LEX_NAMES_FAMILY: &[u8] = include_bytes!("../../assets/lexicon/names/names.family");
pub const LEX_LASTNAME_5K: &[u8] = include_bytes!("../../assets/lexicon/names/lastname.5k");

pub fn inflate(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::DeflateDecoder::new(bytes)
        .read_to_end(&mut out)
        .map_err(|e| format!("解压内嵌资产失败: {e}"))?;
    Ok(out)
}
