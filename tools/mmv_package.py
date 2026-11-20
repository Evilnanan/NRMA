"""Build a separate ME3 overlay; MMV files remain external, read-only inputs."""
from __future__ import annotations

import copy
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import tomllib

# The pinned MMV profile labels these as Optional, but omits optional=true.
# Correct only the generated overlay profile when the corresponding DLL is absent.
OPTIONAL_MMV_NATIVES = {'NightreignFPSFOV.dll', 'UnlockTheFps.dll', 'UltrawideFix.dll'}
PORTABLE_DIRECTORY = 'MayhemAugments'
PORTABLE_PROFILE = 'Mayhem Augments.me3'


def local_profile(overlay: Path) -> Path:
    current = overlay / 'Mayhem Augments-MMV.me3'
    legacy = overlay / 'NRHex-MMV.me3'
    return legacy if legacy.is_file() and not current.exists() else current


def installed_overlay(mmv: Path) -> dict | None:
    path = mmv / PORTABLE_PROFILE
    if not path.exists():
        return None
    parsed = tomllib.loads(path.read_text(encoding='utf-8-sig'))
    packages = [p for p in parsed.get('packages', []) if p.get('id') in ('mayhem-augments-mmv', 'nrhex-mmv')]
    natives = [n for n in parsed.get('natives', []) if n.get('id') in ('mayhem-augments-mmv', 'nrhex-mmv')]
    if (len(packages) != 1 or packages[0].get('path') != PORTABLE_DIRECTORY + '/mod'
            or len(natives) != 1 or natives[0].get('path') not in
            (PORTABLE_DIRECTORY + '/natives/MayhemAugments.dll', PORTABLE_DIRECTORY + '/natives/NRHex.dll', PORTABLE_DIRECTORY + '/mod/natives/NRHex.dll')):
        raise ValueError('Unrecognized file occupies the MA release profile name')
    return parsed


def source_profiles(mmv: Path) -> list[Path]:
    installed = installed_overlay(mmv)
    return sorted(p for p in mmv.glob('*.me3') if not (installed and p.name == PORTABLE_PROFILE))


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def value(v):
    if isinstance(v, bool):
        return str(v).lower()
    if isinstance(v, str):
        return json.dumps(v, ensure_ascii=False)
    if isinstance(v, (int, float)):
        return repr(v)
    if isinstance(v, list):
        return '[' + ', '.join(value(x) for x in v) + ']'
    if isinstance(v, dict):
        return '{ ' + ', '.join(f'{json.dumps(k)} = {value(x)}' for k, x in v.items()) + ' }'
    raise ValueError(f'Unsupported profile field: {v!r}')


def profile(source: Path, overlay: Path, preserve_fixes=True) -> tuple[str, dict]:
    original = tomllib.loads(source.read_text(encoding='utf-8-sig'))
    merged = copy.deepcopy(original)
    if original.get('profileVersion') != 'v1' or len(original.get('packages', [])) != 1:
        raise ValueError('MMV profile structure changed; review package references')
    # Keep every original option, including missing optional DLL references.
    for kind in ('packages', 'natives'):
        for entry in merged.get(kind, []):
            key = 'path' if 'path' in entry else 'source'
            path = Path(entry[key])
            entry[key] = str(path if path.is_absolute() else (source.parent / path).resolve())
            if 'config' in entry:
                path = Path(entry['config'])
                entry['config'] = str(path if path.is_absolute() else (source.parent / path).resolve())
            if kind == 'natives' and not Path(entry[key]).is_file():
                if Path(entry[key]).name in OPTIONAL_MMV_NATIVES:
                    entry['optional'] = True
                elif not entry.get('optional', False):
                    raise ValueError(f'Required MMV DLL is missing: {entry[key]}')
    base = merged['packages'][0]
    base_id = original['packages'][0].get('id', original['packages'][0].get('path', original['packages'][0].get('source')))
    base['id'] = base_id
    if any(n.get('id') == 'mayhem-augments-mmv' for n in merged.get('natives', [])):
        raise ValueError('Mayhem Augments native ID already present in MMV')
    merged['packages'].append(dict(id='mayhem-augments-mmv', path=str((overlay / 'mod').resolve()),
                                  load_after=[dict(id=base_id, optional=False)]))
    merged.setdefault('natives', []).append(dict(id='mayhem-augments-mmv', path=str((overlay / 'natives/MayhemAugments.dll').resolve())))
    # Independent MMV fixes are opt-in. Preserve an explicitly enabled package;
    # deleting this entry disables it permanently, without rebuilding either mod.
    current_profile = local_profile(overlay)
    if preserve_fixes and current_profile.exists():
        current = tomllib.loads(current_profile.read_text(encoding='utf-8-sig'))
        fixes = [p for p in current.get('packages', []) if p.get('id') == 'mmv-fixes']
        if len(fixes) > 1:
            raise ValueError('Duplicate independent MMV fix package')
        if fixes:
            from mmv_fixes import with_package
            path = Path(fixes[0]['path'])
            merged = with_package(merged, path if path.is_absolute() else overlay / path)
    # Arrays and inline tables preserve unknown options instead of discarding them.
    text = '# Generated Mayhem Augments MMV overlay; original MMV profile is unchanged.\n'
    text += '\n'.join(f'{json.dumps(k)} = {value(v)}' for k, v in merged.items()) + '\n'
    if tomllib.loads(text) != merged:
        raise ValueError('Profile serialization did not round-trip')
    return text, merged


def readable_profile(parsed: dict) -> str:
    lines = []
    for key, item in parsed.items():
        if key in ('supports', 'packages', 'natives'):
            continue
        lines.append(f'{key} = {value(item)}')
    for key in ('supports', 'packages', 'natives'):
        for entry in parsed.get(key, []):
            lines += ['', f'[[{key}]]']
            lines += [f'{k} = {value(v)}' for k, v in entry.items()]
    text = '\n'.join(lines) + '\n'
    if tomllib.loads(text) != parsed:
        raise ValueError('Portable profile serialization did not round-trip')
    return text


def portable_profile(source: Path, fixes: Path | None = None) -> tuple[str, dict, dict]:
    # Reuse the required-DLL and source-profile checks, but never inherit a local
    # overlay profile or its absolute installation paths into a distributable.
    _, parsed = profile(source, source.parent / PORTABLE_DIRECTORY, preserve_fixes=False)
    parsed['packages'] = [p for p in parsed['packages'] if p.get('id') != 'mmv-fixes']
    parsed['natives'] = [p for p in parsed['natives'] if p.get('id') != 'mmv-fixes']
    for kind in ('packages', 'natives'):
        for entry in parsed[kind]:
            for key in ('path', 'source', 'config'):
                if key not in entry:
                    continue
                resolved = Path(entry[key]).resolve()
                if not resolved.is_relative_to(source.parent.resolve()):
                    raise ValueError(f'Non-portable MMV reference outside its directory: {entry[key]}')
                entry[key] = resolved.relative_to(source.parent.resolve()).as_posix()
    # The MMV release follows MMV's online/Arxan settings, unlike the standalone
    # public release. The source entry remains untouched.
    payload = {}
    if fixes is not None:
        from mmv_fixes import checked_package
        checked_package(fixes)
        manifest = json.loads((fixes.parent / 'manifest.json').read_text(encoding='utf8'))
        for name in manifest['files']:
            payload['MMV-Fixes/' + name] = (fixes.parent / name).read_bytes()
        parsed['packages'].append(dict(id='mmv-fixes', path='MMV-Fixes/mod',
            load_after=[dict(id=p['id'], optional=False) for p in parsed['packages']]))
        if 'natives/MMVFixes.dll' in manifest['files']:
            parsed['natives'].append(dict(id='mmv-fixes', path='MMV-Fixes/natives/MMVFixes.dll', load_early=True))
    # Only our namespace and optional independent fix namespace may be copied
    # into MMV. Never overwrite an upstream file, even on a future MMV release.
    installed = installed_overlay(source.parent)
    owned = {PORTABLE_DIRECTORY, PORTABLE_PROFILE} if installed else set()
    if installed and any(p.get('id') == 'mmv-fixes' and p.get('path') == 'MMV-Fixes/mod' for p in installed.get('packages', [])):
        owned.add('MMV-Fixes')
    for name in (PORTABLE_DIRECTORY, PORTABLE_PROFILE, *(['MMV-Fixes'] if fixes else [])):
        if (source.parent / name).exists() and name not in owned:
            raise ValueError(f'Portable package name already exists in MMV: {name}')
    return readable_profile(parsed), parsed, payload


def enabled_fixes(root: Path) -> Path | None:
    """Carry the explicitly enabled independent fixes from the tested profile."""
    path = local_profile(root / 'dist/mmv')
    if not path.exists():
        return None
    entries = [p for p in tomllib.loads(path.read_text(encoding='utf-8-sig')).get('packages', [])
               if p.get('id') == 'mmv-fixes']
    if len(entries) > 1:
        raise ValueError('Duplicate independent MMV fix package')
    if not entries:
        return None
    package = Path(entries[0]['path'])
    return package if package.is_absolute() else (path.parent / package).resolve()


def build(root: Path, mmv: Path, files: dict, me3: Path, install=True) -> dict:
    import pefile
    staging = root / 'work/compat/mmv/staged'
    output = root / 'dist/mmv'
    if output.resolve() != output.absolute():
        raise ValueError('MMV output must not be a linked directory')
    sources = source_profiles(mmv)
    if len(sources) != 1:
        raise ValueError('Select an MMV launch profile explicitly before packaging')
    dll = root / 'work/compat/mmv/targets/runtime/release/MayhemAugments.dll'
    image = pefile.PE(str(dll), fast_load=True)
    image.parse_data_directories(directories=[1])
    imports = {i.name for entry in image.DIRECTORY_ENTRY_IMPORT for i in entry.imports}
    if b'GetAsyncKeyState' in imports or b'CageDemand prepared:' not in dll.read_bytes() or b'CageProbe prepared:' in dll.read_bytes() or b'MMV compatibility: verified Legendary Stones' not in dll.read_bytes():
        raise ValueError('MMV DLL feature check failed')
    image.close()
    profile_name = 'Mayhem Augments-MMV.me3'
    payload = {}
    for name, digest in files.items():
        path = staging / 'mod' / name
        if not path.resolve().is_relative_to((staging / 'mod').resolve()) or sha(path).lower() != digest.lower():
            raise ValueError(f'Staged file missing or changed: {name}')
        payload['mod/' + name] = path.read_bytes()
    if not {'chr/c0000.behbnd.dcx', 'action/variablenameid.txt'} <= files.keys():
        raise ValueError('MMV package lacks the Ram behavior graph or variable registry')
    subprocess.run(['dotnet', 'run', '--project', str(root / 'tools/HexBuild'), '--', str(root),
                    '--ram-sync-verify-package', str(staging / 'mod'), str(mmv / 'mod')],
                   check=True)
    payload['natives/MayhemAugments.dll'] = dll.read_bytes()
    ini = output / 'natives' / 'MayhemAugments.ini'
    if not ini.exists() and (ini.parent / 'NRHex.ini').is_file():
        ini = ini.parent / 'NRHex.ini'
    fallback_ini = root / 'dist/natives/MayhemAugments.ini'
    config = (ini if ini.exists() else fallback_ini).read_text(encoding='utf-8-sig')
    retired = {'VirtualKey', 'TestVirtualKey', 'NewHexVirtualKey', 'ChantVirtualKey', 'GazeVirtualKey', 'ContractVirtualKey',
               'AngelChantVirtualKey', 'StormTestVirtualKey', 'RequireControl', 'CooldownMs', 'SwordNearDistance'}
    config = '\n'.join(line for line in config.splitlines() if line.split('=', 1)[0].strip() not in retired and not line.startswith('; Ctrl+'))
    config = config.replace('[TestSpawn]', '[Settings]')
    payload['natives/MayhemAugments.ini'] = (config + '\n').encode('utf8')
    text, parsed = profile(sources[0], output)
    payload[profile_name] = text.encode('utf8')
    # Validate the concrete profile before publishing any new files.
    preview = root / 'work/compat/mmv/profile-preview-dist.me3'
    preview.write_text(text, encoding='utf8')
    with (preview.parent / 'profile-validation-dist.log').open('w', encoding='utf8') as log:
        subprocess.run([str(me3), 'profile', 'show', '--file', str(preview)], check=True, stdout=log, stderr=subprocess.STDOUT)
    state = root / 'work/compat/mmv/package-state.json'
    previous = json.loads(state.read_text(encoding='utf8')) if state.exists() else {}
    stale = [name for name in previous.keys() - payload.keys() if (output / name).exists()]
    renamed = {'natives/NRHex.dll', 'natives/NRHex.ini', 'NRHex-MMV.me3'}
    if install:
        for name in stale:
            p = output / name
            if name not in renamed or p.resolve() != p.absolute() or not p.resolve().is_relative_to(output.resolve()):
                raise ValueError(f'Retired generated file requires review: {p}')
            if name == 'natives/NRHex.ini':
                if ini != p and p.read_bytes() != ini.read_bytes():
                    raise ValueError(f'Conflicting current and legacy settings: {p}')
            elif sha(p) != previous[name]:
                raise ValueError(f'Retired generated file was edited: {p}')
            if p.suffix == '.dll':
                with p.open('r+b'):
                    pass
    # Do not silently replace manual profile edits or unrelated files on rebuild.
    for name in payload:
        p = output / name
        if p.resolve() != p.absolute():
            raise ValueError(f'MMV output contains a linked path: {p}')
        if install and p.exists() and name != 'natives/MayhemAugments.ini' and sha(p) not in (previous.get(name), hashlib.sha256(payload[name]).hexdigest()):
            raise ValueError(f'Output was edited outside the MMV build: {p}')
    # Fresh build-only directories cannot retain obsolete files from an older
    # layout (notably mod/natives or retired profile files).
    destination = output if install else Path(tempfile.mkdtemp(prefix='ready-dist-', dir=root / 'work/compat/mmv'))
    if install:
        # Detect a loaded DLL before replacing any resources. A build-only result
        # can be prepared while the user keeps the current expedition running.
        current_dll = output / 'natives' / 'MayhemAugments.dll'
        if current_dll.exists():
            with current_dll.open('r+b'):
                pass
    for name, data in payload.items():
        p = destination / name
        p.parent.mkdir(parents=True, exist_ok=True)
        temporary = p.with_name(p.name + '.ma-new')
        temporary.write_bytes(data)
        temporary.replace(p)
    hashes = {name: sha(destination / name) for name in payload}
    if install:
        for name in stale:
            (output / name).unlink()
        temporary_state = state.with_suffix('.tmp')
        temporary_state.write_text(json.dumps(hashes, indent=2), encoding='utf8')
        temporary_state.replace(state)
    return dict(output=str(output), files=hashes, profile=str(output / profile_name),
                variant='dist', installed=install, stagedPackage=str(destination),
                portableOverlay=False, independentFixesIncluded=False,
                mmvProfile=str(sources[0]), mmvProfileSha256=sha(sources[0]),
                testHotkeys=False, profileValidated=True, gameplayVerified=False, nativeCoexistenceVerified=False,
                packageOrder=[p['id'] for p in parsed['packages']])
