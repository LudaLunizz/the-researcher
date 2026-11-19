// SPDX-License-Identifier: MIT
//! Small state files next to the config: `names.json` (display names of documents and research
//! sets; the library is never touched) and `ui.json` (the last time chip and grouping per tab).

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::bands::Chip;
use crate::library::{self, Choices};

/// {id: name} under `key` ("docs" or "sets") of names.json.
pub fn names(path: &str, key: &str) -> BTreeMap<String, String> {
    library::load_choices(path)
        .get(key)
        .and_then(Value::as_object)
        .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
        .unwrap_or_default()
}

/// Apply renames under `key` ("" removes the name) and save.
pub fn rename(path: &str, key: &str, changes: &BTreeMap<String, String>) -> std::io::Result<()> {
    if changes.is_empty() {
        return Ok(());
    }
    let mut all = library::load_choices(path);
    let entry = all.entry(key.to_string()).or_insert_with(|| json!({}));
    if !entry.is_object() {
        *entry = json!({});
    }
    let m = entry.as_object_mut().expect("just made an object");
    for (id, name) in changes {
        if name.is_empty() {
            m.remove(id);
        } else {
            m.insert(id.clone(), Value::String(name.clone()));
        }
    }
    library::save_choices(path, &all)
}

/// (chip, group by time) last used in a tab; first run: chats on Week by time, research on All
/// by folder.
pub fn ui(path: &str, chats: bool) -> (Chip, bool) {
    let all: Choices = library::load_choices(path);
    let rec = all.get(if chats { "chats" } else { "research" });
    let chip = rec.and_then(|r| r.get("chip")).and_then(Value::as_str).and_then(Chip::from_key);
    let by_time = rec.and_then(|r| r.get("group")).and_then(Value::as_str).map(|g| g == "time");
    (chip.unwrap_or(if chats { Chip::Week } else { Chip::All }), by_time.unwrap_or(chats))
}

pub fn save_ui(path: &str, chats: bool, chip: Chip, by_time: bool) -> std::io::Result<()> {
    let mut all = library::load_choices(path);
    all.insert(
        (if chats { "chats" } else { "research" }).to_string(),
        json!({"chip": chip.key(), "group": if by_time { "time" } else { "folder" }}),
    );
    library::save_choices(path, &all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_ui_round_trip() {
        let d = std::env::temp_dir().join(format!("researcher-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (n, u) = (d.join("names.json"), d.join("ui.json"));
        let (n, u) = (n.to_str().unwrap(), u.to_str().unwrap());
        assert_eq!(ui(u, true), (Chip::Week, true));
        assert_eq!(ui(u, false), (Chip::All, false));
        save_ui(u, true, Chip::Month, false).unwrap();
        assert_eq!(ui(u, true), (Chip::Month, false));
        assert_eq!(ui(u, false), (Chip::All, false));
        let ch = |k: &str, v: &str| BTreeMap::from([(k.to_string(), v.to_string())]);
        rename(n, "docs", &ch("rust/linux", "Linux notes")).unwrap();
        rename(n, "sets", &ch("rust", "Rust")).unwrap();
        assert_eq!(names(n, "docs"), ch("rust/linux", "Linux notes"));
        rename(n, "docs", &ch("rust/linux", "")).unwrap();
        assert!(names(n, "docs").is_empty());
        assert_eq!(names(n, "sets"), ch("rust", "Rust"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
