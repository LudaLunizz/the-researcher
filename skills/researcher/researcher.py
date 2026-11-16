#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# The Researcher. Copyright (c) 2026 LudaLunizz
# Licensed under the MIT License; see the file LICENSE for details.
"""The Researcher: a research loader for Claude (saved research into a Claude Code session).

Usage: researcher.py [SET ...]             popup, then write the ticked docs as .txt files
       researcher.py --setup SET [...]     popup only, to change the saved defaults
       researcher.py --no-popup SET [...]  use the saved defaults, ask nothing
       researcher.py --list                show sets, saved defaults, library size
       researcher.py --init [LIBRARY]      write a starter config.toml
       researcher.py --sessions | --rename ID NAME | --resume ID | --session-load ID
                                           closed-session tracking (opt-in: [sessions] in config)

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
folder. `exclude = [globs]` in config.toml leaves documents out of the library. When `x.html` and `x.pdf` both exist, the HTML one is used.
"""
import argparse
import fnmatch
import html
import json
import os
import re
import shlex
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
# The Researcher: config
# The folder that holds your research documents (.html, .md, .txt, .pdf).
library = "{library}"

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


class Texts:
    """Lazy {doc id: plain text}. `d in texts` is False for documents that cannot be read."""

    def __init__(self, paths):
        self.paths, self.cache, self.errors = paths, {}, {}

    def __contains__(self, d):
        if d in self.cache:
            return True
        if d not in self.paths or d in self.errors:
            return False
        try:
            self.cache[d] = to_text(self.paths[d])
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

def popup(sections, texts, setup, command=""):
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
    esc = lambda t: GLib.markup_escape_text(t, -1)
    loop = GLib.MainLoop()
    result = {"value": None}
    boxes = {}  # set name -> [(doc, CheckButton)]
    remember = {}

    def k(d):
        return len(texts.get(d, "")) / CHARS_PER_K

    def folder(d):
        return d.rpartition("/")[0]

    win = Gtk.Window(title="The Researcher - " +
                     ("change saved presets" if setup else "choose research to load"))
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
        head.set_markup(f"<big><b>Claude Code wants to load research</b></big> for {names}\n"
                        "Tick what this session needs, then press <b>Load</b>. Nothing is sent to "
                        "Claude until then; <b>Cancel</b> loads nothing."
                        + (f"\n<small>Asked by: <tt>{esc(command)}</tt></small>" if command else ""))
    outer.append(head)
    scroll = Gtk.ScrolledWindow(vexpand=True)
    body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
    scroll.set_child(body)
    outer.append(scroll)
    total = Gtk.Label(xalign=0)

    def update(*_):
        c = {d for lst in boxes.values() for d, cb in lst if cb.get_active()}
        total.set_markup(f"<b>{len(c)} documents, ~{sum(k(d) for d in c):.0f}k tokens</b>"
                         + ("" if setup else " will be loaded"))
        ok.set_label(f"Save presets" if setup else f"Load {len(c)} documents")
        ok.set_sensitive(setup or bool(c))

    def add_docs(target, docs, sec, lst):
        """Check boxes for `docs`, with a small folder heading when more than one folder shows."""
        folders = {folder(d) for d in docs}
        last = None
        for d in docs:
            if len(folders) > 1 and folder(d) != last:
                last = folder(d)
                h = Gtk.Label(xalign=0)
                h.set_markup(f"<small><b>{esc(last + '/' if last else '(top level)')}</b></small>")
                h.set_margin_top(6)
                h.set_margin_start(12)
                target.append(h)
            cb = Gtk.CheckButton()
            l = Gtk.Label(xalign=0)
            shown = d.rpartition("/")[2] if len(folders) > 1 else d
            if d in texts:
                extra = "  <span foreground='#ffcc33'><b>NEW</b></span>" if d in sec["new"] else ""
                l.set_markup(f"{esc(shown)}  <small>~{k(d):.0f}k</small>{extra}")
                cb.set_active(d in sec["ticked"])
            else:
                l.set_markup(f"<s>{esc(shown)}</s>  <span foreground='#ff6666'>"
                             "MISSING or unreadable</span>")
                cb.set_sensitive(False)
            cb.set_child(l)
            cb.set_margin_start(24 if len(folders) > 1 else 12)
            cb.connect("toggled", update)
            target.append(cb)
            lst.append((d, cb))

    for sec in sections:
        n = sec["name"]
        custom = sec["kind"] == "custom"
        if sec["first"]:
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
        for text, fn in (("Select all", lambda l: [cb.set_active(cb.get_sensitive()) for _, cb in l]),
                         ("Select none", lambda l: [cb.set_active(False) for _, cb in l]),
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
            ex = Gtk.Expander(label=("Your library: tick the research to use"
                                     if not sec["main"] else
                                     f"Add other research from your library ({len(sec['other'])} more)"))
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


# ---------------------------------------------------------------- sessions (opt-in)
# Closed Claude Code sessions: a registry keyed by the FULL session id, so a name can never
# be separated from the id it belongs to. Names are taken from Claude Code's own title or the
# first prompt, and can be renamed (a manual name is never overwritten by a rescan).

SESSION_ID_RE = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
NAME_MAX = 80
SESSION_LOAD_MAX = 200000  # chars; a longer session keeps its start and its end
SESSION_DEFAULTS = {"idle_minutes": 10, "projects_dir": "~/.claude/projects"}


def sessions_settings(cfg):
    sec = cfg.get("sessions")
    if not isinstance(sec, dict) or not sec.get("enabled"):
        return None
    out = dict(SESSION_DEFAULTS)
    for k in out:
        if k in sec:
            out[k] = sec[k]
    out["projects_dir"] = os.path.abspath(os.path.expanduser(str(out["projects_dir"])))
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
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(reg, f, indent=1, ensure_ascii=False)
    os.replace(tmp, path)


def scan_sessions(reg, settings, now=None):
    """Add closed sessions to the registry and refresh the changed ones. Returns the number added."""
    import time
    now = time.time() if now is None else now
    added = 0
    base = settings["projects_dir"]
    try:
        projects = sorted(os.listdir(base))
    except OSError:
        return 0
    for proj in projects:
        pdir = os.path.join(base, proj)
        try:
            files = os.listdir(pdir)
        except OSError:
            continue
        for fn in files:
            sid = fn[:-6]
            if not fn.endswith(".jsonl") or not SESSION_ID_RE.fullmatch(sid):
                continue
            path = os.path.join(pdir, fn)
            try:
                mtime = os.path.getmtime(path)
            except OSError:
                continue
            if now - mtime < float(settings["idle_minutes"]) * 60:
                continue  # probably still open
            rec = reg["sessions"].get(sid)
            if rec and rec.get("mtime") == mtime and rec.get("file") == path:
                continue
            info = session_info(read_transcript(path))
            if not info["first"] and not info["title"]:
                continue  # nothing was ever asked
            auto = clean_name(info["title"] or info["first"])
            if rec is None:
                rec = reg["sessions"][sid] = {"name": auto, "name_source": "auto",
                                              "registered": time.strftime("%Y-%m-%d %H:%M")}
                added += 1
            elif rec.get("name_source") != "manual":
                rec["name"] = auto
            rec.update(file=path, mtime=mtime, project=proj, cwd=info["cwd"],
                       started=info["started"], last=info["last"])
    return added


def find_session(reg, ident):
    """A registry id from a full id or a unique prefix (4+ characters)."""
    ident = ident.strip().lower()
    if ident in reg["sessions"]:
        return ident
    hits = [i for i in reg["sessions"] if len(ident) >= 4 and i.startswith(ident)]
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
    cmd = f"claude --resume {sid}"
    return f"cd {shlex.quote(cwd)} && {cmd}" if cwd else cmd


def session_text(sid, rec):
    """The conversation of a session as plain text: prompts and replies, no tool traffic."""
    parts = [f"SESSION: {rec['name']}", f"SESSION ID: {sid}",
             f"PROJECT FOLDER: {rec.get('cwd') or rec.get('project', '')}",
             f"LAST ACTIVE: {(rec.get('last') or '')[:16].replace('T', ' ')}", ""]
    for e in read_transcript(rec["file"]):
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


def sessions_main(args, cfg, reg_path, out_dir):
    settings = sessions_settings(cfg)
    if settings is None:
        print("SESSIONS OFF: add `[sessions]` with `enabled = true` to the config to track "
              "closed sessions; nothing done")
        return 7
    reg = load_registry(reg_path)
    added = scan_sessions(reg, settings)
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
            return 0
    except ConfigError as e:
        print(f"SESSION ERROR: {e}")
        return 8
    items = sorted(reg["sessions"].items(),
                   key=lambda kv: kv[1].get("last") or kv[1].get("started") or "", reverse=True)
    print(f"{len(items)} closed sessions tracked ({added} new); newest first")
    for sid, rec in items[:args.limit]:
        print(session_line(sid, rec))
    if len(items) > args.limit:
        print(f"... {len(items) - args.limit} more (--limit N)")
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
    ap.add_argument("--limit", type=int, default=30, help="sessions shown by --sessions (default 30)")
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

    if args.sessions or args.rename or args.resume or args.session_load:
        try:
            cfg = load_config(cfg_path)
        except ConfigError as e:
            print(f"CONFIG ERROR: {e}")
            return 6
        return sessions_main(args, cfg, os.path.join(os.path.dirname(cfg_path), "sessions.json"),
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
    texts = Texts(paths)
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
