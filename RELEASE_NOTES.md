One window for chats and research, time chips, naming, and project profiles (issue #77, PR #78). Also in this release: HTML and Markdown comments are dropped before the text reaches the model (PR #76), and CI audits Cargo.lock (PR #74).

- **Chats | Research tabs in one window.** `researcher SET` opens on Research, `researcher --context` opens on Chats, and `--start code|chats|research` picks the tab. One Load button loads what is ticked in all tabs, and the tab the window opened on prints first. The other tab loads nothing until you click it (then it shows "loading…"), so a research load never waits for a chat scan. Without a window only the command's own side loads.
- **Time chips.** Day | Week | Month | All (24 hours, 7 days, 30 days) with counts, and "Group by: time | folder" (Today, Yesterday, Earlier this week, Earlier this month, Older). A ticked row the chip hides is still loaded. The last choice per tab is kept in `ui.json`.
- **Names.** Rename a chat in the window (or `--rename ID NAME`; an empty name gives the automatic name back). Documents and research sets get display names in `names.json` (your files are never touched), shown as `DOC "name" (id)` before the path. A chat preset can be renamed next to Remember, which moves its saved ticks.
- **Project profiles.** `researcher --profile NAME [PRESET]` opens Code | Chats | Research with preset buttons that tick files, chats and documents at once ("full", "gui", "libraries"). The window can save the ticks as a preset (`profiles.json`). `[profiles.NAME] root` and `code` globs in the config define the Code tab. Source files print as `CODE id` followed by their path; Claude reads them from disk, and code is not counted in the usage log.
- **HTML and Markdown comments** (`<!-- ... -->`) are dropped as a whole before the text reaches the model (TEXT_VERSION 3, so text copies are made again once).
- The Rust and the Python version stay in parity, with tests in both, including a headless test of the real window. Linux x86_64 only for now; PDFs still need `pdftotext` (poppler-utils).

Install: download `researcher-v0.6.0-x86_64-linux`, check it against `SHA256SUMS.txt`, and put it on your PATH as `researcher`:

```bash
sha256sum -c SHA256SUMS.txt
install -m 755 researcher-v0.6.0-x86_64-linux ~/.local/bin/researcher
```

Or build it yourself with `cargo install --locked --path .`. Then install the skills as described in the README.
