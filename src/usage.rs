// SPDX-License-Identifier: MIT
//! The local, anonymous usage log: one line per load in `<data dir>/researcher/usage.jsonl`.
//!
//! Counts and character sums only: never a title, path, set name or library location. Nothing is
//! ever sent anywhere; `--usage` prints totals and `--usage-export` writes a file to share by hand.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;

use serde_json::{Value, json};

use crate::text::Texts;
use crate::util;

pub const USAGE_SCHEMA: i64 = 1;
const USAGE_KINDS: [&str; 4] = ["pdf", "html", "md", "txt"];
/// ~2.4 characters per token, measured with count_tokens (see README).
pub const CHARS_PER_K: usize = 2400;

pub fn data_home() -> String {
    let base = dirs::data_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| util::expanduser("~/.local/share"));
    util::join(&base, crate::APP)
}

pub fn usage_file() -> String {
    util::join(&data_home(), "usage.jsonl")
}

/// False only with `[usage] log = false` in the config.
pub fn enabled(cfg: &toml::Table) -> bool {
    !matches!(cfg.get("usage"), Some(toml::Value::Table(t)) if t.get("log") == Some(&toml::Value::Boolean(false)))
}

/// A random id for this install, made on first use; delete the file to start a new one.
fn install_id() -> std::io::Result<String> {
    let path = util::join(&data_home(), "usage-id");
    if let Ok(s) = fs::read_to_string(&path) {
        let v = util::py_strip(&s);
        if v.len() == 16 && v.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Ok(v.to_string());
        }
    }
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
    let val: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    fs::create_dir_all(data_home())?;
    fs::write(&path, format!("{val}\n"))?;
    Ok(val)
}

fn format_of(path: &str) -> &'static str {
    let ext = util::ext_lower(path);
    let ext = ext.trim_start_matches('.');
    if ext == "htm" {
        return "html";
    }
    USAGE_KINDS.iter().find(|k| **k == ext).copied().unwrap_or("txt")
}

/// The numbers of one load. Characters, not tokens: the reader applies its own ratio.
pub fn record(docs: &[String], texts: &mut Texts, paths: &BTreeMap<String, String>) -> std::io::Result<Value> {
    let mut fmt: BTreeMap<&str, usize> = USAGE_KINDS.iter().map(|k| (*k, 0)).collect();
    for d in docs {
        *fmt.entry(format_of(&paths[d])).or_default() += 1;
    }
    let lib: Vec<&String> = paths.keys().filter(|d| texts.has(d)).collect();
    let lib_chars: usize = lib.iter().map(|d| texts.len(d)).sum();
    let mut src_bytes = 0u64;
    for d in docs {
        src_bytes += fs::metadata(&paths[d])?.len();
    }
    Ok(json!({
        "v": USAGE_SCHEMA,
        "day": chrono::Local::now().format("%Y-%m-%d").to_string(),
        "id": install_id()?,
        "docs": docs.len(),
        "fmt": fmt,
        "src_bytes": src_bytes,
        "raw": docs.iter().map(|d| texts.raw_chars(d)).sum::<usize>(),
        "chars": docs.iter().map(|d| texts.len(d)).sum::<usize>(),
        "lib_docs": lib.len(),
        "lib_chars": lib_chars,
        "tk": CHARS_PER_K,
    }))
}

/// Append one record. A failure here must never break a load.
pub fn log(rec: &Value) {
    let _ = (|| -> std::io::Result<()> {
        fs::create_dir_all(data_home())?;
        let mut f = fs::OpenOptions::new().create(true).append(true).open(usage_file())?;
        writeln!(f, "{}", serde_json::to_string(&util::sorted(rec)).unwrap_or_default())
    })();
}

/// Records from the log; lines that are not this schema are skipped.
pub fn read() -> Vec<Value> {
    let Ok(text) = fs::read_to_string(usage_file()) else { return Vec::new() };
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|r| r.is_object() && r.get("v").and_then(Value::as_i64) == Some(USAGE_SCHEMA))
        .collect()
}

fn num(r: &Value, k: &str) -> f64 {
    r.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

#[derive(Debug, Default)]
pub struct Summary {
    pub loads: usize,
    pub docs: f64,
    pub raw: f64,
    pub chars: f64,
    pub squeezed: f64,
    pub unticked: f64,
    pub tokens: f64,
    pub tokens_squeezed: f64,
    pub tokens_unticked: f64,
}

/// Totals over records. Characters are summed; tokens use each record's own ratio.
pub fn summary(recs: &[Value]) -> Summary {
    let tok = |chars: f64, r: &Value| {
        let tk = num(r, "tk");
        chars / if tk != 0.0 { tk } else { CHARS_PER_K as f64 } * 1000.0
    };
    let mut t = Summary { loads: recs.len(), ..Summary::default() };
    for r in recs {
        let (raw, chars, lib) = (num(r, "raw"), num(r, "chars"), num(r, "lib_chars"));
        t.docs += num(r, "docs");
        t.raw += raw;
        t.chars += chars;
        t.unticked += (lib - chars).max(0.0);
        t.tokens += tok(chars, r);
        t.tokens_squeezed += tok(raw - chars, r);
        t.tokens_unticked += tok((lib - chars).max(0.0), r);
    }
    t.squeezed = t.raw - t.chars;
    t
}

pub fn report(recs: &[Value]) -> String {
    if recs.is_empty() {
        return "no usage recorded yet (a log line is written after every load)".into();
    }
    let t = summary(recs);
    let n = t.loads as f64;
    let mut days: Vec<&str> = recs.iter().map(|r| r.get("day").and_then(Value::as_str).unwrap_or("")).collect();
    days.sort_unstable();
    days.dedup();
    let pct = if t.raw != 0.0 { (100.0 * t.squeezed / t.raw).floor() } else { 0.0 };
    [
        format!("usage log: {}", usage_file()),
        format!(
            "{} loads on {} days ({} to {}), {} documents loaded",
            t.loads,
            days.len(),
            days[0],
            days[days.len() - 1],
            t.docs as i64
        ),
        format!(
            "loaded:        {:>10} chars  (~{:.0}k tokens, {:.1}k per load)",
            t.chars as i64,
            t.tokens / 1000.0,
            t.tokens / n / 1000.0
        ),
        format!(
            "squeezed away: {:>10} chars  (~{:.0}k tokens, {}% of the converted text)",
            t.squeezed as i64,
            t.tokens_squeezed / 1000.0,
            pct as i64
        ),
        format!(
            "left unticked: {:>10} chars  (~{:.0}k tokens, against loading the whole library every time: a ceiling, not a measurement)",
            t.unticked as i64,
            t.tokens_unticked / 1000.0
        ),
        "tokens are estimates from characters, not counted by a tokenizer".into(),
    ]
    .join("\n")
}

/// Write the records as one JSON file to look at and send by hand. Returns how many.
pub fn export(path: &str) -> std::io::Result<usize> {
    let recs = read();
    let body = util::json_indent1(&util::sorted(&json!({"schema": USAGE_SCHEMA, "loads": recs})));
    fs::write(path, format!("{body}\n"))?;
    Ok(recs.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_math() {
        let recs = vec![
            json!({"v": 1, "docs": 2, "raw": 4600, "chars": 2300, "lib_chars": 23000, "tk": 2300}),
            json!({"v": 1, "docs": 1, "raw": 1000, "chars": 1000, "lib_chars": 1000, "tk": 2300}),
        ];
        let t = summary(&recs);
        assert_eq!(t.squeezed, 2300.0);
        assert_eq!(t.unticked, 20700.0);
        assert!((t.tokens_squeezed - 1000.0).abs() < 1e-9);
        assert!((t.tokens_unticked - 9000.0).abs() < 1e-9);
    }

    #[test]
    fn formats() {
        assert_eq!(format_of("/a/x.HTM"), "html");
        assert_eq!(format_of("/a/x.pdf"), "pdf");
        assert_eq!(format_of("/a/x.weird"), "txt");
    }
}
