// SPDX-License-Identifier: MIT
//! End-to-end tests of the `researcher` binary, ported from tests/test_researcher.py.
//!
//! Every run sets RESEARCHER_NO_POPUP=1 (tests must never open a window) and points all XDG
//! folders into a temporary directory (the usage log must never reach a real home).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_researcher");
const SID1: &str = "11111111-1111-4111-8111-111111111111";
const SID2: &str = "22222222-2222-4222-8222-222222222222";
const SID3: &str = "33333333-3333-4333-8333-333333333333";

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

struct Env {
    tmp: PathBuf,
    lib: PathBuf,
    conf_dir: PathBuf,
    out: PathBuf,
    data: PathBuf,
    cfg: PathBuf,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.tmp);
    }
}

impl Env {
    fn new() -> Env {
        static N: AtomicUsize = AtomicUsize::new(0);
        let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();
        let tmp = std::env::temp_dir().join(format!(
            "researcher-test-{}-{}-{nanos}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let lib = tmp.join("lib");
        write(&lib.join("rust/linux.html"), "<h1>Linux</h1><p>one</p><script>x()</script>");
        write(&lib.join("rust/memory-a.md"), "# A");
        write(&lib.join("rust/memory-b.txt"), "B");
        write(&lib.join("web/seo.md"), "seo");
        write(&lib.join("app/roadmap/index.html"), "<p>road</p>");
        write(&lib.join(".hidden/x.md"), "no");
        write(&lib.join("rust/notes.png"), "no");
        let conf_dir = tmp.join("conf");
        let cfg = conf_dir.join("config.toml");
        write(
            &cfg,
            &format!(
                "library = \"{}\"\ndefault_set = \"rust\"\n[aliases]\nrs = \"rust\"\n[sets.rust]\n\
                 docs = [\"rust/linux\", \"rust/memory-*\"]\n[sets.app]\ninclude = [\"rust\"]\n\
                 docs = [\"app/roadmap\", \"app/ghost\"]\n",
                lib.display()
            ),
        );
        Env { out: tmp.join("out"), data: tmp.join("data"), lib, conf_dir, cfg, tmp }
    }

    fn cmd(&self) -> Command {
        let mut c = Command::new(BIN);
        c.env("RESEARCHER_NO_POPUP", "1")
            .env("XDG_DATA_HOME", &self.data)
            .env("XDG_CONFIG_HOME", self.tmp.join("xdg-config"))
            .env("XDG_CACHE_HOME", self.tmp.join("xdg-cache"))
            .env_remove("RESEARCHER_LIBRARY");
        c
    }

    /// (exit code, stdout) of `researcher --config CFG --out OUT ARGS...`
    fn run(&self, args: &[&str]) -> (i32, String) {
        let o = self.cmd().arg("--config").arg(&self.cfg).arg("--out").arg(&self.out).args(args).output().unwrap();
        (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).into_owned())
    }

    fn usage_file(&self) -> PathBuf {
        self.data.join("researcher/usage.jsonl")
    }

    fn records(&self) -> Vec<Value> {
        read(&self.usage_file()).lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    fn out_txts(&self) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(&self.out)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|f| f.ends_with(".txt"))
            .collect();
        v.sort();
        v
    }

    fn append_cfg(&self, text: &str) {
        write(&self.cfg, &(read(&self.cfg) + text));
    }
}

// ---------------------------------------------------------------- usage log

#[test]
fn load_logs_counts_only() {
    let e = Env::new();
    e.run(&["--no-popup", "rust"]);
    let recs = e.records();
    assert_eq!(recs.len(), 1);
    let r = &recs[0];
    assert_eq!(r["docs"], 3);
    assert_eq!(r["fmt"], serde_json::json!({"pdf": 0, "html": 1, "md": 1, "txt": 1}));
    assert_eq!(r["lib_docs"], 5);
    assert!(r["lib_chars"].as_u64() > r["chars"].as_u64());
    assert!(r["raw"].as_u64() >= r["chars"].as_u64());
    let id = r["id"].as_str().unwrap();
    assert!(id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    assert_eq!(r["tk"], 2300);
}

#[test]
fn log_holds_no_names_paths_or_titles() {
    let e = Env::new();
    e.run(&["--no-popup", "rust"]);
    let raw = read(&e.usage_file()) + &read(&e.data.join("researcher/usage-id"));
    for word in ["rust", "linux", "memory", "roadmap", "seo", "ludalunizz"] {
        assert!(!raw.contains(word), "{word} in the log");
    }
    assert!(!raw.contains(&e.lib.display().to_string()));
    assert!(!raw.contains(&e.tmp.display().to_string()));
}

#[test]
fn same_install_id_every_load() {
    let e = Env::new();
    e.run(&["--no-popup", "rust"]);
    e.run(&["--no-popup", "rs"]);
    let r = e.records();
    assert_eq!(r[0]["id"], r[1]["id"]);
}

#[test]
fn log_can_be_switched_off() {
    let e = Env::new();
    e.append_cfg("\n[usage]\nlog = false\n");
    assert_eq!(e.run(&["--no-popup", "rust"]).0, 0);
    assert!(!e.usage_file().exists());
    assert!(!e.data.join("researcher/usage-id").exists());
}

#[test]
fn setup_sync_list_and_nothing_loaded_do_not_log() {
    let e = Env::new();
    e.run(&["--sync"]);
    e.run(&["--list"]);
    e.run(&["--no-popup", "newproj"]); // custom set, nothing chosen
    assert!(!e.usage_file().exists());
}

#[test]
fn unwritable_log_does_not_break_a_load() {
    let e = Env::new();
    write(&e.data.join("researcher"), "a file where the folder should be");
    assert_eq!(e.run(&["--no-popup", "rust"]).0, 0);
}

#[test]
fn usage_report_and_export() {
    let e = Env::new();
    e.run(&["--no-popup", "rust"]);
    let (code, out) = e.run(&["--usage"]);
    assert_eq!(code, 0);
    assert!(out.contains("1 loads"), "{out}");
    assert!(out.contains("squeezed away"));
    let dst = e.tmp.join("export.json");
    let (_, out) = e.run(&["--usage-export", dst.to_str().unwrap()]);
    assert!(out.contains("Nothing was sent"));
    let data: Value = serde_json::from_str(&read(&dst)).unwrap();
    assert_eq!(data["schema"], 1);
    assert_eq!(data["loads"].as_array().unwrap().len(), 1);
}

#[test]
fn usage_with_no_log() {
    let e = Env::new();
    assert!(e.run(&["--usage"]).1.contains("no usage recorded yet"));
}

#[test]
fn bad_usage_lines_skipped() {
    let e = Env::new();
    write(&e.usage_file(), "not json\n{\"v\": 99}\n{\"v\": 1, \"docs\": 1, \"day\": \"2026-11-16\"}\n");
    assert!(e.run(&["--usage"]).1.contains("1 loads on 1 days"));
}

// ---------------------------------------------------------------- library, sets, main flow

#[test]
fn scan_ids_and_hidden_and_other_files() {
    let e = Env::new();
    let (code, out) = e.run(&["--sync"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("text copy: 5 of 5 docs"), "{out}");
    assert_eq!(
        e.out_txts(),
        ["app - roadmap.txt", "rust - linux.txt", "rust - memory-a.txt", "rust - memory-b.txt", "web - seo.txt"]
    );
}

#[test]
fn exclude_globs() {
    let e = Env::new();
    e.append_cfg(""); // keep the file, add exclude at the top
    write(&e.cfg, &format!("exclude = [\"web/*\", \"app/*\"]\n{}", read(&e.cfg)));
    let (_, out) = e.run(&["--sync"]);
    assert!(out.contains("text copy: 3 of 3 docs"), "{out}");
}

#[test]
fn html_preferred_over_pdf() {
    let e = Env::new();
    write(&e.lib.join("web/seo.html"), "<p>h</p>");
    write(&e.lib.join("web/seo.pdf"), "%PDF");
    e.run(&["--sync"]);
    assert!(
        read(&e.out.join("web - seo.txt")).starts_with(&format!("SOURCE: {}", e.lib.join("web/seo.html").display()))
    );
}

#[test]
fn no_popup_first_use_loads_everything_in_set() {
    let e = Env::new();
    let (code, out) = e.run(&["--no-popup", "rust"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("total: 3 docs"), "{out}");
    // the usage log sizes the whole library, so every document gets its text copy
    assert_eq!(
        e.out_txts(),
        ["app - roadmap.txt", "rust - linux.txt", "rust - memory-a.txt", "rust - memory-b.txt", "web - seo.txt"]
    );
    let body = read(&e.out.join("rust - linux.txt"));
    assert!(body.contains("Linux"));
    assert!(!body.contains("x()"));
}

#[test]
fn printed_paths_are_the_copies() {
    let e = Env::new();
    let (_, out) = e.run(&["--no-popup", "rust"]);
    let paths: Vec<&str> = out.lines().filter_map(|l| l.split_once(" chars  ").map(|(_, p)| p)).collect();
    assert_eq!(paths.len(), 3);
    for p in paths {
        assert!(Path::new(p).is_file(), "{p}");
    }
}

#[test]
fn alias_and_default_set() {
    let e = Env::new();
    assert_eq!(e.run(&["--no-popup", "rs"]).0, 0);
    assert_eq!(e.run(&["--no-popup"]).0, 0);
}

#[test]
fn missing_doc_reported() {
    let e = Env::new();
    let (code, out) = e.run(&["--no-popup", "app"]);
    assert_eq!(code, 0);
    assert!(out.contains("MISSING  app/ghost"), "{out}");
    assert!(out.contains("total: 4 docs"), "{out}");
}

#[test]
fn env_switch_blocks_popup() {
    let e = Env::new();
    let (code, out) = e.run(&["rust"]); // no --no-popup flag, env switch only
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("NO POPUP ("));
}

#[test]
fn custom_without_choice_loads_nothing() {
    let e = Env::new();
    let (code, out) = e.run(&["--no-popup", "newproj"]);
    assert_eq!(code, 5);
    assert!(out.contains("nothing loaded"));
}

#[test]
fn custom_set_with_saved_choice() {
    let e = Env::new();
    write(
        &e.conf_dir.join("choices.json"),
        r#"{"side": {"custom": true, "selected": ["web/seo"], "known": ["app/roadmap", "rust/linux", "rust/memory-a", "rust/memory-b"]}}"#,
    );
    let (code, out) = e.run(&["--no-popup", "side"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("total: 1 docs"), "{out}"); // web/seo is NEW but was chosen before
    let (_, out) = e.run(&["--list"]);
    assert!(out.contains("(custom, saved default 1)"), "{out}");
}

#[test]
fn defined_set_new_docs_ticked() {
    let e = Env::new();
    write(
        &e.conf_dir.join("choices.json"),
        r#"{"rust": {"selected": ["rust/linux"], "known": ["rust/linux", "rust/memory-a"]}}"#,
    );
    let (_, out) = e.run(&["--no-popup", "rust"]);
    assert!(out.contains("total: 2 docs"), "{out}");
    assert!(out.contains("left out by choice (1): rust/memory-a"), "{out}");
}

#[test]
fn unknown_set_exit_2() {
    let e = Env::new();
    assert_eq!(e.run(&["--no-popup", "Not A Name!"]).0, 2);
}

#[test]
fn no_library() {
    let e = Env::new();
    write(&e.cfg, "");
    let (code, out) = e.run(&["--no-popup", "x"]);
    assert_eq!(code, 6);
    assert!(out.contains("NO LIBRARY"));
}

#[test]
fn bad_toml() {
    let e = Env::new();
    write(&e.cfg, "library = [");
    let (code, out) = e.run(&["--no-popup", "x"]);
    assert_eq!(code, 6);
    assert!(out.contains("CONFIG ERROR"));
}

#[test]
fn include_loop_is_a_config_error() {
    let e = Env::new();
    e.append_cfg("[sets.a]\ninclude = [\"b\"]\n[sets.b]\ninclude = [\"a\"]\n");
    let (code, out) = e.run(&["--no-popup", "rust"]);
    assert_eq!(code, 6);
    assert!(out.contains("loop"), "{out}");
}

#[test]
fn text_copies_persist_and_foreign_files_kept() {
    let e = Env::new();
    e.run(&["--no-popup", "rust"]);
    let keep = e.out.join("mine.txt");
    write(&keep, "mine");
    write(&e.cfg, &read(&e.cfg).replace(r#"docs = ["rust/linux", "rust/memory-*"]"#, r#"docs = ["rust/linux"]"#));
    let (_, out) = e.run(&["--no-popup", "rust"]);
    assert!(out.contains("total: 1 docs"), "{out}");
    let files = e.out_txts();
    assert!(files.contains(&"mine.txt".to_string()));
    assert!(files.contains(&"rust - memory-a.txt".to_string())); // not loaded this time, but kept
    assert_eq!(read(&keep), "mine");
}

fn snapshot(dir: &Path) -> Vec<(PathBuf, String, SystemTime)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.clone(), read(&p), fs::metadata(&p).unwrap().modified().unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn library_is_never_changed() {
    let e = Env::new();
    let before = snapshot(&e.lib);
    e.run(&["--no-popup", "rust"]);
    e.run(&["--sync"]);
    assert_eq!(before, snapshot(&e.lib));
}

#[test]
fn copy_reused_until_source_changes() {
    let e = Env::new();
    e.run(&["--no-popup", "rust"]);
    let copy = e.out.join("rust - memory-a.txt");
    write(&copy, "SOURCE: x\n\nEDITED COPY\n"); // an unchanged source must not be converted again
    e.run(&["--no-popup", "rust"]);
    assert!(read(&copy).contains("EDITED COPY"));
    std::thread::sleep(Duration::from_millis(20));
    write(&e.lib.join("rust/memory-a.md"), "# A changed");
    e.run(&["--no-popup", "rust"]);
    assert!(read(&copy).contains("A changed"));
    assert!(!read(&copy).contains("EDITED"));
}

#[test]
fn copy_remade_when_converter_version_changes() {
    let e = Env::new();
    e.run(&["--no-popup", "rust"]);
    let copy = e.out.join("rust - memory-a.txt");
    write(&copy, "SOURCE: x\n\nOLD FORMAT\n");
    let idx = e.out.join(".index.json");
    let mut index: Value = serde_json::from_str(&read(&idx)).unwrap();
    index["rust/memory-a"]["v"] = Value::from(1); // a copy made by an older converter
    write(&idx, &index.to_string());
    e.run(&["--no-popup", "rust"]);
    assert!(!read(&copy).contains("OLD FORMAT"));
}

#[test]
fn text_dir_in_config() {
    let e = Env::new();
    let elsewhere = e.tmp.join("mytext");
    write(&e.cfg, &format!("text_dir = \"{}\"\n{}", elsewhere.display(), read(&e.cfg)));
    let o = e.cmd().arg("--config").arg(&e.cfg).args(["--no-popup", "rust"]).output().unwrap();
    assert!(o.status.success());
    assert!(elsewhere.join("rust - linux.txt").is_file());
}

#[test]
fn saved_default_respected() {
    let e = Env::new();
    write(
        &e.conf_dir.join("choices.json"),
        r#"{"rust": {"selected": ["rust/linux"], "known": ["rust/linux", "rust/memory-a", "rust/memory-b"]}}"#,
    );
    let (_, out) = e.run(&["--no-popup", "rust"]);
    assert!(out.contains("total: 1 docs"), "{out}");
    assert!(out.contains("left out by choice (2)"), "{out}");
}

#[test]
fn extra_doc_from_library_is_loaded_with_saved_preset() {
    let e = Env::new();
    write(
        &e.conf_dir.join("choices.json"),
        r#"{"rust": {"selected": ["rust/linux", "web/seo"], "known": ["rust/linux", "rust/memory-a", "rust/memory-b"]}}"#,
    );
    let (_, out) = e.run(&["--no-popup", "rust"]);
    assert!(out.contains("total: 2 docs"), "{out}");
    assert!(out.contains("web - seo.txt"), "{out}");
}

#[test]
fn setup_saves_defaults_and_loads_nothing() {
    let e = Env::new();
    let (code, out) = e.run(&["--no-popup", "--setup", "rust"]);
    assert_eq!(code, 0);
    assert!(out.contains("SAVED default for rust: 3 of 3 docs"), "{out}");
    assert!(out.contains("setup only: nothing loaded"));
    let saved: Value = serde_json::from_str(&read(&e.conf_dir.join("choices.json"))).unwrap();
    assert_eq!(saved["rust"]["selected"].as_array().unwrap().len(), 3);
    assert!(!e.usage_file().exists());
}

#[test]
fn list() {
    let e = Env::new();
    let (code, out) = e.run(&["--list"]);
    assert_eq!(code, 0);
    assert!(out.contains("library:"));
    assert!(out.contains("rust"));
    assert!(out.contains("all"));
    assert!(out.contains("aliases: rs=rust"));
}

#[test]
fn init() {
    let e = Env::new();
    fs::remove_file(&e.cfg).unwrap();
    let lib = e.lib.display().to_string();
    let o = e.cmd().arg("--config").arg(&e.cfg).args(["--init", &lib]).output().unwrap();
    assert_eq!(o.status.code(), Some(0));
    let cfg: toml::Table = read(&e.cfg).parse().unwrap();
    assert_eq!(cfg["library"].as_str(), Some(lib.as_str()));
    let o = e.cmd().arg("--config").arg(&e.cfg).args(["--init", &lib]).output().unwrap();
    assert_eq!(o.status.code(), Some(1)); // no overwrite
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn example_library_loads() {
    let e = Env::new();
    let lib = repo().join("examples/library");
    let (code, out) = e.run(&["--library", lib.to_str().unwrap(), "--no-popup", "all"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("total: 8 docs"), "{out}");
}

#[test]
fn example_config_parses() {
    let cfg: toml::Table = read(&repo().join("examples/config.toml")).parse().unwrap();
    assert!(cfg["sets"].as_table().unwrap().contains_key("myapp"));
}

// ---------------------------------------------------------------- sessions

fn jl(path: &Path, entries: &[Value], age_secs: u64) {
    let body: Vec<String> = entries.iter().map(Value::to_string).collect();
    write(path, &(body.join("\n") + "\nnot json\n"));
    let t = SystemTime::now() - Duration::from_secs(age_secs);
    fs::File::options().write(true).open(path).unwrap().set_modified(t).unwrap();
}

fn user(text: &str) -> Value {
    serde_json::json!({"type": "user", "timestamp": "2026-11-16T10:00:00Z", "cwd": "/work/app",
                       "message": {"role": "user", "content": text}})
}

fn assistant(text: &str) -> Value {
    serde_json::json!({"type": "assistant", "timestamp": "2026-11-16T10:01:00Z",
        "message": {"role": "assistant", "content": [
            {"type": "thinking", "thinking": "hmm"}, {"type": "text", "text": text},
            {"type": "tool_use", "name": "Bash", "input": {"command": "secret-tool-call"}}]}})
}

struct Sess {
    e: Env,
    proj: PathBuf,
    reg: PathBuf,
}

fn sessions_env() -> Sess {
    let e = Env::new();
    let proj = e.tmp.join("projects");
    e.append_cfg(&format!("\n[sessions]\nenabled = true\nprojects_dir = \"{}\"\n", proj.display()));
    let d = proj.join("-work-app");
    let mut meta = user("<system-reminder>ignore me</system-reminder>");
    meta["isMeta"] = Value::Bool(true);
    jl(
        &d.join(format!("{SID1}.jsonl")),
        &[
            serde_json::json!({"type": "file-history-snapshot"}),
            meta,
            user("<command-message>x</command-message>"),
            user("Fix the popup crash on   start-up"),
            assistant("Found it, the timeout was wrong."),
            serde_json::json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": "TOOLOUT"}]}}),
        ],
        3600,
    );
    jl(
        &d.join(format!("{SID2}.jsonl")),
        &[user("Second one"), serde_json::json!({"type": "ai-title", "aiTitle": "Nice title"})],
        3600,
    );
    jl(&d.join(format!("{SID3}.jsonl")), &[user("Still open")], 5); // modified just now
    jl(&d.join("agent-abc.jsonl"), &[user("not a session id")], 3600);
    let reg = e.conf_dir.join("sessions.json");
    Sess { e, proj, reg }
}

#[test]
fn sessions_off_by_default() {
    let s = sessions_env();
    write(&s.e.cfg, &format!("library = \"{}\"\n", s.e.lib.display()));
    let (code, out) = s.e.run(&["--sessions"]);
    assert_eq!(code, 7);
    assert!(out.contains("SESSIONS OFF"));
    assert!(!s.reg.exists());
}

#[test]
fn lists_closed_sessions_with_full_id() {
    let s = sessions_env();
    let (code, out) = s.e.run(&["--sessions"]);
    assert_eq!(code, 0);
    assert!(out.contains(&format!("{SID1}  2026-11-16 10:01  Fix the popup crash on start-up")), "{out}");
    assert!(out.contains(SID2));
    assert!(out.contains("Nice title")); // Claude Code's own title wins over the first prompt
    assert!(!out.contains(SID3)); // still open: not tracked yet
    assert!(!out.contains("agent-abc"));
    assert!(out.contains("2 closed sessions"));
}

#[test]
fn rename_sticks_and_keeps_the_id() {
    let s = sessions_env();
    s.e.run(&["--sessions"]);
    let (code, out) = s.e.run(&["--rename", &SID1[..8], "popup crash"]);
    assert_eq!(code, 0);
    assert!(out.contains(&format!("\"popup crash\" ({SID1})")), "{out}");
    let (_, out) = s.e.run(&["--sessions"]); // a rescan must not undo a manual name
    assert!(out.contains(&format!("{SID1}  2026-11-16 10:01  popup crash")), "{out}");
    let reg: Value = serde_json::from_str(&read(&s.reg)).unwrap();
    assert_eq!(reg["sessions"][SID1]["name_source"], "manual");
}

#[test]
fn auto_name_follows_a_later_title() {
    let s = sessions_env();
    s.e.run(&["--sessions"]);
    let path = s.proj.join("-work-app").join(format!("{SID1}.jsonl"));
    jl(
        &path,
        &[user("Fix the popup crash"), serde_json::json!({"type": "custom-title", "customTitle": "Crash hunt"})],
        3000,
    );
    let (_, out) = s.e.run(&["--sessions"]);
    assert!(out.contains("Crash hunt"), "{out}");
}

#[test]
fn ambiguous_and_unknown_ids() {
    let s = sessions_env();
    s.e.run(&["--sessions"]);
    let (code, out) = s.e.run(&["--resume", "ffff"]);
    assert_eq!(code, 8);
    assert!(out.contains("no session matches"));
    assert_eq!(s.e.run(&["--rename", "", "x"]).0, 8);
}

#[test]
fn resume_prints_cd_and_full_id() {
    let s = sessions_env();
    s.e.run(&["--sessions"]);
    let (code, out) = s.e.run(&["--resume", SID1]);
    assert_eq!(code, 0);
    assert!(out.contains(&format!("cd /work/app && claude --resume {SID1}")), "{out}");
}

#[test]
fn session_load_writes_conversation_without_tool_traffic() {
    let s = sessions_env();
    s.e.run(&["--sessions"]);
    let (code, out) = s.e.run(&["--session-load", &SID1[..6]]);
    assert_eq!(code, 0, "{out}");
    let path =
        out.lines().find_map(|l| l.split_once("chars  ").filter(|_| !l.contains("total")).map(|(_, p)| p)).unwrap();
    let text = read(Path::new(path));
    assert!(text.contains(&format!("SESSION ID: {SID1}")));
    assert!(text.contains("USER: Fix the popup crash on   start-up"));
    assert!(text.contains("ASSISTANT: Found it"));
    for bad in ["ignore me", "TOOLOUT", "secret-tool-call", "hmm", "command-message"] {
        assert!(!text.contains(bad), "{bad}");
    }
}

#[test]
fn sessions_need_no_library() {
    let s = sessions_env();
    write(&s.e.cfg, &format!("[sessions]\nenabled = true\nprojects_dir = \"{}\"\n", s.proj.display()));
    assert_eq!(s.e.run(&["--sessions"]).0, 0);
}

// ---------------------------------------------------------------- parity with the Python version

/// Both versions share the text copy (same TEXT_VERSION), so they must write the same text.
#[test]
fn same_text_copies_as_python() {
    let py = repo().join("skills/researcher/researcher.py");
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("python3 not found: parity check skipped");
        return;
    }
    let e = Env::new();
    let lib = repo().join("examples/library");
    write(
        &e.lib.join("extra/entities.html"),
        "<p>caf&eacute; &amp; &#8211; &nbsp;x</p><table><tr><td>a</td><td>b</td></tr></table>",
    );
    write(&e.lib.join("extra/rules.md"), "Title\r\n=====\r\n\r\n\r\n- a\n    - b\n\u{200b}----------\n");
    for (name, l) in [("rs", lib.as_path()), ("rs2", e.lib.as_path())] {
        let rs_out = e.tmp.join(name);
        let py_out = e.tmp.join(format!("{name}-py"));
        let o = e
            .cmd()
            .args(["--library", l.to_str().unwrap(), "--out", rs_out.to_str().unwrap(), "--sync"])
            .output()
            .unwrap();
        assert!(o.status.success());
        let o = Command::new("python3")
            .arg("-I")
            .arg(&py)
            .args(["--library", l.to_str().unwrap(), "--out", py_out.to_str().unwrap(), "--sync"])
            .env("RESEARCHER_NO_POPUP", "1")
            .env("XDG_DATA_HOME", &e.data)
            .env("XDG_CONFIG_HOME", e.tmp.join("xdg-config"))
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let mut files: Vec<String> = fs::read_dir(&py_out)
            .unwrap()
            .flatten()
            .map(|x| x.file_name().to_string_lossy().into_owned())
            .filter(|f| f.ends_with(".txt"))
            .collect();
        files.sort();
        assert!(!files.is_empty());
        for f in files {
            assert_eq!(read(&rs_out.join(&f)), read(&py_out.join(&f)), "{f} differs");
        }
        let rs_idx: Value = serde_json::from_str(&read(&rs_out.join(".index.json"))).unwrap();
        let py_idx: Value = serde_json::from_str(&read(&py_out.join(".index.json"))).unwrap();
        assert_eq!(rs_idx, py_idx, "the indexes differ");
    }
}

// ---------------------------------------------------------------- first use

#[test]
fn no_library_without_window_asks_nothing() {
    // with RESEARCHER_NO_POPUP the first-use window must not open; the old message stays
    let e = Env::new();
    write(&e.cfg, "default_set = \"x\"\n");
    let (code, out) = e.run(&["x"]);
    assert_eq!(code, 6);
    assert!(out.contains("NO LIBRARY"), "{out}");
    assert_eq!(read(&e.cfg), "default_set = \"x\"\n"); // config untouched
}

#[test]
fn no_display_falls_back_to_the_message() {
    let e = Env::new();
    write(&e.cfg, "");
    let o = e
        .cmd()
        .env_remove("RESEARCHER_NO_POPUP")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .arg("--config")
        .arg(&e.cfg)
        .arg("rust")
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&o.stdout);
    assert_eq!(o.status.code(), Some(6), "{out}");
    assert!(out.contains("NO LIBRARY") && out.contains("no window to ask"), "{out}");
}

#[test]
fn empty_library_says_so() {
    let e = Env::new();
    let empty = e.tmp.join("empty");
    fs::create_dir_all(&empty).unwrap();
    let (code, out) = e.run(&["--library", empty.to_str().unwrap(), "--no-popup", "rust"]);
    assert_eq!(code, 6);
    assert!(out.contains("EMPTY LIBRARY"), "{out}");
    let (code, out) = e.run(&["--library", empty.to_str().unwrap(), "--list"]);
    assert_eq!(code, 0, "{out}");
}
