The Researcher now works in opencode as well (issue #59, PR #60).

- **`/researcher SET` in opencode**: a command file, `opencode/commands/researcher.md`, next to the existing skill. Same program, config, library, text copy and usage log as in Claude Code.
- `SKILL.md` no longer names Claude Code tools (file-reading tool, shell timeout, the folder of the SKILL.md for the Python fallback).
- README: how to install it for opencode, a permission rule so opencode does not ask before reading the researcher's own folders, and what is not available there (`/load-context` and `--sessions` read Claude Code's session files).
- The program itself is unchanged since v0.5.1.
- Linux x86_64 only for now. PDFs still need `pdftotext` (poppler-utils).

Install: download `researcher-v0.5.2-x86_64-linux`, check it against `SHA256SUMS.txt`, and put it on your PATH as `researcher`:

```bash
sha256sum -c SHA256SUMS.txt
install -m 755 researcher-v0.5.2-x86_64-linux ~/.local/bin/researcher
```

Or build it yourself with `cargo install --locked --path .`. Then install the skills as described in the README.
