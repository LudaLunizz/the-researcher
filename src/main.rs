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
use std::process::ExitCode;

use clap::Parser;
use serde_json::{Value, json};

use library::{ConfigError, SetKind};
use popup::{Outcome, POPUP_TIMEOUT};
use text::Texts;
use usage::CHARS_PER_K;

pub const APP: &str = "researcher";

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
    /// write the anonymous usage records to FILE to look at and send by hand; nothing is sent
    #[arg(long, value_name = "FILE", num_args = 0..=1, default_missing_value = "researcher-usage.json")]
    usage_export: Option<String>,
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
# `researcher --usage`. Nothing is sent anywhere. Switch it off with:
# [usage]
# log = false
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

fn main() -> ExitCode {
    let args = Args::parse();
    let code = run(args);
    ExitCode::from(code.clamp(0, 255) as u8)
}

fn run(args: Args) -> i32 {
    let cfg_path = args.config.clone().unwrap_or_else(|| util::join(&config_home(), "config.toml"));
    let cfg_dir = util::dirname(&cfg_path);
    let state_path = util::join(&cfg_dir, "choices.json");

    if args.usage {
        println!("{}", usage::report(&usage::read()));
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
    let Some(lib) = lib_setting.map(|l| util::user_path(&l)) else {
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

    let no_popup = args.no_popup || std::env::var_os("RESEARCHER_NO_POPUP").is_some_and(|v| !v.is_empty());
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
    if !docs.is_empty()
        && usage::enabled(&cfg)
        && let Ok(rec) = usage::record(&docs, &mut texts, &paths)
    {
        usage::log(&rec); // the log must never break a load
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
    if !skipped.is_empty() {
        println!("left out by choice ({}): {}", skipped.len(), skipped.join("; "));
    }
    println!("total: {} docs, {total} chars (~{}k tokens)", docs.len(), total / CHARS_PER_K);
    Ok(0)
}
