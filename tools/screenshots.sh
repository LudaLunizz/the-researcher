#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# The Researcher. Copyright (c) 2026 LudaLunizz
#
# Make the README screenshots of the popup from the made-up example library and the made-up
# chats below (for /load-context).
#
# Usage: tools/screenshots.sh [OUTPUT_DIR]      (default docs/img)
#
# Needs a display: it opens the real windows six times for about a second each. The program
# saves each window as a PNG from inside egui (RESEARCHER_SCREENSHOT; no screenshot tool, no
# desktop in the picture) and closes it as Cancel. It runs with a throwaway HOME, config and
# saved choices and only reads examples/library and the made-up chats it writes into that throwaway home, so none of your own research, settings or
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

printf 'library = "%s"\ndefault_set = "rust"\n\n[sets.rust]\ndocs = ["rust/*"]\n' "$root/examples/library" > "$conf/config.toml"
# a saved preset made before rust/performance existed, so that one shows as NEW
cat > "$conf/choices.json" <<'JSON'
{"rust": {"selected": ["rust/linux-notes", "rust/security"],
          "known": ["myapp/notes", "myapp/roadmap", "rust/linux-notes", "rust/memory-usage",
                    "rust/security", "web/microdata", "web/seo"]}}
JSON
shoot popup-saved-preset.png rust
shoot popup-new-set.png side-project

# --- /load-context: made-up earlier chats in two made-up project folders -----------------------
projects="$home/.claude/projects"
day() { date -u -d "-$1 days" +%F; } # N days ago, so the Week chip always has chats to show
chat() { # FOLDER ID DAY FIRST-PROMPT REPLY
    local dir="$projects/$1"
    mkdir -p "$dir"
    printf '%s\n%s\n' \
        "{\"type\":\"user\",\"cwd\":\"/home/you/$1\",\"timestamp\":\"$3T10:00:00.000Z\",\"message\":{\"role\":\"user\",\"content\":\"$4\"}}" \
        "{\"type\":\"assistant\",\"timestamp\":\"$3T10:05:00.000Z\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"$5\"}]}}" \
        > "$dir/$2.jsonl"
}
filler=$(printf 'The plan is to read the code first, change one thing at a time and run the tests after every step. %.0s' $(seq 1 40))
chat myapp 11111111-1111-4111-8111-111111111111 "$(day 0)" "Why does the login page flash before it redirects?" "$filler"
chat myapp 22222222-2222-4222-8222-222222222222 "$(day 1)" "Add a dark mode switch to the settings page" "$filler"
chat myapp 33333333-3333-4333-8333-333333333333 "$(day 9)" "Review the database migration before we release" "$filler$filler"
chat notes-cli 44444444-4444-4444-8444-444444444444 "$(day 3)" "Make the search command faster on big folders" "$filler$filler$filler"
chat notes-cli 55555555-5555-4555-8555-555555555555 "$(day 20)" "Write a man page for the export command" "$filler"
printf '[sessions]\nenabled = true\n' >> "$conf/config.toml"

# a preset saved before two of the chats existed, so those show as NEW
cat > "$conf/contexts.json" <<'JSON'
{"default": {"selected": ["11111111-1111-4111-8111-111111111111", "44444444-4444-4444-8444-444444444444"],
             "known": ["11111111-1111-4111-8111-111111111111", "33333333-3333-4333-8333-333333333333",
                       "44444444-4444-4444-8444-444444444444"]}}
JSON
shoot context-saved-preset.png --context
shoot context-new-preset.png --context another-job

# --- a project profile: a made-up source tree, presets that tick code, chats and research -----
src="$home/src/myapp"
mkdir -p "$src/src/gui" "$src/src/store"
printf 'fn main() {\n    myapp::run();\n}\n' > "$src/src/main.rs"
printf '// the settings window\n' > "$src/src/gui/settings.rs"
printf '// the login page\n' > "$src/src/gui/login.rs"
printf '// the database layer\n' > "$src/src/store/db.rs"
printf '[package]\nname = "myapp"\n' > "$src/Cargo.toml"
printf '\n[profiles.myapp]\nroot = "%s"\ncode = ["src/*", "Cargo.toml"]\n' "$src" >> "$conf/config.toml"
cat > "$conf/profiles.json" <<'JSON'
{"myapp": {"presets": {
  "full": {"code": ["Cargo.toml", "src/gui/login.rs", "src/gui/settings.rs", "src/main.rs", "src/store/db.rs"],
           "chats": [], "research": ["myapp/notes", "myapp/roadmap"]},
  "gui": {"code": ["src/gui/login.rs", "src/gui/settings.rs"],
          "chats": ["11111111-1111-4111-8111-111111111111", "22222222-2222-4222-8222-222222222222"],
          "research": ["myapp/notes"]}}}}
JSON
shoot profile.png --profile myapp gui
