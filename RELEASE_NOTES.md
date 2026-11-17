/load-context and --sessions now work for opencode chats (issue #70, PR #71).

- **`[sessions] opencode = true`** lists opencode's chats next to Claude Code's in `--sessions`, `--context`, `--rename`, `--session-load` and `--resume`, in the Rust and the Python version. The researcher asks the `opencode` program (`session list`, `export`) and keeps each chat as a plain transcript in `sessions-opencode/` next to `sessions.json`; a chat is exported once and again only after it changed (four at a time). Set `opencode_cmd` when the program is not on PATH, for example inside a sandbox.
- opencode's own title is the picker label; `--resume` prints `cd <folder> && opencode -s <id>`; ids are matched without regard to case; inside opencode the chat you run it from is left out. A program that cannot be run prints `OPENCODE SKIPPED (...)` and the Claude Code chats still load.
- New `opencode/commands/load-context.md` for `/load-context` in opencode; README, skill and example config describe it. Off unless you switch it on.
- Linux x86_64 only for now. PDFs still need `pdftotext` (poppler-utils).

Install: download `researcher-v0.5.5-x86_64-linux`, check it against `SHA256SUMS.txt`, and put it on your PATH as `researcher`:

```bash
sha256sum -c SHA256SUMS.txt
install -m 755 researcher-v0.5.5-x86_64-linux ~/.local/bin/researcher
```

Or build it yourself with `cargo install --locked --path .`. Then install the skills as described in the README.
