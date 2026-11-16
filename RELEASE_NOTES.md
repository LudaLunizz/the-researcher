Five bug fixes (issue #31, PR #32). Same fixes in the Python version.

- **Set names keep their case:** `[sets.MyRust]` in the config is now found by `researcher MyRust` (or `myrust`); before, the name was lowercased and treated as a new custom set.
- **No more shared text copies:** two documents whose ids give the same file name (`a/b` and `a - b`) get two copies; the second one's name carries a short hash, e.g. `a - b (f0551209).txt`. Before, one document was loaded twice and the other was lost.
- **No stale copies:** the source is checked before it is converted, so a file that changes during conversion is converted again on the next run.
- **Two runs at once are safe:** temporary files carry the process id, so two loads at the same time no longer write into one `.tmp` file.
- **Shorter output:** the `left out by choice` line only counts the documents when there are more than 20 (custom sets and `all`) instead of naming the whole library.
- Linux x86_64 only for now. PDFs still need `pdftotext` (poppler-utils).

Install: download `researcher-v0.2.2-x86_64-linux`, check it against `SHA256SUMS.txt`, and put it on your PATH as `researcher`:

```bash
sha256sum -c SHA256SUMS.txt
install -m 755 researcher-v0.2.2-x86_64-linux ~/.local/bin/researcher
```

Or build it yourself with `cargo install --locked --path .`. Then install the skill as described in the README.
