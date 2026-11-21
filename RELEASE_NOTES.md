Ten bug fixes from a review (issue #89, PR #90). Same in the Rust and the Python version unless it says otherwise.

- **Your library stays read-only.** A text folder that does not exist yet inside a symlinked library is now refused with `CONFIG ERROR` (Rust wrote its copies into the library).
- **Saved choices are kept.** A state file that exists but cannot be read (permissions, I/O) is kept as `NAME.bad` instead of being overwritten on the next save.
- **Code keeps its lines.** Markdown and text files keep repeated lines (`count += 1` twice stays twice); only PDF and HTML drop exact repeats.
- **No lost HTML text.** A bare `<` (`x < 3 and y > 2`) is text, not the start of a tag.
- **No hang in the Code tab.** Named pipes and sockets under a profile root are skipped.
- **Chats:** a pasted prompt counts as your prompt (it was dropped from loads and names); Claude Code's compaction summaries are no longer loaded as `USER:` text; a saved chat that is running right now is no longer reported as `MISSING`.
- **Python version:** `--config c.toml` (a bare file name) saves again.
- **Usage receiver (Worker):** badges show `1M` instead of `1000k` just under a million.
- Every text copy is made once more on the first load (TEXT_VERSION 5).
- Linux x86_64 only for now; PDFs still need `pdftotext` (poppler-utils).

Install: download `researcher-v0.6.3-x86_64-linux`, check it against `SHA256SUMS.txt`, and put it on your PATH as `researcher`:

```bash
sha256sum -c SHA256SUMS.txt
install -m 755 researcher-v0.6.3-x86_64-linux ~/.local/bin/researcher
```

Or build it yourself with `cargo install --locked --path .`. Then install the skills as described in the README.
