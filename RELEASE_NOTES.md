Fixes from a review of the whole program (issue #84, PR #85). Same in the Rust and the Python version unless it says otherwise.

- **No more lost text in copies.** A `<!--` inside a `<script>`, or inside Markdown code, was read as an unclosed comment and everything after it was dropped from the copy. Comments and scripts are now removed in one pass, and Markdown only drops closed comments (or an unclosed one that starts a line). Every text copy is made once more on the first load.
- **Your own files are safe.** A `text_dir` (or `--out`) inside the library is refused with `CONFIG ERROR` instead of overwriting your files.
- **Saved choices are kept.** Defaults, chat presets and renames are read again just before saving, so a save made by another window meanwhile stays. A state file that cannot be read is kept as `NAME.bad` instead of being wiped. A symlinked config stays a link and keeps its permissions.
- **Long file names** get a shortened copy name with a hash instead of failing as UNREADABLE.
- **Window:** "Select shown" leaves a folded "other chats" list alone; leaving a rename unchanged saves nothing; "Back to saved preset" skips missing documents; the time limit counts from the start, so a slow first scan cannot run past it (Rust window).
- **Chats:** inside opencode the chat you type in is never offered; chat dates are your local day; a profile preset without a window keeps your window settings, reports missing chats and no longer crashes on an unknown one; `--session-load` counts only chats that can be loaded.
- **Python version:** no crash on file names or PDF text that are not UTF-8; config values of the wrong type give `CONFIG ERROR`; an empty library exits 6; failed writes print a message instead of a traceback.
- **Usage receiver (Worker):** stricter limits on each record, so one request cannot inflate the shared numbers.
- Linux x86_64 only for now; PDFs still need `pdftotext` (poppler-utils).

Install: download `researcher-v0.6.2-x86_64-linux`, check it against `SHA256SUMS.txt`, and put it on your PATH as `researcher`:

```bash
sha256sum -c SHA256SUMS.txt
install -m 755 researcher-v0.6.2-x86_64-linux ~/.local/bin/researcher
```

Or build it yourself with `cargo install --locked --path .`. Then install the skills as described in the README.
