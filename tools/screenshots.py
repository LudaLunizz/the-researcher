#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# The Researcher. Copyright (C) 2026 LudaLunizz
"""Make the README screenshots of the popup from the made-up example library.

Usage: tools/screenshots.py [OUTPUT_DIR]      (default docs/img)

Needs a display: it opens the real popup twice for about two seconds each, captures the
window content from inside GTK (no screenshot tool, no desktop in the picture) and closes
it. It uses its own temporary config and saved choices and only reads examples/library,
so none of your own research or settings is touched or shown.
"""
import importlib.util
import json
import os
import sys
import tempfile

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Graphene", "1.0")
from gi.repository import GLib, Gtk  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
spec = importlib.util.spec_from_file_location(
    "researcher", os.path.join(ROOT, "skills", "researcher", "researcher.py"))
R = importlib.util.module_from_spec(spec)
spec.loader.exec_module(R)

LIB = os.path.join(ROOT, "examples", "library")
SHOT = {"path": None}
_present = Gtk.Window.present


def present(self):
    self.set_default_size(720, 560)  # less empty space in the pictures
    _present(self)

    def snap():
        w, h = self.get_width(), self.get_height()
        snapshot = Gtk.Snapshot()
        Gtk.WidgetPaintable.new(self).snapshot(snapshot, w, h)
        node = snapshot.to_node()
        texture = self.get_native().get_renderer().render_texture(node, None)
        texture.save_to_png(SHOT["path"])
        self.close()  # the popup treats this as Cancel: nothing is loaded
        return False

    GLib.timeout_add(1500, snap)


Gtk.Window.present = present


def shoot(conf_dir, out_dir, name, set_name):
    SHOT["path"] = os.path.join(out_dir, name)
    code = R.main(["--config", os.path.join(conf_dir, "config.toml"),
                   "--out", os.path.join(conf_dir, "out"), set_name])
    assert code == 3, f"popup ended with exit {code}"  # 3 = closed, nothing loaded
    print("wrote", SHOT["path"])


def main():
    out_dir = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "docs", "img"))
    os.makedirs(out_dir, exist_ok=True)
    os.environ.pop("RESEARCHER_NO_POPUP", None)
    with tempfile.TemporaryDirectory() as conf_dir:
        with open(os.path.join(conf_dir, "config.toml"), "w", encoding="utf-8") as f:
            f.write(f'library = "{LIB}"\n\n[sets.rust]\ndocs = ["rust/*"]\n')
        catalog = list(R.scan(LIB))
        known = [d for d in catalog if d != "rust/performance"]  # performance shows as NEW
        with open(os.path.join(conf_dir, "choices.json"), "w", encoding="utf-8") as f:
            json.dump({"rust": {"selected": ["rust/linux-notes", "rust/security"],
                                "known": known}}, f)
        shoot(conf_dir, out_dir, "popup-saved-preset.png", "rust")
        shoot(conf_dir, out_dir, "popup-new-set.png", "side-project")


if __name__ == "__main__":
    main()
