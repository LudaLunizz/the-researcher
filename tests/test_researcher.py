import contextlib
import importlib.util
import io
import json
import os
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SPEC = importlib.util.spec_from_file_location(
    "researcher", os.path.join(HERE, "..", "skills", "researcher", "researcher.py"))
R = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(R)


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def write(path, text=""):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)


class Env(unittest.TestCase):
    def setUp(self):
        os.environ["RESEARCHER_NO_POPUP"] = "1"  # tests must never open a window
        self.addCleanup(os.environ.pop, "RESEARCHER_NO_POPUP", None)
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.data = os.path.join(self.tmp.name, "data")  # the usage log must never reach a real home
        os.environ["XDG_DATA_HOME"] = self.data
        self.addCleanup(os.environ.pop, "XDG_DATA_HOME", None)
        self.lib = os.path.join(self.tmp.name, "lib")
        self.conf_dir = os.path.join(self.tmp.name, "conf")
        self.out = os.path.join(self.tmp.name, "out")
        write(os.path.join(self.lib, "rust", "linux.html"), "<h1>Linux</h1><p>one</p><script>x()</script>")
        write(os.path.join(self.lib, "rust", "memory-a.md"), "# A")
        write(os.path.join(self.lib, "rust", "memory-b.txt"), "B")
        write(os.path.join(self.lib, "web", "seo.md"), "seo")
        write(os.path.join(self.lib, "app", "roadmap", "index.html"), "<p>road</p>")
        write(os.path.join(self.lib, ".hidden", "x.md"), "no")
        write(os.path.join(self.lib, "rust", "notes.png"), "no")
        self.cfg = os.path.join(self.conf_dir, "config.toml")
        write(self.cfg, f'''library = "{self.lib}"
default_set = "rust"
[aliases]
rs = "rust"
[sets.rust]
docs = ["rust/linux", "rust/memory-*"]
[sets.app]
include = ["rust"]
docs = ["app/roadmap", "app/ghost"]
''')

    def run_main(self, *args):
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            code = R.main(["--config", self.cfg, "--out", self.out, *args])
        return code, buf.getvalue()


class SqueezeTests(unittest.TestCase):
    def test_pdf_layout_padding_and_blank_runs(self):
        raw = "Title\n\n\n\n      Indented  line  here\nName          Value          Unit\n"
        out = R.squeeze(raw, "pdf")
        self.assertEqual(out, "Title\n\nIndented line here\nName | Value | Unit")

    def test_pdf_page_furniture_removed_content_kept(self):
        pages = [f"Report 2026 - page {i}\nreal text {i}\nSecret Report\n" for i in range(1, 6)]
        out = R.squeeze("\f".join(pages), "pdf")
        self.assertNotIn("Report 2026", out)
        self.assertNotIn("Secret Report", out)
        for i in range(1, 6):
            self.assertIn(f"real text {i}", out)

    def test_pdf_short_document_keeps_its_header(self):
        self.assertIn("Report", R.squeeze("Report\nbody\f\nReport\nmore\n", "pdf"))

    def test_rules_and_invisible_characters_go(self):
        out = R.squeeze("a\u200bb\u00a0c\n\u2500\u2500\u2500\u2500\n----------\nd", "html")
        self.assertEqual(out, "ab c\n\nd")

    def test_markdown_keeps_indentation_and_short_rules(self):
        raw = "- a\n    - nested\n\n```\n  code\n```\n---\n"
        self.assertEqual(R.squeeze(raw, "plain"), raw.strip())
        self.assertEqual(R.squeeze("x\n==========\ny", "plain"), "x\n\ny")

    def test_exact_repeat_lines_collapse(self):
        self.assertEqual(R.squeeze("same\nsame\nother", "plain"), "same\nother")


class UsageTests(Env):
    def records(self):
        with open(R.usage_file(), encoding="utf-8") as f:
            return [json.loads(line) for line in f]

    def test_load_logs_counts_only(self):
        self.run_main("--no-popup", "rust")
        (rec,) = self.records()
        self.assertEqual(rec["docs"], 3)
        self.assertEqual(rec["fmt"], {"pdf": 0, "html": 1, "md": 1, "txt": 1})
        self.assertEqual(rec["lib_docs"], 5)
        self.assertGreater(rec["lib_chars"], rec["chars"])
        self.assertGreaterEqual(rec["raw"], rec["chars"])
        self.assertRegex(rec["id"], r"^[0-9a-f]{16}$")
        self.assertEqual(rec["tk"], R.CHARS_PER_K)

    def test_log_holds_no_names_paths_or_titles(self):
        self.run_main("--no-popup", "rust")
        raw = read(R.usage_file()) + read(os.path.join(self.data, "researcher", "usage-id"))
        for word in ("rust", "linux", "memory", "roadmap", "seo", self.lib, self.tmp.name, "ludalunizz"):
            self.assertNotIn(word, raw)

    def test_same_install_id_every_load(self):
        self.run_main("--no-popup", "rust")
        self.run_main("--no-popup", "rs")
        a, b = self.records()
        self.assertEqual(a["id"], b["id"])

    def test_log_can_be_switched_off(self):
        write(self.cfg, read(self.cfg) + "\n[usage]\nlog = false\n")
        self.assertEqual(self.run_main("--no-popup", "rust")[0], 0)
        self.assertFalse(os.path.exists(R.usage_file()))
        self.assertFalse(os.path.exists(os.path.join(self.data, "researcher", "usage-id")))

    def test_setup_sync_list_and_nothing_loaded_do_not_log(self):
        self.run_main("--sync")
        self.run_main("--list")
        self.run_main("--no-popup", "newproj")  # custom set, nothing chosen
        self.assertFalse(os.path.exists(R.usage_file()))

    def test_unwritable_log_does_not_break_a_load(self):
        write(os.path.join(self.data, "researcher"), "a file where the folder should be")
        self.assertEqual(self.run_main("--no-popup", "rust")[0], 0)

    def test_usage_report_and_export(self):
        self.run_main("--no-popup", "rust")
        code, out = self.run_main("--usage")
        self.assertEqual(code, 0)
        self.assertIn("1 loads", out)
        self.assertIn("squeezed away", out)
        dst = os.path.join(self.tmp.name, "export.json")
        code, out = self.run_main("--usage-export", dst)
        self.assertIn("Nothing was sent", out)
        data = json.loads(read(dst))
        self.assertEqual(data["schema"], 1)
        self.assertEqual(len(data["loads"]), 1)

    def test_usage_with_no_log(self):
        self.assertIn("no usage recorded yet", self.run_main("--usage")[1])

    def test_summary_math(self):
        recs = [{"v": 1, "docs": 2, "raw": 4600, "chars": 2300, "lib_chars": 23000, "tk": 2300},
                {"v": 1, "docs": 1, "raw": 1000, "chars": 1000, "lib_chars": 1000, "tk": 2300}]
        t = R.usage_summary(recs)
        self.assertEqual(t["squeezed"], 2300)
        self.assertEqual(t["unticked"], 20700)
        self.assertAlmostEqual(t["tokens_squeezed"], 1000)
        self.assertAlmostEqual(t["tokens_unticked"], 9000)

    def test_bad_lines_skipped(self):
        write(R.usage_file(), 'not json\n{"v": 99}\n{"v": 1, "docs": 1}\n')
        self.assertEqual(len(R.read_usage()), 1)


class ScanTests(Env):
    def test_scan_ids(self):
        ids = list(R.scan(self.lib))
        self.assertEqual(ids, ["app/roadmap", "rust/linux", "rust/memory-a", "rust/memory-b", "web/seo"])

    def test_exclude_globs(self):
        ids = list(R.scan(self.lib, ["web/*", "app/*"]))
        self.assertEqual(ids, ["rust/linux", "rust/memory-a", "rust/memory-b"])

    def test_html_preferred_over_pdf(self):
        write(os.path.join(self.lib, "web", "seo.html"), "<p>h</p>")
        write(os.path.join(self.lib, "web", "seo.pdf"), "%PDF")
        self.assertTrue(R.scan(self.lib)["web/seo"].endswith("seo.html"))

    def test_html_text(self):
        t = R.html_text("<h1>T</h1><ul><li>a</li><li>b &amp; c</li></ul><style>p{}</style>")
        self.assertEqual(t, "T\n- a\n- b & c")


class SetTests(Env):
    def test_glob_include_and_missing(self):
        cat = list(R.scan(self.lib))
        cfg = R.load_config(self.cfg)
        sets, kinds = R.build_sets(cfg, cat, {}, [])
        self.assertEqual(sets["rust"], ["rust/linux", "rust/memory-a", "rust/memory-b"])
        self.assertEqual(sets["app"], sets["rust"] + ["app/roadmap", "app/ghost"])
        self.assertEqual(kinds["all"], "all")
        self.assertEqual(sets["all"], cat)

    def test_include_loop(self):
        defs = {"a": {"include": ["b"]}, "b": {"include": ["a"]}}
        with self.assertRaises(R.ConfigError):
            R.expand("a", defs, [])

    def test_unknown_include(self):
        with self.assertRaises(R.ConfigError):
            R.expand("a", {"a": {"include": ["zz"]}}, [])

    def test_custom_set_first_use_ticks_nothing(self):
        cat = list(R.scan(self.lib))
        sets, kinds = R.build_sets({}, cat, {}, ["side"])
        self.assertEqual(kinds["side"], "custom")
        texts = {d: "x" for d in cat}
        sec = R.sections_for(["side"], sets, kinds, {}, texts, False)[0]
        self.assertEqual(sec["ticked"], set())
        self.assertTrue(sec["first"] and sec["remember"])

    def test_custom_set_new_docs_not_ticked(self):
        cat = list(R.scan(self.lib))
        conf = {"side": {"custom": True, "selected": ["web/seo"], "known": cat[:-1]}}
        sets, kinds = R.build_sets({}, cat, conf, ["side"])
        texts = {d: "x" for d in cat}
        sec = R.sections_for(["side"], sets, kinds, conf, texts, False)[0]
        self.assertEqual(sec["new"], {cat[-1]})
        self.assertEqual(sec["ticked"], {"web/seo"})

    def test_defined_set_new_docs_ticked(self):
        conf = {"rust": {"selected": ["rust/linux"], "known": ["rust/linux", "rust/memory-a"]}}
        cat = list(R.scan(self.lib))
        sets, kinds = R.build_sets(R.load_config(self.cfg), cat, conf, [])
        texts = {d: "x" for d in cat}
        sec = R.sections_for(["rust"], sets, kinds, conf, texts, False)[0]
        self.assertEqual(sec["new"], {"rust/memory-b"})
        self.assertEqual(sec["ticked"], {"rust/linux", "rust/memory-b"})

    def test_defined_set_offers_rest_of_library_and_keeps_extras(self):
        conf = {"rust": {"selected": ["rust/linux", "web/seo"],
                         "known": ["rust/linux", "rust/memory-a", "rust/memory-b"]}}
        cat = list(R.scan(self.lib))
        sets, kinds = R.build_sets(R.load_config(self.cfg), cat, conf, [])
        texts = {d: "x" for d in cat}
        sec = R.sections_for(["rust"], sets, kinds, conf, texts, False)[0]
        self.assertEqual(sec["main"], sets["rust"] + ["web/seo"])
        self.assertEqual(sec["other"], ["app/roadmap"])
        self.assertEqual(sec["ticked"], {"rust/linux", "web/seo"})

    def test_extra_doc_from_library_is_loaded_with_saved_preset(self):
        write(os.path.join(self.conf_dir, "choices.json"), json.dumps(
            {"rust": {"selected": ["rust/linux", "web/seo"],
                      "known": ["rust/linux", "rust/memory-a", "rust/memory-b"]}}))
        code, out = self.run_main("--no-popup", "rust")
        self.assertIn("total: 2 docs", out)
        self.assertTrue(os.path.exists(os.path.join(self.out, "web - seo.txt")))

    def test_invalid_name_not_custom(self):
        sets, _ = R.build_sets({}, [], {}, ["Bad Name!"])
        self.assertNotIn("Bad Name!", sets)


class MainTests(Env):
    def test_no_popup_first_use_loads_everything_in_set(self):
        code, out = self.run_main("--no-popup", "rust")
        self.assertEqual(code, 0, out)
        self.assertIn("total: 3 docs", out)
        files = sorted(f for f in os.listdir(self.out) if f.endswith(".txt"))
        # the usage log sizes the whole library, so every document gets its text copy
        self.assertEqual(files, ["app - roadmap.txt", "rust - linux.txt", "rust - memory-a.txt",
                                 "rust - memory-b.txt", "web - seo.txt"])
        with open(os.path.join(self.out, "rust - linux.txt"), encoding="utf-8") as f:
            body = f.read()
        self.assertIn("Linux", body)
        self.assertNotIn("x()", body)

    def test_alias_and_default_set(self):
        self.assertEqual(self.run_main("--no-popup", "rs")[0], 0)
        self.assertEqual(self.run_main("--no-popup")[0], 0)

    def test_set_names_keep_their_case(self):
        with open(self.cfg, "a", encoding="utf-8") as f:
            f.write('[sets.MyRust]\ndocs = ["rust/linux"]\n')
        code, out = self.run_main("--no-popup", "MyRust")
        self.assertEqual(code, 0, out)
        self.assertIn("total: 1 docs", out)
        self.assertEqual(R.resolve_names(["myrust"], R.load_config(self.cfg)), ["MyRust"])

    def test_two_ids_with_one_store_name_get_two_copies(self):
        write(os.path.join(self.lib, "web - seo.md"), "other seo")
        for _ in range(2):
            paths = R.scan(self.lib, [])
            texts = R.Texts(paths, self.out)
            self.assertEqual(texts["web/seo"], "seo")
            self.assertEqual(texts["web - seo"], "other seo")
            self.assertNotEqual(texts.path("web/seo"), texts.path("web - seo"))
            texts.flush()
        self.assertEqual(R.id_hash("a - b"), 0xF0551209)  # pinned: the Rust version must agree
        self.assertEqual(R.id_hash("é/x"), 0x1DA23496)

    def test_long_left_out_list_is_only_counted(self):
        for i in range(25):
            write(os.path.join(self.lib, "many", f"d{i:02}.md"), f"doc {i}")
        write(os.path.join(self.conf_dir, "choices.json"), json.dumps(
            {"mine": {"selected": ["web/seo"], "known": [], "custom": True}}))
        code, out = self.run_main("--no-popup", "mine")
        self.assertEqual(code, 0, out)
        self.assertIn("left out by choice (29): not named (more than 20)", out)
        self.assertNotIn("many/d00", out)

    def test_missing_doc_reported(self):
        code, out = self.run_main("--no-popup", "app")
        self.assertEqual(code, 0)
        self.assertIn("MISSING  app/ghost", out)
        self.assertIn("total: 4 docs", out)

    def test_env_switch_blocks_popup(self):
        code, out = self.run_main("rust")  # no --no-popup flag, env switch only
        self.assertEqual(code, 0, out)
        self.assertNotIn("NO POPUP (", out)

    def test_custom_without_choice_loads_nothing(self):
        code, out = self.run_main("--no-popup", "newproj")
        self.assertEqual(code, 5)
        self.assertIn("nothing loaded", out)

    def test_unknown_set_exit_2(self):
        self.assertEqual(self.run_main("--no-popup", "Not A Name!")[0], 2)

    def test_no_library(self):
        write(self.cfg, "")
        code, out = self.run_main("--no-popup", "x")
        self.assertEqual(code, 6)
        self.assertIn("NO LIBRARY", out)

    def test_bad_toml(self):
        write(self.cfg, "library = [")
        self.assertEqual(self.run_main("--no-popup", "x")[0], 6)

    def test_text_copies_persist_and_foreign_files_kept(self):
        self.run_main("--no-popup", "rust")
        keep = os.path.join(self.out, "mine.txt")
        write(keep, "mine")
        write(self.cfg, read(self.cfg).replace(
            'docs = ["rust/linux", "rust/memory-*"]', 'docs = ["rust/linux"]'))
        code, out = self.run_main("--no-popup", "rust")
        self.assertIn("total: 1 docs", out)
        files = sorted(f for f in os.listdir(self.out) if f.endswith(".txt"))
        self.assertIn("mine.txt", files)
        self.assertIn("rust - memory-a.txt", files)  # not loaded this time, but kept
        self.assertEqual(read(keep), "mine")

    def test_library_is_never_changed(self):
        before = {}
        for root, _, fs in os.walk(self.lib):
            for fn in fs:
                p = os.path.join(root, fn)
                before[p] = (read(p), os.stat(p).st_mtime_ns)
        self.run_main("--no-popup", "rust")
        self.run_main("--sync")
        after = {}
        for root, _, fs in os.walk(self.lib):
            for fn in fs:
                p = os.path.join(root, fn)
                after[p] = (read(p), os.stat(p).st_mtime_ns)
        self.assertEqual(before, after)

    def test_copy_reused_until_source_changes(self):
        self.run_main("--no-popup", "rust")
        copy = os.path.join(self.out, "rust - memory-a.txt")
        head = read(copy).split("\n", 1)[0]  # the SOURCE line must stay
        write(copy, f"{head}\n\nEDITED COPY\n")  # an unchanged source must not be converted again
        _, out = self.run_main("--no-popup", "rust")
        self.assertIn("EDITED COPY", read(copy))
        write(os.path.join(self.lib, "rust", "memory-a.md"), "# A changed")
        self.run_main("--no-popup", "rust")
        self.assertIn("A changed", read(copy))
        self.assertNotIn("EDITED", read(copy))

    def test_copy_remade_when_converter_version_changes(self):
        self.run_main("--no-popup", "rust")
        copy = os.path.join(self.out, "rust - memory-a.txt")
        write(copy, "SOURCE: x\n\nOLD FORMAT\n")
        old, R.TEXT_VERSION = R.TEXT_VERSION, R.TEXT_VERSION + 1
        self.addCleanup(setattr, R, "TEXT_VERSION", old)
        self.run_main("--no-popup", "rust")
        self.assertNotIn("OLD FORMAT", read(copy))

    def test_sync_copies_whole_library_and_loads_nothing(self):
        code, out = self.run_main("--sync")
        self.assertEqual(code, 0, out)
        self.assertIn("text copy: 5 of 5 docs", out)
        self.assertTrue(os.path.isfile(os.path.join(self.out, "web - seo.txt")))
        self.assertTrue(os.path.isfile(os.path.join(self.out, "app - roadmap.txt")))

    def test_text_dir_in_config(self):
        elsewhere = os.path.join(self.tmp.name, "mytext")
        write(self.cfg, f'text_dir = "{elsewhere}"\n' + read(self.cfg))
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            R.main(["--config", self.cfg, "--no-popup", "rust"])
        self.assertTrue(os.path.isfile(os.path.join(elsewhere, "rust - linux.txt")))

    def test_saved_default_respected(self):
        write(os.path.join(self.conf_dir, "choices.json"), json.dumps(
            {"rust": {"selected": ["rust/linux"], "known": ["rust/linux", "rust/memory-a", "rust/memory-b"]}}))
        code, out = self.run_main("--no-popup", "rust")
        self.assertIn("total: 1 docs", out)
        self.assertIn("left out by choice (2)", out)

    def test_list(self):
        code, out = self.run_main("--list")
        self.assertEqual(code, 0)
        self.assertIn("library:", out)
        self.assertIn("rust", out)
        self.assertIn("all", out)

    def test_init(self):
        os.remove(self.cfg)
        self.assertEqual(R.main(["--config", self.cfg, "--init", self.lib]), 0)
        self.assertIn(self.lib, read(self.cfg))
        self.assertEqual(R.load_config(self.cfg)["library"], self.lib)
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(R.main(["--config", self.cfg, "--init", self.lib]), 1)  # no overwrite

    def test_example_library_loads(self):
        lib = os.path.join(HERE, "..", "examples", "library")
        code, out = self.run_main("--library", lib, "--no-popup", "all")
        self.assertEqual(code, 0, out)
        self.assertIn("total: 8 docs", out)

    def test_example_config_parses(self):
        cfg = R.load_config(os.path.join(HERE, "..", "examples", "config.toml"))
        self.assertIn("myapp", cfg["sets"])

SID1 = "11111111-1111-4111-8111-111111111111"
SID2 = "22222222-2222-4222-8222-222222222222"
SID3 = "33333333-3333-4333-8333-333333333333"


def jl(path, entries, age=3600):
    write(path, "\n".join(json.dumps(e) for e in entries) + "\nnot json\n")
    t = __import__("time").time() - age
    os.utime(path, (t, t))


def user(text, ts="2026-11-16T10:00:00Z", **kw):
    return {"type": "user", "timestamp": ts, "cwd": "/work/app",
            "message": {"role": "user", "content": text}, **kw}


def assistant(text, ts="2026-11-16T10:01:00Z"):
    return {"type": "assistant", "timestamp": ts,
            "message": {"role": "assistant", "content": [
                {"type": "thinking", "thinking": "hmm"}, {"type": "text", "text": text},
                {"type": "tool_use", "name": "Bash", "input": {"command": "secret-tool-call"}}]}}


class SessionTests(Env):
    def setUp(self):
        super().setUp()
        self.proj = os.path.join(self.tmp.name, "projects")
        self.reg = os.path.join(self.conf_dir, "sessions.json")
        with open(self.cfg, "a", encoding="utf-8") as f:
            f.write(f'\n[sessions]\nenabled = true\nprojects_dir = "{self.proj}"\n')
        d = os.path.join(self.proj, "-work-app")
        jl(os.path.join(d, SID1 + ".jsonl"), [
            {"type": "file-history-snapshot"},
            user("<system-reminder>ignore me</system-reminder>", isMeta=True),
            user("<command-message>x</command-message>"),
            user("Fix the popup crash on   start-up"),
            assistant("Found it, the timeout was wrong."),
            {"type": "user", "message": {"content": [{"type": "tool_result", "content": "TOOLOUT"}]}},
        ])
        jl(os.path.join(d, SID2 + ".jsonl"), [user("Second one"), {"type": "ai-title", "aiTitle": "Nice title"}])
        jl(os.path.join(d, SID3 + ".jsonl"), [user("Still open")], age=5)  # modified just now
        jl(os.path.join(d, "agent-abc.jsonl"), [user("not a session id")])

    def test_off_by_default(self):
        write(self.cfg, f'library = "{self.lib}"\n')
        code, out = self.run_main("--sessions")
        self.assertEqual(code, 7)
        self.assertIn("SESSIONS OFF", out)
        self.assertFalse(os.path.exists(self.reg))

    def test_lists_closed_sessions_with_full_id(self):
        code, out = self.run_main("--sessions")
        self.assertEqual(code, 0)
        self.assertIn(f"{SID1}  2026-11-16 10:01  Fix the popup crash on start-up", out)
        self.assertIn(f"{SID2}", out)
        self.assertIn("Nice title", out)  # Claude Code's own title wins over the first prompt
        self.assertNotIn(SID3, out)       # still open: not tracked yet
        self.assertNotIn("agent-abc", out)
        self.assertIn("2 closed sessions", out)

    def test_rename_sticks_and_keeps_the_id(self):
        self.run_main("--sessions")
        code, out = self.run_main("--rename", SID1[:8], "popup crash")
        self.assertEqual(code, 0)
        self.assertIn(f'"popup crash" ({SID1})', out)
        _, out = self.run_main("--sessions")  # a rescan must not undo a manual name
        self.assertIn(f"{SID1}  2026-11-16 10:01  popup crash", out)
        with open(self.reg, encoding="utf-8") as f:
            reg = json.load(f)
        self.assertEqual(reg["sessions"][SID1]["name_source"], "manual")

    def test_auto_name_follows_a_later_title_but_manual_does_not(self):
        self.run_main("--sessions")
        path = os.path.join(self.proj, "-work-app", SID1 + ".jsonl")
        jl(path, [user("Fix the popup crash"), {"type": "custom-title", "customTitle": "Crash hunt"}])
        _, out = self.run_main("--sessions")
        self.assertIn("Crash hunt", out)

    def test_ambiguous_and_unknown_ids(self):
        self.run_main("--sessions")
        code, out = self.run_main("--resume", "ffff")
        self.assertEqual(code, 8)
        self.assertIn("no session matches", out)
        code, out = self.run_main("--rename", "", "x")
        self.assertEqual(code, 8)

    def test_resume_prints_cd_and_full_id(self):
        self.run_main("--sessions")
        code, out = self.run_main("--resume", SID1)
        self.assertEqual(code, 0)
        self.assertIn(f"cd /work/app && claude --resume {SID1}", out)

    def test_session_load_writes_conversation_without_tool_traffic(self):
        self.run_main("--sessions")
        code, out = self.run_main("--session-load", SID1[:6])
        self.assertEqual(code, 0)
        path = [l.split("chars  ", 1)[1] for l in out.splitlines() if "chars  " in l and "total" not in l][0]
        text = read(path)
        self.assertIn(f"SESSION ID: {SID1}", text)
        self.assertIn("USER: Fix the popup crash on   start-up", text)
        self.assertIn("ASSISTANT: Found it", text)
        for bad in ("ignore me", "TOOLOUT", "secret-tool-call", "hmm", "command-message"):
            self.assertNotIn(bad, text)

    def test_no_library_needed(self):
        write(self.cfg, f'[sessions]\nenabled = true\nprojects_dir = "{self.proj}"\n')
        self.assertEqual(self.run_main("--sessions")[0], 0)



if __name__ == "__main__":
    unittest.main()
