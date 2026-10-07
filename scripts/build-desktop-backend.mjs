import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const manifest = "src-tauri/Cargo.toml";
if (process.argv.includes("--dev")) {
  execFileSync("cargo", ["build", "--manifest-path", manifest, "--locked", "-p", "releash-backend", "--bin", "releash-backend", "-p", "releash", "--bin", "releash"], { stdio: "inherit" });
} else {
  const host = execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/^host: (.+)$/m)[1];
  const target = process.env.TAURI_ENV_TARGET_TRIPLE ?? host;
  const profile = process.env.TAURI_ENV_DEBUG === "true" ? "debug" : "release";
  const directory = JSON.parse(execFileSync("cargo", ["metadata", "--manifest-path", manifest, "--no-deps", "--format-version", "1"], { encoding: "utf8" })).target_directory;
  const targets = target === "universal-apple-darwin" ? ["aarch64-apple-darwin", "x86_64-apple-darwin"] : [target];
  for (const architecture of targets) {
    execFileSync("cargo", ["build", "--manifest-path", manifest, "--locked", "-p", "releash-backend", "--bin", "releash-backend", "-p", "releash", "--bin", "releash", "--target", architecture, ...(profile === "release" ? ["--release"] : []), ...(architecture.endsWith("apple-darwin") ? ["--features", "vendored-openssl"] : [])], { stdio: "inherit" });
  }
  const sidecars = "src-tauri/releash-desktop/binaries";
  mkdirSync(sidecars, { recursive: true });
  for (const binary of ["releash-backend", "releash"]) {
    for (const architecture of targets) copyFileSync(join(directory, architecture, profile, binary), join(sidecars, `${binary}-${architecture}`));
    if (targets.length === 2) {
      execFileSync("lipo", ["-create", ...targets.map((architecture) => join(directory, architecture, profile, binary)), "-output", join(sidecars, `${binary}-${target}`)], { stdio: "inherit" });
    }
  }
}
