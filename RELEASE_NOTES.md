A security policy, no code changes (issue #39, PR #42). The program is the same as v0.4.0; only the version number differs.

- **`SECURITY.md`:** how to report a vulnerability privately (the Security tab, "Report a vulnerability"), what to include, that only the latest release gets fixes, about 90 days to fix a verified problem, what is in scope (the binary and `researcher.py`, the release files, the usage Worker), and that a document with instructions aimed at an AI is a known limit described in the README, not a vulnerability by itself.
- Linux x86_64 only for now. PDFs still need `pdftotext` (poppler-utils).

Install: download `researcher-v0.4.1-x86_64-linux`, check it against `SHA256SUMS.txt`, and put it on your PATH as `researcher`:

```bash
sha256sum -c SHA256SUMS.txt
install -m 755 researcher-v0.4.1-x86_64-linux ~/.local/bin/researcher
```

Or build it yourself with `cargo install --locked --path .`. Then install the skills as described in the README.
