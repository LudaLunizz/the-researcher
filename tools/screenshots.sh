#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# The Researcher. Copyright (c) 2026 LudaLunizz
#
# Make the README screenshots of the popup from the made-up example library.
#
# Usage: tools/screenshots.sh [OUTPUT_DIR]      (default docs/img)
#
# Needs a display: it opens the real windows three times for about a second each. The program
# saves each window as a PNG from inside egui (RESEARCHER_SCREENSHOT; no screenshot tool, no
# desktop in the picture) and closes it as Cancel. It runs with a throwaway HOME, config and
# saved choices and only reads examples/library, so none of your own research, settings or
# paths is touched or shown.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "${1:-$root/docs/img}"
out=$(cd "${1:-$root/docs/img}" && pwd)
cargo build --release --quiet --manifest-path "$root/Cargo.toml"
bin="$root/target/release/researcher"

home=$(mktemp -d)
trap 'rm -rf "$home"' EXIT
conf="$home/.config/researcher"
mkdir -p "$conf"
export HOME="$home" XDG_CONFIG_HOME="$home/.config" XDG_DATA_HOME="$home/.local/share" XDG_CACHE_HOME="$home/.cache"
unset RESEARCHER_NO_POPUP RESEARCHER_LIBRARY RESEARCHER_DEADLINE RESEARCHER_FIRST_USE_DONE

shoot() { # NAME ARGS...
    local name=$1
    shift
    local code=0
    RESEARCHER_SCREENSHOT="$out/$name" "$bin" "$@" > /dev/null || code=$?
    [ "$code" = 3 ] || { echo "popup ended with exit $code, expected 3 (closed, nothing loaded)" >&2; exit 1; }
    echo "wrote $out/$name"
}

# first use: no library in the config yet
: > "$conf/config.toml"
shoot first-use.png rust

printf 'library = "%s"\n\n[sets.rust]\ndocs = ["rust/*"]\n' "$root/examples/library" > "$conf/config.toml"
# a saved preset made before rust/performance existed, so that one shows as NEW
cat > "$conf/choices.json" <<'JSON'
{"rust": {"selected": ["rust/linux-notes", "rust/security"],
          "known": ["myapp/notes", "myapp/roadmap", "rust/linux-notes", "rust/memory-usage",
                    "rust/security", "web/microdata", "web/seo"]}}
JSON
shoot popup-saved-preset.png rust
shoot popup-new-set.png side-project
