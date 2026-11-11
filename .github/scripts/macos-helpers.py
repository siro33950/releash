import argparse
import json
import os
from pathlib import Path
import platform
import re
import subprocess


ROOT = Path(__file__).resolve().parents[2]
TARGETS = {"arm64": "aarch64-apple-darwin", "x86_64": "x86_64-apple-darwin"}
DEBUG_IDENTIFIER = "com.releash.macos.dev"


def development_identity():
    project = (ROOT / "clients/macos/project.yml").read_text()
    team = re.search(r"^\s*DEVELOPMENT_TEAM: (\w+)$", project, re.M).group(1)
    identities = subprocess.check_output(["security", "find-identity", "-v", "-p", "codesigning"], text=True)
    for digest, name in re.findall(r'([0-9A-F]{40}) "(Apple Development: [^"]+)"', identities):
        certificates = subprocess.check_output(
            ["security", "find-certificate", "-a", "-Z", "-p", "-c", name], text=True
        )
        pem = re.search(rf"SHA-1 hash: {digest}\n(-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----)",
                        certificates, re.S)
        subject = subprocess.check_output(["openssl", "x509", "-noout", "-subject"], input=pem.group(1), text=True)
        if re.search(rf"\bOU ?= ?{team}\b", subject):
            return digest
    return "-"


def sign_development(paths):
    identity = development_identity()
    for path in paths:
        subprocess.run([
            "codesign", "--force", "--timestamp=none", "--sign", identity,
            "--identifier", f"{DEBUG_IDENTIFIER}.{Path(path).name}", str(path),
        ], check=True)


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
    if configuration == "Debug":
        sign_development([output / "releashd", output / "releash"])


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("configuration", choices=["Debug", "Release"])
    parser.add_argument("--arch", nargs="+", choices=TARGETS)
    args = parser.parse_args()
    build_helpers(args.configuration, args.arch or (
        [platform.machine()] if args.configuration == "Debug" else list(TARGETS)
    ))
