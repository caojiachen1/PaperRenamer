//! Assets compiled into the binary: pdfium, header CRF model, lexicons.

pub const PDFIUM_DLL: &[u8] = include_bytes!("../../assets/pdfium.dll");
pub const HEADER_MODEL: &[u8] = include_bytes!("../../assets/header-model.wapiti");
pub const LEX_ENGLISH_WF: &[u8] = include_bytes!("../../assets/lexicon/wordforms/english.wf");
pub const LEX_NAMES_FEMALE: &[u8] = include_bytes!("../../assets/lexicon/names/names.female");
pub const LEX_NAMES_MALE: &[u8] = include_bytes!("../../assets/lexicon/names/names.male");
pub const LEX_FIRSTNAME_5K: &[u8] = include_bytes!("../../assets/lexicon/names/firstname.5k");
pub const LEX_NAMES_FAMILY: &[u8] = include_bytes!("../../assets/lexicon/names/names.family");
pub const LEX_LASTNAME_5K: &[u8] = include_bytes!("../../assets/lexicon/names/lastname.5k");
