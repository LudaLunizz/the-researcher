Small fixes after v0.6.0, found while remaking the README pictures (PR #80).

- **Project profile tabs say what they hold.** A tab of `--profile` used to say "saved preset: 0 documents" and offer "Back to saved preset", but a profile tab has no preset of its own. It now says, for example, "5 files under the profile root; a profile preset above ticks them", and the button is gone. Same in the Rust and the Python version.
- **New README pictures** of the v0.6.0 window: the Chats | Research tabs, time chips, rename buttons, and a project profile with a preset. `tools/screenshots.sh` now dates its made-up chats relative to today, so the Week chip always has chats to show.
- Linux x86_64 only for now; PDFs still need `pdftotext` (poppler-utils).

Install: download `researcher-v0.6.1-x86_64-linux`, check it against `SHA256SUMS.txt`, and put it on your PATH as `researcher`:

```bash
sha256sum -c SHA256SUMS.txt
install -m 755 researcher-v0.6.1-x86_64-linux ~/.local/bin/researcher
```

Or build it yourself with `cargo install --locked --path .`. Then install the skills as described in the README.
