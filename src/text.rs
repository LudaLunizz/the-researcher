// SPDX-License-Identifier: MIT
//! Document → plain text, the token squeeze, and the persistent text copy of the library.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::process::Command;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::util::{self, chars, py_rstrip, py_splitlines, py_strip};

/// Bump when `html_text`, `convert` or `squeeze` changes: every stored copy is made again.
/// Shared with the Python version, which writes the same copies.
pub const TEXT_VERSION: i64 = 2;

fn re(p: &str) -> Regex {
    Regex::new(p).expect("valid built-in pattern")
}

static OPEN_SKIP: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)<(script|style|svg)\b"));
static BREAKS: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)<br[\s\x1c-\x1f]*/?>|</(p|div|li|tr|h[1-6]|section|table)>"));
static CELL: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)<(td|th)\b[^>]*>"));
static ITEM: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)<li\b[^>]*>"));
static TAG: LazyLock<Regex> = LazyLock::new(|| re(r"<[^>]+>"));
static INLINE_WS: LazyLock<Regex> = LazyLock::new(|| re(r"[ \t\r\x0c\x0b]+"));
static BLANK_RUNS: LazyLock<Regex> = LazyLock::new(|| re(r"\n[\s\x1c-\x1f]*\n+"));
static PAGE_NUMBER: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)^(page[\s\x1c-\x1f]+)?\d{1,4}([\s\x1c-\x1f]*(/|of)[\s\x1c-\x1f]*\d{1,4})?$"));
static PAGE_WORD: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(page|pagina|blz|seite|p\.)[\s\x1c-\x1f]*\d"));
static RULE: LazyLock<Regex> = LazyLock::new(|| re(r"^[\s\x1c-\x1f|_=*~.\x{b7}\x{2022}\x{2500}-\x{257f}\-]{3,}$"));
static DIGITS: LazyLock<Regex> = LazyLock::new(|| re(r"\d+"));
static WIDE_GAP: LazyLock<Regex> = LazyLock::new(|| re(r" {3,}"));
static SPACE_RUN: LazyLock<Regex> = LazyLock::new(|| re(r"[ \t]{2,}"));

/// Remove `<script>`, `<style>` and `<svg>` elements with their content.
fn drop_skipped(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase(); // same byte offsets as `raw`
    let mut out = String::with_capacity(raw.len());
    let (mut copied, mut pos) = (0, 0);
    while let Some(caps) = OPEN_SKIP.captures_at(raw, pos) {
        let open = caps.get(0).expect("whole match");
        let close = format!("</{}>", caps[1].to_ascii_lowercase());
        match lower[open.end()..].find(&close) {
            Some(i) => {
                let end = open.end() + i + close.len();
                out.push_str(&raw[copied..open.start()]);
                out.push(' ');
                copied = end;
                pos = end;
            }
            None => pos = open.start() + 1, // no closing tag: try the next opening one
        }
        if pos >= raw.len() {
            break;
        }
    }
    out.push_str(&raw[copied..]);
    out
}

/// HTML → plain text (block tags become line breaks, table cells " | ", list items "- ").
pub fn html_text(raw: &str) -> String {
    let t = drop_skipped(raw);
    let t = BREAKS.replace_all(&t, "\n");
    let t = CELL.replace_all(&t, " | ");
    let t = ITEM.replace_all(&t, "- ");
    let t = TAG.replace_all(&t, " ");
    let t = htmlize::unescape(t.as_ref());
    let t = INLINE_WS.replace_all(&t, " ");
    let t = BLANK_RUNS.replace_all(&t, "\n\n");
    let joined = py_splitlines(&t).into_iter().map(py_strip).collect::<Vec<_>>().join("\n");
    py_strip(&joined).to_string()
}

/// How to squeeze a converted document.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Pdf,
    Html,
    /// Markdown and plain text: only tidied, indentation and code are kept.
    Plain,
}

fn invisible(c: char) -> bool {
    matches!(c, '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}' | '\u{ad}')
}

/// Page-numbered lines ("Report | page 3 of 9") match across pages; others only exactly.
fn furniture_key(line: &str) -> String {
    let k = py_strip(line);
    if PAGE_WORD.is_match(k) { DIGITS.replace_all(k, "#").into_owned() } else { k.to_string() }
}

/// Remove page numbers and headers/footers that repeat on most pages of a PDF.
fn drop_page_furniture(pages: Vec<Vec<&str>>) -> Vec<Vec<&str>> {
    let cands: Vec<Vec<usize>> = pages
        .iter()
        .map(|lines| {
            let idx: Vec<usize> = (0..lines.len()).filter(|&i| !py_strip(lines[i]).is_empty()).collect();
            let mut c: Vec<usize> = idx.iter().take(3).copied().collect();
            c.extend(idx.iter().skip(idx.len().saturating_sub(3)));
            c.sort_unstable();
            c.dedup();
            c
        })
        .collect();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (lines, cand) in pages.iter().zip(&cands) {
        let mut keys: Vec<String> = cand.iter().map(|&i| furniture_key(lines[i])).collect();
        keys.sort();
        keys.dedup();
        for k in keys {
            *seen.entry(k).or_default() += 1;
        }
    }
    let need = 3.max(pages.len() * 2 / 5);
    pages
        .into_iter()
        .zip(cands)
        .map(|(lines, cand)| {
            lines
                .into_iter()
                .enumerate()
                .filter(|(i, ln)| {
                    let k = py_strip(ln);
                    let furniture = cand.contains(i)
                        && !k.is_empty()
                        && (PAGE_NUMBER.is_match(k) || seen.get(&furniture_key(k)).copied().unwrap_or(0) >= need);
                    !furniture
                })
                .map(|(_, ln)| ln)
                .collect()
        })
        .collect()
}

/// A line made of one of `-_=*~` repeated five or more times.
fn plain_rule(s: &str) -> bool {
    let mut it = s.chars();
    match it.next() {
        Some(c) if "-_=*~".contains(c) => chars(s) >= 5 && it.all(|x| x == c),
        _ => false,
    }
}

/// Fewer tokens, same content.
pub fn squeeze(text: &str, kind: Kind) -> String {
    let text: String = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\u{a0}', " ")
        .chars()
        .filter(|&c| !invisible(c))
        .collect();
    let text = if kind == Kind::Pdf {
        let pages: Vec<Vec<&str>> = text.split('\x0c').map(|pg| pg.split('\n').collect()).collect();
        let pages = if pages.len() >= 3 { drop_page_furniture(pages) } else { pages };
        pages.iter().map(|pg| pg.join("\n")).collect::<Vec<_>>().join("\n")
    } else {
        text
    };
    let mut out: Vec<String> = Vec::new();
    let mut blank = true;
    for ln in text.split('\n') {
        let mut ln = py_rstrip(ln).to_string();
        if kind != Kind::Plain {
            // layout whitespace means nothing here; markdown/text may use it
            let s = WIDE_GAP.replace_all(py_strip(&ln), " | ").into_owned();
            ln = SPACE_RUN.replace_all(&s, " ").into_owned();
            if RULE.is_match(&ln) {
                ln.clear();
            }
        } else if plain_rule(py_strip(&ln)) {
            ln.clear();
        }
        if py_strip(&ln).is_empty() {
            if !blank {
                out.push(String::new());
            }
            blank = true;
            continue;
        }
        if out.last() == Some(&ln) {
            continue; // an exact repeat of the line above
        }
        out.push(ln);
        blank = false;
    }
    py_strip(&out.join("\n")).to_string()
}

/// (text before squeezing, squeezed text) of one document.
pub fn convert(path: &str) -> Result<(String, String), String> {
    let ext = util::ext_lower(path);
    if ext == ".pdf" {
        let out = Command::new("pdftotext").args(["-layout", path, "-"]).output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "pdftotext not found (install poppler-utils)".to_string()
            } else {
                format!("pdftotext failed: {e}")
            }
        })?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let err = py_strip(&err);
            let code = out.status.code().map_or("killed".to_string(), |c| c.to_string());
            return Err(format!("pdftotext failed: {}", if err.is_empty() { code.as_str() } else { err }));
        }
        let raw = util::universal_newlines(&String::from_utf8_lossy(&out.stdout));
        let text = squeeze(&raw, Kind::Pdf);
        return Ok((raw, text));
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let raw = util::universal_newlines(&String::from_utf8_lossy(&bytes));
    if ext == ".html" || ext == ".htm" {
        let raw = html_text(&raw);
        let text = squeeze(&raw, Kind::Html);
        return Ok((raw, text));
    }
    let text = squeeze(&raw, Kind::Plain);
    Ok((raw, text))
}

/// File name of a document's copy: `rust/memory` → `rust - memory.txt`.
pub fn store_name(d: &str) -> String {
    format!("{}.txt", d.replace('/', " - "))
}

/// FNV-1a (32 bit) of the id's UTF-8 bytes; the same number in the Python version.
fn id_hash(d: &str) -> u32 {
    d.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193))
}

/// `store_name`, unless another document already uses that file (`a/b` and `a - b`): then the
/// id's hash goes into the name, so two documents never share one copy.
fn free_store_name(d: &str, index: &Map<String, Value>) -> String {
    let file = store_name(d);
    let taken = index.iter().any(|(id, r)| id != d && r.get("file").and_then(Value::as_str) == Some(file.as_str()));
    if taken { format!("{} ({:08x}).txt", d.replace('/', " - "), id_hash(d)) } else { file }
}

/// Lazy {doc id: plain text}, kept as a persistent copy in the researcher's own folder.
///
/// A document is converted once and again only when its source file changed (size, mtime) or
/// TEXT_VERSION went up. The library itself is only ever read. Copies of documents that left
/// the library stay. The index is written when the value is dropped (or on `flush`).
pub struct Texts {
    paths: BTreeMap<String, String>,
    store: String,
    cache: HashMap<String, String>,
    raw: HashMap<String, usize>,
    pub errors: HashMap<String, String>,
    index: Map<String, Value>,
    dirty: bool,
}

impl Texts {
    pub fn new(paths: &BTreeMap<String, String>, store: &str) -> Self {
        let index = fs::read_to_string(util::join(store, ".index.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| match v {
                Value::Object(m) => Some(m),
                _ => None,
            })
            .unwrap_or_default();
        Texts {
            paths: paths.clone(),
            store: store.to_string(),
            cache: HashMap::new(),
            raw: HashMap::new(),
            errors: HashMap::new(),
            index,
            dirty: false,
        }
    }

    /// Text of a fresh stored copy, or None.
    fn stored(&self, d: &str) -> Option<String> {
        let rec = self.index.get(d)?.as_object()?;
        let src = &self.paths[d];
        let st = fs::metadata(src).ok()?;
        let fresh = rec.get("v").and_then(Value::as_i64) == Some(TEXT_VERSION)
            && rec.get("src").and_then(Value::as_str) == Some(src.as_str())
            && rec.get("size").and_then(Value::as_u64) == Some(st.len())
            && rec.get("mtime_ns").and_then(Value::as_i64) == Some(util::mtime_ns(&st));
        if !fresh {
            return None;
        }
        let file = rec.get("file")?.as_str()?;
        let body = fs::read_to_string(util::join(&self.store, file)).ok()?;
        let (head, text) = body.split_once("\n\n")?;
        if head != format!("SOURCE: {src}") {
            return None; // the file holds another document's copy
        }
        Some(text.strip_suffix('\n').unwrap_or(text).to_string())
    }

    /// `st` is the source as it was *before* converting: if it changes meanwhile, the next run
    /// sees a different size or mtime and converts it again.
    fn save(&mut self, d: &str, text: &str, raw: usize, st: &fs::Metadata) -> std::io::Result<()> {
        fs::create_dir_all(&self.store)?;
        let src = self.paths[d].clone();
        let file = free_store_name(d, &self.index);
        util::write_atomic(&util::join(&self.store, &file), format!("SOURCE: {src}\n\n{text}\n").as_bytes())?;
        self.index.insert(
            d.to_string(),
            json!({"file": file, "src": src, "size": st.len(), "mtime_ns": util::mtime_ns(st),
                   "raw": raw, "v": TEXT_VERSION}),
        );
        self.dirty = true;
        Ok(())
    }

    /// True when `d` is in the library and readable; converts it on first use.
    pub fn has(&mut self, d: &str) -> bool {
        if self.cache.contains_key(d) {
            return true;
        }
        if !self.paths.contains_key(d) || self.errors.contains_key(d) {
            return false;
        }
        let text = match self.stored(d) {
            Some(t) => t,
            None => {
                let src = self.paths[d].clone();
                let res = fs::metadata(&src).map_err(|e| e.to_string()).and_then(|st| {
                    let (raw, text) = convert(&src)?;
                    self.save(d, &text, chars(&raw), &st).map_err(|e| e.to_string())?;
                    Ok(text)
                });
                match res {
                    Ok(t) => t,
                    Err(e) => {
                        self.errors.insert(d.to_string(), e);
                        return false;
                    }
                }
            }
        };
        let raw = self.index.get(d).and_then(|r| r.get("raw")).and_then(Value::as_u64);
        self.raw.insert(d.to_string(), raw.map_or_else(|| chars(&text), |n| n as usize));
        self.cache.insert(d.to_string(), text);
        true
    }

    /// The text of a readable document ("" otherwise).
    pub fn get(&mut self, d: &str) -> &str {
        if self.has(d) { &self.cache[d] } else { "" }
    }

    /// Characters of the text of a readable document.
    pub fn len(&mut self, d: &str) -> usize {
        chars(self.get(d))
    }

    /// Characters of the converted text before squeezing.
    pub fn raw_chars(&mut self, d: &str) -> usize {
        if !self.has(d) {
            return 0;
        }
        self.raw.get(d).copied().unwrap_or_else(|| chars(&self.cache[d]))
    }

    /// The stored .txt of a readable document.
    pub fn path(&self, d: &str) -> String {
        let file = self.index.get(d).and_then(|r| r.get("file")).and_then(Value::as_str).unwrap_or_default();
        util::join(&self.store, file)
    }

    pub fn error(&self, d: &str) -> &str {
        self.errors.get(d).map_or("", String::as_str)
    }

    pub fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        let body = util::json_indent1(&util::sorted(&Value::Object(self.index.clone())));
        if util::write_atomic(&util::join(&self.store, ".index.json"), body.as_bytes()).is_ok() {
            self.dirty = false; // on failure the copies stay valid; the next run checks them again
        }
    }
}

impl Drop for Texts {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_layout_padding_and_blank_runs() {
        let raw = "Title\n\n\n\n      Indented  line  here\nName          Value          Unit\n";
        assert_eq!(squeeze(raw, Kind::Pdf), "Title\n\nIndented line here\nName | Value | Unit");
    }

    #[test]
    fn pdf_page_furniture_removed_content_kept() {
        let pages: Vec<String> =
            (1..6).map(|i| format!("Report 2026 - page {i}\nreal text {i}\nSecret Report\n")).collect();
        let out = squeeze(&pages.join("\x0c"), Kind::Pdf);
        assert!(!out.contains("Report 2026"));
        assert!(!out.contains("Secret Report"));
        for i in 1..6 {
            assert!(out.contains(&format!("real text {i}")));
        }
    }

    #[test]
    fn pdf_short_document_keeps_its_header() {
        assert!(squeeze("Report\nbody\x0c\nReport\nmore\n", Kind::Pdf).contains("Report"));
    }

    #[test]
    fn rules_and_invisible_characters_go() {
        let out = squeeze("a\u{200b}b\u{a0}c\n\u{2500}\u{2500}\u{2500}\u{2500}\n----------\nd", Kind::Html);
        assert_eq!(out, "ab c\n\nd");
    }

    #[test]
    fn markdown_keeps_indentation_and_short_rules() {
        let raw = "- a\n    - nested\n\n```\n  code\n```\n---\n";
        assert_eq!(squeeze(raw, Kind::Plain), raw.trim());
        assert_eq!(squeeze("x\n==========\ny", Kind::Plain), "x\n\ny");
    }

    #[test]
    fn exact_repeat_lines_collapse() {
        assert_eq!(squeeze("same\nsame\nother", Kind::Plain), "same\nother");
    }

    #[test]
    fn two_ids_with_one_store_name_get_two_copies() {
        let base = std::env::temp_dir().join(format!("researcher-unit-{}-names", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("lib/a")).unwrap();
        let p = |s: &str| base.join(s).to_string_lossy().into_owned();
        fs::write(p("lib/a/b.md"), "one").unwrap();
        fs::write(p("lib/a - b.md"), "two").unwrap();
        let paths: BTreeMap<String, String> =
            [("a/b".to_string(), p("lib/a/b.md")), ("a - b".to_string(), p("lib/a - b.md"))].into();
        for _ in 0..2 {
            let mut t = Texts::new(&paths, &p("store"));
            assert_eq!(t.get("a/b"), "one");
            assert_eq!(t.get("a - b"), "two");
            assert_ne!(t.path("a/b"), t.path("a - b"));
            assert!(t.path("a - b").ends_with(&format!("a - b ({:08x}).txt", id_hash("a - b"))));
        }
        assert_eq!(id_hash("a - b"), 0xf055_1209);
        assert_eq!(id_hash("é/x"), 0x1da2_3496); // pinned: the Python version must agree
    }

    #[test]
    fn html_to_text() {
        let t = html_text("<h1>T</h1><ul><li>a</li><li>b &amp; c</li></ul><style>p{}</style>");
        assert_eq!(t, "T\n- a\n- b & c");
    }

    #[test]
    fn script_style_svg_removed_even_unclosed_first() {
        assert_eq!(html_text("<p>a</p><SCRIPT>x()</script><p>b</p>"), "a\nb");
        assert_eq!(html_text("<svg>open<p>keep</p><style>s</style>"), "open keep");
    }
}
