"""Reuse the verified first-stage MMV parameter and message merge."""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import zipfile
import zlib

from local_paths import GAME, SMITHBOX
from tested_build import digest

MESSAGES = tuple(f"msg/{language}/{kind}_dlc01.msgbnd.dcx"
                 for language in ("engus", "zhocn") for kind in ("item", "menu"))
FILES = ("regulation.bin", *MESSAGES)


def input_key(root, mmv, tool):
    # This stage reads three versions of the four texts, three regulations, and the
    # MMV player animation for the Wylder repair. Other adapters run independently.
    paths = [GAME / "regulation.bin", *GAME.glob("oo2core*.dll"),
             *(root / "assets/original" / name for name in MESSAGES),
             *(folder / name for folder in (root / "dist/mod", mmv / "mod") for name in FILES),
             mmv / "mod/chr/c0000.anibnd.dcx",
             *(SMITHBOX / "Assets/PARAM/NR/Defs").glob("*.xml"),
             *tool.parent.glob("*.dll"), *tool.parent.glob("*.json"), Path(__file__)]
    data = {str(path): digest(path) for path in paths}
    return hashlib.sha256(json.dumps(data, sort_keys=True).encode()).hexdigest()


def valid_payload(manifest, files):
    return (isinstance(manifest, dict) and manifest.get("stage") == "partial-preview"
            and manifest.get("schemaVersion") == 1 and manifest.get("releaseReady") is False
            and type(manifest.get("parameterTables")) is int
            and manifest.get("files") == {name: hashlib.sha256(files[name]).hexdigest().upper() for name in FILES})


def read_cache(path, key):
    try:
        with zipfile.ZipFile(path) as archive:
            if archive.read("key") != key.encode():
                return None
            raw = archive.read("manifest.json")
            files = {name: archive.read(name) for name in FILES}
            if valid_payload(json.loads(raw), files):
                return raw, files
    except (FileNotFoundError, zipfile.BadZipFile, KeyError, ValueError, EOFError, zlib.error):
        pass
    return None


def merge(root, mmv, reports, tool):
    stage = reports / "staged"
    cache = reports / "param-text-cache.zip"
    key = input_key(root, mmv, tool)
    hit = read_cache(cache, key)
    if hit is not None:
        raw, files = hit
        (stage / "manifest.json").unlink(missing_ok=True)
        for name, data in files.items():
            target = stage / "mod" / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            if digest(target) != hashlib.sha256(data).hexdigest():
                raise ValueError(f"MMV cached stage readback failed: {name}")
        if input_key(root, mmv, tool) != key:
            raise ValueError("MMV parameter/text inputs changed during cache restore")
        (stage / "manifest.json").write_bytes(raw)
        (reports / "stage.log").write_text("Reused verified MMV parameters and messages.\n", encoding="utf8")
        print("Reused verified MMV parameters and messages.", flush=True)
        return
    with (reports / "stage.log").open("w", encoding="utf8") as log:
        subprocess.run(["dotnet", str(tool), str(root), "--mmv-stage-safe", str(mmv)],
                       cwd=root, stdout=log, stderr=subprocess.STDOUT, check=True)
    raw = (stage / "manifest.json").read_bytes()
    files = {name: (stage / "mod" / name).read_bytes() for name in FILES}
    if not valid_payload(json.loads(raw), files) or input_key(root, mmv, tool) != key:
        raise ValueError("MMV parameter/text inputs or staged outputs changed during merge")
    # Later adapters edit these files in place. Store a separate immutable snapshot.
    with tempfile.TemporaryDirectory(dir=reports) as temp:
        new_cache = Path(temp) / cache.name
        with zipfile.ZipFile(new_cache, "w") as archive:
            archive.writestr("key", key)
            archive.writestr("manifest.json", raw)
            for name, data in files.items():
                archive.writestr(name, data)
        new_cache.replace(cache)
