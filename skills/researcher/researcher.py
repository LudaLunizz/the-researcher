#!/usr/bin/env python3
"""Researcher for Claude: load saved research documents into a Claude Code session.

Usage: researcher.py [SET ...]             popup, then write the ticked docs as .txt files
       researcher.py --setup SET [...]     popup only, to change the saved defaults
       researcher.py --no-popup SET [...]  use the saved defaults, ask nothing
       researcher.py --list                show sets, saved defaults, library size
       researcher.py --init [LIBRARY]      write a starter config.toml

A "library" is a folder of documents (.html, .md, .txt, .pdf). A "set" is a named list
of documents from it, defined in config.toml. A GTK 4 popup lets the user tick what to
load, so no tokens are spent on documents that are not needed. The ticked documents are
converted to plain text in a cache folder and their paths are printed, so the skill can
Read them in full. Nothing in the library is ever changed.

Sets:
- defined sets: `[sets.NAME]` in config.toml with `docs` (ids or globs like "rust/*")
  and `include` (other sets);
- `all`: every document in the library;
- custom sets: any other name made of lowercase letters, digits, `.`, `_`, `-`. The first
  call lists the whole library with nothing ticked; what the user ticks is saved as that
  set. Later calls show the chosen docs ticked, the rest folded away, and documents added
  since marked NEW (never ticked on their own).

A document id is its path inside the library without the extension (`rust/memory`). A
folder that holds an `index.html` / `index.md` counts as one document named after the
folder. When `x.html` and `x.pdf` both exist, the HTML one is used.
"""
import argparse
import fnmatch
import html
import json
import os
import re
import subprocess
import sys

try:
    import tomllib
except ImportError:  # Python < 3.11
    tomllib = None

APP = "researcher"
DOC_EXTS = (".html", ".htm", ".md", ".txt", ".pdf")  # preference order when names collide
INDEX_NAMES = tuple("index" + e for e in DOC_EXTS)
POPUP_TIMEOUT = 540  # seconds; the skill runs this with a 600 s Bash timeout
NAME_RE = re.compile(r"[a-z0-9][a-z0-9._-]*")
RESERVED = {"all"}
CHARS_PER_K = 4000  # ~4 chars per token


class ConfigError(Exception):
    pass


# ---------------------------------------------------------------- paths and config

def config_home():
    base = os.environ.get("XDG_CONFIG_HOME") or os.path.expanduser("~/.config")
    return os.path.join(base, APP)


def cache_home():
    base = os.environ.get("XDG_CACHE_HOME") or os.path.expanduser("~/.cache")
    return os.path.join(base, APP)


def load_config(path):
    """config.toml as a dict; {} when the file does not exist."""
    try:
        with open(path, "rb") as f:
            raw = f.read()
    except FileNotFoundError:
        return {}
    if tomllib is None:
        raise ConfigError("Python 3.11 or newer is needed to read config.toml")
    try:
        return tomllib.loads(raw.decode("utf-8"))
    except (tomllib.TOMLDecodeError, UnicodeDecodeError) as e:
        raise ConfigError(f"{path}: {e}")


def library_path(cfg, override=None):
    p = override or os.environ.get("RESEARCHER_LIBRARY") or cfg.get("library")
    return os.path.abspath(os.path.expanduser(p)) if p else None


STARTER = """\
# Researcher for Claude: config
# The folder that holds your research documents (.html, .md, .txt, .pdf).
library = "{library}"

# Used when /researcher is run without a set name.
# default_set = "main"

# Short names for sets.
# [aliases]
# rs = "rust"

# A set is a list of document ids (path inside the library, no extension).
# Globs work: "rust/*". `include` pulls in other sets.
# [sets.rust]
# docs = ["rust/linux-notes", "rust/memory-*"]
#
# [sets.myapp]
# include = ["rust"]
# docs = ["myapp/roadmap"]
"""


def init_config(path, library):
    if os.path.exists(path):
        print(f"{path} already exists; not overwriting")
        return 1
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(STARTER.format(library=library.replace("\\", "\\\\").replace('"', '\\"')))
    print(f"wrote {path}")
    return 0


# ---------------------------------------------------------------- the library

def scan(library):
    """{doc id: file path} for every document under `library`."""
    found = {}  # id -> (preference, path)

    def offer(doc_id, path, ext):
        pref = DOC_EXTS.index(ext)
        if doc_id not in found or pref < found[doc_id][0]:
            found[doc_id] = (pref, path)

    for root, dirs, files in os.walk(library):
        dirs[:] = sorted(d for d in dirs if not d.startswith("."))
        rel = os.path.relpath(root, library)
        rel = "" if rel == "." else rel.replace(os.sep, "/")
        index = next((n for n in INDEX_NAMES if n in files), None)
        if index and rel:  # a folder with an index file is one document
            offer(rel, os.path.join(root, index), os.path.splitext(index)[1])
            dirs[:] = []
            continue
        for fn in sorted(files):
            stem, ext = os.path.splitext(fn)
            if fn.startswith(".") or ext.lower() not in DOC_EXTS:
                continue
            path = os.path.join(root, fn)
            if os.path.isfile(path):
                offer(f"{rel}/{stem}" if rel else stem, path, ext.lower())
    return {k: v[1] for k, v in sorted(found.items())}


def html_text(raw):
    t = re.sub(r"(?is)<(script|style|svg)\b.*?</\1>", " ", raw)
    t = re.sub(r"(?i)<br\s*/?>|</(p|div|li|tr|h[1-6]|section|table)>", "\n", t)
    t = re.sub(r"(?i)<(td|th)\b[^>]*>", " | ", t)
    t = re.sub(r"(?i)<li\b[^>]*>", "- ", t)
    t = re.sub(r"<[^>]+>", " ", t)
    t = html.unescape(t)
    t = re.sub(r"[ \t\r\f\v]+", " ", t)
    t = re.sub(r"\n\s*\n+", "\n\n", t)
    return "\n".join(line.strip() for line in t.splitlines()).strip()


def to_text(path):
    """Plain text of one document. Raises OSError / RuntimeError when unreadable."""
    ext = os.path.splitext(path)[1].lower()
    if ext == ".pdf":
        try:
            r = subprocess.run(["pdftotext", "-layout", path, "-"], capture_output=True,
                               text=True, check=True)
        except FileNotFoundError:
            raise RuntimeError("pdftotext not found (install poppler-utils)")
        except subprocess.CalledProcessError as e:
            raise RuntimeError(f"pdftotext failed: {e.stderr.strip() or e.returncode}")
        return r.stdout
    with open(path, encoding="utf-8", errors="replace") as f:
        raw = f.read()
    return html_text(raw) if ext in (".html", ".htm") else raw


# ---------------------------------------------------------------- sets

def expand(name, defs, catalog, stack=()):
    """Document ids of a configured set: includes first, then its own docs, globs expanded."""
    if name in stack:
        raise ConfigError("sets include each other in a loop: " + " -> ".join(stack + (name,)))
    spec = defs[name]
    if not isinstance(spec, dict):
        raise ConfigError(f"[sets.{name}] must be a table")
    out = []
    for inc in spec.get("include", []):
        if inc not in defs:
            raise ConfigError(f"set '{name}' includes unknown set '{inc}'")
        out += expand(inc, defs, catalog, stack + (name,))
    for pat in spec.get("docs", []):
        hits = [d for d in catalog if fnmatch.fnmatchcase(d, pat)] if re.search(r"[*?\[]", pat) else []
        out += hits or [pat]  # no hit stays in the list and shows up as MISSING
    return list(dict.fromkeys(out))


def build_sets(cfg, catalog, conf, names):
    """(sets, kinds) for everything that may be asked for.

    kinds: 'set' (from config), 'all' (whole library) or 'custom' (user ticks from the library).
    """
    defs = cfg.get("sets", {})
    sets = {n: expand(n, defs, catalog) for n in defs}
    kinds = {n: "set" for n in defs}
    sets["all"], kinds["all"] = list(catalog), "all"
    for n in list(conf) + list(names):
        if n in sets:
            continue
        if n in conf and not conf[n].get("custom"):
            continue  # saved choice of a set that no longer exists
        if n not in conf and not NAME_RE.fullmatch(n):
            continue
        sets[n], kinds[n] = list(catalog), "custom"
    return sets, kinds


def sections_for(names, sets, kinds, conf, texts, setup):
    """One section per requested set: its docs, the pre-ticked ones, the NEW ones."""
    out = []
    for n in names:
        docs = sets[n]
        saved = conf.get(n)
        if kinds[n] == "custom":  # nothing ticked on first use, new docs never auto-ticked
            if saved is None:
                ticked, new, remember = set(), set(), True
            else:
                new = {d for d in docs if d not in set(saved.get("known", []))}
                ticked = set(saved.get("selected", [])) & set(docs)
                remember = bool(new) or setup
        elif saved is None:  # first use: everything ticked, remember it
            ticked, new, remember = set(docs), set(), True
        else:
            new = {d for d in docs if d not in set(saved.get("known", []))}
            ticked = (set(saved.get("selected", [])) | new) & set(docs)
            remember = bool(new) or setup
        out.append({"name": n, "kind": kinds[n], "docs": docs, "new": new,
                    "ticked": {d for d in ticked if d in texts},  # missing files cannot load
                    "remember": remember, "first": saved is None})
    return out


def resolve_names(argv_names, cfg):
    aliases = {k.lower(): v for k, v in cfg.get("aliases", {}).items()}
    names = [aliases.get(n.lower(), n.lower()) for n in argv_names]
    if not names:
        d = cfg.get("default_set")
        names = [d] if d else []
    return list(dict.fromkeys(names))


# ---------------------------------------------------------------- saved choices

def load_conf(path):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def save_conf(path, conf):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(conf, f, indent=1, ensure_ascii=False)
    os.replace(tmp, path)


# ---------------------------------------------------------------- popup

def popup(sections, texts, setup):
    """Show the chooser. Returns {set: (ticked set, remember)}, None if cancelled or "timeout".
    Raises RuntimeError when no display/GTK 4 is available."""
    try:
        import gi
        gi.require_version("Gtk", "4.0")
        from gi.repository import GLib, Gtk
    except (ImportError, ValueError) as e:
        raise RuntimeError(f"GTK 4 not available ({e})")
    if not Gtk.init_check():
        raise RuntimeError("no display")
    Gtk.Settings.get_default().set_property("gtk-application-prefer-dark-theme", True)
    esc = GLib.markup_escape_text
    loop = GLib.MainLoop()
    result = {"value": None}
    boxes = {}  # set name -> [(doc, CheckButton)]
    remember = {}

    def k(d):
        return len(texts.get(d, "")) / CHARS_PER_K

    win = Gtk.Window(title="Researcher" + (" - change defaults" if setup else ""))
    win.set_default_size(720, 760)
    outer = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
    for m in ("top", "bottom", "start", "end"):
        getattr(outer, f"set_margin_{m}")(14)
    head = Gtk.Label(xalign=0, wrap=True)
    head.set_markup("<b>Tick the research to load.</b>  Nothing is sent to Claude until you "
                    "press " + ("<b>Save</b>." if setup else "<b>Load</b>.")
                    + "  NEW = added since this set's default was saved.")
    outer.append(head)
    scroll = Gtk.ScrolledWindow(vexpand=True)
    body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
    scroll.set_child(body)
    outer.append(scroll)
    total = Gtk.Label(xalign=0)

    def chosen():
        return {d for lst in boxes.values() for d, cb in lst if cb.get_active()}

    def update(*_):
        c = chosen()
        total.set_markup(f"<b>{len(c)} docs, ~{sum(k(d) for d in c):.0f}k tokens</b>"
                         + ("" if setup else "  (to be loaded)"))

    for sec in sections:
        n = sec["name"]
        custom = sec["kind"] == "custom"
        note = "  · first use" if sec["first"] else (
            f"  · {len(sec['new'])} new" if sec["new"] else "")
        lbl = Gtk.Label(xalign=0)
        lbl.set_margin_top(10)
        lbl.set_markup(f"<big><b>{esc(n, -1)}</b></big>  <small>"
                       f"{'custom set' if custom else 'set'}, {len(sec['docs'])} docs{note}</small>")
        body.append(lbl)
        row = Gtk.Box(spacing=6)
        lst = []
        for text, fn in (("All", lambda l: [cb.set_active(cb.get_sensitive()) for _, cb in l]),
                         ("None", lambda l: [cb.set_active(False) for _, cb in l]),
                         ("Saved default", None)):
            b = Gtk.Button(label=text)
            if fn is None:
                b.connect("clicked", lambda _b, l=lst, s=sec["ticked"]:
                          [cb.set_active(d in s) for d, cb in l])
                if sec["first"]:
                    b.set_sensitive(False)
            else:
                b.connect("clicked", lambda _b, l=lst, f=fn: f(l))
            row.append(b)
        body.append(row)
        fold = None  # custom sets fold the unticked, not-new docs under "Other research"
        if custom and not sec["first"]:
            keep = [d for d in sec["docs"] if d in sec["ticked"] or d in sec["new"]]
            order = keep + [d for d in sec["docs"] if d not in keep]
            fold = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        else:
            keep, order = sec["docs"], sec["docs"]
        for d in order:
            target = fold if (fold is not None and d not in keep) else body
            cb = Gtk.CheckButton()
            l = Gtk.Label(xalign=0)
            if d in texts:
                extra = "  <span foreground='#ffcc33'><b>NEW</b></span>" if d in sec["new"] else ""
                l.set_markup(f"{esc(d, -1)}  <small>~{k(d):.0f}k</small>{extra}")
                cb.set_active(d in sec["ticked"])
            else:
                l.set_markup(f"<s>{esc(d, -1)}</s>  <span foreground='#ff6666'>MISSING</span>")
                cb.set_sensitive(False)
            cb.set_child(l)
            cb.set_margin_start(12)
            cb.connect("toggled", update)
            target.append(cb)
            lst.append((d, cb))
        if fold is not None and len(order) > len(keep):
            ex = Gtk.Expander(label=f"Other research ({len(order) - len(keep)})")
            ex.set_margin_start(12)
            ex.set_child(fold)
            body.append(ex)
        boxes[n] = lst
        rb = Gtk.CheckButton(label=f"Save this choice as the default for {n}")
        rb.set_active(sec["remember"])
        if setup:
            rb.set_sensitive(False)
        rb.set_margin_start(12)
        body.append(rb)
        remember[n] = rb

    outer.append(total)
    bar = Gtk.Box(spacing=8, halign=Gtk.Align.END)
    cancel = Gtk.Button(label="Cancel")
    ok = Gtk.Button(label="Save" if setup else "Load")
    ok.add_css_class("suggested-action")
    bar.append(cancel)
    bar.append(ok)
    outer.append(bar)
    win.set_child(outer)
    win.set_default_widget(ok)

    def finish(value):
        result["value"] = value
        win.destroy()
        loop.quit()

    def done(*_):
        finish({n: ({d for d, cb in lst if cb.get_active()}, remember[n].get_active())
                for n, lst in boxes.items()})

    cancel.connect("clicked", lambda *_: finish(None))
    ok.connect("clicked", done)
    win.connect("close-request", lambda *_: (finish(None), False)[1])
    keys = Gtk.EventControllerKey()
    keys.connect("key-pressed", lambda _c, kv, _kc, _s:
                 (finish(None), True)[1] if kv == 0xff1b else False)  # Escape
    win.add_controller(keys)
    GLib.timeout_add_seconds(POPUP_TIMEOUT, lambda: (finish("timeout"), False)[1])
    update()
    win.present()
    loop.run()
    return result["value"]


# ---------------------------------------------------------------- output

def write_outputs(out_dir, docs, texts, paths):
    """Write one .txt per document; only files this tool wrote earlier are removed first."""
    os.makedirs(out_dir, exist_ok=True)
    manifest = os.path.join(out_dir, ".manifest.json")
    try:
        with open(manifest, encoding="utf-8") as f:
            old = json.load(f)
    except (OSError, ValueError):
        old = []
    for fn in old:
        if os.path.basename(fn) == fn:  # never follow a path out of the folder
            try:
                os.remove(os.path.join(out_dir, fn))
            except OSError:
                pass
    written = []
    for d in docs:
        fn = d.replace("/", " - ") + ".txt"
        with open(os.path.join(out_dir, fn), "w", encoding="utf-8") as f:
            f.write(f"SOURCE: {paths[d]}\n\n{texts[d]}\n")
        written.append(fn)
    with open(manifest, "w", encoding="utf-8") as f:
        json.dump(written, f)
    return [os.path.join(out_dir, fn) for fn in written]


# ---------------------------------------------------------------- main

def main(argv=None):
    ap = argparse.ArgumentParser(prog="researcher.py", add_help=True,
                                 description="Load saved research documents into Claude Code.")
    ap.add_argument("sets", nargs="*", metavar="SET")
    ap.add_argument("--setup", action="store_true", help="change saved defaults, load nothing")
    ap.add_argument("--no-popup", action="store_true", help="use the saved defaults, ask nothing")
    ap.add_argument("--list", action="store_true", help="list sets and saved defaults")
    ap.add_argument("--init", nargs="?", const="", metavar="LIBRARY", help="write a starter config")
    ap.add_argument("--config", help="config.toml path (default ~/.config/researcher/config.toml)")
    ap.add_argument("--library", help="library folder (overrides config and RESEARCHER_LIBRARY)")
    ap.add_argument("--out", help="folder for the converted .txt files (default ~/.cache/researcher)")
    args = ap.parse_args(argv)

    cfg_path = args.config or os.path.join(config_home(), "config.toml")
    state_path = os.path.join(os.path.dirname(cfg_path), "choices.json")

    if args.init is not None:
        lib = args.init or os.path.expanduser("~/research")
        return init_config(cfg_path, lib)

    try:
        cfg = load_config(cfg_path)
        library = library_path(cfg, args.library)
        if not library:
            print(f"NO LIBRARY: set `library` in {cfg_path} (or run --init, or set "
                  "RESEARCHER_LIBRARY); nothing loaded")
            return 6
        if not os.path.isdir(library):
            print(f"NO LIBRARY: {library} is not a folder; nothing loaded")
            return 6
        paths = scan(library)
        conf = load_conf(state_path)
        names = resolve_names(args.sets, cfg)
        sets, kinds = build_sets(cfg, list(paths), conf, names)
    except ConfigError as e:
        print(f"CONFIG ERROR: {e}")
        return 6

    if args.list:
        print(f"library: {library} ({len(paths)} documents)")
        for n, docs in sets.items():
            if n == "all" or kinds[n] != "custom" or n in conf:
                saved = conf.get(n, {}).get("selected")
                sel = f", saved default {len(saved)}" if saved is not None else ""
                print(f"{n:20} {len(docs):>4} docs  ({kinds[n]}{sel})")
        if cfg.get("aliases"):
            print("aliases: " + ", ".join(f"{a}={b}" for a, b in cfg["aliases"].items()))
        return 0

    if not names:
        print("NO SET: name a set, or set `default_set` in the config; "
              "known: " + ", ".join(sets))
        return 2
    bad = [n for n in names if n not in sets]
    if bad:
        print(f"unknown set(s): {', '.join(bad)} (names use lowercase letters, digits, . _ -); "
              f"known: {', '.join(sets)}")
        return 2

    alldocs = list(dict.fromkeys(d for n in names for d in sets[n]))
    texts, missing = {}, []
    for d in alldocs:
        p = paths.get(d)
        if p is None:
            missing.append(f"MISSING  {d} (not in {library})")
            continue
        try:
            texts[d] = to_text(p)
        except (OSError, RuntimeError) as e:
            missing.append(f"UNREADABLE  {p} ({e})")
    secs = sections_for(names, sets, kinds, conf, texts, args.setup)

    picks = None
    if not args.no_popup:
        try:
            picks = popup(secs, texts, args.setup)
        except RuntimeError as e:
            print(f"NO POPUP ({e}): using the saved defaults")
        else:
            if picks is None:
                print("CANCELLED: the user closed the popup, nothing loaded")
                return 3
            if picks == "timeout":
                print(f"TIMEOUT: popup closed after {POPUP_TIMEOUT} s without a choice, "
                      "nothing loaded")
                return 4
    new_custom = [n for n in names if kinds[n] == "custom" and n not in conf]
    if picks is None and new_custom:
        print(f"NO POPUP: {', '.join(new_custom)} has no saved choice yet; nothing loaded")
        return 5
    if picks is None:  # --no-popup or no display: saved defaults, nothing saved
        picks = {s["name"]: (s["ticked"], False) for s in secs}

    for n, (ticked, keep) in picks.items():
        if keep or args.setup:
            conf[n] = {"selected": [d for d in sets[n] if d in ticked], "known": list(sets[n])}
            if kinds[n] == "custom":
                conf[n]["custom"] = True
            print(f"SAVED default for {n}: {len(ticked)} of {len(sets[n])} docs")
    if any(keep or args.setup for _, keep in picks.values()):
        save_conf(state_path, conf)
    if args.setup:
        print("setup only: nothing loaded")
        return 0

    chosen = {d for t, _ in picks.values() for d in t}
    docs = [d for d in alldocs if d in chosen and d in texts]
    written = write_outputs(args.out or cache_home(), docs, texts, paths)
    total = 0
    for d, dst in zip(docs, written):
        total += len(texts[d])
        print(f"{len(texts[d]):>7} chars  {dst}")
    for line in missing:
        print(line)
    skipped = [d for d in alldocs if d in texts and d not in chosen]
    if skipped:
        print(f"left out by choice ({len(skipped)}): " + "; ".join(skipped))
    print(f"total: {len(docs)} docs, {total} chars (~{total // CHARS_PER_K}k tokens)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
