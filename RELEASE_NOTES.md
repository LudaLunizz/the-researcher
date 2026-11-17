The popup names the program that asked for the load (issue #62, PR #63).

- **Popup title and the "Nothing is sent to ..." line** say `opencode` when opencode started the loader (it sets `OPENCODE=1` for its tools), `Claude Code` otherwise. Before, they always said Claude Code. `RESEARCHER_HOST="name"` overrides the name. Rust and Python popups.
- README: a line about it in the opencode section.
- Nothing else changed since v0.5.2.
- Linux x86_64 only for now. PDFs still need `pdftotext` (poppler-utils).

Install: download `researcher-v0.5.3-x86_64-linux`, check it against `SHA256SUMS.txt`, and put it on your PATH as `researcher`:

```bash
sha256sum -c SHA256SUMS.txt
install -m 755 researcher-v0.5.3-x86_64-linux ~/.local/bin/researcher
```

Or build it yourself with `cargo install --locked --path .`. Then install the skills as described in the README.
