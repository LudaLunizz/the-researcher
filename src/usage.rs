// SPDX-License-Identifier: MIT
//! The local, anonymous usage log: one line per load in `<data dir>/researcher/usage.jsonl`.
//!
//! Counts and character sums only: never a title, path, set name or library location.
//! `--usage` prints totals and `--usage-export` writes a file to look at. Records are only sent
//! when the user said yes (`[usage] share = true`, set with `--usage-share on`): `--usage-send`
//! sends the ones not sent yet, and after a load that happens in the background.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::time::Duration;

use serde_json::{Value, json};

use crate::text::Texts;
use crate::util;

pub const USAGE_SCHEMA: i64 = 1;
/// The request shape `--usage-send` posts: the records plus `seq`, their line number in the log.
pub const SEND_SCHEMA: i64 = 2;
/// The project's receiver (worker/ in the repo); `[usage] url` points elsewhere.
pub const DEFAULT_URL: &str = "https://the-researcher-usage.ludalunizz.workers.dev/v1/usage";
/// Records per request (the receiver takes up to 200).
const SEND_BATCH: usize = 200;
const SEND_TIMEOUT: Duration = Duration::from_secs(15);
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

fn sent_file() -> String {
    util::join(&data_home(), "usage-sent")
}

fn usage_table(cfg: &toml::Table) -> Option<&toml::Table> {
    cfg.get("usage").and_then(toml::Value::as_table)
}

/// False only with `[usage] log = false` in the config.
pub fn enabled(cfg: &toml::Table) -> bool {
    usage_table(cfg).and_then(|t| t.get("log")) != Some(&toml::Value::Boolean(false))
}

/// The user's answer to "share the anonymous usage counts?": `[usage] share = true/false`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Share {
    On,
    Off,
    Undecided,
}

impl Share {
    pub fn label(self) -> &'static str {
        match self {
            Share::On => "on",
            Share::Off => "off",
            Share::Undecided => "not decided",
        }
    }
}

pub fn share(cfg: &toml::Table) -> Share {
    match usage_table(cfg).and_then(|t| t.get("share")) {
        Some(toml::Value::Boolean(true)) => Share::On,
        Some(toml::Value::Boolean(false)) => Share::Off,
        _ => Share::Undecided,
    }
}

/// Where records are sent: `[usage] url`, or the project's receiver.
pub fn url(cfg: &toml::Table) -> String {
    usage_table(cfg)
        .and_then(|t| t.get("url"))
        .and_then(toml::Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_URL)
        .to_string()
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

/// The records not sent yet, oldest first, each with `seq` = its line number in the log. The
/// line number is what tells two identical loads on one day apart, and it stays the same when a
/// record is sent again, so a resend is stored once.
pub fn pending() -> Vec<Value> {
    let Ok(text) = fs::read_to_string(usage_file()) else { return Vec::new() };
    let lines: Vec<&str> = text.lines().collect();
    let mut done =
        fs::read_to_string(sent_file()).ok().and_then(|s| util::py_strip(&s).parse::<usize>().ok()).unwrap_or(0);
    if done > lines.len() {
        done = 0; // the log was deleted or cut: start again
    }
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate().skip(done) {
        let Ok(mut rec) = serde_json::from_str::<Value>(line) else { continue };
        if rec.is_object() && rec.get("v").and_then(Value::as_i64) == Some(USAGE_SCHEMA) {
            rec["seq"] = json!(i + 1);
            out.push(util::sorted(&rec));
        }
    }
    out
}

/// The body `--usage-send` posts for these records.
pub fn payload(recs: &[Value]) -> Value {
    util::sorted(&json!({"schema": SEND_SCHEMA, "loads": recs}))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Sent {
    /// records the receiver took (new or already there)
    pub sent: usize,
    /// of those, rows it did not have yet
    pub new: u64,
    /// records it refused (they are skipped, so one bad line can never block the rest)
    pub refused: usize,
}

/// One POST: (HTTP status, body), or an error when there was no answer.
fn post(url: &str, body: &str) -> Result<(u16, String), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(SEND_TIMEOUT))
        .http_status_as_error(false)
        .user_agent(format!("the-researcher/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut resp = agent.post(url).header("content-type", "application/json").send(body).map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    Ok((status, text))
}

/// Send the records in batches. After each batch the sent marker moves on, so a failure later
/// on never sends the earlier batches twice (and if it did, the receiver stores them once).
pub fn send(url: &str, recs: &[Value]) -> Result<Sent, (Sent, String)> {
    let mut done = Sent::default();
    for batch in recs.chunks(SEND_BATCH) {
        let mut left: Vec<&Value> = batch.iter().collect();
        while !left.is_empty() {
            let body = serde_json::to_string(&payload(&left.iter().map(|v| (*v).clone()).collect::<Vec<_>>()))
                .unwrap_or_default();
            let (status, text) = post(url, &body).map_err(|e| (std::mem::take(&mut done), e))?;
            let reply: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if status == 200 && reply.get("ok") == Some(&Value::Bool(true)) {
                done.sent += left.len();
                done.new += reply.get("new").and_then(Value::as_u64).unwrap_or(0);
                break;
            }
            let err = reply.get("error").and_then(Value::as_str).unwrap_or("").to_string();
            // "record N: ..." names the one record the receiver refused: skip it, send the rest
            let bad = err
                .strip_prefix("record ")
                .and_then(|r| r.split(':').next())
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|n| status == 400 && *n < left.len());
            match bad {
                Some(n) => {
                    left.remove(n);
                    done.refused += 1;
                }
                None => {
                    let why = if err.is_empty() { util::py_strip(&text).chars().take(200).collect() } else { err };
                    return Err((done, format!("HTTP {status}: {why}")));
                }
            }
        }
        if let Some(last) = batch.last().and_then(|r| r.get("seq")).and_then(Value::as_u64) {
            let _ = fs::create_dir_all(data_home())
                .and_then(|_| util::write_atomic(&sent_file(), format!("{last}\n").as_bytes()));
        }
    }
    Ok(done)
}

/// Start `researcher --usage-send` on its own and return at once: a load never waits on the
/// network, and the sender outlives the load (its own process group on Unix).
pub fn send_in_background(cfg_path: &str) {
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["--usage-send", "--config", cfg_path])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let _ = cmd.spawn();
}

/// The last line of `--usage`: is anything sent, and how much is waiting.
pub fn share_line(share: Share, waiting: usize) -> String {
    match share {
        Share::On => {
            format!("sharing: on, {waiting} records waiting to be sent (researcher --usage-send sends them now)")
        }
        Share::Off => "sharing: off, nothing is sent (researcher --usage-share on|off)".into(),
        Share::Undecided => "sharing: not decided, nothing is sent (researcher --usage-share on|off)".into(),
    }
}

/// Write the records as one JSON file to look at. Returns how many.
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
