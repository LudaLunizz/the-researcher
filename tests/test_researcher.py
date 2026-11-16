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
        self.assertEqual(files, ["rust - linux.txt", "rust - memory-a.txt", "rust - memory-b.txt"])
        with open(os.path.join(self.out, "rust - linux.txt"), encoding="utf-8") as f:
            body = f.read()
        self.assertIn("Linux", body)
        self.assertNotIn("x()", body)

    def test_alias_and_default_set(self):
        self.assertEqual(self.run_main("--no-popup", "rs")[0], 0)
        self.assertEqual(self.run_main("--no-popup")[0], 0)

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

    def test_old_outputs_removed_but_foreign_files_kept(self):
        self.run_main("--no-popup", "rust")
        keep = os.path.join(self.out, "mine.txt")
        write(keep, "mine")
        write(self.cfg, read(self.cfg).replace(
            'docs = ["rust/linux", "rust/memory-*"]', 'docs = ["rust/linux"]'))
        self.run_main("--no-popup", "rust")
        files = sorted(f for f in os.listdir(self.out) if f.endswith(".txt"))
        self.assertEqual(files, ["mine.txt", "rust - linux.txt"])

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


if __name__ == "__main__":
    unittest.main()
