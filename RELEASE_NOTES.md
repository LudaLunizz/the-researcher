The first load now asks where your research is.

- **First use: choose your research folder.** When no library is set yet, a small window asks for the folder that holds your research, or the folder where you will make or copy it from now on. Type a path or press **Choose folder…** for the system folder dialog (the desktop portal on Linux, no GTK). A folder that does not exist yet is made. The choice is saved as `library` in `config.toml` (a starter config is written when there is none; an existing one keeps its comments), and the chooser opens right after. Without a display, or with `--no-popup`, nothing is asked and you get `NO LIBRARY` as before.
- **Empty library:** a clear `EMPTY LIBRARY` message instead of an empty chooser.
- **New README screenshots** of the egui windows, made with `tools/screenshots.sh` (replaces the GTK `screenshots.py`).
- **Fix:** with nothing ticked, the popup total showed `~-0k tokens`.
- Linux x86_64 only for now. PDFs still need `pdftotext` (poppler-utils).

Install: download `researcher-v0.2.0-x86_64-linux`, check it against `SHA256SUMS.txt`, and put it on your PATH as `researcher`:

```bash
sha256sum -c SHA256SUMS.txt
install -m 755 researcher-v0.2.0-x86_64-linux ~/.local/bin/researcher
```

Or build it yourself with `cargo install --locked --path .`. Then install the skill as described in the README.
