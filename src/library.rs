// SPDX-License-Identifier: MIT
//! The library on disk, the config, sets and the popup sections.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

use crate::text::Texts;
use crate::util;

/// Preference order when two files have the same id (`x.html` wins over `x.pdf`).
pub const DOC_EXTS: [&str; 5] = [".html", ".htm", ".md", ".txt", ".pdf"];

static NAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9._-]*$").expect("valid"));

#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub type Config = toml::Table;

/// config.toml as a table; empty when the file does not exist.
pub fn load_config(path: &str) -> Result<Config, ConfigError> {
    let raw = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::new()),
        Err(e) => return Err(ConfigError(format!("{path}: {e}"))),
    };
    let text = String::from_utf8(raw).map_err(|e| ConfigError(format!("{path}: {e}")))?;
    text.parse::<Config>().map_err(|e| ConfigError(format!("{path}: {}", e.to_string().trim_end())))
}

/// A string value of the config, or an error naming the key.
pub fn cfg_str<'a>(cfg: &'a Config, key: &str) -> Result<Option<&'a str>, ConfigError> {
    match cfg.get(key) {
        None => Ok(None),
        Some(toml::Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(ConfigError(format!("`{key}` must be a string"))),
    }
}

/// A list of strings from the config (`exclude`, `docs`, `include`).
pub fn str_list(v: Option<&toml::Value>, what: &str) -> Result<Vec<String>, ConfigError> {
    match v {
        None => Ok(Vec::new()),
        Some(toml::Value::Array(a)) => a
            .iter()
            .map(|x| {
                x.as_str().map(str::to_string).ok_or_else(|| ConfigError(format!("{what} must be a list of strings")))
            })
            .collect(),
        Some(_) => Err(ConfigError(format!("{what} must be a list of strings"))),
    }
}

// ---------------------------------------------------------------- globs

/// A compiled `fnmatch.fnmatchcase` pattern: `*` and `?` also match `/`, `[seq]` and `[!seq]`.
pub struct Glob(Regex);

impl Glob {
    pub fn new(pat: &str) -> Glob {
        let p: Vec<char> = pat.chars().collect();
        let (n, mut i) = (p.len(), 0);
        let mut out = String::from("(?s)^(?:");
        while i < n {
            let c = p[i];
            i += 1;
            match c {
                '*' => {
                    while i < n && p[i] == '*' {
                        i += 1;
                    }
                    out.push_str(".*");
                }
                '?' => out.push('.'),
                '[' => {
                    let mut j = i;
                    if j < n && p[j] == '!' {
                        j += 1;
                    }
                    if j < n && p[j] == ']' {
                        j += 1;
                    }
                    while j < n && p[j] != ']' {
                        j += 1;
                    }
                    if j >= n {
                        out.push_str(r"\[");
                        continue;
                    }
                    let mut stuff: &[char] = &p[i..j];
                    i = j + 1;
                    let mut class = String::from("[");
                    if stuff.first() == Some(&'!') {
                        class.push('^');
                        stuff = &stuff[1..];
                    }
                    if stuff.is_empty() {
                        // `[!]` with nothing else: any character (an empty `[]` cannot occur)
                        out.push_str(if class == "[^" { "." } else { "[^\\s\\S]" });
                        continue;
                    }
                    for (k, &ch) in stuff.iter().enumerate() {
                        let range = ch == '-' && k > 0 && k + 1 < stuff.len();
                        if !range && "\\[]^&~-".contains(ch) {
                            class.push('\\');
                        }
                        class.push(ch);
                    }
                    class.push(']');
                    out.push_str(&class);
                }
                other => out.push_str(&regex::escape(&other.to_string())),
            }
        }
        out.push_str(")$");
        Glob(Regex::new(&out).unwrap_or_else(|_| Regex::new(&format!("^{}$", regex::escape(pat))).expect("escaped")))
    }

    pub fn matches(&self, s: &str) -> bool {
        self.0.is_match(s)
    }
}

pub fn is_glob(pat: &str) -> bool {
    pat.contains(['*', '?', '['])
}

// ---------------------------------------------------------------- the library

/// {doc id: file path} for every document under `library`, minus ids matching `exclude` globs.
///
/// A document id is its path inside the library without the extension. A folder that holds an
/// `index.*` file counts as one document named after the folder. Hidden files and folders are
/// skipped, and links to folders are not followed.
pub fn scan(library: &str, exclude: &[String]) -> BTreeMap<String, String> {
    let mut found: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut offer = |id: String, path: String, ext: &str| {
        let pref = DOC_EXTS.iter().position(|e| *e == ext).unwrap_or(DOC_EXTS.len());
        match found.get(&id) {
            Some((p, _)) if *p <= pref => {}
            _ => {
                found.insert(id, (pref, path));
            }
        }
    };
    let mut stack = vec![(library.to_string(), String::new())];
    while let Some((root, rel)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&root) else { continue };
        let (mut dirs, mut files) = (Vec::new(), Vec::new());
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = util::join(&root, &name);
            let is_link = e.file_type().is_ok_and(|t| t.is_symlink());
            if fs::metadata(&path).is_ok_and(|m| m.is_dir()) {
                if !name.starts_with('.') && !is_link {
                    dirs.push(name);
                }
            } else {
                files.push(name);
            }
        }
        let index = DOC_EXTS.iter().map(|e| format!("index{e}")).find(|n| files.contains(n));
        if let Some(index) = index.filter(|_| !rel.is_empty()) {
            let ext = util::split_ext(&index).1.to_string();
            offer(rel.clone(), util::join(&root, &index), &ext);
            continue; // a folder with an index file is one document
        }
        files.sort();
        for f in files {
            let (stem, ext) = util::split_ext(&f);
            let ext = ext.to_lowercase();
            if f.starts_with('.') || !DOC_EXTS.contains(&ext.as_str()) {
                continue;
            }
            let path = util::join(&root, &f);
            if fs::metadata(&path).is_ok_and(|m| m.is_file()) {
                let id = if rel.is_empty() { stem.to_string() } else { format!("{rel}/{stem}") };
                offer(id, path, &ext);
            }
        }
        dirs.sort();
        for d in dirs.into_iter().rev() {
            let sub = if rel.is_empty() { d.clone() } else { format!("{rel}/{d}") };
            stack.push((util::join(&root, &d), sub));
        }
    }
    let globs: Vec<Glob> = exclude.iter().map(|p| Glob::new(p)).collect();
    found.into_iter().filter(|(k, _)| !globs.iter().any(|g| g.matches(k))).map(|(k, (_, p))| (k, p)).collect()
}

// ---------------------------------------------------------------- sets

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SetKind {
    /// Defined in config.toml.
    Set,
    /// The whole library.
    All,
    /// Any other valid name: the user ticks it together from the library.
    Custom,
}

impl SetKind {
    pub fn label(self) -> &'static str {
        match self {
            SetKind::Set => "set",
            SetKind::All => "all",
            SetKind::Custom => "custom",
        }
    }
}

/// Every set that may be asked for, in order: config sets, `all`, then custom ones.
pub struct Sets {
    pub order: Vec<String>,
    pub docs: BTreeMap<String, Vec<String>>,
    pub kinds: BTreeMap<String, SetKind>,
}

impl Sets {
    fn add(&mut self, name: &str, docs: Vec<String>, kind: SetKind) {
        self.order.push(name.to_string());
        self.docs.insert(name.to_string(), docs);
        self.kinds.insert(name.to_string(), kind);
    }

    pub fn contains(&self, name: &str) -> bool {
        self.docs.contains_key(name)
    }
}

fn set_defs(cfg: &Config) -> Result<toml::Table, ConfigError> {
    match cfg.get("sets") {
        None => Ok(toml::Table::new()),
        Some(toml::Value::Table(t)) => Ok(t.clone()),
        Some(_) => Err(ConfigError("[sets] must be a table".into())),
    }
}

/// Document ids of a configured set: includes first, then its own docs, globs expanded.
pub fn expand(
    name: &str,
    defs: &toml::Table,
    catalog: &[String],
    stack: &[String],
) -> Result<Vec<String>, ConfigError> {
    if stack.iter().any(|s| s == name) {
        let mut chain: Vec<&str> = stack.iter().map(String::as_str).collect();
        chain.push(name);
        return Err(ConfigError(format!("sets include each other in a loop: {}", chain.join(" -> "))));
    }
    let Some(toml::Value::Table(spec)) = defs.get(name) else {
        return Err(ConfigError(format!("[sets.{name}] must be a table")));
    };
    let mut inner = stack.to_vec();
    inner.push(name.to_string());
    let mut out = Vec::new();
    for inc in str_list(spec.get("include"), &format!("[sets.{name}] include"))? {
        if !defs.contains_key(&inc) {
            return Err(ConfigError(format!("set '{name}' includes unknown set '{inc}'")));
        }
        out.extend(expand(&inc, defs, catalog, &inner)?);
    }
    for pat in str_list(spec.get("docs"), &format!("[sets.{name}] docs"))? {
        let hits: Vec<String> = if is_glob(&pat) {
            let g = Glob::new(&pat);
            catalog.iter().filter(|d| g.matches(d)).cloned().collect()
        } else {
            Vec::new()
        };
        if hits.is_empty() {
            out.push(pat); // no hit stays in the list and shows up as MISSING
        } else {
            out.extend(hits);
        }
    }
    Ok(dedupe(out))
}

/// Keep the first of each item, in order.
pub fn dedupe<T: Clone + Eq + std::hash::Hash>(items: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut seen = HashSet::new();
    items.into_iter().filter(|x| seen.insert(x.clone())).collect()
}

/// Saved choices (`choices.json`): set name → {selected, known, custom}.
pub type Choices = Map<String, Value>;

fn saved<'a>(conf: &'a Choices, name: &str) -> Option<&'a Map<String, Value>> {
    conf.get(name).map(|v| v.as_object().unwrap_or(&EMPTY))
}

static EMPTY: LazyLock<Map<String, Value>> = LazyLock::new(Map::new);

fn saved_list(rec: Option<&Map<String, Value>>, key: &str) -> BTreeSet<String> {
    rec.and_then(|r| r.get(key))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

pub fn build_sets(cfg: &Config, catalog: &[String], conf: &Choices, names: &[String]) -> Result<Sets, ConfigError> {
    let defs = set_defs(cfg)?;
    let mut sets = Sets { order: Vec::new(), docs: BTreeMap::new(), kinds: BTreeMap::new() };
    for n in defs.keys() {
        let docs = expand(n, &defs, catalog, &[])?;
        sets.add(n, docs, SetKind::Set);
    }
    if !sets.contains("all") {
        sets.add("all", catalog.to_vec(), SetKind::All);
    } else {
        sets.docs.insert("all".into(), catalog.to_vec());
        sets.kinds.insert("all".into(), SetKind::All);
    }
    for n in conf.keys().chain(names.iter()) {
        if sets.contains(n) {
            continue;
        }
        if let Some(rec) = saved(conf, n) {
            if !util::truthy_json(rec.get("custom")) {
                continue; // saved choice of a set that no longer exists
            }
        } else if !NAME_RE.is_match(n) {
            continue;
        }
        sets.add(n, catalog.to_vec(), SetKind::Custom);
    }
    Ok(sets)
}

/// Set names from the command line (aliases applied, lower case), or the config's default_set.
pub fn resolve_names(argv: &[String], cfg: &Config) -> Vec<String> {
    let aliases: BTreeMap<String, String> = match cfg.get("aliases") {
        Some(toml::Value::Table(t)) => {
            t.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.to_lowercase(), v.to_string()))).collect()
        }
        _ => BTreeMap::new(),
    };
    let mut names: Vec<String> = argv
        .iter()
        .map(|n| {
            let low = n.to_lowercase();
            aliases.get(&low).cloned().unwrap_or(low)
        })
        .collect();
    if names.is_empty()
        && let Some(d) = cfg.get("default_set").and_then(toml::Value::as_str).filter(|d| !d.is_empty())
    {
        names.push(d.to_string());
    }
    dedupe(names)
}

// ---------------------------------------------------------------- sections

/// One requested set as the popup shows it.
#[derive(Debug, Clone)]
pub struct Section {
    pub name: String,
    pub kind: SetKind,
    /// What the set is made of (for a custom set: the whole library).
    pub docs: Vec<String>,
    /// Shown at the top: the set's own documents plus extras saved earlier.
    pub main: Vec<String>,
    /// The rest of the library, offered as "add from your library".
    pub other: Vec<String>,
    /// Added since the preset was saved.
    pub new: BTreeSet<String>,
    /// Pre-ticked documents (only readable ones).
    pub ticked: BTreeSet<String>,
    pub remember: bool,
    /// No preset saved yet.
    pub first: bool,
}

pub fn sections_for(names: &[String], sets: &Sets, conf: &Choices, texts: &mut Texts, setup: bool) -> Vec<Section> {
    let catalog = &sets.docs["all"];
    let mut out = Vec::new();
    for n in names {
        let docs = sets.docs[n].clone();
        let kind = sets.kinds[n];
        let rec = saved(conf, n);
        let has = rec.is_some_and(|r| !r.is_empty()); // Python's `if saved:`
        let known = saved_list(rec, "known");
        let selected = saved_list(rec, "selected");
        let new: BTreeSet<String> =
            if has { docs.iter().filter(|d| !known.contains(*d)).cloned().collect() } else { BTreeSet::new() };
        let remember = rec.is_none() || !new.is_empty() || setup;
        let (ticked, main): (BTreeSet<String>, Vec<String>) = if kind == SetKind::Custom {
            // nothing ticked on first use, new documents never ticked on their own
            let ticked = docs.iter().filter(|d| selected.contains(*d)).cloned().collect();
            let main = if has {
                docs.iter().filter(|d| selected.contains(*d) || new.contains(*d)).cloned().collect()
            } else {
                Vec::new()
            };
            (ticked, main)
        } else {
            let ticked =
                if rec.is_none() { docs.iter().cloned().collect() } else { selected.union(&new).cloned().collect() };
            let mut main = docs.clone();
            let own: HashSet<&String> = docs.iter().collect();
            main.extend(catalog.iter().filter(|d| selected.contains(*d) && !own.contains(d)).cloned());
            (ticked, main)
        };
        let in_main: HashSet<&String> = main.iter().collect();
        let other = catalog.iter().filter(|d| !in_main.contains(d)).cloned().collect();
        let ticked = ticked.into_iter().filter(|d| texts.has(d)).collect();
        out.push(Section { name: n.clone(), kind, docs, main, other, new, ticked, remember, first: rec.is_none() });
    }
    out
}

pub fn load_choices(path: &str) -> Choices {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default()
}

pub fn save_choices(path: &str, conf: &Choices) -> std::io::Result<()> {
    util::make_parent(path)?;
    util::write_atomic(path, util::json_indent1(&Value::Object(conf.clone())).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_like_fnmatchcase() {
        assert!(Glob::new("rust/*").matches("rust/a/b"));
        assert!(Glob::new("rust/memory-?").matches("rust/memory-a"));
        assert!(!Glob::new("rust/memory-?").matches("rust/memory-ab"));
        assert!(Glob::new("[a-c]x").matches("bx"));
        assert!(!Glob::new("[!a-c]x").matches("bx"));
        assert!(Glob::new("a[").matches("a["));
        assert!(Glob::new("a.b").matches("a.b"));
        assert!(!Glob::new("a.b").matches("axb"));
        assert!(!Glob::new("Rust/*").matches("rust/x"));
    }

    #[test]
    fn include_loop_and_unknown_include() {
        let defs: toml::Table = "[a]\ninclude=[\"b\"]\n[b]\ninclude=[\"a\"]\n".parse().unwrap();
        assert!(expand("a", &defs, &[], &[]).unwrap_err().0.contains("loop"));
        let defs: toml::Table = "[a]\ninclude=[\"zz\"]\n".parse().unwrap();
        assert!(expand("a", &defs, &[], &[]).unwrap_err().0.contains("unknown set"));
    }

    #[test]
    fn invalid_name_not_custom() {
        let s = build_sets(&Config::new(), &[], &Choices::new(), &["Bad Name!".into()]).unwrap();
        assert!(!s.contains("Bad Name!"));
    }
}
