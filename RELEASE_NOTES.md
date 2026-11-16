Every use is counted, and the squeezed number includes reading text instead of the HTML or PDF original (issue #46, PR #47). Same in the Python version.

- **Chats loaded as context are uses too.** `--context` and `--session-load` now write a usage line and, when sharing is on, send it in the background, like a library load. Listing, setting up and cancelled popups load nothing and log nothing. A chat is read as it is written, so it adds no squeeze and no conversion saving; the chats you left unticked count as not loaded.
- **Tokens squeezed now includes the conversion saving.** Reading the converted text instead of the original costs fewer tokens: each loaded HTML character saves 0.73 of a text character and each PDF character 1.2 (raw HTML is 1.73x its text, a PDF read as a document 2.2x; measured on 85 reports, see the README). Markdown, text files and chats save nothing there. `researcher --usage` has a new `converted:` line.
- **What changed in a line:** records are version 2: `fmt` also counts `chat`, and `fmt_chars` holds the loaded characters per format. It is still only numbers (no titles, paths, names or chat text). `researcher --usage-send --dry-run` shows exactly what goes. Version 1 lines (older installs, earlier loads) still read and send, and add only their squeeze part.
- **The receiver** (`worker/`) accepts both versions. An existing database needs `worker/migrations/0002_formats.sql` once; see `worker/README.md`.
- Linux x86_64 only for now. PDFs still need `pdftotext` (poppler-utils).

Install: download `researcher-v0.5.0-x86_64-linux`, check it against `SHA256SUMS.txt`, and put it on your PATH as `researcher`:

```bash
sha256sum -c SHA256SUMS.txt
install -m 755 researcher-v0.5.0-x86_64-linux ~/.local/bin/researcher
```

Or build it yourself with `cargo install --locked --path .`. Then install the skills as described in the README.
