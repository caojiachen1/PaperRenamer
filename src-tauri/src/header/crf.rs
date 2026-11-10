//! Wapiti linear-chain CRF model loading and Viterbi inference.
//!
//! Model format (wapiti `mdl_save` + `rdr_save` + `qrk_save`):
//! ```text
//! #mdl#<type>#<nact>
//! #rdr#<npats>/<ntoks>/<autouni>
//! <npats netstrings: pattern sources>
//! #qrk#<nlabels>
//! <nlabels netstrings: label names>
//! #qrk#<nobs>
//! <nobs netstrings: observation strings>
//! <nact lines: flat_index=hexfloat>
//! ```
//! Netstring = `<byte_len>:<bytes>,\n`.
//!
//! Flat weight indices follow wapiti's `mdl_sync`: unigram observations own
//! `Y` consecutive slots, bigram observations own `Y*Y` slots (`yp*Y+y`).

use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub struct CrfModel {
    pub labels: Vec<String>,
    patterns: Vec<Pattern>,
    obs: HashMap<Box<[u8]>, u32>,
    uni: Vec<Vec<(u8, f32)>>,
    bi: Vec<Vec<(u8, u8, f32)>>,
}

enum Item {
    Lit(Vec<u8>),
    Col { off: i32, col: usize },
}

struct Pattern {
    items: Vec<Item>,
}

impl CrfModel {
    pub fn load(path: &Path) -> Result<Self, String> {
        let data = fs::read(path).map_err(|e| format!("读取 CRF 模型失败: {e}"))?;
        parse_model(&data)
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        parse_model(data)
    }

    pub fn num_labels(&self) -> usize {
        self.labels.len()
    }

    /// Viterbi decode: `cols[t][c]` is the c-th feature column of token t.
    pub fn label(&self, cols: &[Vec<String>]) -> Vec<u8> {
        let y = self.labels.len();
        let t_len = cols.len();
        if t_len == 0 || y == 0 {
            return Vec::new();
        }

        let mut uni = vec![vec![0f32; y]; t_len];
        let mut bi: Vec<Vec<(u8, u8, f32)>> = vec![Vec::new(); t_len];

        for (t, row) in cols.iter().enumerate() {
            for pat in &self.patterns {
                let Some(obs) = expand(pat, cols, t) else {
                    continue;
                };
                let Some(&id) = self.obs.get(obs.as_slice()) else {
                    continue;
                };
                let id = id as usize;
                for &(l, w) in &self.uni[id] {
                    uni[t][l as usize] += w;
                }
                if t > 0 {
                    for &(yp, yy, w) in &self.bi[id] {
                        bi[t].push((yp, yy, w));
                    }
                }
            }
            let _ = row;
        }

        // Transition matrix is sparse per position; accumulate into a dense one
        // only for positions that actually carry bigram features.
        let mut score = vec![vec![f32::NEG_INFINITY; y]; t_len];
        let mut back = vec![vec![0u8; y]; t_len];
        for l in 0..y {
            score[0][l] = uni[0][l];
        }
        let mut trans = vec![0f32; y * y];
        for t in 1..t_len {
            trans.iter_mut().for_each(|v| *v = 0.0);
            for &(yp, yy, w) in &bi[t] {
                trans[yp as usize * y + yy as usize] += w;
            }
            for l in 0..y {
                let mut best = f32::NEG_INFINITY;
                let mut best_prev = 0u8;
                for p in 0..y {
                    let cand = score[t - 1][p] + trans[p * y + l];
                    if cand > best {
                        best = cand;
                        best_prev = p as u8;
                    }
                }
                score[t][l] = best + uni[t][l];
                back[t][l] = best_prev;
            }
        }

        let mut seq = vec![0u8; t_len];
        let mut arg = 0u8;
        let mut best = f32::NEG_INFINITY;
        for l in 0..y {
            if score[t_len - 1][l] > best {
                best = score[t_len - 1][l];
                arg = l as u8;
            }
        }
        seq[t_len - 1] = arg;
        for t in (0..t_len - 1).rev() {
            seq[t] = back[t + 1][seq[t + 1] as usize];
        }
        seq
    }
}

struct Buf<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Buf<'a> {
    fn line(&mut self) -> &'a [u8] {
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != b'\n' {
            self.pos += 1;
        }
        let end = self.pos;
        if self.pos < self.data.len() {
            self.pos += 1;
        }
        &self.data[start..end]
    }

    fn netstring(&mut self) -> Result<&'a [u8], String> {
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        if self.pos == start || self.data.get(self.pos) != Some(&b':') {
            return Err("CRF 模型格式错误（netstring 长度）".to_string());
        }
        let len: usize = std::str::from_utf8(&self.data[start..self.pos])
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| "CRF 模型格式错误（netstring 长度解析）".to_string())?;
        self.pos += 1;
        let s = self.pos;
        let e = s + len;
        if e > self.data.len() {
            return Err("CRF 模型格式错误（netstring 越界）".to_string());
        }
        self.pos = e;
        if self.data.get(self.pos) == Some(&b',') {
            self.pos += 1;
        }
        if self.data.get(self.pos) == Some(&b'\n') {
            self.pos += 1;
        }
        Ok(&self.data[s..e])
    }
}

fn parse_model(data: &[u8]) -> Result<CrfModel, String> {
    let mut buf = Buf { data, pos: 0 };

    let head = buf.line();
    let head_str = String::from_utf8_lossy(head);
    let mut parts = head_str.split('#');
    // "#mdl#2#15545" → "", "mdl", "2", "15545"
    if parts.next() != Some("") || parts.next() != Some("mdl") {
        return Err("CRF 模型格式错误（缺少 #mdl）".to_string());
    }
    let _mtype: u32 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "CRF 模型格式错误（模型类型）".to_string())?;
    let nact: usize = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "CRF 模型格式错误（权重数）".to_string())?;

    let rdr = String::from_utf8_lossy(buf.line()).to_string();
    let mut rdr_parts = rdr.trim_start_matches("#rdr#").split('/');
    let npats: usize = rdr_parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "CRF 模型格式错误（#rdr）".to_string())?;

    let mut patterns = Vec::with_capacity(npats);
    for _ in 0..npats {
        let src = buf.netstring()?;
        patterns.push(compile_pattern(src));
    }

    let qrk = String::from_utf8_lossy(buf.line()).to_string();
    let nlabels: usize = qrk
        .trim_start_matches("#qrk#")
        .parse()
        .map_err(|_| "CRF 模型格式错误（标签数）".to_string())?;
    let mut labels = Vec::with_capacity(nlabels);
    for _ in 0..nlabels {
        labels.push(String::from_utf8_lossy(buf.netstring()?).to_string());
    }

    let qrk = String::from_utf8_lossy(buf.line()).to_string();
    let nobs: usize = qrk
        .trim_start_matches("#qrk#")
        .parse()
        .map_err(|_| "CRF 模型格式错误（观测数）".to_string())?;
    let mut obs: HashMap<Box<[u8]>, u32> = HashMap::with_capacity(nobs);
    let mut kinds = Vec::with_capacity(nobs);
    for i in 0..nobs {
        let s = buf.netstring()?;
        kinds.push(match s.first() {
            Some(b'u') => 1u8,
            Some(b'b') => 2u8,
            Some(b'*') => 3u8,
            _ => 0u8,
        });
        obs.insert(s.to_vec().into_boxed_slice(), i as u32);
    }

    // Rebuild wapiti's flat offset layout, then bucket sparse weights per observation.
    let y = nlabels as u64;
    let mut uni: Vec<Vec<(u8, f32)>> = vec![Vec::new(); nobs];
    let mut bi: Vec<Vec<(u8, u8, f32)>> = vec![Vec::new(); nobs];
    let mut offsets = Vec::with_capacity(nobs);
    let mut start = 0u64;
    for (i, kind) in kinds.iter().enumerate() {
        let mut uoff = u64::MAX;
        let mut boff = u64::MAX;
        if kind & 1 != 0 {
            uoff = start;
            start += y;
        }
        if kind & 2 != 0 {
            boff = start;
            start += y * y;
        }
        offsets.push((uoff, boff));
        let _ = i;
    }

    for _ in 0..nact {
        let line = buf.line();
        let s = std::str::from_utf8(line).map_err(|_| "CRF 权重行不是 UTF-8".to_string())?;
        let (idx_s, w_s) = s
            .split_once('=')
            .ok_or_else(|| "CRF 权重行格式错误".to_string())?;
        let idx: u64 = idx_s
            .parse()
            .map_err(|_| "CRF 权重索引解析失败".to_string())?;
        let w = parse_hex_float(w_s).ok_or_else(|| "CRF 权重解析失败".to_string())?;

        // Find the observation owning this flat index (offsets are monotonic).
        let o = offsets
            .binary_search_by(|&(uoff, boff)| {
                let lo = if uoff != u64::MAX { uoff } else { boff };
                let hi = if uoff != u64::MAX {
                    uoff + y
                } else {
                    boff + y * y
                };
                if idx < lo {
                    std::cmp::Ordering::Greater
                } else if idx >= hi {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .map_err(|_| "CRF 权重索引不在任何观测区间内".to_string())?;
        let (uoff, boff) = offsets[o];
        if uoff != u64::MAX && idx >= uoff && idx < uoff + y {
            uni[o].push(((idx - uoff) as u8, w as f32));
        } else {
            let d = idx - boff;
            bi[o].push(((d / y) as u8, (d % y) as u8, w as f32));
        }
    }

    Ok(CrfModel {
        labels,
        patterns,
        obs,
        uni,
        bi,
    })
}

fn compile_pattern(src: &[u8]) -> Pattern {
    let mut items = Vec::new();
    let mut lit: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < src.len() {
        if src[i] == b'%' && i + 2 < src.len() && src[i + 1] == b'x' && src[i + 2] == b'[' {
            let mut j = i + 3;
            let off_start = j;
            while j < src.len() && src[j] != b',' {
                j += 1;
            }
            let off: i32 = std::str::from_utf8(&src[off_start..j])
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            j += 1;
            let col_start = j;
            while j < src.len() && src[j] != b']' {
                j += 1;
            }
            let col: usize = std::str::from_utf8(&src[col_start..j])
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            if !lit.is_empty() {
                items.push(Item::Lit(std::mem::take(&mut lit)));
            }
            items.push(Item::Col { off, col });
            i = j + 1;
        } else {
            lit.push(src[i]);
            i += 1;
        }
    }
    if !lit.is_empty() {
        items.push(Item::Lit(lit));
    }
    Pattern { items }
}

fn expand(pat: &Pattern, cols: &[Vec<String>], t: usize) -> Option<Vec<u8>> {
    const BEFORE: [&str; 5] = ["_x-1", "_x-2", "_x-3", "_x-4", "_x-#"];
    const AFTER: [&str; 5] = ["_x+1", "_x+2", "_x+3", "_x+4", "_x+#"];
    let mut out = Vec::with_capacity(32);
    for item in &pat.items {
        match item {
            Item::Lit(b) => out.extend_from_slice(b),
            Item::Col { off, col } => {
                let pos = t as i64 + *off as i64;
                if pos < 0 {
                    let k = ((-pos - 1) as usize).min(4);
                    out.extend_from_slice(BEFORE[k].as_bytes());
                } else if pos as usize >= cols.len() {
                    let k = (pos as usize - cols.len()).min(4);
                    out.extend_from_slice(AFTER[k].as_bytes());
                } else {
                    let row = &cols[pos as usize];
                    let v = row.get(*col)?;
                    out.extend_from_slice(v.as_bytes());
                }
            }
        }
    }
    Some(out)
}

/// C99 hex float: `[-]0x<hex>[.<hex>]p[+-]<dec>`
fn parse_hex_float(s: &str) -> Option<f64> {
    let s = s.trim();
    let (neg, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let s = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    let (mant, exp) = s.split_once(['p', 'P'])?;
    let exp: i32 = exp.parse().ok()?;
    let (int_part, frac_part) = match mant.split_once('.') {
        Some((i, f)) => (i, f),
        None => (mant, ""),
    };
    let mut val = 0f64;
    for c in int_part.chars() {
        val = val * 16.0 + c.to_digit(16)? as f64;
    }
    let mut scale = 1.0 / 16.0;
    for c in frac_part.chars() {
        val += c.to_digit(16)? as f64 * scale;
        scale /= 16.0;
    }
    val *= 2f64.powi(exp);
    Some(if neg { -val } else { val })
}

#[cfg(test)]
mod tests {
    use super::parse_hex_float;

    #[test]
    fn hex_float_roundtrip() {
        assert!((parse_hex_float("0x1.0cfa7dc3b07ecp+0").unwrap() - 1.050686480552438).abs() < 1e-12);
        assert!((parse_hex_float("0x1.ed0d5fc28acc2p-3").unwrap() - 0.24076213074698387).abs() < 1e-12);
        assert_eq!(parse_hex_float("0x0p+0"), Some(0.0));
        assert_eq!(parse_hex_float("garbage"), None);
    }
}
