#!/usr/bin/env python3
"""Build a Horizon dedicated server with Grim-Horizon, reproducibly.

Steps (each logged, each re-runnable):
  1. export Horizon at the commit the bundle pins (git archive; the checkout
     you point at is never modified),
  2. assemble the horizon-mixins bundle (integration/horizon/grim-horizon plus
     the engine crate as its `engine/` bundle crate),
  3. horizon-mixins apply: stage the modded source,
  4. cargo build --locked -p horizon-client --no-default-features (the server
     build Horizon ships as `horizon-engine`),
  5. copy the binary to dist/.

Usage:
  python3 tools/build_server.py --horizon ../upstream/horizon \
      --mixins ../upstream/horizon-mixins [--profile fast|release] [--check-only]
"""
import argparse
import os
import shutil
import subprocess
import sys
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BUNDLE = ROOT / "integration" / "horizon" / "grim-horizon"
ENGINE = ROOT / "crates" / "grim-engine"
WORK = ROOT / "work"


def run(cmd, **kw):
    print("+", " ".join(str(c) for c in cmd), flush=True)
    subprocess.run([str(c) for c in cmd], check=True, **kw)


def pinned_revision():
    text = (BUNDLE / "horizon-mixin.toml").read_text()
    m = re.search(r'^horizon_revision\s*=\s*"([0-9a-f]{40})"', text, re.M)
    if not m:
        sys.exit("horizon-mixin.toml has no [compatibility] horizon_revision")
    return m.group(1)


def export_horizon(horizon: Path, rev: str) -> Path:
    out = WORK / "horizon-src"
    stamp = out / ".horizon-revision"
    if stamp.exists() and stamp.read_text().strip() == rev:
        print(f"Horizon {rev[:10]} already exported")
        return out
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    archive = subprocess.run(["git", "-C", str(horizon), "archive", "--format=tar", rev], check=True, capture_output=True).stdout
    subprocess.run(["tar", "-x", "-C", str(out)], input=archive, check=True)
    # Links are left out, as Horizon's launcher does: horizon-mixins refuses
    # them and nothing needs them to build (CLAUDE.md -> AGENTS.md).
    for path in sorted(out.rglob("*")):
        if path.is_symlink():
            path.unlink()
    stamp.write_text(rev + "\n")
    return out


def assemble_bundle() -> Path:
    mods = WORK / "mods"
    dest = mods / "grim-horizon"
    if mods.exists():
        shutil.rmtree(mods)
    shutil.copytree(BUNDLE, dest)
    ignore = shutil.ignore_patterns("target", "Cargo.lock", "*.log")
    shutil.copytree(ENGINE, dest / "engine", ignore=ignore)
    # The bundle crate carries the license of the code it contains.
    shutil.copy(ROOT / "LICENSE", dest / "engine" / "LICENSE")
    return mods


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--horizon", required=True, type=Path, help="a Horizon git checkout containing the pinned commit")
    ap.add_argument("--mixins", required=True, type=Path, help="a horizon-mixins checkout (built with cargo build --release)")
    ap.add_argument("--profile", default="fast", choices=["fast", "release", "dev"])
    ap.add_argument("--check-only", action="store_true", help="cargo check instead of build")
    ap.add_argument("--target-dir", type=Path, default=WORK / "target")
    args = ap.parse_args()

    rev = pinned_revision()
    tool = args.mixins / "target" / "release" / "horizon-mixins"
    if not tool.exists():
        run(["cargo", "build", "--locked", "--release", "-p", "horizon-mixins-cli"], cwd=args.mixins)

    src = export_horizon(args.horizon.resolve(), rev)
    mods = assemble_bundle()
    staged = WORK / "staged"
    # horizon-mixins replaces an existing stage atomically only on Linux:
    # stage into a fresh folder, then sync by content so unchanged files keep
    # their mtimes and Cargo rebuilds only what the mods touched.
    fresh = WORK / "staged-fresh"
    if fresh.exists():
        shutil.rmtree(fresh)
    run([tool, "apply", "--workspace", src, "--mods", mods, "--out", fresh])
    staged.mkdir(exist_ok=True)
    run(["rsync", "-rlc", "--delete", f"{fresh}/", f"{staged}/"])
    shutil.rmtree(fresh)

    env = dict(os.environ, CARGO_TARGET_DIR=str(args.target_dir))
    verb = "check" if args.check_only else "build"
    run(["cargo", verb, "--locked", "--profile", args.profile, "-p", "horizon-client", "--no-default-features", "--bins"], cwd=staged, env=env)
    if args.check_only:
        return
    profile_dir = "debug" if args.profile == "dev" else args.profile
    exe = "horizon-client.exe" if sys.platform == "win32" else "horizon-client"
    built = args.target_dir / profile_dir / exe
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    out = dist / ("horizon-engine-grim.exe" if sys.platform == "win32" else "horizon-engine-grim")
    shutil.copy2(built, out)
    print(f"Built {out} (Horizon {rev[:10]} + Grim-Horizon)")


if __name__ == "__main__":
    main()
