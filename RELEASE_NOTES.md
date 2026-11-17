The skill describes the popup only from the program's output (issue #67, PR #68).

- **`SKILL.md`**: the model used to announce "the popup is open" before running the loader, and then said "no popup shown" or "loaded the defaults directly" after you had pressed Load, because it cannot see the window. It now announces only the start and reports what happened from the output lines: document paths = Load pressed, `SAVED default for ...` = the choice was kept, `NO POPUP` / `CANCELLED` / `TIMEOUT` as before.
- The program itself is unchanged since v0.5.3.
- Linux x86_64 only for now. PDFs still need `pdftotext` (poppler-utils).

Install: download `researcher-v0.5.4-x86_64-linux`, check it against `SHA256SUMS.txt`, and put it on your PATH as `researcher`:

```bash
sha256sum -c SHA256SUMS.txt
install -m 755 researcher-v0.5.4-x86_64-linux ~/.local/bin/researcher
```

Or build it yourself with `cargo install --locked --path .`. Then install the skills as described in the README.
