// SPDX-License-Identifier: MIT
//! Closed Claude Code sessions (opt-in): a registry keyed by the FULL session id, so a name can
//! never be separated from the id it belongs to. Names come from Claude Code's own title or the
//! first prompt, and can be renamed (a manual name is never overwritten by a rescan).

use std::fs;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::library::{Config, ConfigError};
use crate::usage::CHARS_PER_K;
use crate::util::{self, chars, py_strip};

static SESSION_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$").expect("valid"));
static WS_RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\s\x1c-\x1f]+").expect("valid"));
const NAME_MAX: usize = 80;
/// Characters; a longer session keeps its start and its end.
const SESSION_LOAD_MAX: usize = 200_000;

pub struct Settings {
    idle_minutes: f64,
    projects_dir: String,
}

/// The `[sessions]` settings, or None when sessions are off.
pub fn settings(cfg: &Config) -> Option<Settings> {
    let Some(toml::Value::Table(sec)) = cfg.get("sessions") else { return None };
    if !util::truthy_toml(sec.get("enabled")) {
        return None;
    }
    let idle_minutes = match sec.get("idle_minutes") {
        Some(toml::Value::Integer(i)) => *i as f64,
        Some(toml::Value::Float(f)) => *f,
        Some(toml::Value::String(s)) => s.trim().parse().unwrap_or(10.0),
        _ => 10.0,
    };
    let dir = match sec.get("projects_dir") {
        Some(toml::Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => "~/.claude/projects".into(),
    };
    Some(Settings { idle_minutes, projects_dir: util::user_path(&dir) })
}

fn clean_name(text: &str) -> String {
    let t = WS_RUN.replace_all(text, " ");
    let t = py_strip(&t);
    if chars(t) <= NAME_MAX {
        t.to_string()
    } else {
        format!("{}…", util::py_rstrip(util::take_chars(t, NAME_MAX - 1)))
    }
}

/// The plain text of a message: a string, or the text blocks of a list (tools skipped).
fn block_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .map(|b| b.get("text").and_then(Value::as_str).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Entries of a session .jsonl; unreadable or broken lines are skipped.
fn read_transcript(path: &str) -> Vec<Map<String, Value>> {
    let Ok(bytes) = fs::read(path) else { return Vec::new() };
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|l| match serde_json::from_str::<Value>(l) {
            Ok(Value::Object(m)) => Some(m),
            _ => None,
        })
        .collect()
}

fn s<'a>(e: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    e.get(k).and_then(Value::as_str)
}

fn message_text(e: &Map<String, Value>) -> String {
    block_text(e.get("message").and_then(Value::as_object).and_then(|m| m.get("content")))
}

/// The user's own prompt of an entry, or "" (tool results, meta and system text, commands).
fn real_prompt(e: &Map<String, Value>) -> String {
    if s(e, "type") != Some("user") || util::truthy_json(e.get("isMeta")) {
        return String::new();
    }
    let text = message_text(e);
    let text = py_strip(&text);
    if text.is_empty() || text.starts_with('<') { String::new() } else { text.to_string() }
}

#[derive(Default)]
struct Info {
    title: Option<String>,
    first: Option<String>,
    cwd: Option<String>,
    started: Option<String>,
    last: Option<String>,
}

fn session_info(entries: &[Map<String, Value>]) -> Info {
    let mut i = Info::default();
    for e in entries {
        match s(e, "type") {
            Some("custom-title") if s(e, "customTitle").is_some_and(|t| !t.is_empty()) => {
                i.title = s(e, "customTitle").map(str::to_string);
            }
            Some("ai-title") if s(e, "aiTitle").is_some_and(|t| !t.is_empty()) && i.title.is_none() => {
                i.title = s(e, "aiTitle").map(str::to_string);
            }
            _ => {}
        }
        if i.first.is_none() {
            i.first = Some(real_prompt(e)).filter(|p| !p.is_empty());
        }
        if i.cwd.as_deref().is_none_or(str::is_empty) {
            i.cwd = s(e, "cwd").map(str::to_string);
        }
        if let Some(ts) = s(e, "timestamp") {
            if i.started.is_none() {
                i.started = Some(ts.to_string());
            }
            i.last = Some(ts.to_string());
        }
    }
    i
}

fn load_registry(path: &str) -> Map<String, Value> {
    let reg = fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
    match reg {
        Some(Value::Object(m)) if m.get("sessions").is_some_and(Value::is_object) => m,
        _ => json!({"version": 1, "sessions": {}}).as_object().cloned().expect("object"),
    }
}

fn save_registry(path: &str, reg: &Map<String, Value>) -> std::io::Result<()> {
    util::make_parent(path)?;
    util::write_atomic(path, util::json_indent1(&Value::Object(reg.clone())).as_bytes())
}

fn sessions_mut(reg: &mut Map<String, Value>) -> &mut Map<String, Value> {
    reg.get_mut("sessions").and_then(Value::as_object_mut).expect("checked by load_registry")
}

fn sessions(reg: &Map<String, Value>) -> &Map<String, Value> {
    reg.get("sessions").and_then(Value::as_object).expect("checked by load_registry")
}

fn opt(v: &Option<String>) -> Value {
    v.as_ref().map_or(Value::Null, |s| Value::String(s.clone()))
}

/// Add closed sessions to the registry and refresh the changed ones. Returns the number added.
fn scan_sessions(reg: &mut Map<String, Value>, st: &Settings) -> usize {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
    let Ok(rd) = fs::read_dir(&st.projects_dir) else { return 0 };
    let mut projects: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    projects.sort();
    let mut added = 0;
    for proj in projects {
        let pdir = util::join(&st.projects_dir, &proj);
        let Ok(rd) = fs::read_dir(&pdir) else { continue };
        let mut files: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        files.sort();
        for f in files {
            let Some(sid) = f.strip_suffix(".jsonl") else { continue };
            if !SESSION_ID_RE.is_match(sid) {
                continue;
            }
            let path = util::join(&pdir, &f);
            let Ok(meta) = fs::metadata(&path) else { continue };
            let mtime = util::mtime_secs(&meta);
            if now - mtime < st.idle_minutes * 60.0 {
                continue; // probably still open
            }
            let old = sessions(reg).get(sid).and_then(Value::as_object);
            if old.is_some_and(|r| {
                r.get("mtime").and_then(Value::as_f64) == Some(mtime) && s(r, "file") == Some(path.as_str())
            }) {
                continue;
            }
            let info = session_info(&read_transcript(&path));
            let Some(base) = info.title.as_ref().or(info.first.as_ref()) else {
                continue; // nothing was ever asked
            };
            let auto = clean_name(base);
            let map = sessions_mut(reg);
            let rec = match map.get_mut(sid).and_then(Value::as_object_mut) {
                Some(rec) => {
                    if s(rec, "name_source") != Some("manual") {
                        rec.insert("name".into(), Value::String(auto));
                    }
                    rec
                }
                None => {
                    added += 1;
                    let registered = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
                    map.insert(sid.to_string(), json!({"name": auto, "name_source": "auto", "registered": registered}));
                    map.get_mut(sid).and_then(Value::as_object_mut).expect("just inserted")
                }
            };
            rec.insert("file".into(), Value::String(path.clone()));
            rec.insert("mtime".into(), json!(mtime));
            rec.insert("project".into(), Value::String(proj.clone()));
            rec.insert("cwd".into(), opt(&info.cwd));
            rec.insert("started".into(), opt(&info.started));
            rec.insert("last".into(), opt(&info.last));
        }
    }
    added
}

/// A registry id from a full id or a unique prefix (4+ characters).
fn find_session(reg: &Map<String, Value>, ident: &str) -> Result<String, ConfigError> {
    let ident = py_strip(ident).to_lowercase();
    let map = sessions(reg);
    if map.contains_key(&ident) {
        return Ok(ident);
    }
    let hits: Vec<&String> = map.keys().filter(|i| chars(&ident) >= 4 && i.starts_with(&ident)).collect();
    match hits.len() {
        1 => Ok(hits[0].clone()),
        0 => Err(ConfigError(format!("no session matches '{ident}'"))),
        n => Err(ConfigError(format!("'{ident}' matches {n} sessions; give more of the id"))),
    }
}

fn rec_name(rec: &Value) -> &str {
    rec.get("name").and_then(Value::as_str).unwrap_or("")
}

/// Name and full id, always together.
fn label(sid: &str, rec: &Value) -> String {
    format!("\"{}\" ({sid})", rec_name(rec))
}

fn when(rec: &Value) -> String {
    let t = ["last", "started"]
        .iter()
        .filter_map(|k| rec.get(*k).and_then(Value::as_str))
        .find(|t| !t.is_empty())
        .unwrap_or("");
    util::take_chars(t, 16).replace('T', " ")
}

fn session_text(sid: &str, rec: &Value) -> String {
    let get = |k: &str| rec.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let folder = Some(get("cwd")).filter(|c| !c.is_empty()).unwrap_or_else(|| get("project"));
    let mut parts = vec![
        format!("SESSION: {}", rec_name(rec)),
        format!("SESSION ID: {sid}"),
        format!("PROJECT FOLDER: {folder}"),
        format!("LAST ACTIVE: {}", util::take_chars(&get("last"), 16).replace('T', " ")),
        String::new(),
    ];
    for e in read_transcript(&get("file")) {
        let (who, text) = match s(&e, "type") {
            Some("user") => ("USER", real_prompt(&e)),
            Some("assistant") => ("ASSISTANT", py_strip(&message_text(&e)).to_string()),
            _ => continue,
        };
        if !text.is_empty() {
            parts.push(format!("{who}: {text}\n"));
        }
    }
    let body = parts.join("\n");
    let n = chars(&body);
    if n <= SESSION_LOAD_MAX {
        return body;
    }
    let keep = SESSION_LOAD_MAX / 5;
    format!(
        "{}\n\n[... {} characters left out of the middle ...]\n\n{}",
        util::take_chars(&body, keep),
        n - SESSION_LOAD_MAX,
        util::last_chars(&body, SESSION_LOAD_MAX - keep)
    )
}

/// Write one .txt per (name, text); only files this tool wrote earlier are removed first.
fn write_outputs(out_dir: &str, docs: &[(String, String, String)]) -> std::io::Result<Vec<String>> {
    fs::create_dir_all(out_dir)?;
    let manifest = util::join(out_dir, ".manifest.json");
    let old: Vec<String> = fs::read_to_string(&manifest)
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok())
        .unwrap_or_default();
    for f in old {
        if !f.contains(['/', '\\']) && !f.is_empty() && f != "." && f != ".." {
            let _ = fs::remove_file(util::join(out_dir, &f)); // never follow a path out of the folder
        }
    }
    let mut written = Vec::new();
    for (d, src, text) in docs {
        let f = crate::text::store_name(d);
        fs::write(util::join(out_dir, &f), format!("SOURCE: {src}\n\n{text}\n"))?;
        written.push(f);
    }
    fs::write(&manifest, serde_json::to_string(&written).unwrap_or_default())?;
    Ok(written.iter().map(|f| util::join(out_dir, f)).collect())
}

pub enum Action {
    List { limit: usize },
    Rename { id: String, name: String },
    Resume { id: String },
    Load { id: String },
}

pub fn main(action: Action, cfg: &Config, reg_path: &str, out_dir: &str) -> i32 {
    let Some(st) = settings(cfg) else {
        println!(
            "SESSIONS OFF: add `[sessions]` with `enabled = true` to the config to track closed sessions; nothing done"
        );
        return 7;
    };
    let mut reg = load_registry(reg_path);
    let added = scan_sessions(&mut reg, &st);
    let result = (|| -> Result<Option<i32>, ConfigError> {
        let io = |e: std::io::Error| ConfigError(e.to_string());
        match &action {
            Action::Rename { id, name } => {
                let sid = find_session(&reg, id)?;
                let name = clean_name(name);
                if name.is_empty() {
                    return Err(ConfigError("the new name is empty".into()));
                }
                let rec = sessions_mut(&mut reg).get_mut(&sid).and_then(Value::as_object_mut).expect("found");
                rec.insert("name".into(), Value::String(name));
                rec.insert("name_source".into(), "manual".into());
                save_registry(reg_path, &reg).map_err(io)?;
                println!("RENAMED {}", label(&sid, &sessions(&reg)[&sid]));
                return Ok(Some(0));
            }
            Action::Resume { id } => {
                let _ = save_registry(reg_path, &reg);
                let sid = find_session(&reg, id)?;
                let rec = &sessions(&reg)[&sid];
                println!("SESSION {}", label(&sid, rec));
                let cmd = format!("claude --resume {sid}");
                match rec.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()) {
                    Some(cwd) => println!("cd {} && {cmd}", util::shell_quote(cwd)),
                    None => println!("{cmd}"),
                }
                return Ok(Some(0));
            }
            Action::Load { id } => {
                let _ = save_registry(reg_path, &reg);
                let sid = find_session(&reg, id)?;
                let rec = sessions(&reg)[&sid].clone();
                let file = rec.get("file").and_then(Value::as_str).unwrap_or("").to_string();
                if !fs::metadata(&file).is_ok_and(|m| m.is_file()) {
                    return Err(ConfigError(format!("the transcript of {} is gone", label(&sid, &rec))));
                }
                let text = session_text(&sid, &rec);
                let written = write_outputs(out_dir, &[(format!("sessions/{sid}"), file, text.clone())]).map_err(io)?;
                let n = chars(&text);
                println!("SESSION {}", label(&sid, &rec));
                println!("{n:>7} chars  {}", written[0]);
                println!("total: 1 session, {n} chars (~{}k tokens)", n / CHARS_PER_K);
                return Ok(Some(0));
            }
            Action::List { .. } => {
                let _ = save_registry(reg_path, &reg);
            }
        }
        Ok(None)
    })();
    match result {
        Ok(Some(code)) => return code,
        Ok(None) => {}
        Err(e) => {
            println!("SESSION ERROR: {e}");
            return 8;
        }
    }
    let Action::List { limit } = action else { return 0 };
    let mut items: Vec<(&String, &Value)> = sessions(&reg).iter().collect();
    let key = |rec: &Value| -> String {
        ["last", "started"]
            .iter()
            .filter_map(|k| rec.get(*k).and_then(Value::as_str))
            .find(|t| !t.is_empty())
            .unwrap_or("")
            .to_string()
    };
    items.sort_by_key(|(_, r)| std::cmp::Reverse(key(r)));
    println!("{} closed sessions tracked ({added} new); newest first", items.len());
    for (sid, rec) in items.iter().take(limit) {
        println!("{sid}  {:16}  {}", when(rec), rec_name(rec));
    }
    if items.len() > limit {
        println!("... {} more (--limit N)", items.len() - limit);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_cleaned_and_cut() {
        assert_eq!(clean_name("  a \n  b  "), "a b");
        let long = "x".repeat(100);
        let c = clean_name(&long);
        assert_eq!(chars(&c), NAME_MAX);
        assert!(c.ends_with('…'));
    }
}
