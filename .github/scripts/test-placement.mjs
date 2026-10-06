import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, basename, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

export function placementErrors(path, source, implementationExists = existsSync) {
  if (/^(?:\.github|\.ast-grep)\//.test(path)) return [];
  const testFile = /(?:_test\.rs|\.(?:test|spec)\.[cm]?[jt]sx?)$/.test(path);
  const testAttribute = /#\s*\[\s*(?:(?:tokio|actix_web)::)?test(?:\s*\([^\]]*\))?\s*\]/.test(source);
  const auxiliary = /(?:^|\/)test_support\//.test(path) || /(?:^|\/)test_helpers[^/]*\.rs$/.test(path) || /^(?:tests\/(?:helpers|fixtures)\/|src-tauri\/(?:releash-desktop\/)?tests\/support\/|src\/test\/)/.test(path);
  if (auxiliary) return testFile || testAttribute ? [`${path}: 補助ファイルの置き場所にテストを置かないでください`] : [];
  const rustSource = /^(?:src-tauri\/src\/|src-tauri\/releash-desktop\/src\/)/.test(path);
  const rustIntegration = /^(?:src-tauri\/tests\/|src-tauri\/releash-desktop\/tests\/)/.test(path);
  const errors = [];
  if (/\.(?:test|spec)\.[cm]?[jt]sx?$/.test(path) && !(path.startsWith("src/") && /\.test\.tsx?$/.test(path)) && !/^tests\/(?:integration|behavior)\//.test(path)) errors.push("フロントのテストの置き場所が規約と一致しません");
  if (path.endsWith("_test.rs") && !rustSource && !rustIntegration) errors.push("Rust テストの置き場所が規約と一致しません");
  if (path.endsWith(".rs") && testAttribute && !rustIntegration && !(rustSource && path.endsWith("_test.rs"))) errors.push("#[test] は *_test.rs または Rust の tests/ に置いてください");
  if (rustSource && path.endsWith("_test.rs") && !implementationExists(path.replace(/_test\.rs$/, ".rs"))) errors.push("同じディレクトリに対応する実装がありません");
  if (rustSource && path.endsWith(".rs") && !path.endsWith("_test.rs")) {
    const imports = [...source.matchAll(/#\[path\s*=\s*"([^"/]+_test\.rs)"\]\s*(?:#\[(?:[^\[\]"]|"(?:\\.|[^"\\])*"|\[[^\]]*\])*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;/g)];
    if (imports.length > 1) errors.push("一つの実装に取り込む単体テストファイルは一つだけにしてください");
    for (const [, file, module] of imports) {
      if (file !== basename(path, ".rs") + "_test.rs" || module !== file.replace(/_test\.rs$/, "_tests")) errors.push(`${file} の取り込み先とモジュール名が実装に対応していません`);
      if (!implementationExists(join(dirname(path), file))) errors.push(`${file} が存在しません`);
    }
  }
  return errors.map(message => `${path}: ${message}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const paths = execFileSync("git", ["ls-files", "--cached", "--others", "--exclude-standard", "-z"]).toString().split("\0").filter(path => path && existsSync(path));
  const errors = paths.flatMap(path => placementErrors(path, /\.(?:rs|[cm]?[jt]sx?)$/.test(path) ? readFileSync(path, "utf8") : ""));
  for (const error of errors) console.error(error);
  if (errors.length) process.exitCode = 1;
}
