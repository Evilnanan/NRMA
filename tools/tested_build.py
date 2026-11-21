"""Capture the inputs and installed payload of a successful local test build."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
DLL = "natives/MayhemAugments.dll"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read(path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False, mode="w", encoding="utf8") as stream:
        temp = Path(stream.name)
        json.dump(value, stream, ensure_ascii=False, sort_keys=True)
    try:
        temp.replace(path)
    finally:
        temp.unlink(missing_ok=True)


def receipt_path(root, variant):
    return root / "work/build" / f"tested-{variant}.json"


def resources(folder):
    return {p.relative_to(folder).as_posix(): digest(p) for p in folder.rglob("*")
            if p.is_file() and p.suffix != ".log" and "natives" not in p.relative_to(folder).parts}


def sources(root, variant):
    # Include working-tree edits and new files, including before their first commit.
    specs = ["Cargo.*", "build.rs", "src", "build.ps1", "build-texts.ps1", "tools",
             "assets", "data", "docs/hexes.md", "docs/hexes.en.md", ".cargo", "rust-toolchain*"]
    if variant == "mmv":
        specs.append("compat/mmv")
    paths = subprocess.check_output(["git", "ls-files", "-z", "--cached", "--others",
                                     "--exclude-standard", "--", *specs], cwd=root)
    groups = {"runtime": {}, "texts": {}, "generators": {}, "adapter": {}}
    for raw in paths.split(b"\0"):
        if not raw:
            continue
        name = raw.decode("utf-8", "surrogateescape")
        path = root / name
        if not path.is_file():
            continue
        if name.startswith(("src/", "Cargo.", ".cargo/", "rust-toolchain")) or name == "build.rs":
            group = "runtime"
        elif name.startswith("compat/mmv/") or "mmv" in path.name.lower():
            if variant != "mmv":
                continue
            group = "adapter"
        elif name in ("docs/hexes.md", "docs/hexes.en.md", "data/text/status-effects.json"):
            group = "texts"
        else:
            group = "generators"
        groups[group][name] = digest(path)
    # Explicit alternate Rust data inputs are also part of a standalone build.
    if variant == "standalone":
        for key, value in os.environ.items():
            if key.startswith("NRHEX_") and key.endswith("_DATA") and value:
                groups["runtime"][key] = digest(Path(value))
    for key in ("NRHEX_CARGO_FEATURES", "NRHEX_CARGO_TARGET", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS",
                "CARGO_BUILD_TARGET", "RUSTUP_TOOLCHAIN"):
        if key in os.environ:
            groups["runtime"][key] = os.environ[key]
    return groups


def snapshot(root, variant):
    result = sources(root, variant)
    if variant == "mmv":
        from mmv_package import enabled_fixes
        from mmv_fixes import checked_package
        result["shared"] = resources(root / "dist/mod")
        fixes = enabled_fixes(root)
        if fixes is not None:
            checked_package(fixes)
            result["fixes"] = read(fixes.parent / "manifest.json")["files"]
    return result


def installed(root, variant):
    dist = root / ("dist/mmv" if variant == "mmv" else "dist")
    return {**{"mod/" + k: v for k, v in resources(dist / "mod").items()}, DLL: digest(dist / DLL)}


def record(root, variant, before, expected_dll, *, sfx=None, expected_files=None):
    current = snapshot(root, variant)
    if before != current:
        raise ValueError("Build inputs changed during verification; retry before promoting this build")
    payload = installed(root, variant)
    if payload[DLL] != expected_dll:
        raise ValueError("Installed DLL differs from the verified build")
    if expected_files is not None:
        expected = {k: v for k, v in expected_files.items() if k.startswith("mod/") or k == DLL}
        if expected != payload:
            raise ValueError("Installed MMV resources differ from the verified stage")
    data = dict(schema=1, variant=variant, sources=current, payload=payload)
    if sfx:
        data["originalSfx"] = sfx
    save(receipt_path(root, variant), data)


def plan(root, variant):
    path = receipt_path(root, variant)
    if not path.is_file():
        return "initial", "No installed test build recorded for this variant"
    receipt = read(path)
    if receipt.get("schema") != 1 or receipt.get("variant") != variant:
        return "initial", "The previous build record needs refreshing"
    current = snapshot(root, variant)
    old = receipt["sources"]
    if current["generators"] != old["generators"]:
        return "resources", "Resource or generator inputs changed"
    try:
        payload = installed(root, variant)
    except FileNotFoundError:
        payload = {"mod/" + k: v for k, v in resources(root / ("dist/mmv/mod" if variant == "mmv" else "dist/mod")).items()}
    if variant == "standalone":
        changed = {k for k in payload.keys() | receipt["payload"].keys()
                   if k != DLL and payload.get(k) != receipt["payload"].get(k)}
        if any(not k.startswith("mod/msg/") for k in changed):
            return "resources", "Installed standalone resources changed"
        if changed:
            return "texts", "Installed text resources changed"
    if current["texts"] != old["texts"]:
        return "texts", "Text inputs changed"
    if current != old or payload != receipt["payload"]:
        return "build", "Runtime, shared inputs or installed payload changed"
    return "reuse", "Installed DLL, resources and build inputs match"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["snapshot", "ready", "record", "plan", "next-patch"])
    parser.add_argument("variant", choices=["standalone", "mmv"])
    parser.add_argument("--dll", type=Path)
    parser.add_argument("--before", type=Path)
    args = parser.parse_args()
    if args.action == "snapshot":
        save(args.before, snapshot(ROOT, args.variant))
    elif args.action == "ready":
        before = read(args.before)
        current = snapshot(ROOT, args.variant)
        # The standalone generators legitimately replace compiled Rust catalogs.
        if any(current[k] != before[k] for k in current if k != "runtime"):
            raise ValueError("Resource inputs changed during generation")
        save(args.before, {"sources": current, "resources": resources(ROOT / "dist/mod")})
    elif args.action == "record":
        ready = read(args.before)
        if ready["resources"] != resources(ROOT / "dist/mod"):
            raise ValueError("Standalone resources changed during verification")
        record(ROOT, args.variant, ready["sources"], digest(args.dll))
    elif args.action == "plan":
        action, reason = plan(ROOT, args.variant)
        print(action)
    else:
        version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
        labels = ["Mayhem Augments-", "Mayhem Augments-MMV-"]
        if any((ROOT / "releases" / f"{label}{version}.zip").is_file() for label in labels):
            major, minor, patch = version.split(".")
            new = f"{major}.{minor}.{int(patch) + 1}"
            files = [ROOT / "Cargo.toml", ROOT / "Cargo.lock"]
            old = f'name = "mayhem-augments"\nversion = "{version}"'.encode()
            for path in files:
                if path.read_bytes().replace(b"\r\n", b"\n").count(old) != 1:
                    raise ValueError(f"Cannot identify current package version in {path}")
            for path in files:
                data = path.read_bytes()
                key = old.replace(b"\n", b"\r\n") if b"\r\n" in data else old
                path.write_bytes(data.replace(key, key.replace(version.encode(), new.encode())))
            version = new
        print(f"Test and release version: {version}")


if __name__ == "__main__":
    main()
