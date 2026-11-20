"""Produce a clean Mayhem Augments standalone or MMV folder and verified ZIP."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib
import zipfile

import pefile

ROOT = Path(__file__).resolve().parents[1]
NAME = 'Mayhem Augments'


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-directory', type=Path, default=ROOT / 'releases')
    parser.add_argument('--variant', choices=['standalone', 'mmv'], default='standalone')
    parser.add_argument('--mmv-root', type=Path, default=os.environ.get('NRHEX_MMV', 'D:/Users/OwO/Documents/me3-profiles/MMV'))
    args = parser.parse_args()
    mmv = args.variant == 'mmv'
    dist = ROOT / ('dist/mmv' if mmv else 'dist')
    prefix = 'MayhemAugments/' if mmv else ''
    mmv_root = args.mmv_root.resolve()
    if mmv and args.output_directory.resolve().is_relative_to(mmv_root):
        raise ValueError('Choose an output directory outside the installed MMV')
    output = args.output_directory.resolve()
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text(encoding='utf8'))['package']['version']
    label = f'{NAME}-MMV' if mmv else NAME
    folder = output / f'{label}-{version}'
    archive = folder.with_name(folder.name + '.zip')
    if folder.exists() or archive.exists():
        raise ValueError('Release already exists; choose another output directory or update the version')
    dll_file = dist / 'natives/MayhemAugments.dll'
    dll = dll_file.read_bytes()
    assert version.encode() in dll, f'Rebuild {args.variant} for the current version'
    with pefile.PE(data=dll, fast_load=True) as image:
        image.parse_data_directories(directories=[1])
        imports = {i.name for entry in image.DIRECTORY_ENTRY_IMPORT for i in entry.imports}
    assert b'GetAsyncKeyState' not in imports, 'Test hotkeys must not ship'
    assert b'CageDemand prepared:' in dll, 'Required cage-demand runtime missing'
    assert b'CageProbe prepared:' not in dll, 'Diagnostic build must not ship'
    assert (b'MMV compatibility: verified Legendary Stones' in dll) == mmv, 'Wrong runtime for the release variant'
    # Never copy dist itself, its testmods, nested MMV, profiles or logs.
    payload = {}
    inputs = {}
    for p in sorted((dist / 'mod').rglob('*')):
        if not p.is_file() or 'natives' in p.relative_to(dist / 'mod').parts:
            continue
        if p.suffix == '.log':
            continue
        is_registry = p == dist / 'mod/action/variablenameid.txt'
        if (not is_registry and p.suffix not in ('.bin', '.dcx', '.hks')) or p.resolve() != p.absolute():
            raise ValueError(f'Unexpected release asset: {p}')
        data = p.read_bytes()
        payload[prefix + p.relative_to(dist).as_posix()] = data
        inputs[p] = sha(data)
    assert {prefix + 'mod/regulation.bin', prefix + 'mod/event/common.emevd.dcx'} <= payload.keys()
    payload[prefix + 'natives/MayhemAugments.dll'] = dll
    payload[prefix + 'natives/MayhemAugments.ini'] = b'[Settings]\n'
    profile = f'{NAME}.me3'
    payload[profile] = b'''profileVersion = "v1"

[[supports]]
game = "nightreign"

[[natives]]
path = "natives/MayhemAugments.dll"

[[packages]]
id = "mayhem-augments"
source = "mod"
'''
    if mmv:
        from mmv_build import inventory, validate_lock, LOCK
        from mmv_package import source_profiles, portable_profile, enabled_fixes
        validate_lock(json.loads(LOCK.read_text(encoding='utf8')), inventory(mmv_root))
        sources = source_profiles(mmv_root)
        if len(sources) != 1:
            raise ValueError('Expected one upstream MMV profile')
        state = json.loads((ROOT / 'work/compat/mmv/package-state.json').read_text(encoding='utf8'))
        installed = {p.relative_to(dist).as_posix(): digest for p, digest in inputs.items()}
        installed['natives/MayhemAugments.dll'] = sha(dll)
        expected = {name: h for name, h in state.items() if name.startswith('mod/') or name == 'natives/MayhemAugments.dll'}
        if installed != expected:
            raise ValueError('MMV distribution differs from its verified build. Rebuild dist/mmv first.')
        text, _, fixes = portable_profile(sources[0], enabled_fixes(ROOT))
        payload[profile] = text.encode('utf8')
        payload.update(fixes)
    me3 = Path(os.environ.get('NRHEX_ME3', str(Path(ROOT.anchor) / 'Program Files/me3/bin/me3.exe')))
    reports = ROOT / '.work/release'
    reports.mkdir(parents=True, exist_ok=True)
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.mayhem-stage-', dir=output) as temporary:
        stage = Path(temporary) / folder.name
        for name, data in payload.items():
            p = stage / name
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_bytes(data)
            assert p.read_bytes() == data, name
        subprocess.run(['dotnet', 'run', '--project', str(ROOT / 'tools/HexBuild'), '--', str(ROOT),
                        '--ram-sync-verify-package', str(stage / prefix / 'mod'), str(mmv_root / 'mod') if mmv else 'original'], check=True)
        with (reports / f'profile-validation-{args.variant}.log').open('w', encoding='utf8') as log:
            subprocess.run([str(me3), 'profile', 'show', '--file', str(stage / profile)],
                           check=True, stdout=log, stderr=subprocess.STDOUT)
        zip_prefix = '' if mmv else folder.name + '/'
        staged_zip = Path(temporary) / archive.name
        with zipfile.ZipFile(staged_zip, 'w', zipfile.ZIP_DEFLATED, compresslevel=6) as z:
            for name in payload:
                z.write(stage / name, zip_prefix + name)
        with zipfile.ZipFile(staged_zip) as z:
            assert z.testzip() is None
            assert set(z.namelist()) == {zip_prefix + name for name in payload}
            for name, data in payload.items():
                assert sha(z.read(zip_prefix + name)) == sha(data), name
        assert all(sha(p.read_bytes()) == h for p, h in inputs.items()), 'Resources changed while packaging'
        assert dll_file.read_bytes() == dll
        stage.rename(folder)
        try:
            staged_zip.rename(archive)
        except OSError:
            folder.rename(stage)
            raise
    report = dict(name=NAME, version=version, folder=str(folder), archive=str(archive),
                  archiveSha256=sha(archive.read_bytes()), files={name: sha(data) for name, data in payload.items()},
                  testHotkeys=False, mmv=mmv, independentFixesIncluded=any(p.startswith('MMV-Fixes/') for p in payload), profileValidated=True,
                  archiveReadbackVerified=True, gameplayVerified=False)
    (reports / (f'{version}-mmv.json' if mmv else f'{version}.json')).write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf8')
    print(f'PASS: {label} {version}; {len(payload)} verified files; {archive}')


if __name__ == '__main__':
    main()
