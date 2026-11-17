#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# The Researcher. Copyright (c) 2026 LudaLunizz
# Licensed under the MIT License; see the file LICENSE for details.
"""The Researcher: a research loader for Claude (saved research into a Claude Code session).

Usage: researcher.py [SET ...]             popup, then write the ticked docs as .txt files
       researcher.py --setup SET [...]     popup only, to change the saved defaults
       researcher.py --no-popup SET [...]  use the saved defaults, ask nothing
       researcher.py --list                show sets, saved defaults, library size
       researcher.py --sync                make/refresh the plain-text copy of the whole library
       researcher.py --usage               local totals: characters and tokens saved
       researcher.py --usage-export [FILE] write the anonymous usage records to a file (not sent)
       researcher.py --usage-share on|off  share the anonymous usage counts with the project or not
       researcher.py --usage-send [--dry-run]  send the records not sent yet (only when sharing is on)
       researcher.py --init [LIBRARY]      write a starter config.toml
       researcher.py --sessions | --rename ID NAME | --resume ID | --session-load ID
                                           closed-session tracking (opt-in: [sessions] in config)

A "library" is a folder of documents (.html, .md, .txt, .pdf). A "set" is a named list
of documents from it, defined in config.toml. A GTK 4 popup lets the user tick what to
load, so no tokens are spent on documents that are not needed. The ticked documents are
read from a plain-text copy in the researcher's own folder (made once per document, redone
when the source changes; PDF and HTML are squeezed for tokens) and their paths are printed,
so the skill can Read them in full. Nothing in the library is ever changed or moved.

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
folder. `exclude = [globs]` in config.toml leaves documents out of the library. When `x.html` and `x.pdf` both exist, the HTML one is used.
"""
import argparse
import atexit
import datetime
import fnmatch
import html
import json
import os
import re
import shlex
import subprocess
import sys
import urllib.error
import urllib.request
import uuid

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
CHARS_PER_K = 2400  # ~2.4 chars per token, measured with count_tokens (see README)
SKIPPED_NAMED = 20  # up to this many left-out documents are named; more are only counted


class ConfigError(Exception):
    pass


# ---------------------------------------------------------------- paths and config

def config_home():
    base = os.environ.get("XDG_CONFIG_HOME") or os.path.expanduser("~/.config")
    return os.path.join(base, APP)


def cache_home():
    base = os.environ.get("XDG_CACHE_HOME") or os.path.expanduser("~/.cache")
    return os.path.join(base, APP)


def text_home():
    """The researcher's own folder: a plain-text copy of every library document."""
    base = os.environ.get("XDG_DATA_HOME") or os.path.expanduser("~/.local/share")
    return os.path.join(base, APP, "text")


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
# The Researcher: config
# The folder that holds your research documents (.html, .md, .txt, .pdf).
library = "{library}"

# Where the plain-text copy of your library is kept (default ~/.local/share/researcher/text).
# Your own files are only read, never changed; Claude reads these copies instead.
# text_dir = "~/.local/share/researcher/text"

# Documents to leave out of the library entirely (id globs).
# exclude = ["scratch/*", "drafts/*"]

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

# A local, anonymous usage log (counts and character sums, never titles or paths) feeds
# `researcher.py --usage`. `share = true` (or `researcher.py --usage-share on`) also sends those
# counts to the project; nothing is sent unless you set it. Switch the log off with:
# [usage]
# log = false
# share = false
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

def scan(library, exclude=()):
    """{doc id: file path} for every document under `library`, minus ids matching `exclude` globs."""
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
    return {k: v[1] for k, v in sorted(found.items())
            if not any(fnmatch.fnmatchcase(k, pat) for pat in exclude)}


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


TEXT_VERSION = 2  # bump when to_text() or squeeze() changes: every stored copy is made again
INVISIBLE = dict.fromkeys(map(ord, "\u200b\u200c\u200d\u2060\ufeff\u00ad"))  # zero-width, soft hyphen
PAGE_NUMBER_RE = re.compile(r"(?i)^(page\s+)?\d{1,4}(\s*(/|of)\s*\d{1,4})?$")
PAGE_WORD_RE = re.compile(r"(?i)\b(page|pagina|blz|seite|p\.)\s*\d")
RULE_RE = re.compile(r"^[\s|_=*~.\u00b7\u2022\u2500-\u257f-]{3,}$")  # lines of only decoration


def furniture_key(line):
    """Page-numbered lines ("Report | page 3 of 9") match across pages; others only exactly."""
    k = line.strip()
    return re.sub(r"\d+", "#", k) if PAGE_WORD_RE.search(k) else k


def drop_page_furniture(pages):
    """Remove page numbers and headers/footers that repeat on most pages of a PDF."""
    cands = []  # per page: indexes of the first/last three non-empty lines
    for lines in pages:
        idx = [i for i, ln in enumerate(lines) if ln.strip()]
        cands.append(set(idx[:3] + idx[-3:]))
    seen = {}
    for lines, cand in zip(pages, cands):
        for key in {furniture_key(lines[i]) for i in cand}:
            seen[key] = seen.get(key, 0) + 1
    need = max(3, int(len(pages) * 0.4))
    out = []
    for lines, cand in zip(pages, cands):
        keep = []
        for i, ln in enumerate(lines):
            k = ln.strip()
            if i in cand and k and (PAGE_NUMBER_RE.match(k) or seen[furniture_key(k)] >= need):
                continue
            keep.append(ln)
        out.append(keep)
    return out


def squeeze(text, kind):
    """Fewer tokens, same content. kind: 'pdf', 'html' or 'plain' (md/txt are only tidied)."""
    text = text.replace("\r\n", "\n").replace("\r", "\n").replace("\u00a0", " ").translate(INVISIBLE)
    if kind == "pdf":
        pages = [pg.split("\n") for pg in text.split("\f")]
        text = "\n".join("\n".join(pg) for pg in (drop_page_furniture(pages) if len(pages) >= 3 else pages))
    out, blank = [], True
    for ln in text.split("\n"):
        ln = ln.rstrip()
        if kind != "plain":  # layout whitespace means nothing here; markdown/text may use it
            ln = re.sub(r" {3,}", " | ", ln.strip())
            ln = re.sub(r"[ \t]{2,}", " ", ln)
            if RULE_RE.match(ln):
                ln = ""
        elif re.fullmatch(r"([-_=*~])\1{4,}", ln.strip()):
            ln = ""
        if not ln.strip():
            if not blank:
                out.append("")
            blank = True
            continue
        if out and ln == out[-1]:  # an exact repeat of the line above
            continue
        out.append(ln)
        blank = False
    return "\n".join(out).strip()


def convert(path):
    """(text before squeezing, squeezed text) of one document. Raises OSError / RuntimeError."""
    ext = os.path.splitext(path)[1].lower()
    if ext == ".pdf":
        try:
            r = subprocess.run(["pdftotext", "-layout", path, "-"], capture_output=True,
                               text=True, check=True)
        except FileNotFoundError:
            raise RuntimeError("pdftotext not found (install poppler-utils)")
        except subprocess.CalledProcessError as e:
            raise RuntimeError(f"pdftotext failed: {e.stderr.strip() or e.returncode}")
        return r.stdout, squeeze(r.stdout, "pdf")
    with open(path, encoding="utf-8", errors="replace") as f:
        raw = f.read()
    if ext in (".html", ".htm"):
        raw = html_text(raw)
        return raw, squeeze(raw, "html")
    return raw, squeeze(raw, "plain")


def to_text(path):
    """Plain text of one document, squeezed."""
    return convert(path)[1]


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


def store_name(d):
    return d.replace("/", " - ") + ".txt"


def id_hash(d):
    """FNV-1a (32 bit) of the id's UTF-8 bytes; the same number in the Rust version."""
    h = 0x811C9DC5
    for b in d.encode("utf-8"):
        h = ((h ^ b) * 0x01000193) & 0xFFFFFFFF
    return h


def free_store_name(d, index):
    """store_name, unless another document already uses that file (`a/b` and `a - b`): then the
    id's hash goes into the name, so two documents never share one copy."""
    fn = store_name(d)
    if any(i != d and isinstance(r, dict) and r.get("file") == fn for i, r in index.items()):
        return f"{d.replace('/', ' - ')} ({id_hash(d):08x}).txt"
    return fn


class Texts:
    """Lazy {doc id: plain text}, kept as a persistent copy in the researcher's own folder.

    A document is converted once and again only when its source file changed (size, mtime) or
    TEXT_VERSION went up. The library itself is only ever read. `d in texts` is False for
    documents that cannot be read; copies of documents that left the library stay.
    """

    def __init__(self, paths, store=None):
        self.paths, self.store, self.cache, self.errors = paths, store, {}, {}
        self.raw = {}
        self.index, self.dirty = {}, False
        if store:
            try:
                with open(os.path.join(store, ".index.json"), encoding="utf-8") as f:
                    self.index = json.load(f)
            except (OSError, ValueError):
                self.index = {}
            if not isinstance(self.index, dict):
                self.index = {}

    def _stored(self, d):
        """Text of a fresh stored copy, or None."""
        rec = self.index.get(d)
        try:
            st = os.stat(self.paths[d])
            if (isinstance(rec, dict) and rec.get("v") == TEXT_VERSION
                    and rec.get("src") == self.paths[d] and rec.get("size") == st.st_size
                    and rec.get("mtime_ns") == st.st_mtime_ns):
                with open(os.path.join(self.store, rec["file"]), encoding="utf-8") as f:
                    head, text = f.read().split("\n\n", 1)
                if head == f"SOURCE: {self.paths[d]}":  # else: another document's copy
                    return text.removesuffix("\n")
        except (OSError, KeyError, ValueError, TypeError):
            pass
        return None

    def raw_chars(self, d):
        """Characters of the converted text before squeezing (the whole copy is read once)."""
        return self.raw.get(d, len(self.cache.get(d, "")))

    def _save(self, d, text, raw, st):
        """`st` is the source as it was before converting: if it changes meanwhile, the next
        run sees another size or mtime and converts it again."""
        os.makedirs(self.store, exist_ok=True)
        fn = free_store_name(d, self.index)
        tmp = os.path.join(self.store, f"{fn}.{os.getpid()}.tmp")
        with open(tmp, "w", encoding="utf-8") as f:
            f.write(f"SOURCE: {self.paths[d]}\n\n{text}\n")
        os.replace(tmp, os.path.join(self.store, fn))
        self.index[d] = {"file": fn, "src": self.paths[d], "size": st.st_size,
                         "mtime_ns": st.st_mtime_ns, "raw": raw, "v": TEXT_VERSION}
        self.dirty = True

    def __contains__(self, d):
        if d in self.cache:
            return True
        if d not in self.paths or d in self.errors:
            return False
        try:
            text = self._stored(d) if self.store else None
            if text is None:
                st = os.stat(self.paths[d])
                raw, text = convert(self.paths[d])
                if self.store:
                    self._save(d, text, len(raw), st)
            self.cache[d] = text
            self.raw[d] = (self.index.get(d) or {}).get("raw", len(text))
        except (OSError, RuntimeError) as e:
            self.errors[d] = str(e)
            return False
        return True

    def __getitem__(self, d):
        if d not in self:
            raise KeyError(d)
        return self.cache[d]

    def get(self, d, default=""):
        return self.cache[d] if d in self else default

    def path(self, d):
        """The stored .txt of a readable document."""
        return os.path.join(self.store, self.index[d]["file"])

    def flush(self):
        if self.store and self.dirty:
            tmp = os.path.join(self.store, f".index.json.{os.getpid()}.tmp")
            try:
                with open(tmp, "w", encoding="utf-8") as f:
                    json.dump(self.index, f, indent=1, sort_keys=True)
                os.replace(tmp, os.path.join(self.store, ".index.json"))
            except OSError:
                return  # the copies are still valid; the next run just checks them again
            self.dirty = False


def sections_for(names, sets, kinds, conf, texts, setup):
    """One section per requested set.

    docs     what the set is made of (for a custom set: the whole library)
    main     documents shown at the top: the set's own plus extras saved earlier
    other    the rest of the library, offered as "add from your library"
    ticked   pre-ticked documents; new = added since the preset was saved
    """
    catalog = sets["all"]
    out = []
    for n in names:
        docs = sets[n]
        saved = conf.get(n)
        known = set(saved.get("known", [])) if saved else set()
        selected = set(saved.get("selected", [])) if saved else set()
        new = {d for d in docs if d not in known} if saved else set()
        if kinds[n] == "custom":  # nothing ticked on first use, new docs never auto-ticked
            ticked = selected & set(docs)
            remember = saved is None or bool(new) or setup
            main = [d for d in docs if d in ticked or d in new] if saved else []
        else:
            ticked = set(docs) if saved is None else (selected | new)
            remember = saved is None or bool(new) or setup
            main = docs + [d for d in catalog if d in selected and d not in docs]
        other = [d for d in catalog if d not in set(main)]
        out.append({"name": n, "kind": kinds[n], "docs": docs, "main": main, "other": other,
                    "new": new, "ticked": {d for d in ticked if d in texts},  # unreadable: no
                    "remember": remember, "first": saved is None})
    return out


def resolve_names(argv_names, cfg):
    aliases = {k.lower(): v for k, v in cfg.get("aliases", {}).items()}
    sets = cfg.get("sets", {}) if isinstance(cfg.get("sets"), dict) else {}
    defined = {k.lower(): k for k in reversed(list(sets))}  # `[sets.MyRust]` is reached by `myrust`

    def canon(n):
        return n if n in sets else defined.get(n.lower(), n)
    names = [canon(aliases.get(n.lower(), n.lower())) for n in argv_names]
    if not names:
        d = cfg.get("default_set")
        names = [canon(d)] if d else []
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
    tmp = f"{path}.{os.getpid()}.tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(conf, f, indent=1, ensure_ascii=False)
    os.replace(tmp, path)


# ---------------------------------------------------------------- popup


def host_name():
    """The program that asked for the load, for the popup text: RESEARCHER_HOST if set, opencode when
    opencode started us (it sets OPENCODE=1 for its tools), otherwise Claude Code."""
    over = (os.environ.get("RESEARCHER_HOST") or "").strip()
    if over:
        return over
    return "opencode" if "OPENCODE" in os.environ else "Claude Code"

def popup(sections, texts, setup, command="", look=None):
    """Show the chooser. Returns {set: (ticked set, remember)}, None if cancelled or "timeout".
    Raises RuntimeError when no display/GTK 4 is available.

    look (for --context): {"labels": {id: shown}, "notes": {id: small text}, "groups": {id:
    heading}, "chats": True}; by default rows show the document id under its folder."""
    look = look or {}
    labels, notes, groups = look.get("labels", {}), look.get("notes", {}), look.get("groups", {})
    sizes = look.get("sizes", {})
    chats = bool(look.get("chats"))
    noun = "chats" if chats else "documents"
    try:
        import gi
        gi.require_version("Gtk", "4.0")
        from gi.repository import GLib, Gtk
    except (ImportError, ValueError) as e:
        raise RuntimeError(f"GTK 4 not available ({e})")
    if not Gtk.init_check():
        raise RuntimeError("no display")
    Gtk.Settings.get_default().set_property("gtk-application-prefer-dark-theme", True)
    esc = lambda t: GLib.markup_escape_text(t, -1)
    loop = GLib.MainLoop()
    result = {"value": None}
    boxes = {}  # set name -> [(doc, CheckButton)]
    remember = {}

    def k(d):
        return (sizes[d] if d in sizes else len(texts.get(d, ""))) / CHARS_PER_K

    def folder(d):
        return d.rpartition("/")[0]

    def group(d):
        if d in groups:
            return groups[d]
        return folder(d) + "/" if folder(d) else "(top level)"

    win = Gtk.Window(title="The Researcher - " + ("change saved presets" if setup else
                     "choose chats to load" if chats else "choose research to load"))
    win.set_default_size(720, 780)
    outer = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
    for m in ("top", "bottom", "start", "end"):
        getattr(outer, f"set_margin_{m}")(14)
    names = ", ".join(f"<b>{esc(s['name'])}</b>" for s in sections)
    head = Gtk.Label(xalign=0, wrap=True)
    if setup:
        head.set_markup(f"<big><b>Change the saved presets</b></big> for {names}\n"
                        "Tick what each should contain, then press <b>Save presets</b>. "
                        "Nothing is loaded.")
    else:
        what = "earlier chats" if chats else "research"
        host = host_name()
        head.set_markup(f"<big><b>{esc(host)} wants to load {what}</b></big> for {names}\n"
                        "Tick what this session needs, then press <b>Load</b>. Nothing is sent to "
                        f"{esc('Claude' if host == 'Claude Code' else host)} until then; "
                        "<b>Cancel</b> loads nothing."
                        + (f"\n<small>Asked by: <tt>{esc(command)}</tt></small>" if command else ""))
    outer.append(head)
    search = Gtk.SearchEntry(placeholder_text="Filter: words from the first prompt or the folder"
                             if chats else "Filter: words from the name")
    outer.append(search)
    rows = []  # (lower-case search text, CheckButton, heading label or None)
    scroll = Gtk.ScrolledWindow(vexpand=True)
    body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
    scroll.set_child(body)
    outer.append(scroll)
    total = Gtk.Label(xalign=0)

    def update(*_):
        c = {d for lst in boxes.values() for d, cb in lst if cb.get_active()}
        total.set_markup(f"<b>{len(c)} {noun}, ~{sum(k(d) for d in c):.0f}k tokens</b>"
                         + ("" if setup else " will be loaded"))
        ok.set_label("Save presets" if setup else f"Load {len(c)} {noun}")
        ok.set_sensitive(setup or bool(c))

    def add_docs(target, docs, sec, lst):
        """Check boxes for `docs`, with a small folder heading when more than one folder shows."""
        grouped = len({group(d) for d in docs}) > 1 or bool(groups)
        last = h = None
        for d in docs:
            if grouped and group(d) != last:
                last = group(d)
                h = Gtk.Label(xalign=0)
                h.set_markup(f"<small><b>{esc(last)}</b></small>")
                h.set_margin_top(6)
                h.set_margin_start(12)
                target.append(h)
            cb = Gtk.CheckButton()
            l = Gtk.Label(xalign=0)
            shown = labels.get(d) or (d.rpartition("/")[2] if grouped else d)
            rows.append((f"{shown} {group(d)} {d}".lower(), cb, h))
            if d in texts:
                note = f"  <small><span alpha='60%'>{esc(notes[d])}</span></small>" if notes.get(d) else ""
                extra = "  <span foreground='#ffcc33'><b>NEW</b></span>" if d in sec["new"] else ""
                l.set_markup(f"{esc(shown)}  <small>~{k(d):.0f}k</small>{note}{extra}")
                cb.set_active(d in sec["ticked"])
            else:
                l.set_markup(f"<s>{esc(shown)}</s>  <span foreground='#ff6666'>"
                             "MISSING or unreadable</span>")
                cb.set_sensitive(False)
            cb.set_child(l)
            cb.set_margin_start(24 if grouped else 12)
            cb.connect("toggled", update)
            target.append(cb)
            lst.append((d, cb))

    expanders = []
    for sec in sections:
        n = sec["name"]
        custom = sec["kind"] == "custom"
        if chats:
            sub = ("new preset: nothing saved yet. Tick the chats this one needs; tick Remember to "
                   "keep them as its preset." if sec["first"] else
                   f"saved preset: {len(sec['ticked'])} chats" + (
                       f", {len(sec['new'])} newer chats since (marked NEW)" if sec["new"] else ""))
        elif sec["first"]:
            sub = ("new set: nothing saved yet. Tick the research this one needs; it is saved "
                   "as its preset." if custom else
                   f"{len(sec['docs'])} documents (no preset saved yet, all ticked)")
        else:
            sub = f"saved preset: {len(sec['ticked'] - sec['new'])} documents" + (
                f", {len(sec['new'])} new in the library" if sec["new"] else "")
        lbl = Gtk.Label(xalign=0, wrap=True)
        lbl.set_margin_top(12)
        lbl.set_markup(f"<big><b>{esc(n)}</b></big>\n<small>{esc(sub)}</small>")
        body.append(lbl)
        lst = []
        row = Gtk.Box(spacing=6)
        for text, fn in (("Select all", lambda l: [cb.set_active(cb.get_sensitive())
                                                   for _, cb in l if cb.get_visible()]),
                         ("Select none", lambda l: [cb.set_active(False)
                                                    for _, cb in l if cb.get_visible()]),
                         ("Back to saved preset", None)):
            b = Gtk.Button(label=text)
            if fn is None:
                b.connect("clicked", lambda _b, l=lst, s=sec["ticked"]:
                          [cb.set_active(d in s) for d, cb in l])
                b.set_sensitive(not sec["first"])
            else:
                b.connect("clicked", lambda _b, l=lst, f=fn: f(l))
            row.append(b)
        body.append(row)
        if sec["main"]:
            add_docs(body, sec["main"], sec, lst)
        if sec["other"]:
            inner = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
            if chats:
                label = ("Your chats: tick the ones to load" if not sec["main"] else
                         f"Add other chats ({len(sec['other'])} more)")
            else:
                label = ("Your library: tick the research to use" if not sec["main"] else
                         f"Add other research from your library ({len(sec['other'])} more)")
            ex = Gtk.Expander(label=label)
            expanders.append(ex)
            ex.set_margin_start(12)
            ex.set_margin_top(6)
            ex.set_expanded(not sec["main"])
            ex.set_child(inner)
            body.append(ex)
            add_docs(inner, sec["other"], sec, lst)
        boxes[n] = lst
        rb = Gtk.CheckButton(label=f"Remember this choice as the preset for {n}")
        rb.set_active(sec["remember"])
        if setup:
            rb.set_sensitive(False)
        rb.set_margin_start(12)
        rb.set_margin_top(6)
        body.append(rb)
        remember[n] = rb

    outer.append(total)
    bar = Gtk.Box(spacing=8, halign=Gtk.Align.END)
    cancel = Gtk.Button(label="Close without saving" if setup else "Cancel (load nothing)")
    ok = Gtk.Button(label="Load")
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

    def refilter(*_):
        q = search.get_text().strip().lower()
        shown_heads = set()
        for find, cb, h in rows:
            hit = not q or q in find
            cb.set_visible(hit)
            if hit and h is not None:
                shown_heads.add(id(h))
        for _, _, h in rows:
            if h is not None:
                h.set_visible(not q or id(h) in shown_heads)
        if q:
            for ex in expanders:
                ex.set_expanded(True)

    search.connect("search-changed", refilter)
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


# ---------------------------------------------------------------- usage log (local, anonymous)
# One line per load in ~/.local/share/researcher/usage.jsonl: counts and character sums only.
# Never a title, path, set name or library location. The user can look at the file, print
# totals (--usage) and write an export (--usage-export). Records are only sent when the user
# said yes (`[usage] share = true`, set with --usage-share on): --usage-send sends the ones not
# sent yet, and after a load that happens in the background.

EXPORT_SCHEMA = 1  # the file --usage-export writes; each record inside has its own version
# Record versions: 1 had counts per format (pdf, html, md, txt); 2 adds chats loaded as context to
# `fmt` and `fmt_chars`, the loaded characters per format. Both are read and sent.
RECORD_VERSION = 2
# Reading the converted text instead of the original costs fewer tokens: measured with count_tokens
# on 85 reports (README, "What it saves"), raw HTML = 1.73x its text and a PDF read as a document
# = 2.2x. So per converted character HTML saves 0.73 and PDF 1.2 (markdown, text and chats are read
# as they are). Keep these equal to SAVED_PER_CHAR in worker/src/index.js.
SAVED_PER_CHAR = {"html": 0.73, "pdf": 1.2}
SEND_SCHEMA = 2  # the request --usage-send posts: the records plus `seq`, their log line number
DEFAULT_URL = "https://the-researcher-usage.ludalunizz.workers.dev/v1/usage"  # worker/ in the repo
SEND_BATCH = 200  # records per request (the receiver takes up to 200)
SEND_TIMEOUT = 15  # seconds
USAGE_KINDS = ("pdf", "html", "md", "txt")
FORMS = USAGE_KINDS + ("chat",)  # what a record counts per format


def data_home():
    base = os.environ.get("XDG_DATA_HOME") or os.path.expanduser("~/.local/share")
    return os.path.join(base, APP)


def usage_file():
    return os.path.join(data_home(), "usage.jsonl")


def sent_file():
    return os.path.join(data_home(), "usage-sent")


def write_atomic(path, text):
    tmp = f"{path}.{os.getpid()}.tmp"
    with open(tmp, "w", encoding="utf-8", newline="") as f:  # line endings exactly as given
        f.write(text)
    os.replace(tmp, path)


def usage_section(cfg):
    sec = cfg.get("usage")
    return sec if isinstance(sec, dict) else {}


def usage_enabled(cfg):
    return usage_section(cfg).get("log") is not False


def usage_share(cfg):
    """The user's answer to "share the anonymous usage counts?": "on", "off" or "not decided"."""
    val = usage_section(cfg).get("share")
    return "on" if val is True else "off" if val is False else "not decided"


def usage_url(cfg):
    """Where records are sent: `[usage] url`, or the project's receiver."""
    val = usage_section(cfg).get("url")
    return val if isinstance(val, str) and val else DEFAULT_URL


def install_id():
    """A random id for this install, made on first use; delete the file to start a new one."""
    path = os.path.join(data_home(), "usage-id")
    try:
        with open(path, encoding="utf-8") as f:
            val = f.read().strip()
        if re.fullmatch(r"[0-9a-f]{16}", val):
            return val
    except OSError:
        pass
    val = uuid.uuid4().hex[:16]
    os.makedirs(data_home(), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(val + "\n")
    return val


def format_of(path):
    ext = os.path.splitext(path)[1].lower().lstrip(".")
    return "html" if ext == "htm" else ext if ext in USAGE_KINDS else "txt"


def known_version(rec):
    return isinstance(rec, dict) and rec.get("v") in (1, 2)


def usage_build(counts, src_bytes, raw, lib):
    """One load as numbers. counts = {format: (documents, characters)}; lib = (documents, characters)
    of everything that could have been loaded. Characters, not tokens: the reader applies its ratio."""
    fmt = {k: counts.get(k, (0, 0))[0] for k in FORMS}
    fmt_chars = {k: counts.get(k, (0, 0))[1] for k in FORMS}
    return {"v": RECORD_VERSION, "day": datetime.date.today().isoformat(), "id": install_id(),
            "docs": sum(fmt.values()), "fmt": fmt, "fmt_chars": fmt_chars,
            "src_bytes": src_bytes, "raw": raw, "chars": sum(fmt_chars.values()),
            "lib_docs": lib[0], "lib_chars": lib[1], "tk": CHARS_PER_K}


def usage_record(docs, texts, paths):
    """The record of one library load."""
    counts, src_bytes = {}, 0
    for d in docs:
        n, c = counts.get(format_of(paths[d]), (0, 0))
        counts[format_of(paths[d])] = (n + 1, c + len(texts[d]))
        src_bytes += os.path.getsize(paths[d])
    lib = [d for d in paths if d in texts]
    return usage_build(counts, src_bytes, sum(texts.raw_chars(d) for d in docs),
                       (len(lib), sum(len(texts[d]) for d in lib)))


def file_size(path):
    try:
        return os.path.getsize(path)
    except OSError:
        return 0  # a transcript that vanished must not break a load


def chat_usage_record(loaded, lib):
    """The record of earlier Claude Code chats loaded as context: loaded = [(characters, transcript
    bytes)] of each chat loaded; lib = (chats, characters) of every chat up to the newest one picked. A chat
    is read as it is written, so nothing is squeezed (raw = chars)."""
    chars = sum(c for c, _ in loaded)
    return usage_build({"chat": (len(loaded), chars)}, sum(b for _, b in loaded), chars, lib)


def finish_usage(cfg, cfg_path, rec):
    """Log the record of a finished load and, when the user said yes, send it in the background.
    Returns the one line to print when sharing is not decided yet. Never breaks a load."""
    log_usage(rec)
    share = usage_share(cfg)
    if share == "on":
        send_in_background(cfg_path)
    elif share == "not decided":
        return "USAGE SHARING: not decided (researcher --usage-share on|off)"
    return None


def usage_of_chats(cfg, cfg_path, loaded, lib):
    """Log a chat load (and send it when sharing is on). Returns the sharing note, if any."""
    if not loaded or not usage_enabled(cfg):
        return None
    try:
        return finish_usage(cfg, cfg_path, chat_usage_record(loaded, lib))
    except OSError:
        return None


def log_usage(rec):
    """Append one record. A failure here must never break a load."""
    try:
        os.makedirs(data_home(), exist_ok=True)
        with open(usage_file(), "a", encoding="utf-8") as f:
            f.write(json.dumps(rec, sort_keys=True) + "\n")
    except OSError:
        pass


def read_usage():
    """Records from the log; lines that are not this schema are skipped."""
    out = []
    try:
        with open(usage_file(), encoding="utf-8") as f:
            for line in f:
                try:
                    rec = json.loads(line)
                except ValueError:
                    continue
                if known_version(rec):
                    out.append(rec)
    except OSError:
        pass
    return out


def usage_summary(recs):
    """Totals over records. Characters are summed; tokens use each record's own ratio."""
    def tok(chars, rec):
        return chars / (rec.get("tk") or CHARS_PER_K) * 1000

    tot = {k: sum(r.get(k, 0) for r in recs) for k in ("docs", "src_bytes", "raw", "chars")}
    tot["squeezed"] = tot["raw"] - tot["chars"]
    tot["unticked"] = sum(max(0, r.get("lib_chars", 0) - r.get("chars", 0)) for r in recs)
    tot["tokens"] = sum(tok(r.get("chars", 0), r) for r in recs)
    tot["tokens_squeezed"] = sum(tok(r.get("raw", 0) - r.get("chars", 0), r) for r in recs)
    tot["tokens_unticked"] = sum(tok(max(0, r.get("lib_chars", 0) - r.get("chars", 0)), r)
                                 for r in recs)
    tot["loads"] = len(recs)
    # reading text instead of the HTML/PDF originals (records that say their formats only)
    with_formats = [r for r in recs if isinstance(r.get("fmt_chars"), dict)]
    tot["loads_with_formats"] = len(with_formats)
    tot["tokens_converted"] = sum(
        tok(sum(per * r["fmt_chars"].get(k, 0) for k, per in SAVED_PER_CHAR.items()), r)
        for r in with_formats)
    return tot


def usage_report(recs):
    if not recs:
        return "no usage recorded yet (a log line is written after every load)"
    t = usage_summary(recs)
    n = t["loads"]
    days = sorted({r.get("day", "") for r in recs})
    pct = 100 * t["squeezed"] // t["raw"] if t["raw"] else 0
    return "\n".join([
        f"usage log: {usage_file()}",
        f"{n} loads on {len(days)} days ({days[0]} to {days[-1]}), {t['docs']} documents loaded",
        f"loaded:        {t['chars']:>10} chars  (~{t['tokens'] / 1000:.0f}k tokens, "
        f"{t['tokens'] / n / 1000:.1f}k per load)",
        f"squeezed away: {t['squeezed']:>10} chars  (~{t['tokens_squeezed'] / 1000:.0f}k tokens, "
        f"{pct}% of the converted text)",
        f"converted:     ~{t['tokens_converted'] / 1000:.0f}k tokens saved by reading text instead of "
        f"the HTML/PDF originals (from measured ratios, {t['loads_with_formats']} of {n} loads say "
        "their formats)",
        f"left unticked: {t['unticked']:>10} chars  (~{t['tokens_unticked'] / 1000:.0f}k tokens, "
        "against loading the whole library every time: a ceiling, not a measurement)",
        "tokens are estimates from characters, not counted by a tokenizer",
    ])


def usage_pending():
    """The records not sent yet, oldest first, each with `seq` = its line number in the log. The
    line number tells two identical loads on one day apart, and it stays the same when a record
    is sent again, so a resend is stored once."""
    try:
        with open(usage_file(), encoding="utf-8") as f:
            lines = f.read().split("\n")
    except OSError:
        return []
    if lines and lines[-1] == "":
        lines.pop()  # the newline after the last record
    try:
        with open(sent_file(), encoding="utf-8") as f:
            done = int(f.read().strip())
    except (OSError, ValueError):
        done = 0
    if not 0 <= done <= len(lines):
        done = 0  # the log was deleted or cut: start again
    out = []
    for i, line in enumerate(lines[done:], done + 1):
        try:
            rec = json.loads(line)
        except ValueError:
            continue
        if known_version(rec):
            rec["seq"] = i
            out.append(rec)
    return out


def usage_payload(recs):
    """The body --usage-send posts for these records."""
    return {"schema": SEND_SCHEMA, "loads": recs}


def post_json(url, body):
    """One POST: (HTTP status, body text). OSError when there was no answer."""
    req = urllib.request.Request(url, data=json.dumps(body, sort_keys=True).encode("utf-8"),
                                 headers={"Content-Type": "application/json",
                                          "User-Agent": "the-researcher (python)"},
                                 method="POST")
    try:
        with urllib.request.urlopen(req, timeout=SEND_TIMEOUT) as r:
            return r.status, r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        try:
            return e.code, e.read().decode("utf-8", "replace")
        finally:
            e.close()
    except (urllib.error.URLError, ValueError) as e:
        raise OSError(str(getattr(e, "reason", e)))


def usage_send(url, recs):
    """Send in batches -> (done, error or None); done = {"sent", "new", "refused"}. After each
    batch the sent marker moves on, so a later failure never sends the earlier ones twice (and
    if it did, the receiver stores them once). A record the receiver refuses is skipped, so one
    bad line can never block the rest."""
    done = {"sent": 0, "new": 0, "refused": 0}
    for start in range(0, len(recs), SEND_BATCH):
        batch = recs[start:start + SEND_BATCH]
        left = list(batch)
        while left:
            try:
                status, text = post_json(url, usage_payload(left))
            except OSError as e:
                return done, str(e)
            try:
                reply = json.loads(text)
            except ValueError:
                reply = None
            reply = reply if isinstance(reply, dict) else {}
            if status == 200 and reply.get("ok") is True:
                done["sent"] += len(left)
                new = reply.get("new")
                done["new"] += new if isinstance(new, int) and new >= 0 else 0
                break
            err = reply.get("error") if isinstance(reply.get("error"), str) else ""
            m = re.match(r"record (\d+):", err)
            if status == 400 and m and int(m.group(1)) < len(left):
                # the receiver names the one record it refused: skip it, send the rest
                del left[int(m.group(1))]
                done["refused"] += 1
                continue
            return done, f"HTTP {status}: {err or text.strip()[:200]}"
        try:
            os.makedirs(data_home(), exist_ok=True)
            write_atomic(sent_file(), f"{batch[-1]['seq']}\n")
        except OSError:
            pass
    return done, None


def send_in_background(cfg_path):
    """Start --usage-send on its own and return at once: a load never waits on the network,
    and the sender outlives the load (its own session on POSIX)."""
    kw = {"start_new_session": True} if os.name == "posix" else {}
    try:
        subprocess.Popen([sys.executable, "-I", os.path.abspath(__file__), "--usage-send",
                          "--config", os.path.abspath(cfg_path)], stdin=subprocess.DEVNULL,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, **kw)
    except OSError:
        pass


def share_line(share, waiting):
    """The last line of --usage: is anything sent, and how much is waiting."""
    if share == "on":
        return (f"sharing: on, {waiting} records waiting to be sent "
                "(researcher --usage-send sends them now)")
    return f"sharing: {share}, nothing is sent (researcher --usage-share on|off)"


def set_share(cfg_path, on):
    """Put `share = true|false` under [usage] in the config (comments and the rest kept)."""
    line = f"share = {'true' if on else 'false'}"
    try:
        with open(cfg_path, encoding="utf-8", newline="") as f:  # keep the file's line endings
            old = f.read()
    except FileNotFoundError:
        old = None
    if old is None:
        body = f"[usage]\n{line}\n"
    else:
        h = re.search(r"(?m)^[ \t]*\[usage\][ \t]*(#[^\r\n]*)?\r?$", old)
        if not h and not old.strip():
            body = f"[usage]\n{line}\n"
        elif not h:  # no [usage] table yet: a new one at the end, after a blank line
            body = (old if old.endswith("\n") else old + "\n") + f"\n[usage]\n{line}\n"
        else:
            nl = old.find("\n", h.end())
            start = len(old) if nl < 0 else nl + 1
            nxt = re.search(r"(?m)^[ \t]*\[", old[start:])
            end = start + nxt.start() if nxt else len(old)
            section = old[start:end]
            key = re.compile(r"(?m)^[ \t]*share[ \t]*=.*$")
            if key.search(section):
                section = key.sub(lambda _: line, section, count=1)
            else:
                section = f"{line}\n{section}"
            head = old[:start] if old[:start].endswith("\n") else old[:start] + "\n"
            body = head + section + old[end:]
    if tomllib is None:
        raise ConfigError("Python 3.11 or newer is needed to read config.toml")
    try:
        parsed = tomllib.loads(body)
    except tomllib.TOMLDecodeError as e:
        raise ConfigError(str(e))
    if usage_share(parsed) != ("on" if on else "off"):
        raise ConfigError("the config would not read back the same answer")
    os.makedirs(os.path.dirname(os.path.abspath(cfg_path)), exist_ok=True)
    write_atomic(cfg_path, body)


def usage_send_main(cfg, dry_run):
    """--usage-send: send the records not sent yet. Exit 9 when sharing is not on."""
    share, url, recs = usage_share(cfg), usage_url(cfg), usage_pending()
    if dry_run:
        print(f"DRY RUN: {len(recs)} records would be sent to {url} (sharing: {share}); "
              "nothing was sent")
        if recs:
            print(json.dumps(usage_payload(recs), indent=1, sort_keys=True))
        return 0
    if share != "on":
        print(f"USAGE SHARING: {share}; nothing sent (turn it on with: researcher --usage-share on)")
        return 9
    if not recs:
        print("NOTHING TO SEND: every usage record was sent already")
        return 0
    done, err = usage_send(url, recs)
    if err is None:
        refused = (f", {done['refused']} refused by the receiver and skipped"
                   if done["refused"] else "")
        print(f"SENT: {done['sent']} records to {url} ({done['new']} new there{refused})")
        return 0
    print(f"SEND FAILED: {err}; {done['sent']} records sent before that, "
          f"{len(recs) - done['sent'] - done['refused']} stay queued for the next try")
    return 1


def usage_export(path):
    """Write the records as one JSON file to look at."""
    recs = read_usage()
    with open(path, "w", encoding="utf-8") as f:
        json.dump({"schema": EXPORT_SCHEMA, "loads": recs}, f, indent=1, sort_keys=True)
        f.write("\n")
    return len(recs)


# ---------------------------------------------------------------- sessions (opt-in)
# Closed Claude Code sessions: a registry keyed by the FULL session id, so a name can never
# be separated from the id it belongs to. Names are taken from Claude Code's own title or the
# first prompt, and can be renamed (a manual name is never overwritten by a rescan).

SESSION_ID_RE = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
# opencode session ids: `ses_` and letters/digits (case matters)
OC_ID_RE = re.compile(r"ses_[A-Za-z0-9]+")
NAME_MAX = 80
SESSION_LOAD_MAX = 200000  # chars; a longer session keeps its start and its end
SESSION_DEFAULTS = {"idle_minutes": 10, "projects_dir": "~/.claude/projects", "opencode": False,
                    "opencode_cmd": "opencode"}


def sessions_settings(cfg):
    sec = cfg.get("sessions")
    if not isinstance(sec, dict) or not sec.get("enabled"):
        return None
    out = dict(SESSION_DEFAULTS)
    for k in out:
        if k in sec:
            out[k] = sec[k]
    out["projects_dir"] = os.path.abspath(os.path.expanduser(str(out["projects_dir"])))
    out["opencode"] = bool(out["opencode"])
    cmd = out["opencode_cmd"]
    out["opencode_cmd"] = os.path.expanduser(cmd.strip()) if isinstance(cmd, str) and cmd.strip() else "opencode"
    return out


def clean_name(text):
    text = re.sub(r"\s+", " ", str(text)).strip()
    return text if len(text) <= NAME_MAX else text[:NAME_MAX - 1].rstrip() + "…"


def block_text(content):
    """The plain text of a message: a string, or the text blocks of a list (tools skipped)."""
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(b.get("text", "") for b in content
                         if isinstance(b, dict) and b.get("type") == "text")
    return ""


def read_transcript(path):
    """Entries of a session .jsonl; unreadable or broken lines are skipped."""
    out = []
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                try:
                    d = json.loads(line)
                except ValueError:
                    continue
                if isinstance(d, dict):
                    out.append(d)
    except OSError:
        pass
    return out


def real_prompt(entry):
    """The user's own prompt of an entry, or '' (tool results, meta and system text, commands)."""
    if entry.get("type") != "user" or entry.get("isMeta"):
        return ""
    msg = entry.get("message")
    text = block_text(msg.get("content") if isinstance(msg, dict) else "").strip()
    return "" if not text or text.startswith("<") else text


def session_info(entries):
    title = first = cwd = None
    started = last = None
    for e in entries:
        if e.get("type") == "custom-title" and e.get("customTitle"):
            title = e["customTitle"]
        elif e.get("type") == "ai-title" and e.get("aiTitle") and not title:
            title = e["aiTitle"]
        if first is None:
            first = real_prompt(e) or None
        cwd = cwd or e.get("cwd")
        ts = e.get("timestamp")
        if isinstance(ts, str):
            started = started or ts
            last = ts
    return {"title": title, "first": first, "cwd": cwd, "started": started, "last": last}


def load_registry(path):
    try:
        with open(path, encoding="utf-8") as f:
            reg = json.load(f)
        if isinstance(reg.get("sessions"), dict):
            return reg
    except (OSError, ValueError, AttributeError):
        pass
    return {"version": 1, "sessions": {}}


def save_registry(path, reg):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = f"{path}.{os.getpid()}.tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(reg, f, indent=1, ensure_ascii=False)
    os.replace(tmp, path)


def register(reg, sid, path, mtime, proj, entries, source=None):
    """Add or refresh the registry record of one session from its transcript. Returns 1 when the
    session is new, 0 when it was known or nothing was ever asked."""
    import time
    info = session_info(entries)
    if not info["first"] and not info["title"]:
        return 0  # nothing was ever asked
    if source == "opencode" and info["title"]:
        # a chat started from a command or skill opens with its whole expanded text: opencode's
        # own title is the better label
        info["first"] = info["title"]
    auto = clean_name(info["title"] or info["first"])
    rec = reg["sessions"].get(sid)
    added = 0
    if rec is None:
        rec = reg["sessions"][sid] = {"name": auto, "name_source": "auto",
                                      "registered": time.strftime("%Y-%m-%d %H:%M")}
        added = 1
    elif rec.get("name_source") != "manual":
        rec["name"] = auto
    rec.update(file=path, mtime=mtime, project=proj, cwd=info["cwd"],
               started=info["started"], last=info["last"],
               first=clean_name(info["first"]) if info["first"] else None)
    if source:
        rec["source"] = source
    rec["chars"] = len(session_text(sid, rec, entries))
    return added


def up_to_date(reg, sid, path, mtime):
    """True when the registry already holds this version of the session's transcript."""
    rec = reg["sessions"].get(sid)
    return bool(rec and rec.get("mtime") == mtime and rec.get("file") == path
                and "first" in rec and "chars" in rec)


def scan_sessions(reg, settings, now=None, idle=None, skip="", reg_path="", in_opencode=False):
    """Add sessions untouched for `idle` seconds (default: idle_minutes) to the registry and
    refresh the changed ones (never `skip`, the session that runs this). Returns the number added.
    opencode chats are added too when `[sessions] opencode` is on; `in_opencode` = this runs inside
    opencode (its newest chat is the one that runs this)."""
    import time
    now = time.time() if now is None else now
    idle = float(settings["idle_minutes"]) * 60 if idle is None else idle
    added = 0
    base = settings["projects_dir"]
    try:
        projects = sorted(os.listdir(base))
    except OSError:
        projects = []
    for proj in projects:
        pdir = os.path.join(base, proj)
        try:
            files = os.listdir(pdir)
        except OSError:
            continue
        for fn in files:
            sid = fn[:-6]
            if not fn.endswith(".jsonl") or not SESSION_ID_RE.fullmatch(sid) or sid == skip:
                continue
            path = os.path.join(pdir, fn)
            try:
                mtime = os.path.getmtime(path)
            except OSError:
                continue
            if now - mtime < idle:
                continue  # probably still open
            if up_to_date(reg, sid, path, mtime):
                continue
            added += register(reg, sid, path, mtime, proj, read_transcript(path))
    if settings["opencode"]:
        added += scan_opencode(reg, settings, now, idle, reg_path, in_opencode)
    return added


def oc_run(cmd, args):
    """Run the opencode program and return its standard output."""
    try:
        out = subprocess.run([cmd, *args], stdin=subprocess.DEVNULL, capture_output=True, check=False)
    except OSError as e:
        raise ConfigError(f"{cmd}: {e.strerror or e}") from e
    if out.returncode != 0:
        raise ConfigError(f"{cmd} {' '.join(args)} failed")
    return out.stdout


def iso_ms(ms):
    t = datetime.datetime.fromtimestamp(ms // 1000, datetime.timezone.utc)
    return t.strftime("%Y-%m-%dT%H:%M:%S.") + f"{ms % 1000:03d}Z"


def oc_text(parts):
    """The text parts of an opencode message (tool calls, reasoning, synthetic and ignored text left out)."""
    texts = [p.get("text", "") for p in parts or []
             if isinstance(p, dict) and p.get("type") == "text"
             and not p.get("synthetic") and not p.get("ignored") and isinstance(p.get("text"), str)]
    return "\n".join(texts).strip()


def oc_entries(export, directory):
    """`opencode export` as transcript entries in the shape of a Claude Code .jsonl (title, user and
    assistant text with timestamps), so the rest of the program reads both the same way."""
    out = []
    info = export.get("info") if isinstance(export, dict) else None
    title = info.get("title") if isinstance(info, dict) else ""
    if isinstance(title, str) and title and not title.startswith("New session - ") and not title.startswith("<"):
        out.append({"type": "ai-title", "aiTitle": title})
    for m in (export.get("messages") if isinstance(export, dict) else None) or []:
        mi = m.get("info") if isinstance(m, dict) else None
        role = mi.get("role") if isinstance(mi, dict) else ""
        text = oc_text(m.get("parts"))
        if not text or role not in ("user", "assistant"):
            continue
        created = (mi.get("time") or {}).get("created") if isinstance(mi.get("time"), dict) else 0
        content = text if role == "user" else [{"type": "text", "text": text}]
        out.append({"type": role, "timestamp": iso_ms(int(created or 0)), "cwd": directory,
                    "message": {"role": role, "content": content}})
    return out


def scan_opencode(reg, settings, now, idle, reg_path, in_opencode):
    """Add opencode's chats (top-level ones; sub-agent sessions are not listed by opencode) to the
    registry. Each changed chat is exported once and cached as a transcript next to the registry."""
    from concurrent.futures import ThreadPoolExecutor
    cmd = settings["opencode_cmd"]
    try:
        listing = json.loads(oc_run(cmd, ["session", "list", "--format", "json"]))
        if not isinstance(listing, list):
            raise ConfigError("unreadable session list")
    except (ConfigError, ValueError) as e:
        print(f"OPENCODE SKIPPED ({e}): opencode chats left out")
        return 0

    def upd(v):
        u = v.get("updated") if isinstance(v, dict) else 0
        return u / 1000.0 if isinstance(u, (int, float)) else 0.0
    live = None
    if in_opencode:  # the chat that runs this is the newest one, still being written
        recent = [v for v in listing if isinstance(v, dict) and now - upd(v) < 600]
        if recent:
            live = max(recent, key=upd).get("id")
    cache = os.path.join(os.path.dirname(reg_path), "sessions-opencode")
    todo = []
    for v in listing:
        sid = v.get("id") if isinstance(v, dict) else None
        if not isinstance(sid, str) or not OC_ID_RE.fullmatch(sid):
            continue
        mtime = upd(v)
        if sid == live or now - mtime < idle:
            continue
        path = os.path.join(cache, f"{sid}.jsonl")
        if up_to_date(reg, sid, path, mtime) and os.path.isfile(path):
            continue
        todo.append((sid, mtime, v.get("directory") or "", path))

    def export(item):
        try:
            return json.loads(oc_run(cmd, ["export", item[0]]))
        except (ConfigError, ValueError):
            return None
    # opencode takes about two seconds per export: four at a time
    with ThreadPoolExecutor(max_workers=4) as pool:
        exports = list(pool.map(export, todo))
    added = 0
    for (sid, mtime, directory, path), ex in zip(todo, exports):
        if ex is None:
            continue
        entries = oc_entries(ex, directory)
        try:
            os.makedirs(cache, exist_ok=True)
            tmp = f"{path}.{os.getpid()}.tmp"
            with open(tmp, "w", encoding="utf-8") as f:
                f.write("".join(json.dumps(e, ensure_ascii=False) + "\n" for e in entries))
            os.replace(tmp, path)
        except OSError:
            continue
        added += register(reg, sid, path, mtime, "opencode", entries, source="opencode")
    return added


def find_session(reg, ident):
    """A registry id from a full id or a unique prefix (4+ characters)."""
    ident = ident.strip().lower()
    # opencode ids mix upper and lower case: match without case, answer with the real id
    exact = [i for i in reg["sessions"] if i.lower() == ident]
    if exact:
        return exact[0]
    hits = [i for i in reg["sessions"] if len(ident) >= 4 and i.lower().startswith(ident)]
    if len(hits) == 1:
        return hits[0]
    raise ConfigError(f"no session matches '{ident}'" if not hits else
                      f"'{ident}' matches {len(hits)} sessions; give more of the id")


def session_label(sid, rec):
    """Name and full id, always together."""
    return f'"{rec["name"]}" ({sid})'


def session_line(sid, rec):
    when = (rec.get("last") or rec.get("started") or "")[:16].replace("T", " ")
    return f"{sid}  {when:16}  {rec['name']}"


def resume_command(sid, rec):
    cwd = rec.get("cwd")
    cmd = f"opencode -s {sid}" if rec.get("source") == "opencode" else f"claude --resume {sid}"
    return f"cd {shlex.quote(cwd)} && {cmd}" if cwd else cmd


def session_text(sid, rec, entries=None):
    """The conversation of a session as plain text: prompts and replies, no tool traffic."""
    parts = [f"SESSION: {rec['name']}", f"SESSION ID: {sid}",
             f"PROJECT FOLDER: {rec.get('cwd') or rec.get('project', '')}",
             f"LAST ACTIVE: {(rec.get('last') or '')[:16].replace('T', ' ')}", ""]
    for e in read_transcript(rec["file"]) if entries is None else entries:
        if e.get("type") == "user":
            text = real_prompt(e)
            who = "USER"
        elif e.get("type") == "assistant":
            msg = e.get("message")
            text = block_text(msg.get("content") if isinstance(msg, dict) else "").strip()
            who = "ASSISTANT"
        else:
            continue
        if text:
            parts.append(f"{who}: {text}\n")
    body = "\n".join(parts)
    if len(body) > SESSION_LOAD_MAX:
        keep = SESSION_LOAD_MAX // 5
        body = (body[:keep] + f"\n\n[... {len(body) - SESSION_LOAD_MAX} characters left out "
                "of the middle ...]\n\n" + body[-(SESSION_LOAD_MAX - keep):])
    return body


def sessions_main(args, cfg, cfg_path, reg_path, out_dir):
    settings = sessions_settings(cfg)
    if settings is None:
        print("SESSIONS OFF: add `[sessions]` with `enabled = true` to the config to track "
              "closed sessions; nothing done")
        return 7
    reg = load_registry(reg_path)
    added = scan_sessions(reg, settings, reg_path=reg_path, in_opencode="OPENCODE" in os.environ)
    try:
        if args.rename:
            sid = find_session(reg, args.rename[0])
            name = clean_name(args.rename[1])
            if not name:
                raise ConfigError("the new name is empty")
            reg["sessions"][sid].update(name=name, name_source="manual")
            save_registry(reg_path, reg)
            print(f"RENAMED {session_label(sid, reg['sessions'][sid])}")
            return 0
        save_registry(reg_path, reg)
        if args.resume:
            sid = find_session(reg, args.resume)
            print(f"SESSION {session_label(sid, reg['sessions'][sid])}")
            print(resume_command(sid, reg["sessions"][sid]))
            return 0
        if args.session_load:
            sid = find_session(reg, args.session_load)
            rec = reg["sessions"][sid]
            if not os.path.isfile(rec.get("file", "")):
                raise ConfigError(f"the transcript of {session_label(sid, rec)} is gone")
            text = session_text(sid, rec)
            doc = f"sessions/{sid}"
            written = write_outputs(out_dir, [doc], {doc: text}, {doc: rec["file"]})
            print(f"SESSION {session_label(sid, rec)}")
            print(f"{len(text):>7} chars  {written[0]}")
            print(f"total: 1 session, {len(text)} chars (~{len(text) // CHARS_PER_K}k tokens)")
            # one chat, against every chat up to it (last active at or before it)
            note = usage_of_chats(cfg, cfg_path, [(len(text), file_size(rec["file"]))],
                                  chats_up_to(reg["sessions"].values(), last_of(rec)))
            if note:
                print(note)
            return 0
    except ConfigError as e:
        print(f"SESSION ERROR: {e}")
        return 8
    # --context also registers sessions that were open a moment ago; the list shows closed ones
    import time
    now, idle = time.time(), float(settings["idle_minutes"]) * 60
    items = sorted(((sid, rec) for sid, rec in reg["sessions"].items()
                    if not isinstance(rec.get("mtime"), (int, float)) or now - rec["mtime"] >= idle),
                   key=lambda kv: kv[1].get("last") or kv[1].get("started") or "", reverse=True)
    print(f"{len(items)} closed sessions tracked ({added} new); newest first")
    for sid, rec in items[:args.limit]:
        print(session_line(sid, rec))
    if len(items) > args.limit:
        print(f"... {len(items) - args.limit} more (--limit N)")
    return 0


# ---------------------------------------------------------------- --context: earlier chats

PRESET_RE = re.compile(r"[a-z0-9][a-z0-9._-]*")
DEFAULT_PRESET = "default"


def shown_name(rec):
    """What a chat is called in the picker: a name the user gave it, otherwise its first prompt."""
    if rec.get("first") and rec.get("name_source") != "manual":
        return rec["first"]
    return rec.get("name", "")


def chat_folder(rec):
    """The heading a chat sits under: its project folder (~ for the home folder)."""
    cwd = rec.get("cwd")
    if cwd:
        home = os.path.expanduser("~")
        if cwd == home or cwd.startswith(home + os.sep):
            return "~" + cwd[len(home):]
        return cwd
    return rec.get("project", "")


def last_of(rec):
    return rec.get("last") or rec.get("started") or ""


def chats_up_to(recs, cutoff):
    """(chats, characters) of the chats last active at or before `cutoff` (all of them when the
    cutoff is unknown): what loading everything up to that chat would have cost."""
    n = c = 0
    for r in recs:
        if not cutoff or last_of(r) <= cutoff:
            n += 1
            c += r.get("chars") or 0
    return n, c


def chat_catalog(reg, skip):
    """Every chat that can be loaded, by folder (the folder used most recently first), newest
    first inside a folder."""
    folders = {}
    for sid, rec in reg["sessions"].items():
        if sid == skip or not os.path.isfile(rec.get("file") or ""):
            continue
        folders.setdefault(chat_folder(rec), []).append((last_of(rec), sid))
    # sorts are stable (also with reverse=True): newest first, ties by name
    order = sorted(sorted(folders), key=lambda f: max(t for t, _ in folders[f]), reverse=True)
    out = []
    for f in order:
        inner = sorted(sorted(folders[f], key=lambda x: x[1]), key=lambda x: x[0], reverse=True)
        out += [sid for _, sid in inner]
    return out


def chat_sections(names, catalog, presets, setup):
    """One popup section per preset, like a custom set; Remember is only pre-ticked on first use
    (new chats come all the time; a one-off load must not replace the preset)."""
    out = []
    for n in names:
        saved = presets.get(n)
        selected = set(saved.get("selected", [])) if saved else set()
        known = set(saved.get("known", [])) if saved else set()
        main = [d for d in catalog if d in selected]
        out.append({"name": n, "kind": "custom", "docs": list(catalog), "main": main,
                    "other": [d for d in catalog if d not in selected],
                    "new": {d for d in catalog if d not in known} if saved else set(),
                    "ticked": set(main), "remember": saved is None or setup,
                    "first": saved is None})
    return out


def context_main(args, cfg, cfg_path, reg_path, presets_path, out_dir):
    """--context [PRESET ...]: pick earlier chats (all folders) in the popup and write each as a
    .txt to load as context. Presets are saved like custom sets, in their own file."""
    settings = sessions_settings(cfg)
    if settings is None:
        print("SESSIONS OFF: add `[sessions]` with `enabled = true` to the config to load earlier "
              "chats; nothing done")
        return 7
    presets = load_conf(presets_path)
    if args.list:
        if not presets:
            print("no context presets saved yet")
        for n, rec in presets.items():
            print(f"{n:20} {len(set(rec.get('selected', []))):>4} chats")
        return 0
    names = list(dict.fromkeys(n.strip().lower() for n in args.sets)) or [DEFAULT_PRESET]
    bad = [n for n in names if not PRESET_RE.fullmatch(n)]
    if bad:
        print(f"bad preset name(s): {', '.join(bad)} (use lowercase letters, digits, . _ -)")
        return 2

    me = os.environ.get("CLAUDE_CODE_SESSION_ID", "").lower()  # never offer the chat running this
    reg = load_registry(reg_path)
    scan_sessions(reg, settings, idle=0, skip=me, reg_path=reg_path, in_opencode="OPENCODE" in os.environ)
    try:
        save_registry(reg_path, reg)
    except OSError as e:
        print(f"could not save the session registry to {reg_path}: {e}")
    catalog = chat_catalog(reg, me)
    if not catalog:
        if settings["opencode"]:
            print(f"NO CHATS: no earlier Claude Code ({settings['projects_dir']}) or opencode chats; "
                  "nothing loaded")
        else:
            print(f"NO CHATS: no earlier Claude Code chats in {settings['projects_dir']}; nothing loaded")
        return 0
    recs = reg["sessions"]
    secs = chat_sections(names, catalog, presets, args.setup)

    picks = None
    if not (args.no_popup or os.environ.get("RESEARCHER_NO_POPUP")):
        texts = dict.fromkeys(catalog, "")  # every chat is readable; sizes come from the registry
        look = {"chats": True, "sizes": {sid: int(recs[sid].get("chars") or 0) for sid in catalog}, "labels": {sid: shown_name(recs[sid]) for sid in catalog},
                "groups": {sid: chat_folder(recs[sid]) for sid in catalog},
                "notes": {sid: last_of(recs[sid])[:10] for sid in catalog}}
        try:
            picks = popup(secs, texts, args.setup, "researcher --context " + " ".join(names), look)
        except RuntimeError as e:
            print(f"NO POPUP ({e}): using the saved presets")
        else:
            if picks is None:
                print("CANCELLED: the user closed the popup, nothing loaded")
                return 3
            if picks == "timeout":
                print(f"TIMEOUT: popup closed after {POPUP_TIMEOUT} s without a choice, nothing loaded")
                return 4
    unsaved = [n for n in names if n not in presets]
    if picks is None and unsaved:
        print(f"NO POPUP: context preset {', '.join(unsaved)} has no saved choice yet; nothing loaded")
        return 5
    gone = []
    if picks is None:  # a saved preset can name a chat whose transcript was deleted since
        have = set(catalog)
        gone = list(dict.fromkeys(d for n in names for d in presets[n].get("selected", [])
                                  if d not in have))
        picks = {s["name"]: (s["ticked"], False) for s in secs}

    for n, (ticked, keep) in picks.items():
        if keep or args.setup:
            presets[n] = {"selected": [d for d in catalog if d in ticked], "known": list(catalog)}
            print(f"SAVED context preset {n}: {len(ticked)} of {len(catalog)} chats")
    if any(keep or args.setup for _, keep in picks.values()):
        try:
            save_conf(presets_path, presets)
        except OSError as e:
            print(f"could not save the presets to {presets_path}: {e}")
    if args.setup:
        print("setup only: nothing loaded")
        return 0

    chosen = {d for t, _ in picks.values() for d in t}
    sids = [d for d in catalog if d in chosen]
    docs = [f"sessions/{sid}" for sid in sids]
    texts = {f"sessions/{sid}": session_text(sid, recs[sid]) for sid in sids}
    try:
        written = write_outputs(out_dir, docs, texts, {f"sessions/{sid}": recs[sid]["file"] for sid in sids})
    except OSError as e:
        print(f"SESSION ERROR: could not write to {out_dir}: {e}")
        return 8
    total = 0
    for sid, doc, path in zip(sids, docs, written):
        total += len(texts[doc])
        print(f'CHAT "{shown_name(recs[sid])}" ({sid})')
        print(f"{len(texts[doc]):>7} chars  {path}")
    for sid in gone:
        print(f"MISSING  chat {sid} (its transcript is gone)")
    print(f"total: {len(docs)} chats, {total} chars (~{total // CHARS_PER_K}k tokens)")
    # the "whole library" of this load: every chat up to the newest one picked
    newest = max((last_of(recs[sid]) for sid in sids), default="")
    note = usage_of_chats(cfg, cfg_path,
                          [(len(texts[doc]), file_size(recs[sid]["file"]))
                           for sid, doc in zip(sids, docs)],
                          chats_up_to((recs[sid] for sid in catalog), newest))
    if note:
        print(note)
    return 0


# ---------------------------------------------------------------- main

def main(argv=None):
    ap = argparse.ArgumentParser(prog="researcher.py", add_help=True,
                                 description="Load saved research documents into Claude Code.")
    ap.add_argument("sets", nargs="*", metavar="SET")
    ap.add_argument("--setup", action="store_true", help="change saved defaults, load nothing")
    ap.add_argument("--no-popup", action="store_true",
                    help="use the saved defaults, ask nothing (same as RESEARCHER_NO_POPUP=1)")
    ap.add_argument("--list", action="store_true", help="list sets and saved defaults")
    ap.add_argument("--sessions", action="store_true",
                    help="list tracked closed sessions (needs [sessions] enabled = true)")
    ap.add_argument("--rename", nargs=2, metavar=("ID", "NAME"), help="rename a tracked session")
    ap.add_argument("--resume", metavar="ID", help="print the command that resumes a session")
    ap.add_argument("--session-load", metavar="ID",
                    help="write a tracked session's conversation as a .txt to load as context")
    ap.add_argument("--context", action="store_true",
                    help="pick earlier Claude Code chats (all folders) to load as context; "
                         "SET names are presets")
    ap.add_argument("--limit", type=int, default=30, help="sessions shown by --sessions (default 30)")
    ap.add_argument("--init", nargs="?", const="", metavar="LIBRARY", help="write a starter config")
    ap.add_argument("--config", help="config.toml path (default ~/.config/researcher/config.toml)")
    ap.add_argument("--library", help="library folder (overrides config and RESEARCHER_LIBRARY)")
    ap.add_argument("--sync", action="store_true",
                    help="make or refresh the plain-text copy of the whole library, load nothing")
    ap.add_argument("--usage", action="store_true",
                    help="print the local usage totals (characters and estimated tokens saved)")
    ap.add_argument("--usage-export", nargs="?", const="researcher-usage.json", metavar="FILE",
                    help="write the anonymous usage records to FILE (default researcher-usage.json) "
                         "to look at; nothing is sent")
    ap.add_argument("--usage-share", choices=("on", "off"),
                    help="share the anonymous usage counts with the project (on) or not (off); "
                         "saved in the config")
    ap.add_argument("--usage-send", action="store_true",
                    help="send the usage records not sent yet (only when sharing is on)")
    ap.add_argument("--dry-run", action="store_true",
                    help="with --usage-send: print what would be sent, send nothing")
    ap.add_argument("--out", help="the researcher's text folder (default ~/.local/share/researcher/text, "
                                  "or text_dir in the config); session loads go to ~/.cache/researcher")
    args = ap.parse_args(argv)
    if args.dry_run and not args.usage_send:
        ap.error("--dry-run goes with --usage-send")

    cfg_path = args.config or os.path.join(config_home(), "config.toml")
    state_path = os.path.join(os.path.dirname(cfg_path), "choices.json")

    if args.usage or args.usage_send or args.usage_share:
        try:
            cfg = load_config(cfg_path)
        except ConfigError as e:
            print(f"CONFIG ERROR: {e}")
            return 6
        if args.usage_share:
            on = args.usage_share == "on"
            try:
                set_share(cfg_path, on)
            except (ConfigError, OSError) as e:
                print(f"CONFIG ERROR: could not save `share` under [usage] in {cfg_path}: {e}")
                return 6
            if on:
                print(f"USAGE SHARING: on (saved as `share = true` under [usage] in {cfg_path}); "
                      f"the anonymous counts are sent to {usage_url(cfg)} after each load. "
                      "Undo: researcher --usage-share off")
            else:
                print(f"USAGE SHARING: off (saved as `share = false` under [usage] in {cfg_path}); "
                      "nothing is sent")
            return 0
        if args.usage_send:
            return usage_send_main(cfg, args.dry_run)
        print(usage_report(read_usage()))
        print(share_line(usage_share(cfg), len(usage_pending())))
        return 0
    if args.usage_export:
        n = usage_export(args.usage_export)
        print(f"wrote {n} records to {os.path.abspath(args.usage_export)}")
        print("each record: day, random install id, document counts per format, character sums "
              "(source bytes, converted, squeezed, whole library). No titles, paths or set names. "
              "Nothing was sent.")
        return 0

    if args.init is not None:
        lib = args.init or os.path.expanduser("~/research")
        return init_config(cfg_path, lib)

    if args.context:
        try:
            cfg = load_config(cfg_path)
        except ConfigError as e:
            print(f"CONFIG ERROR: {e}")
            return 6
        conf_dir = os.path.dirname(cfg_path)
        return context_main(args, cfg, cfg_path, os.path.join(conf_dir, "sessions.json"),
                            os.path.join(conf_dir, "contexts.json"), args.out or cache_home())

    if args.sessions or args.rename or args.resume or args.session_load:
        try:
            cfg = load_config(cfg_path)
        except ConfigError as e:
            print(f"CONFIG ERROR: {e}")
            return 6
        return sessions_main(args, cfg, cfg_path,
                             os.path.join(os.path.dirname(cfg_path), "sessions.json"),
                             args.out or cache_home())

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
        paths = scan(library, cfg.get("exclude", []))
        conf = load_conf(state_path)
        names = resolve_names(args.sets, cfg)
        sets, kinds = build_sets(cfg, list(paths), conf, names)
    except ConfigError as e:
        print(f"CONFIG ERROR: {e}")
        return 6

    store = os.path.abspath(os.path.expanduser(args.out or cfg.get("text_dir") or text_home()))

    if args.sync:
        texts = Texts(paths, store)
        ok = [d for d in paths if d in texts]
        texts.flush()
        chars = sum(len(texts[d]) for d in ok)
        for d in paths:
            if d not in texts:
                print(f"UNREADABLE  {paths[d]} ({texts.errors[d]})")
        print(f"text copy: {len(ok)} of {len(paths)} docs in {store}, {chars} chars "
              f"(~{chars // CHARS_PER_K}k tokens)")
        return 0

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
    texts = Texts(paths, store)
    atexit.register(texts.flush)
    missing = []
    for d in alldocs:
        if d not in paths:
            missing.append(f"MISSING  {d} (not in {library})")
        elif d not in texts:
            missing.append(f"UNREADABLE  {paths[d]} ({texts.errors[d]})")
    secs = sections_for(names, sets, kinds, conf, texts, args.setup)

    picks = None
    if not (args.no_popup or os.environ.get("RESEARCHER_NO_POPUP")):
        try:
            picks = popup(secs, texts, args.setup, "researcher " + " ".join(names))
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

    order = {s["name"]: s["main"] + s["other"] for s in secs}
    for n, (ticked, keep) in picks.items():
        if keep or args.setup:
            conf[n] = {"selected": [d for d in order[n] if d in ticked], "known": list(sets[n])}
            if kinds[n] == "custom":
                conf[n]["custom"] = True
            print(f"SAVED default for {n}: {len(ticked)} of {len(sets[n])} docs")
    if any(keep or args.setup for _, keep in picks.values()):
        save_conf(state_path, conf)
    if args.setup:
        print("setup only: nothing loaded")
        return 0

    chosen = {d for t, _ in picks.values() for d in t}
    docs = [d for d in dict.fromkeys(alldocs + [d for s in secs for d in s["main"] + s["other"]])
            if d in chosen and d in texts]
    written = [texts.path(d) for d in docs]
    texts.flush()
    share_note = None
    if docs and usage_enabled(cfg):
        try:  # the log must never break a load
            share_note = finish_usage(cfg, cfg_path, usage_record(docs, texts, paths))
        except OSError:
            pass
    total = 0
    for d, dst in zip(docs, written):
        total += len(texts[d])
        print(f"{len(texts[d]):>7} chars  {dst}")
    for line in missing:
        print(line)
    skipped = [d for d in alldocs if d in texts and d not in chosen]
    if len(skipped) > SKIPPED_NAMED:  # a custom set or `all`: naming it all costs tokens
        print(f"left out by choice ({len(skipped)}): not named (more than {SKIPPED_NAMED})")
    elif skipped:
        print(f"left out by choice ({len(skipped)}): " + "; ".join(skipped))
    print(f"total: {len(docs)} docs, {total} chars (~{total // CHARS_PER_K}k tokens)")
    if share_note:
        print(share_note)
    return 0


if __name__ == "__main__":
    sys.exit(main())
