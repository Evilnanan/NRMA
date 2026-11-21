import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import tested_build as tested


class PromotionTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        for name in ("src/lib.rs", "tools/generate.py", "docs/hexes.md", "compat/mmv/target.json",
                     "dist/mod/regulation.bin", "dist/mmv/mod/regulation.bin", "dist/natives/MayhemAugments.dll",
                     "dist/mmv/natives/MayhemAugments.dll"):
            self.write(name, name)
        for variant in ("standalone", "mmv"):
            self.record(variant)

    def record(self, variant):
        tested.record(self.root, variant, tested.snapshot(self.root, variant),
                      tested.installed(self.root, variant)[tested.DLL])

    def write(self, name, value):
        p = self.root / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(value, encoding="utf8")

    def check(self, variant, expected):
        self.assertEqual(tested.plan(self.root, variant)[0], expected)

    def test_reuse_worktree_not_revision(self):
        subprocess.run(["git", "add", "src/lib.rs"], cwd=self.root, check=True)
        self.write("docs/notes.md", "no effect")
        self.write("dist/natives/MayhemAugments.log", "new log")
        self.check("standalone", "reuse")
        self.check("mmv", "reuse")

    def test_changed_source_and_assets(self):
        self.write("src/new.rs", "new")
        self.check("standalone", "build")
        self.check("mmv", "build")
        self.write("dist/mod/regulation.bin", "different")
        self.check("standalone", "resources")
        self.write("tools/generate.py", "changed")
        self.check("mmv", "resources")

    def test_text_and_adapter(self):
        self.write("compat/mmv/target.json", "changed")
        self.check("standalone", "reuse")
        self.check("mmv", "build")
        self.write("docs/hexes.md", "changed")
        self.check("standalone", "texts")
        self.check("mmv", "texts")

    def test_removed_dll_and_changed_mmv_resource(self):
        (self.root / "dist/natives/MayhemAugments.dll").unlink()
        self.check("standalone", "build")
        self.check("mmv", "reuse")
        self.write("dist/mmv/mod/extra.bin", "new")
        self.check("mmv", "build")

    def test_failed_build_cannot_stamp_new_output(self):
        old = tested.snapshot(self.root, "mmv")
        self.write("src/lib.rs", "during build")
        with self.assertRaises(ValueError):
            tested.record(self.root, "mmv", old, "bad")
        self.check("mmv", "build")

    def test_version_prepared_only_once(self):
        import sys
        import tested_build
        for name in ("Cargo.toml", "Cargo.lock"):
            self.write(name, '[package]\nname = "mayhem-augments"\nversion = "1.2.3"\n')
        self.write("releases/Mayhem Augments-1.2.3.zip", "already released")
        with patch.object(tested_build, "ROOT", self.root), patch.object(sys, "argv", ["receipt", "next-patch", "mmv"]):
            tested.main()
            tested.main()
        for name in ("Cargo.toml", "Cargo.lock"):
            self.assertIn('version = "1.2.4"', (self.root / name).read_text())
