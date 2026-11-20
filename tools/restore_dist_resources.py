"""Fill missing inputs for incremental standalone resource builders."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]


LEGACY_URL = 'https://github.com/Evilnanan/NRHex.git'


def legacy_repo(revision, root=ROOT):
    """Resolve a pinned NRHex fixture without importing old commits into NRMA."""
    cache = root / 'work/nrhex-baseline.git'
    sibling = root.parent / 'NRHex'
    for candidate in (sibling, cache):
        if candidate.is_dir() and subprocess.run(
                ['git', '-C', str(candidate), 'cat-file', '-e', revision + '^{commit}'],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
            return candidate
    cache.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(['git', 'init', '--bare', '--quiet', str(cache)], check=True)
    subprocess.run(['git', '-C', str(cache), 'fetch', '--depth=1', '--no-tags',
                    LEGACY_URL, revision], check=True)
    return cache


def restore(root=ROOT, archive=None):
    baseline = json.loads((root / 'data/standalone-resource-baseline.json').read_text(encoding='utf8'))
    target = root / 'dist/mod'
    missing = [name for name in baseline['files'] if not (target / name).is_file()]
    if not missing:
        return
    revision = baseline['revision']
    archive = archive or os.environ.get('NRHEX_RESOURCE_BASELINE')
    default_zip = root / 'releases' / f'standalone-resource-baseline-{revision[:7]}.zip'
    if not archive and default_zip.is_file():
        archive = default_zip
    with tempfile.TemporaryDirectory(dir=root / 'dist') as tmp:
        staging = Path(tmp)
        def stage(name, data):
            if hashlib.sha256(data).hexdigest() != baseline['files'][name]:
                raise ValueError(f'Wrong resource baseline for {name}')
            path = staging / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)

        if archive:
            with zipfile.ZipFile(archive) as source:
                for name in missing:
                    stage(name, source.read('mod/' + name))
        else:
            repo = legacy_repo(revision, root)
            for name in missing:
                blob = subprocess.run(['git', 'show', f'{revision}:dist/mod/{name}'],
                                      cwd=repo, capture_output=True)
                if blob.returncode:
                    raise SystemExit(f'Missing resource baseline. Set NRHEX_RESOURCE_BASELINE to the baseline ZIP '
                                     f'with resources from NRHex {revision[:7]}.')
                stage(name, blob.stdout)
        for name in missing:
            dest = target / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            # Do not overwrite a working copy or a concurrently installed resource.
            try:
                os.link(staging / name, dest)
            except FileExistsError:
                pass
    print(f'Restored {len(missing)} standalone resource inputs from {revision[:7]}.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', type=Path, help='Offline baseline ZIP with mod/ at its root')
    restore(archive=parser.parse_args().archive)
