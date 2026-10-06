import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

export function placementErrors(path, source) {
  if (path.startsWith(".github/") || /(?:^|\/)test_support\//.test(path) || /(?:^|\/)test_helpers[^/]*\.rs$/.test(path) || /^(?:tests\/(?:helpers|fixtures)\/|src-tauri\/tests\/support\/|src\/test\/)/.test(path)) return [];
  const rustSource = /^(?:src-tauri\/src\/|src-tauri\/releash-desktop\/src\/)/.test(path);
  const rustIntegration = /^(?:src-tauri\/tests\/|src-tauri\/releash-desktop\/tests\/)/.test(path);
  const errors = [];
  if (/\.(?:test|spec)\.[cm]?[jt]sx?$/.test(path) && !(path.startsWith("src/") && /\.test\.tsx?$/.test(path)) && !/^tests\/(?:integration|behavior)\//.test(path)) errors.push("フロントのテストの置き場所が規約と一致しません");
  if (path.endsWith("_test.rs") && !rustSource && !rustIntegration) errors.push("Rust テストの置き場所が規約と一致しません");
  if (path.endsWith(".rs") && /#\s*\[\s*(?:(?:tokio|actix_web)::)?test(?:\s*\([^\]]*\))?\s*\]/.test(source) && !rustIntegration && !(rustSource && path.endsWith("_test.rs"))) errors.push("#[test] は *_test.rs または Rust の tests/ に置いてください");
  return errors.map(message => `${path}: ${message}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const paths = execFileSync("git", ["ls-files", "--cached", "--others", "--exclude-standard", "-z"]).toString().split("\0").filter(path => path && existsSync(path));
  const errors = paths.flatMap(path => placementErrors(path, /\.(?:rs|[cm]?[jt]sx?)$/.test(path) ? readFileSync(path, "utf8") : ""));
  for (const error of errors) console.error(error);
  if (errors.length) process.exitCode = 1;
}
