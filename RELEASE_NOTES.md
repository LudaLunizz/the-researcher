The first release of The Researcher as one program: a Rust port of `researcher.py`.

- **Same tool, one binary.** The command line, output, exit codes and files are the same as the Python version (config, saved choices, the text copy, the usage log, sessions). Both write byte-identical text copies, so you can switch between them at any time.
- **New popup.** Built with egui instead of GTK 4: nothing extra to install on Linux, and it works on Wayland and X11.
- **`/researcher` runs the binary** when it is on your PATH, and falls back to `researcher.py` otherwise.
- **Linux x86_64 only for now.** Windows and macOS builds are checked in CI but not supported yet.
- PDFs still need `pdftotext` (poppler-utils).

Install: download `researcher-v0.1.0-x86_64-linux`, check it against `SHA256SUMS.txt`, and put it on your PATH as `researcher`:

```bash
sha256sum -c SHA256SUMS.txt
install -m 755 researcher-v0.1.0-x86_64-linux ~/.local/bin/researcher
```

Or build it yourself with `cargo install --locked --path .`. Then install the skill as described in the README.
