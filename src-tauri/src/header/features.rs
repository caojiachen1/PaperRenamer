//! Port of GROBID's header token features
//! (HeaderParser.getSectionHeaderFeatured + FeaturesVectorHeader.printVector).

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use super::extract::Block;

pub struct Lexicons {
    words: HashSet<String>,
    first: HashSet<String>,
    last: HashSet<String>,
}

impl Lexicons {
    pub fn from_bytes(
        english_wf: &[u8],
        names_female: &[u8],
        names_male: &[u8],
        firstname_5k: &[u8],
        names_family: &[u8],
        lastname_5k: &[u8],
    ) -> Lexicons {
        let mut words = HashSet::new();
        load_first_field_from_bytes(english_wf, &mut words, '\t');
        let mut first = HashSet::new();
        load_first_field_from_bytes(names_female, &mut first, '\t');
        load_first_field_from_bytes(names_male, &mut first, '\t');
        load_first_field_from_bytes(firstname_5k, &mut first, '\t');
        let mut last = HashSet::new();
        load_first_field_from_bytes(names_family, &mut last, '\t');
        load_first_field_from_bytes(lastname_5k, &mut last, '\t');
        Lexicons { words, first, last }
    }

    pub fn load(lexicon_dir: &Path) -> Lexicons {
        let lex = lexicon_dir;
        let mut words = HashSet::new();
        load_first_field(&lex.join("wordforms").join("english.wf"), &mut words, '\t');
        let mut first = HashSet::new();
        load_first_field(&lex.join("names").join("names.female"), &mut first, '\t');
        load_first_field(&lex.join("names").join("names.male"), &mut first, '\t');
        load_first_field(&lex.join("names").join("firstname.5k"), &mut first, '\t');
        let mut last = HashSet::new();
        load_first_field(&lex.join("names").join("names.family"), &mut last, '\t');
        load_first_field(&lex.join("names").join("lastname.5k"), &mut last, '\t');
        Lexicons { words, first, last }
    }

    fn in_dictionary(&self, s: &str) -> bool {
        let s = s.trim_end_matches(['.', ',', ':', ';']);
        if let Some(i) = s.find('-') {
            return self.words.contains(&s[..i]) && self.words.contains(&s[i + 1..]);
        }
        if let Some(i) = s.find(' ') {
            return self.words.contains(&s[..i]) && self.words.contains(&s[i + 1..]);
        }
        self.words.contains(s)
    }

    fn in_names(&self, s: &str) -> bool {
        self.first.contains(s) || self.last.contains(s)
    }

    pub fn stats(&self) -> (usize, usize, usize) {
        (self.words.len(), self.first.len(), self.last.len())
    }
}

fn load_first_field_from_bytes(bytes: &[u8], set: &mut HashSet<String>, sep: char) {
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let word = line
            .split(sep)
            .next()
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if !word.is_empty() {
            set.insert(word);
        }
    }
}

fn load_first_field(path: &Path, set: &mut HashSet<String>, sep: char) {
    let Ok(bytes) = fs::read(path) else {
        return;
    };
    load_first_field_from_bytes(&bytes, set, sep);
}

#[derive(Clone, Debug)]
pub struct TokenMeta {
    pub text: String,
    pub block: usize,
    pub line: usize,
    pub line_first: bool,
    pub line_last: bool,
    pub space_before: bool,
}

fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_digit)
}

fn has_digit(s: &str) -> bool {
    s.chars().any(is_digit)
}

fn all_capital(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    !s.chars().any(|c| c.is_lowercase())
}

fn is_year(s: &str) -> bool {
    let b = s.as_bytes();
    b.windows(4).any(|w| {
        (w[0] == b'1' || w[0] == b'2')
            && w[1].is_ascii_digit()
            && w[2].is_ascii_digit()
            && w[3].is_ascii_digit()
    })
}

fn is_month(s: &str) -> bool {
    const MONTHS: [&str; 24] = [
        "january", "february", "march", "april", "may", "june", "july", "august", "september",
        "october", "november", "december", "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug",
        "sep", "oct", "nov", "dec",
    ];
    MONTHS.contains(&s)
}

fn is_email(s: &str) -> bool {
    let Some(at) = s.find('@') else { return false };
    let (user, host) = s.split_at(at);
    !user.is_empty() && host.len() > 1 && host.contains('.') && !host.contains(' ')
}

fn is_http(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://") || s.starts_with("www.")
}

fn prefix(s: &str, n: usize) -> &str {
    let end = s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len());
    &s[..end]
}

fn suffix(s: &str, n: usize) -> &str {
    let start = s
        .char_indices()
        .rev()
        .nth(n.saturating_sub(1))
        .map(|(i, _)| i)
        .unwrap_or(0);
    &s[start..]
}

pub fn build(blocks: &[Block], lex: &Lexicons) -> (Vec<Vec<String>>, Vec<TokenMeta>) {
    let mut metas: Vec<TokenMeta> = Vec::new();
    let mut sizes: Vec<f32> = Vec::new();
    let mut n_tokens = 0usize;

    for (bi, block) in blocks.iter().enumerate() {
        let block_len: usize = block.lines.iter().map(|l| l.tokens.len()).sum();
        let mut idx = 0usize;
        for (li, line) in block.lines.iter().enumerate() {
            for (ti, tok) in line.tokens.iter().enumerate() {
                metas.push(TokenMeta {
                    text: tok.text.clone(),
                    block: bi,
                    line: li,
                    line_first: ti == 0,
                    line_last: ti + 1 == line.tokens.len(),
                    space_before: tok.space_before,
                });
                sizes.push(tok.font_size);
                idx += 1;
            }
        }
        let _ = (block_len, idx);
    }

    let mut largest = 0f32;
    let mut smallest = 100000f32;
    let mut sum = 0f32;
    for &s in &sizes {
        if s > largest {
            largest = s;
        }
        if s < smallest {
            smallest = s;
        }
        sum += s;
        n_tokens += 1;
    }
    let average = if n_tokens > 0 { sum / n_tokens as f32 } else { 0.0 };

    let mut cols: Vec<Vec<String>> = Vec::with_capacity(metas.len());
    let mut current_font: Option<String> = None;
    let mut current_font_size: i32 = -1;
    let mut prev_line_left: Option<f32> = None;
    let mut indented = false;

    let mut flat: Vec<(&super::extract::Token, &TokenMeta)> = Vec::with_capacity(metas.len());
    {
        let mut mi = 0usize;
        for block in blocks {
            for line in &block.lines {
                for tok in &line.tokens {
                    flat.push((tok, &metas[mi]));
                    mi += 1;
                }
            }
        }
    }

    for (i, (tok, meta)) in flat.iter().enumerate() {
        let text = &tok.text;
        let block_tok_count = blocks[meta.block].lines.iter().map(|l| l.tokens.len()).sum::<usize>();
        let block_first = i == 0 || flat[i - 1].1.block != meta.block;
        let block_last = i + 1 == flat.len() || flat[i + 1].1.block != meta.block;

        let mut indented_now = indented;
        if meta.line_first {
            if let Some(prev_left) = prev_line_left {
                let char_width = if text.is_empty() {
                    1.0
                } else {
                    tok.width / text.chars().count() as f32
                };
                if tok.x - prev_left > char_width {
                    indented_now = true;
                } else if prev_left - tok.x > char_width {
                    indented_now = false;
                }
            }
            prev_line_left = Some(tok.x);
            indented = indented_now;
        }
        let _ = block_tok_count;

        // font / font size status
        let font_status = match &current_font {
            None => {
                current_font = Some(tok.font.clone());
                "NEWFONT"
            }
            Some(f) if *f != tok.font => {
                current_font = Some(tok.font.clone());
                "NEWFONT"
            }
            Some(_) => "SAMEFONT",
        };
        let new_fs = tok.font_size as i32;
        let fs_status = if current_font_size == -1 {
            current_font_size = new_fs;
            "HIGHERFONT"
        } else if current_font_size == new_fs {
            "SAMEFONTSIZE"
        } else if current_font_size < new_fs {
            current_font_size = new_fs;
            "HIGHERFONT"
        } else {
            current_font_size = new_fs;
            "LOWERFONT"
        };

        let lower = text.to_lowercase();
        let digit = if all_digits(text) {
            "ALLDIGIT"
        } else if has_digit(text) {
            "CONTAINSDIGITS"
        } else {
            "NODIGIT"
        };
        let mut capitalisation = if text.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
            "INITCAP"
        } else {
            "NOCAPS"
        };
        if all_capital(text) {
            capitalisation = "ALLCAP";
        }
        if digit == "ALLDIGIT" {
            capitalisation = "NOCAPS";
        }

        let mut punct_type = if is_punct_run(text) { "PUNCT" } else { "NOPUNCT" };
        punct_type = match text.as_str() {
            "(" | "[" => "OPENBRACKET",
            ")" | "]" => "ENDBRACKET",
            "." => "DOT",
            "," => "COMMA",
            "-" => "HYPHEN",
            "\"" | "'" | "`" => "QUOTE",
            _ => punct_type,
        };

        cols.push(vec![
            text.clone(),                                    // 0
            lower.clone(),                                   // 1
            prefix(text, 1).to_string(),                     // 2
            prefix(text, 2).to_string(),                     // 3
            prefix(text, 3).to_string(),                     // 4
            prefix(text, 4).to_string(),                     // 5
            suffix(text, 1).to_string(),                     // 6
            suffix(text, 2).to_string(),                     // 7
            suffix(text, 3).to_string(),                     // 8
            suffix(text, 4).to_string(),                     // 9
            if block_first { "BLOCKSTART" } else if block_last { "BLOCKEND" } else { "BLOCKIN" }.to_string(), // 10
            if meta.line_first { "LINESTART" } else if meta.line_last { "LINEEND" } else { "LINEIN" }.to_string(), // 11
            if indented_now { "LINEINDENT" } else { "ALIGNEDLEFT" }.to_string(), // 12
            font_status.to_string(),                         // 13
            fs_status.to_string(),                           // 14
            if tok.bold { "1" } else { "0" }.to_string(),    // 15
            if tok.italic { "1" } else { "0" }.to_string(),  // 16
            capitalisation.to_string(),                      // 17
            digit.to_string(),                               // 18
            if text.chars().count() == 1 { "1" } else { "0" }.to_string(), // 19
            if lex.in_names(&lower) { "1" } else { "0" }.to_string(),      // 20
            if lex.in_dictionary(&lower) { "1" } else { "0" }.to_string(), // 21
            if is_year(text) { "1" } else { "0" }.to_string(),             // 22
            if is_month(&lower) { "1" } else { "0" }.to_string(),          // 23
            "0".to_string(),                                 // 24 locationName (gazetteer not ported)
            if is_email(text) { "1" } else { "0" }.to_string(), // 25
            if is_http(&lower) { "1" } else { "0" }.to_string(), // 26
            punct_type.to_string(),                          // 27
            if tok.font_size == largest { "1" } else { "0" }.to_string(),   // 28
            if tok.font_size == smallest { "1" } else { "0" }.to_string(),  // 29
            if tok.font_size > average { "1" } else { "0" }.to_string(),    // 30
        ]);
    }

    (cols, metas)
}

fn is_punct_run(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| matches!(c, ',' | ':' | ';' | '?' | '.'))
}
