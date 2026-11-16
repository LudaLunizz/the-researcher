Token estimates now use 2.4 characters per token.

- **2.4 characters per token** (was 2.3): measured with Anthropic's `count_tokens` on 50 documents. The popup sizes, the `~k tokens` totals and the `tk` field of new usage records use it. Older usage records keep the value they were written with.
- **README:** the "What it saves" numbers now come from 50 documents instead of 4: converted text is 58.7% of the raw HTML, a PDF costs 2.2x the converted text.
- Linux x86_64 only for now. PDFs still need `pdftotext` (poppler-utils).

Install: download `researcher-v0.2.1-x86_64-linux`, check it against `SHA256SUMS.txt`, and put it on your PATH as `researcher`:

```bash
sha256sum -c SHA256SUMS.txt
install -m 755 researcher-v0.2.1-x86_64-linux ~/.local/bin/researcher
```

Or build it yourself with `cargo install --locked --path .`. Then install the skill as described in the README.
