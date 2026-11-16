Load earlier chats as context: `/load-context` (issue #37, PR #38). Same in the Python version.

- **`researcher --context [PRESET]`** opens the chooser with your earlier Claude Code chats from every project folder. Each chat is named by its **first prompt** (or the name you gave it with `--rename`). The most recently used folder comes first, newest chat first, and each row shows its size in tokens and the day it was last active. A filter box finds a chat by words from its first prompt or its folder.
- **Presets** work like custom sets and are stored in `~/.config/researcher/contexts.json`. The first time, nothing is ticked; tick what you need and keep Remember ticked. The next run starts with those chats ticked, and newer chats show NEW but are never ticked on their own. Without a name the preset is `default`. `--setup`, `--list` and `--no-popup` work as they do for sets.
- The chat you run it from is never listed; a chat closed a minute ago is. Each ticked chat is loaded as prompts and replies only, without tool calls or their output, like `--session-load`.
- **New skill `skills/load-context`**: install it next to `researcher` (`cp -r skills/load-context ~/.claude/skills/`) and type `/load-context NAME` in Claude Code.
- Needs `[sessions] enabled = true` in your config, like the other session commands. The session registry now keeps each chat's first prompt and size, so the window opens fast after the first scan.
- Linux x86_64 only for now. PDFs still need `pdftotext` (poppler-utils).

Install: download `researcher-v0.4.0-x86_64-linux`, check it against `SHA256SUMS.txt`, and put it on your PATH as `researcher`:

```bash
sha256sum -c SHA256SUMS.txt
install -m 755 researcher-v0.4.0-x86_64-linux ~/.local/bin/researcher
```

Or build it yourself with `cargo install --locked --path .`. Then install the skills as described in the README.
