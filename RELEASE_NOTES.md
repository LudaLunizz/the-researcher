Opt-in sharing of the anonymous usage counts (issue #34, PR #35). Same in the Python version.

- **Nothing is sent until you say yes.** While you have not decided, a load ends with `USAGE SHARING: not decided` and the skill asks you once. `researcher --usage-share on` saves `share = true` under `[usage]` in your config; `off` saves `share = false`. Only that one line of the config is touched.
- **What is sent:** the same anonymous lines the usage log already holds (day, a random install id, document counts per format, character sums), plus `seq`, the line's number in your log. Never a title, path, set name, library location, GitHub login or account. `researcher --usage-send --dry-run` prints exactly what would go.
- **When:** with sharing on, each load starts the sender in the background, so a load never waits on the network; a failed send is tried again after the next load. `researcher --usage-send` sends right away (exit 9 when sharing is not on). `researcher --usage` ends with the sharing state and how many lines wait.
- **Two identical loads on one day now count twice** (the receiver tells them apart by `seq`); a line sent twice is still stored once.
- **Your own receiver:** `[usage] url = "..."` sends to a copy of `worker/` on your own Cloudflare account instead.
- Linux x86_64 only for now. PDFs still need `pdftotext` (poppler-utils).

Install: download `researcher-v0.3.0-x86_64-linux`, check it against `SHA256SUMS.txt`, and put it on your PATH as `researcher`:

```bash
sha256sum -c SHA256SUMS.txt
install -m 755 researcher-v0.3.0-x86_64-linux ~/.local/bin/researcher
```

Or build it yourself with `cargo install --locked --path .`. Then install the skill as described in the README (the updated `SKILL.md` is what asks the sharing question).
