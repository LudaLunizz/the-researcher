A chat load now saves what catching up on every past chat would have cost (issue #56, PR #57). Same in the Python version.

- **Chats loaded as context are compared with every past chat up to the newest one you picked** (last active at or before it), minus the chats you loaded. Before, they were compared with every chat in the picker, newer ones included. It applies to `--context` and `--session-load`, and counts in tokens like the other numbers.
- README: pictures of the `/load-context` window (issue #54, PR #55).
- Records already sent are not changed; nothing else in the usage format changed.
- Linux x86_64 only for now. PDFs still need `pdftotext` (poppler-utils).

Install: download `researcher-v0.5.1-x86_64-linux`, check it against `SHA256SUMS.txt`, and put it on your PATH as `researcher`:

```bash
sha256sum -c SHA256SUMS.txt
install -m 755 researcher-v0.5.1-x86_64-linux ~/.local/bin/researcher
```

Or build it yourself with `cargo install --locked --path .`. Then install the skills as described in the README.
