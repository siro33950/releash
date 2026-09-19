import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const manifest = "src-tauri/Cargo.toml";
const host = execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/^host: (.+)$/m)[1];
const target = process.env.TAURI_ENV_TARGET_TRIPLE ?? host;
const development = process.argv.includes("--dev");
const profile = development || process.env.TAURI_ENV_DEBUG === "true" ? "debug" : "release";
const directory = JSON.parse(execFileSync("cargo", ["metadata", "--manifest-path", manifest, "--no-deps", "--format-version", "1"], { encoding: "utf8" })).target_directory;
const targets = target === "universal-apple-darwin" ? ["aarch64-apple-darwin", "x86_64-apple-darwin"] : [target];
for (const architecture of targets) {
  execFileSync("cargo", ["build", "--manifest-path", manifest, "--locked", "--no-default-features", "--bin", "releash-backend", "--target", architecture, ...(profile === "release" ? ["--release"] : []), ...(process.argv.includes("--performance") ? ["--features", "performance"] : []), ...(architecture.endsWith("apple-darwin") ? ["--features", "vendored-openssl"] : [])], { stdio: "inherit" });
}
const output = join(directory, development ? "" : target, profile, "releash-backend");
mkdirSync(join(directory, development ? "" : target, profile), { recursive: true });
if (targets.length === 2) {
  execFileSync("lipo", ["-create", ...targets.map((architecture) => join(directory, architecture, profile, "releash-backend")), "-output", output], { stdio: "inherit" });
} else if (development) {
  copyFileSync(join(directory, target, profile, "releash-backend"), output);
}

if (!development && target === host) {
  mkdirSync(join(directory, profile), { recursive: true });
  copyFileSync(output, join(directory, profile, "releash-backend"));
}
