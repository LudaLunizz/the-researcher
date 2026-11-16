# Contributing

Issues and pull requests are welcome. This file is the setup and the exact
commands.

## Setup

| Directory | What |
|---|---|
| `src/` | the Rust program `researcher` (popup in egui) |
| `skills/researcher/` | the Claude Code skill, and `researcher.py`, the Python fallback |
| `skills/load-context/` | the `/load-context` skill |
| `tests/` | `cli.rs` (Rust, includes the parity test) and `test_researcher.py` |
| `worker/` | the Cloudflare Worker that receives opt-in usage counts |
| `examples/` | a made-up library and config to try things on |

You need a stable Rust toolchain with `clippy` and `rustfmt`, Python 3.11 or
newer, and for `worker/` Node 22.13 or newer. PDFs need `pdftotext` (poppler).

The Rust and Python versions must stay in step: same command line, same files,
same exit codes and byte-identical text copies. A change to one usually needs the
same change in the other; `tests/cli.rs` checks the text copies byte for byte.
If you change how text is made, bump `TEXT_VERSION` in **both** versions.

## Before you open a pull request

These are what CI runs (`.github/workflows/ci.yml`):

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
python3 -I -m unittest discover -s tests -v
(cd worker && npm test)              # only when worker/ changed
```

**Never open the popup from a test or script**: set `RESEARCHER_NO_POPUP=1`.
To try the first-use window safely, give it an empty config folder:
`XDG_CONFIG_HOME=$(mktemp -d) researcher rust`. The popup is not covered by CI,
so say in the pull request on which desktop you tried it.

## Conventions

- **Formatting** is `rustfmt` with `rustfmt.toml`.
- **No personal data in the repository**: no real research titles, paths or
  e-mail addresses in code, tests, screenshots or examples. Use the made-up
  library in `examples/`.
- **The usage log stays anonymous**: counts only, never names or paths, and
  nothing is sent unless the user turned sharing on. A change that sends more
  needs an issue first.
- **GitHub Actions are pinned to a full commit SHA** with the version in a
  comment; Dependabot proposes updates.
- Say in `RELEASE_NOTES.md` what a user sees.

## Issues and pull requests

Open an issue first for anything beyond a typo, saying what you want and why.
Work on a branch named `<issue number>-short-name`, and start the pull request
description with `Closes #<number>`. Wait for CI to be green; the maintainer
merges with a rebase, so the history stays linear. Security problems go to
**Security → Report a vulnerability**, not to a public issue (see
[SECURITY.md](SECURITY.md)).

Developed with help from Claude Code; every change is reviewed, tested in CI and
released by the maintainer.

## Releases

Releases are made by the maintainer: a version bump in `Cargo.toml` and
`Cargo.lock`, a new `RELEASE_NOTES.md`, a tag `v<version>` on main, and the
binary plus `SHA256SUMS.txt` built by `.github/workflows/release.yml` (it checks
that the tag matches `Cargo.toml`).
