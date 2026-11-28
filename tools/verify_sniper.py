"""Read back the installed reward, owner range and extended Loretta volley; test proc timing."""
import os
import subprocess
from local_paths import ROOT

subprocess.run(["dotnet", "run", "--project", str(ROOT / "tools/HexBuild"), "--",
                str(ROOT), "--sniper-check"], cwd=ROOT, check=True)
cmd = ["cargo", "test", "--release", "--lib"]
if target := os.environ.get("NRHEX_CARGO_TARGET"):
    cmd.extend(["--target", target])
result = subprocess.run([*cmd, "sniper::tests::"], cwd=ROOT, capture_output=True, text=True, errors="replace")
if result.returncode or "test result: ok." not in result.stdout or "0 passed;" in result.stdout:
    raise SystemExit(result.stdout + result.stderr)
print(result.stdout.strip())
