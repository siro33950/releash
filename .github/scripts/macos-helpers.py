import argparse
import json
import os
from pathlib import Path
import platform
import subprocess


ROOT = Path(__file__).resolve().parents[2]
TARGETS = {"arm64": "aarch64-apple-darwin", "x86_64": "x86_64-apple-darwin"}


def build_helpers(configuration, architectures):
    output = ROOT / "clients/macos/.build/Helpers" / configuration
    if os.environ.get("RELEASH_HELPERS_RESTORED") == "true":
        for helper in ["releashd", "releash"]:
            if not (output / helper).is_file():
                raise SystemExit(f"RELEASH_HELPERS_RESTORED is set but {output / helper} is missing")
        return
    profile = "debug" if configuration == "Debug" else "release"
    targets = [TARGETS[arch] for arch in architectures]
    directory = Path(json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=ROOT, text=True
    ))["target_directory"])
    for target in targets:
        subprocess.run([
            "cargo", "build", "--locked", "--profile", "dev" if profile == "debug" else "release",
            "--target", target, "-p", "releashd", "--bin", "releashd", "-p", "releash", "--bin", "releash",
            *(["--features", "releashd/vendored-openssl"] if configuration == "Release" else []),
        ], cwd=ROOT, check=True)
    output.mkdir(parents=True, exist_ok=True)
    for helper in ["releashd", "releash"]:
        subprocess.run([
            "lipo", "-create", *[str(directory / target / profile / helper) for target in targets],
            "-output", str(output / helper),
        ], check=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("configuration", choices=["Debug", "Release"])
    parser.add_argument("--arch", nargs="+", choices=TARGETS)
    args = parser.parse_args()
    build_helpers(args.configuration, args.arch or (
        [platform.machine()] if args.configuration == "Debug" else list(TARGETS)
    ))
