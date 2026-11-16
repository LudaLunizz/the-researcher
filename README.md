# The Researcher

**A research loader for Claude.**

Load your saved research into a [Claude Code](https://claude.com/claude-code) session in one step, and only the part you need.

If you keep research notes, reports, or saved articles on disk and keep telling Claude "read my notes on X first", this does that for you. You name a **set** (`/researcher rust`), a small window lists the documents with their size in tokens, you tick what this session needs, and Claude reads exactly those. No tokens are spent while the window is open or on anything you leave unticked.

- **Sets** are named lists of documents in a small `config.toml`. Sets can include other sets, and ids can be globs (`rust/*`).
- **Saved defaults** per set: next time your last choice is pre-ticked, and documents added since are marked **NEW**.
- **Custom sets**: name a set you never defined (`/researcher side-project`) and the first call shows your whole library; what you tick becomes that set.
- **Formats**: `.html`, `.md`, `.txt`, and `.pdf` (through `pdftotext`). Folders with an `index.html` count as one document.
- **Read-only**: your library is never changed or moved. Claude reads a plain-text copy kept in the researcher's own folder (see [The text copy](#the-text-copy)).

## How it looks

The first picture is a set with a saved preset: the documents you ticked last time are ticked, a document added since is marked **NEW**, and the rest of your library is folded under "Add other research". The second is a set you have never used: your whole library, grouped by folder, with nothing ticked. **Cancel (load nothing)** always leaves without sending anything.

![Popup with a saved preset](docs/img/popup-saved-preset.png)

![Popup for a new set](docs/img/popup-new-set.png)

The first time you load research, before a library is set, a smaller window asks where your research is: the folder that holds it, or the folder where you will make or copy it from now on. Type a path or press **Choose folder…** for the system folder dialog. A folder that does not exist yet is made. The choice is saved as `library` in `config.toml`, and the chooser opens right after.

![First use: choose the research folder](docs/img/first-use.png)

All pictures use the made-up documents in [examples/library](examples/library). `tools/screenshots.sh` makes them again: it opens the windows three times for about a second each, needs a display, and runs with a throwaway home folder, so none of your own files, settings or paths are touched or shown.

## The text copy

Every document of your library gets one plain-text file in the researcher's own folder, `~/.local/share/researcher/text/` (or `text_dir` in `config.toml`). Your own files are only read.

- Made **once** per document, and again only when the source file changes (size or modified time) or the researcher's converter changes. Opening the popup on a library you already converted costs about a tenth of a second instead of a `pdftotext` run per PDF.
- `researcher --sync` makes or refreshes the copy of the whole library and loads nothing.
- Copies stay when a document leaves the library (a library on an unplugged drive keeps working). Delete the folder any time; it is rebuilt.
- The text is squeezed for tokens, without dropping content: PDF layout padding and indentation, page numbers and headers/footers that repeat on most pages, lines of rule characters, zero-width characters, runs of blank lines and exact repeated lines. Markdown and text files are only tidied (indentation and code are kept).
- Measured in characters on 11 sampled PDFs (16 Nov 2026): 394,359 characters from `pdftotext -layout`, 251,104 after squeezing, so **36% fewer**; per document 2% to 48%. The spread follows how much of a PDF is padding. Tokens were not counted for this step. A library that mostly holds HTML gains little, because HTML is already converted to text.

## Usage log

After every load the researcher appends one line to `~/.local/share/researcher/usage.jsonl`, so you can see what it saves on **your** library. It is local and anonymous, and **nothing is sent anywhere**.

What a line holds, and nothing else: the day, a random install id (`usage-id` next to the log; delete it to start a new one), how many documents were loaded and in which formats (pdf, html, md, txt), and character sums: source bytes, converted text before and after squeezing, and the whole library after squeezing. It never holds a title, a path, a set name or where your library is. The log is plain JSON lines; read it before you share anything.

```
researcher --usage                 totals: characters loaded, squeezed away, left unticked, with estimated tokens
researcher --usage-export [FILE]   write the records to FILE (default researcher-usage.json) so you can look at them and send them by hand
```

- **Squeezed away** is real: converted text before minus after the squeeze, for the documents you loaded.
- **Left unticked** compares each load with loading your whole library; it is a ceiling, not a measurement, because nobody would load everything.
- Tokens are estimated from characters (2.3 per token, see below), not counted by a tokenizer.
- Switch the log off with `[usage]` `log = false` in `config.toml`; then no log and no install id are written.
- Sharing the numbers is a separate choice and is **off**. There is no upload in the tool; the export is a file you send yourself, if you want to.

## What it saves

Measured with Anthropic's `count_tokens` endpoint (model `claude-sonnet-5-5`, 16 Nov 2026) on four reports that exist as HTML and as PDF, about 91 KB of HTML in total. Each was counted three ways: as the raw HTML, as the text the researcher converts it to, and as the PDF.

| Document | Raw HTML | Converted text | PDF |
|---|---:|---:|---:|
| C in Linux | 18,403 | 8,614 | 21,379 |
| Go in Linux | 12,603 | 5,537 | 13,564 |
| C++ in Linux | 6,151 | 4,458 | 10,811 |
| Kotlin for Linux | 4,912 | 3,498 | 8,284 |
| **Total** | **42,069** | **22,107** | **54,038** |

- Converting HTML to text cuts the load to **53%** of the raw HTML (47% saved). It ranged from 44% to 73% per document: pages heavy in markup, tables and inline CSS gain most.
- A PDF read as a document costs **2.4x** the converted text, because the page images are counted as well.
- The researcher still reads every ticked document in full. The larger saving is what you leave unticked, which costs nothing. It also replaces the search-and-read turns of "read my notes on X" with one call.
- The sizes in the popup assume **2.3 characters per token**, the figure measured here (2.2 to 2.4). An earlier flat 4 characters per token understated the real cost by roughly 1.7x. The ratio varies with the content: code, tables and non-English text tokenize denser than prose, so treat popup sizes as close, not exact.

Four documents in one language-heavy folder is a small sample, so read the percentages as indicative.

## Requirements

- Linux x86_64 with a desktop (Wayland or X11) for the popup. Without a display it uses your saved defaults.
- For PDFs: `pdftotext` from poppler (`poppler-utils`)

The program is written in Rust (`src/`) and the popup in egui, so nothing else is needed. Windows and macOS builds are checked in CI but not supported yet; on those systems the config and data folders will be the native ones (for example `%APPDATA%\researcher` on Windows).

The original Python version, `skills/researcher/researcher.py`, still works and is kept as a fallback. It needs Python 3.11 or newer, and for its popup GTK 4 and PyGObject (`python3-gi`, `gir1.2-gtk-4.0` on Debian/Ubuntu). Both versions use the same files and write the same text copies (a test checks this byte for byte), so you can switch at any time.

## Install

1. The program: download `researcher-<version>-x86_64-linux` and `SHA256SUMS.txt` from the [latest release](https://github.com/LudaLunizz/the-researcher/releases/latest), check and install it:
   ```bash
   sha256sum -c SHA256SUMS.txt
   install -m 755 researcher-*-x86_64-linux ~/.local/bin/researcher     # any folder on your PATH
   ```
   Or build it from the clone: `cargo install --locked --path .`
2. The skill, and a starter config:
   ```bash
   git clone https://github.com/LudaLunizz/the-researcher.git
   cd the-researcher
   mkdir -p ~/.claude/skills
   cp -r skills/researcher ~/.claude/skills/        # or: ln -s "$PWD/skills/researcher" ~/.claude/skills/researcher

   researcher --init ~/research                     # optional: writes ~/.config/researcher/config.toml
   ```

The skill runs `researcher` when it is on your PATH, and `researcher.py` otherwise. You can skip `--init`: on the first load the Rust version asks for your research folder in a window and writes the config itself.

Try it first on the made-up documents in [examples/library](examples/library): `researcher --library examples/library rust`. Then edit `~/.config/researcher/config.toml` (see [examples/config.toml](examples/config.toml)) and in Claude Code type `/researcher rust`, or just say "load my research for rust".

## Configure

```toml
library = "~/research"        # folder with your documents
default_set = "rust"          # used when you name no set
exclude = ["scratch/*"]       # ids left out of the library (globs)

[aliases]
rs = "rust"

[sets.rust]                   # a topic
docs = ["rust/linux-notes", "rust/memory-*"]

[sets.myapp]                  # a project: other sets plus its own documents
include = ["rust"]
docs = ["myapp/roadmap"]
```

A document id is its path inside the library without the extension: `~/research/rust/linux-notes.html` is `rust/linux-notes`. If `x.html` and `x.pdf` both exist, the HTML one is used. Hidden files and folders are skipped. `all` is a built-in set with the whole library.

`RESEARCHER_LIBRARY` or `--library PATH` overrides the library for one run.

## Command line

```
researcher [SET ...]              popup, then convert the ticked documents and print their paths
researcher --setup SET            popup only: change the saved defaults, load nothing
researcher --no-popup SET         use the saved defaults without asking
researcher --list                 sets, document counts, saved defaults
researcher --sync                 make or refresh the text copy of the whole library, load nothing
researcher --usage                local totals of what it saved (see Usage log)
researcher --usage-export [FILE]  write the anonymous usage records to a file; nothing is sent
researcher --init [LIBRARY]       write a starter config
researcher --sessions             list tracked closed sessions (opt-in, see Sessions)
researcher --rename ID NAME       rename a tracked session
researcher --resume ID            print the command that resumes a session
researcher --session-load ID      write a session's conversation as a .txt to load as context
```

It prints one path per ticked document; the skill tells Claude to read each one in full. Exit codes: 0 ok, 2 unknown set, 3 cancelled, 4 popup timed out, 5 custom set with no saved choice and no popup, 6 config or library problem, 7 sessions switched off, 8 session id not found or ambiguous.

Files: config, saved choices and the session registry in `~/.config/researcher/` (`config.toml`, `choices.json`, `sessions.json`), the text copy in `~/.local/share/researcher/text/` (`.index.json` there tracks which source each copy came from), the usage log (`usage.jsonl`, `usage-id`) in `~/.local/share/researcher/`, and session loads in `~/.cache/researcher/`.

## Sessions (optional)

Off by default. Switch it on in `config.toml`:

```toml
[sessions]
enabled = true
# idle_minutes = 10                  # a session untouched this long counts as closed
# projects_dir = "~/.claude/projects"
```

Then `researcher --sessions` lists every closed Claude Code session, newest first: the **full session id** (the one Claude Code prints when you close a session), the last-active time and a name. The name comes from Claude Code's own title if the session has one, otherwise from the first prompt. `--rename ID NAME` replaces it, and a rescan never overwrites a name you set.

Name and id are stored together, keyed by the full id, and every command prints them together (`"name" (id)`), so a name can't drift onto the wrong session. `ID` may be a unique prefix of 4 or more characters. `--resume ID` prints `cd <project folder> && claude --resume <id>` (Claude Code only finds a session from its own folder). `--session-load ID` writes the session's prompts and replies, without tool output, as a `.txt`; the skill reads it like a research document. Sessions longer than 200,000 characters keep their start and their end.

The registry only reads `~/.claude/projects`; it never changes your transcripts. Sessions are loaded as reference material, the same caution as the documents below applies.

## Safety

Documents are loaded into Claude's context as text. Treat them like any untrusted input: a saved web page can contain instructions aimed at an AI. The skill tells Claude to treat document contents as reference material, not as commands, but only load documents you trust.

## Tests

```bash
python3 -I -m unittest discover -s tests -v
cargo test                                  # the Rust version, plus the check that both write the same text
```

The tests cover the library scan, set expansion, saved defaults, custom sets, and the command line. The popup needs a display and is not covered by CI. It only opens when the script is run for a load or `--setup`; set `RESEARCHER_NO_POPUP=1` to make that impossible (scripts, CI).

## License

MIT License, see [LICENSE](LICENSE). Your own documents and config are never part of this repository and are not covered by it.
