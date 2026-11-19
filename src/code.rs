// SPDX-License-Identifier: MIT
//! Source files of a project profile, for the Code tab: found under the profile's `root`,
//! matched by its `code` globs. They are loaded by path (Claude reads the source itself): no text
//! copy, and no usage record.

use std::collections::BTreeMap;
use std::fs;

use crate::library::Glob;
use crate::util;

/// Folders never walked into (besides hidden ones): build output and dependencies.
const SKIP_DIRS: [&str; 6] = ["target", "node_modules", "__pycache__", "dist", "build", "venv"];
/// Bigger files are left out: generated or data, not source to read.
const MAX_BYTES: u64 = 1_000_000;

pub struct File {
    pub path: String,
    pub bytes: u64,
    pub mtime: i64,
}

/// Text files under `root` whose path (relative, `/`-separated) matches one of `globs`, by
/// relative path. Hidden files and folders, symlinks, binaries and files over 1 MB are left out.
pub fn scan(root: &str, globs: &[String]) -> BTreeMap<String, File> {
    let globs: Vec<Glob> = globs.iter().map(|g| Glob::new(g)).collect();
    let mut out = BTreeMap::new();
    let mut stack = vec![(root.to_string(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(ft) = e.file_type() else { continue };
            if name.starts_with('.') || ft.is_symlink() {
                continue;
            }
            let path = util::join(&dir, &name);
            let id = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            if ft.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push((path, id));
                }
                continue;
            }
            let Ok(meta) = e.metadata() else { continue };
            if meta.len() > MAX_BYTES || !globs.iter().any(|g| g.matches(&id)) || !is_text(&path) {
                continue;
            }
            out.insert(id, File { path, bytes: meta.len(), mtime: util::mtime_secs(&meta) as i64 });
        }
    }
    out
}

/// No NUL byte in the first 8 KB.
fn is_text(path: &str) -> bool {
    use std::io::Read;
    let mut buf = [0u8; 8192];
    fs::File::open(path).and_then(|mut f| f.read(&mut buf)).is_ok_and(|n| !buf[..n].contains(&0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_text_source_and_skips_the_rest() {
        let d = std::env::temp_dir().join(format!("researcher-code-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let w = |p: &str, b: &[u8]| {
            let f = d.join(p);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, b).unwrap();
        };
        w("src/main.rs", b"fn main() {}");
        w("src/gui/popup.rs", b"// popup");
        w("README.md", b"# x");
        w("target/debug/x.rs", b"built");
        w(".git/config", b"x");
        w("src/blob.rs", b"a\0b");
        let root = d.to_str().unwrap();
        let all: Vec<String> = scan(root, &["*".into()]).into_keys().collect();
        assert_eq!(all, ["README.md", "src/gui/popup.rs", "src/main.rs"]);
        let src: Vec<String> = scan(root, &["src/*".into()]).into_keys().collect();
        assert_eq!(src, ["src/gui/popup.rs", "src/main.rs"]); // `*` crosses folders
        assert_eq!(scan(root, &["src/gui/*".into()]).len(), 1);
        let _ = fs::remove_dir_all(&d);
    }
}
