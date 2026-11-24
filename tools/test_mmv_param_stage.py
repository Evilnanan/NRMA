"""The next MMV adapter mutates the stage, so reuse must restore the original merge."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import mmv_param_stage as stage


class ParamStageTests(unittest.TestCase):
    def test_reuse_invalidation_and_damaged_cache(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            mmv, reports, tool = root / "mmv", root / "reports", root / "tools/HexBuild.dll"
            definitions = root / "Assets/PARAM/NR/Defs/NpcParam.xml"
            inputs = [root / "regulation.bin", tool, definitions, root / "oo2core_test.dll",
                      mmv / "mod/chr/c0000.anibnd.dcx", *(root / "assets/original" / n for n in stage.MESSAGES),
                      *(folder / n for folder in (mmv / "mod", root / "dist/mod") for n in stage.FILES)]
            for path in inputs:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"input")
            reports.mkdir()
            output = reports / "staged"
            def generate(*args, **kwargs):
                hashes = {}
                for name in stage.FILES:
                    target = output / "mod" / name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(name.encode())
                    hashes[name] = hashlib.sha256(target.read_bytes()).hexdigest().upper()
                manifest = dict(schemaVersion=1, stage="partial-preview", releaseReady=False, parameterTables=15, files=hashes)
                (output / "manifest.json").write_text(json.dumps(manifest))
            with patch.multiple(stage, GAME=root, SMITHBOX=root), patch.object(stage.subprocess, "run", side_effect=generate) as build:
                def merge():
                    stage.merge(root, mmv, reports, tool)
                merge()
                (output / "mod/regulation.bin").write_bytes(b"modified by later adapter")
                merge()
                self.assertEqual(build.call_count, 1)
                self.assertEqual((output / "mod/regulation.bin").read_bytes(), b"regulation.bin")
                for index, path in enumerate(inputs, start=2):
                    path.write_bytes(b"changed")
                    merge()
                    self.assertEqual(build.call_count, index, path)
                (reports / "param-text-cache.zip").write_bytes(b"damaged")
                merge()
                self.assertEqual(build.call_count, len(inputs) + 2)
