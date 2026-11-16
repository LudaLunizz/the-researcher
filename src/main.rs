// SPDX-License-Identifier: MIT
// The Researcher. Copyright (c) 2026 LudaLunizz
// Licensed under the MIT License; see the file LICENSE for details.
//! The Researcher: a research loader for Claude (saved research into a Claude Code session).
//!
//! A "library" is a folder of documents (.html, .md, .txt, .pdf). A "set" is a named list of
//! documents from it, defined in config.toml. A popup lets the user tick what to load, so no
//! tokens are spent on documents that are not needed. The ticked documents are read from a
//! plain-text copy in the researcher's own folder (made once per document, redone when the
//! source changes; PDF and HTML are squeezed for tokens) and their paths are printed, so the
//! skill can Read them in full. Nothing in the library is ever changed or moved.
//!
//! Same command line, files and exit codes as `skills/researcher/researcher.py`.

mod library;
mod popup;
mod sessions;
mod text;
mod usage;
mod util;

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::process::{Command, ExitCode};

use clap::Parser;
use serde_json::{Value, json};

use library::{ConfigError, SetKind};
use popup::{FirstUse, Outcome, POPUP_TIMEOUT};
use text::Texts;
use usage::CHARS_PER_K;

pub const APP: &str = "researcher";
/// Up to this many left-out documents are named in the output; more are only counted.
const SKIPPED_NAMED: usize = 20;

#[derive(Parser)]
#[command(name = "researcher", version, about = "Load saved research documents into Claude Code.")]
struct Args {
    #[arg(value_name = "SET")]
    sets: Vec<String>,
    /// change saved defaults, load nothing
    #[arg(long)]
    setup: bool,
    /// use the saved defaults, ask nothing (same as RESEARCHER_NO_POPUP=1)
    #[arg(long)]
    no_popup: bool,
    /// list sets and saved defaults
    #[arg(long)]
    list: bool,
    /// list tracked closed sessions (needs [sessions] enabled = true)
    #[arg(long)]
    sessions: bool,
    /// rename a tracked session
    #[arg(long, num_args = 2, value_names = ["ID", "NAME"], allow_hyphen_values = true)]
    rename: Option<Vec<String>>,
    /// print the command that resumes a session
    #[arg(long, value_name = "ID")]
    resume: Option<String>,
    /// write a tracked session's conversation as a .txt to load as context
    #[arg(long, value_name = "ID")]
    session_load: Option<String>,
    /// sessions shown by --sessions
    #[arg(long, default_value_t = 30)]
    limit: usize,
    /// write a starter config
    #[arg(long, value_name = "LIBRARY", num_args = 0..=1, default_missing_value = "")]
    init: Option<String>,
    /// config.toml path (default: <config dir>/researcher/config.toml)
    #[arg(long)]
    config: Option<String>,
    /// library folder (overrides config and RESEARCHER_LIBRARY)
    #[arg(long)]
    library: Option<String>,
    /// make or refresh the plain-text copy of the whole library, load nothing
    #[arg(long)]
    sync: bool,
    /// print the local usage totals (characters and estimated tokens saved)
    #[arg(long)]
    usage: bool,
    /// write the anonymous usage records to FILE to look at; nothing is sent
    #[arg(long, value_name = "FILE", num_args = 0..=1, default_missing_value = "researcher-usage.json")]
    usage_export: Option<String>,
    /// share the anonymous usage counts with the project (on) or not (off); saved in the config
    #[arg(long, value_name = "on|off", value_parser = ["on", "off"])]
    usage_share: Option<String>,
    /// send the usage records not sent yet (only when sharing is on)
    #[arg(long)]
    usage_send: bool,
    /// with --usage-send: print what would be sent, send nothing
    #[arg(long, requires = "usage_send")]
    dry_run: bool,
    /// the researcher's text folder (default <data dir>/researcher/text, or text_dir in the
    /// config); session loads go to <cache dir>/researcher
    #[arg(long)]
    out: Option<String>,
}

fn dir_or(d: Option<std::path::PathBuf>, fallback: &str) -> String {
    let base = d.map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| util::expanduser(fallback));
    util::join(&base, APP)
}

fn config_home() -> String {
    dir_or(dirs::config_dir(), "~/.config")
}

fn cache_home() -> String {
    dir_or(dirs::cache_dir(), "~/.cache")
}

/// The researcher's own folder: a plain-text copy of every library document.
fn text_home() -> String {
    util::join(&usage::data_home(), "text")
}

const STARTER: &str = r#"# The Researcher: config
# The folder that holds your research documents (.html, .md, .txt, .pdf).
library = "{library}"

# Where the plain-text copy of your library is kept (default ~/.local/share/researcher/text).
# Your own files are only read, never changed; Claude reads these copies instead.
# text_dir = "~/.local/share/researcher/text"

# Documents to leave out of the library entirely (id globs).
# exclude = ["scratch/*", "drafts/*"]

# Used when /researcher is run without a set name.
# default_set = "main"

# Short names for sets.
# [aliases]
# rs = "rust"

# A set is a list of document ids (path inside the library, no extension).
# Globs work: "rust/*". `include` pulls in other sets.
# [sets.rust]
# docs = ["rust/linux-notes", "rust/memory-*"]
#
# [sets.myapp]
# include = ["rust"]
# docs = ["myapp/roadmap"]

# A local, anonymous usage log (counts and character sums, never titles or paths) feeds
# `researcher --usage`. `share = true` (or `researcher --usage-share on`) also sends those
# counts to the project; nothing is sent unless you set it. Switch the log off with:
# [usage]
# log = false
# share = false
"#;

fn init_config(path: &str, library: &str) -> i32 {
    if std::path::Path::new(path).exists() {
        println!("{path} already exists; not overwriting");
        return 1;
    }
    let escaped = library.replace('\\', "\\\\").replace('"', "\\\"");
    let res = util::make_parent(path).and_then(|_| std::fs::write(path, STARTER.replace("{library}", &escaped)));
    match res {
        Ok(()) => {
            println!("wrote {path}");
            0
        }
        Err(e) => {
            println!("CONFIG ERROR: {path}: {e}");
            6
        }
    }
}

/// First use (no library in the config): ask for the research folder, save it, then start
/// again so the chooser opens (a process can open only one window loop).
fn first_use(cfg_path: &str) -> i32 {
    match popup::first_use(&util::expanduser("~/research"), cfg_path) {
        Err(e) => {
            println!(
                "NO LIBRARY: set `library` in {cfg_path} (or run --init, or set RESEARCHER_LIBRARY); \
                 no window to ask ({e}); nothing loaded"
            );
            6
        }
        Ok(FirstUse::Cancelled) => {
            println!("CANCELLED: no research folder chosen, nothing loaded");
            3
        }
        Ok(FirstUse::Timeout) => {
            println!("TIMEOUT: popup closed after {POPUP_TIMEOUT} s without a choice, nothing loaded");
            4
        }
        Ok(FirstUse::Folder(dir)) => {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                println!("NO LIBRARY: could not make {dir}: {e}; nothing loaded");
                return 6;
            }
            if let Err(e) = set_library(cfg_path, &dir) {
                println!("CONFIG ERROR: could not save `library` in {cfg_path}: {e}");
                return 6;
            }
            println!("LIBRARY SET: {dir} (saved as `library` in {cfg_path})");
            let _ = std::io::stdout().flush();
            let me = std::env::current_exe();
            let status = me.and_then(|exe| {
                Command::new(exe).args(std::env::args_os().skip(1)).env("RESEARCHER_FIRST_USE_DONE", "1").status()
            });
            match status {
                Ok(st) => st.code().unwrap_or(1),
                Err(e) => {
                    println!("could not start the chooser ({e}); run the same command again");
                    0
                }
            }
        }
    }
}

/// Put `library = "DIR"` into the config: a starter config when there is none, otherwise the
/// existing `library` line is replaced or a new one goes on top (comments are kept).
fn set_library(cfg_path: &str, dir: &str) -> Result<(), String> {
    let line = format!("library = \"{}\"", dir.replace('\\', "\\\\").replace('"', "\\\""));
    let body = match std::fs::read_to_string(cfg_path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => STARTER.replace("library = \"{library}\"", &line),
        Err(e) => return Err(e.to_string()),
        Ok(old) => {
            let re = regex::Regex::new(r"(?m)^[ \t]*library[ \t]*=.*$").expect("valid");
            let top_level = old.parse::<library::Config>().is_ok_and(|c| c.contains_key("library"));
            if top_level && re.is_match(&old) {
                re.replace(&old, regex::NoExpand(&line)).into_owned()
            } else {
                format!("{line}\n{old}")
            }
        }
    };
    let parsed: library::Config = body.parse().map_err(|e: toml::de::Error| e.to_string())?;
    if parsed.get("library").and_then(toml::Value::as_str) != Some(dir) {
        return Err("the config would not read back the same folder".into());
    }
    util::make_parent(cfg_path).map_err(|e| e.to_string())?;
    util::write_atomic(cfg_path, body.as_bytes()).map_err(|e| e.to_string())
}

/// Put `share = true|false` under `[usage]` in the config (comments and everything else kept).
fn set_share(cfg_path: &str, on: bool) -> Result<(), String> {
    let line = format!("share = {on}");
    let body = match std::fs::read_to_string(cfg_path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => format!("[usage]\n{line}\n"),
        Err(e) => return Err(e.to_string()),
        Ok(old) => {
            let header = regex::Regex::new(r"(?m)^[ \t]*\[usage\][ \t]*(#[^\r\n]*)?\r?$").expect("valid");
            match header.find(&old) {
                // no [usage] table yet: a new one at the end, after a blank line
                None if old.trim().is_empty() => format!("[usage]\n{line}\n"),
                None => {
                    format!("{}\n[usage]\n{line}\n", if old.ends_with('\n') { old.clone() } else { old.clone() + "\n" })
                }
                Some(h) => {
                    let start = old[h.end()..].find('\n').map_or(old.len(), |i| h.end() + i + 1);
                    let next = regex::Regex::new(r"(?m)^[ \t]*\[").expect("valid");
                    let end = next.find(&old[start..]).map_or(old.len(), |m| start + m.start());
                    let key = regex::Regex::new(r"(?m)^[ \t]*share[ \t]*=.*$").expect("valid");
                    let section = &old[start..end];
                    let section = if key.is_match(section) {
                        key.replace(section, regex::NoExpand(&line)).into_owned()
                    } else {
                        format!("{line}\n{section}")
                    };
                    // the header can be the last line, without a newline
                    let head = if old[..start].ends_with('\n') {
                        old[..start].to_string()
                    } else {
                        format!("{}\n", &old[..start])
                    };
                    format!("{head}{section}{}", &old[end..])
                }
            }
        }
    };
    let parsed: library::Config = body.parse().map_err(|e: toml::de::Error| e.to_string())?;
    if usage::share(&parsed) != if on { usage::Share::On } else { usage::Share::Off } {
        return Err("the config would not read back the same answer".into());
    }
    util::make_parent(cfg_path).map_err(|e| e.to_string())?;
    util::write_atomic(cfg_path, body.as_bytes()).map_err(|e| e.to_string())
}

/// `--usage-send`: send the records not sent yet. Exit 9 when sharing is not on.
fn usage_send(cfg: &toml::Table, dry_run: bool) -> i32 {
    let share = usage::share(cfg);
    let url = usage::url(cfg);
    let recs = usage::pending();
    if dry_run {
        println!(
            "DRY RUN: {} records would be sent to {url} (sharing: {}); nothing was sent",
            recs.len(),
            share.label()
        );
        if !recs.is_empty() {
            println!("{}", util::json_indent1(&usage::payload(&recs)));
        }
        return 0;
    }
    if share != usage::Share::On {
        println!("USAGE SHARING: {}; nothing sent (turn it on with: researcher --usage-share on)", share.label());
        return 9;
    }
    if recs.is_empty() {
        println!("NOTHING TO SEND: every usage record was sent already");
        return 0;
    }
    match usage::send(&url, &recs) {
        Ok(d) => {
            let refused = if d.refused > 0 {
                format!(", {} refused by the receiver and skipped", d.refused)
            } else {
                String::new()
            };
            println!("SENT: {} records to {url} ({} new there{refused})", d.sent, d.new);
            0
        }
        Err((d, why)) => {
            println!(
                "SEND FAILED: {why}; {} records sent before that, {} stay queued for the next try",
                d.sent,
                recs.len() - d.sent - d.refused
            );
            1
        }
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    let code = run(args);
    ExitCode::from(code.clamp(0, 255) as u8)
}

fn run(args: Args) -> i32 {
    let cfg_path = args.config.clone().unwrap_or_else(|| util::join(&config_home(), "config.toml"));
    let cfg_dir = util::dirname(&cfg_path);
    let state_path = util::join(&cfg_dir, "choices.json");

    if args.usage || args.usage_send || args.usage_share.is_some() {
        let cfg = match library::load_config(&cfg_path) {
            Ok(c) => c,
            Err(e) => {
                println!("CONFIG ERROR: {e}");
                return 6;
            }
        };
        if let Some(answer) = &args.usage_share {
            let on = answer == "on";
            return match set_share(&cfg_path, on) {
                Ok(()) if on => {
                    println!(
                        "USAGE SHARING: on (saved as `share = true` under [usage] in {cfg_path}); the anonymous \
                         counts are sent to {} after each load. Undo: researcher --usage-share off",
                        usage::url(&cfg)
                    );
                    0
                }
                Ok(()) => {
                    println!(
                        "USAGE SHARING: off (saved as `share = false` under [usage] in {cfg_path}); nothing is sent"
                    );
                    0
                }
                Err(e) => {
                    println!("CONFIG ERROR: could not save `share` under [usage] in {cfg_path}: {e}");
                    6
                }
            };
        }
        if args.usage_send {
            return usage_send(&cfg, args.dry_run);
        }
        println!("{}", usage::report(&usage::read()));
        println!("{}", usage::share_line(usage::share(&cfg), usage::pending().len()));
        return 0;
    }
    if let Some(dst) = &args.usage_export {
        return match usage::export(dst) {
            Ok(n) => {
                println!("wrote {n} records to {}", util::abspath(dst));
                println!(
                    "each record: day, random install id, document counts per format, character sums \
                     (source bytes, converted, squeezed, whole library). No titles, paths or set names. \
                     Nothing was sent."
                );
                0
            }
            Err(e) => {
                println!("could not write {dst}: {e}");
                1
            }
        };
    }
    if let Some(lib) = &args.init {
        let lib = if lib.is_empty() { util::expanduser("~/research") } else { lib.clone() };
        return init_config(&cfg_path, &lib);
    }

    let session_action = if let Some(r) = &args.rename {
        Some(sessions::Action::Rename { id: r[0].clone(), name: r[1].clone() })
    } else if let Some(id) = &args.resume {
        Some(sessions::Action::Resume { id: id.clone() })
    } else if let Some(id) = &args.session_load {
        Some(sessions::Action::Load { id: id.clone() })
    } else if args.sessions {
        Some(sessions::Action::List { limit: args.limit })
    } else {
        None
    };
    if let Some(action) = session_action {
        let cfg = match library::load_config(&cfg_path) {
            Ok(c) => c,
            Err(e) => {
                println!("CONFIG ERROR: {e}");
                return 6;
            }
        };
        let out = args.out.clone().unwrap_or_else(cache_home);
        return sessions::main(action, &cfg, &util::join(&cfg_dir, "sessions.json"), &out);
    }

    match load(&args, &cfg_path, &state_path) {
        Ok(code) => code,
        Err(e) => {
            println!("CONFIG ERROR: {e}");
            6
        }
    }
}

fn load(args: &Args, cfg_path: &str, state_path: &str) -> Result<i32, ConfigError> {
    let cfg = library::load_config(cfg_path)?;
    let env_lib = std::env::var("RESEARCHER_LIBRARY").ok().filter(|s| !s.is_empty());
    let lib_setting = args.library.clone().filter(|s| !s.is_empty()).or(env_lib);
    let lib_setting = match lib_setting {
        Some(l) => Some(l),
        None => library::cfg_str(&cfg, "library")?.filter(|s| !s.is_empty()).map(str::to_string),
    };
    let no_popup = args.no_popup || std::env::var_os("RESEARCHER_NO_POPUP").is_some_and(|v| !v.is_empty());
    let Some(lib) = lib_setting.map(|l| util::user_path(&l)) else {
        let asked = std::env::var_os("RESEARCHER_FIRST_USE_DONE").is_some();
        if !(args.list || args.sync || no_popup || asked) {
            return Ok(first_use(cfg_path));
        }
        println!("NO LIBRARY: set `library` in {cfg_path} (or run --init, or set RESEARCHER_LIBRARY); nothing loaded");
        return Ok(6);
    };
    if !std::path::Path::new(&lib).is_dir() {
        println!("NO LIBRARY: {lib} is not a folder; nothing loaded");
        return Ok(6);
    }
    let paths = library::scan(&lib, &library::str_list(cfg.get("exclude"), "exclude")?);
    let catalog: Vec<String> = paths.keys().cloned().collect();
    let mut conf = library::load_choices(state_path);
    let names = library::resolve_names(&args.sets, &cfg);
    let sets = library::build_sets(&cfg, &catalog, &conf, &names)?;

    let store = match &args.out {
        Some(o) => o.clone(),
        None => library::cfg_str(&cfg, "text_dir")?.filter(|s| !s.is_empty()).map_or_else(text_home, str::to_string),
    };
    let store = util::user_path(&store);

    if args.sync {
        let mut texts = Texts::new(&paths, &store);
        let ok: Vec<&String> = catalog.iter().filter(|d| texts.has(d)).collect();
        texts.flush();
        let chars: usize = ok.iter().map(|d| texts.len(d)).sum();
        for d in &catalog {
            if !texts.has(d) {
                println!("UNREADABLE  {} ({})", paths[d], texts.error(d));
            }
        }
        println!(
            "text copy: {} of {} docs in {store}, {chars} chars (~{}k tokens)",
            ok.len(),
            paths.len(),
            chars / CHARS_PER_K
        );
        return Ok(0);
    }

    if args.list {
        println!("library: {lib} ({} documents)", paths.len());
        for n in &sets.order {
            let kind = sets.kinds[n];
            if n == "all" || kind != SetKind::Custom || conf.contains_key(n) {
                let saved = conf.get(n).and_then(|r| r.get("selected")).and_then(Value::as_array);
                let sel = saved.map_or(String::new(), |s| format!(", saved default {}", s.len()));
                println!("{n:20} {:>4} docs  ({}{sel})", sets.docs[n].len(), kind.label());
            }
        }
        if let Some(toml::Value::Table(a)) = cfg.get("aliases")
            && !a.is_empty()
        {
            let pairs: Vec<String> = a
                .iter()
                .map(|(k, v)| format!("{k}={}", v.as_str().map_or_else(|| v.to_string(), str::to_string)))
                .collect();
            println!("aliases: {}", pairs.join(", "));
        }
        return Ok(0);
    }

    if paths.is_empty() {
        println!(
            "EMPTY LIBRARY: {lib} has no documents yet (.html, .md, .txt, .pdf); save or copy research into it; \
             nothing loaded"
        );
        return Ok(6);
    }
    if names.is_empty() {
        println!("NO SET: name a set, or set `default_set` in the config; known: {}", sets.order.join(", "));
        return Ok(2);
    }
    let bad: Vec<&String> = names.iter().filter(|n| !sets.contains(n)).collect();
    if !bad.is_empty() {
        let bad: Vec<&str> = bad.iter().map(|s| s.as_str()).collect();
        println!(
            "unknown set(s): {} (names use lowercase letters, digits, . _ -); known: {}",
            bad.join(", "),
            sets.order.join(", ")
        );
        return Ok(2);
    }

    let alldocs: Vec<String> = library::dedupe(names.iter().flat_map(|n| sets.docs[n].iter().cloned()));
    let mut texts = Texts::new(&paths, &store);
    let mut missing = Vec::new();
    for d in &alldocs {
        if !paths.contains_key(d) {
            missing.push(format!("MISSING  {d} (not in {lib})"));
        } else if !texts.has(d) {
            missing.push(format!("UNREADABLE  {} ({})", paths[d], texts.error(d)));
        }
    }
    let secs = library::sections_for(&names, &sets, &conf, &mut texts, args.setup);

    let mut picks: Option<Vec<(String, BTreeSet<String>, bool)>> = None;
    if !no_popup {
        let mut sizes = BTreeMap::new();
        for s in &secs {
            for d in s.main.iter().chain(&s.other) {
                if texts.has(d) {
                    sizes.insert(d.clone(), texts.len(d));
                }
            }
        }
        texts.flush();
        match popup::popup(&secs, sizes, args.setup, &format!("researcher {}", names.join(" "))) {
            Err(e) => println!("NO POPUP ({e}): using the saved defaults"),
            Ok(Outcome::Cancelled) => {
                println!("CANCELLED: the user closed the popup, nothing loaded");
                return Ok(3);
            }
            Ok(Outcome::Timeout) => {
                println!("TIMEOUT: popup closed after {POPUP_TIMEOUT} s without a choice, nothing loaded");
                return Ok(4);
            }
            Ok(Outcome::Picked(p)) => picks = Some(p),
        }
    }
    let new_custom: Vec<&str> = names
        .iter()
        .filter(|n| sets.kinds[*n] == SetKind::Custom && !conf.contains_key(*n))
        .map(String::as_str)
        .collect();
    if picks.is_none() && !new_custom.is_empty() {
        println!("NO POPUP: {} has no saved choice yet; nothing loaded", new_custom.join(", "));
        return Ok(5);
    }
    // --no-popup or no display: the saved defaults, nothing saved
    let picks = picks.unwrap_or_else(|| secs.iter().map(|s| (s.name.clone(), s.ticked.clone(), false)).collect());

    let order: BTreeMap<&str, Vec<&String>> =
        secs.iter().map(|s| (s.name.as_str(), s.main.iter().chain(&s.other).collect())).collect();
    let mut saved_any = false;
    for (n, ticked, keep) in &picks {
        if *keep || args.setup {
            let selected: Vec<&String> = order[n.as_str()].iter().filter(|d| ticked.contains(**d)).copied().collect();
            let mut rec = json!({"selected": selected, "known": sets.docs[n]});
            if sets.kinds[n] == SetKind::Custom {
                rec["custom"] = Value::Bool(true);
            }
            conf.insert(n.clone(), rec);
            println!("SAVED default for {n}: {} of {} docs", ticked.len(), sets.docs[n].len());
            saved_any = true;
        }
    }
    if saved_any && let Err(e) = library::save_choices(state_path, &conf) {
        println!("could not save the defaults to {state_path}: {e}");
    }
    if args.setup {
        println!("setup only: nothing loaded");
        return Ok(0);
    }

    let chosen: BTreeSet<&String> = picks.iter().flat_map(|(_, t, _)| t.iter()).collect();
    let shown = secs.iter().flat_map(|s| s.main.iter().chain(&s.other)).cloned();
    let docs: Vec<String> = library::dedupe(alldocs.iter().cloned().chain(shown))
        .into_iter()
        .filter(|d| chosen.contains(d) && texts.has(d))
        .collect();
    let written: Vec<String> = docs.iter().map(|d| texts.path(d)).collect();
    texts.flush();
    let mut share_note = None;
    if !docs.is_empty()
        && usage::enabled(&cfg)
        && let Ok(rec) = usage::record(&docs, &mut texts, &paths)
    {
        usage::log(&rec); // the log must never break a load
        match usage::share(&cfg) {
            usage::Share::On => usage::send_in_background(cfg_path),
            usage::Share::Off => {}
            usage::Share::Undecided => {
                share_note = Some("USAGE SHARING: not decided (researcher --usage-share on|off)");
            }
        }
    }
    let mut total = 0;
    for (d, dst) in docs.iter().zip(&written) {
        let n = texts.len(d);
        total += n;
        println!("{n:>7} chars  {dst}");
    }
    for line in &missing {
        println!("{line}");
    }
    let skipped: Vec<&str> =
        alldocs.iter().filter(|d| !chosen.contains(d) && texts.has(d)).map(String::as_str).collect();
    if skipped.len() > SKIPPED_NAMED {
        // a custom set or `all` leaves out most of the library: naming it all costs tokens
        println!("left out by choice ({}): not named (more than {SKIPPED_NAMED})", skipped.len());
    } else if !skipped.is_empty() {
        println!("left out by choice ({}): {}", skipped.len(), skipped.join("; "));
    }
    println!("total: {} docs, {total} chars (~{}k tokens)", docs.len(), total / CHARS_PER_K);
    if let Some(note) = share_note {
        println!("{note}");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::set_library;

    fn tmp(name: &str) -> String {
        let d = std::env::temp_dir().join(format!("researcher-unit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("config.toml").to_string_lossy().into_owned()
    }

    fn lib_of(path: &str) -> String {
        let c: toml::Table = std::fs::read_to_string(path).unwrap().parse().unwrap();
        c["library"].as_str().unwrap().to_string()
    }

    #[test]
    fn new_config_is_the_starter() {
        let p = tmp("new");
        set_library(&p, "/r/my \"odd\" \\ dir").unwrap();
        assert_eq!(lib_of(&p), "/r/my \"odd\" \\ dir");
        assert!(std::fs::read_to_string(&p).unwrap().contains("# [sets.rust]"));
    }

    #[test]
    fn existing_config_keeps_its_comments_and_sets() {
        let p = tmp("keep");
        std::fs::write(&p, "# mine\ndefault_set = \"a\"\n[sets.a]\ndocs = [\"x\"]\nlibrary = \"in a table\"\n")
            .unwrap();
        set_library(&p, "/r").unwrap();
        let body = std::fs::read_to_string(&p).unwrap();
        assert!(body.starts_with("library = \"/r\"\n# mine"));
        assert!(body.contains("library = \"in a table\""));
        assert_eq!(lib_of(&p), "/r");
    }

    #[test]
    fn empty_library_line_is_replaced() {
        let p = tmp("empty");
        std::fs::write(&p, "library = \"\"\ndefault_set = \"a\"\n").unwrap();
        set_library(&p, "/r").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "library = \"/r\"\ndefault_set = \"a\"\n");
    }
}
