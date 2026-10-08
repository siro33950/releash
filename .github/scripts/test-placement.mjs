import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, basename, join, normalize } from "node:path";

const TEST_ATTRIBUTE = /#\s*\[\s*(?:(?:tokio|actix_web)::)?test(?:\s*\([^\]]*\))?\s*\]/;

function rustCode(source) {
  let result = "";
  const blank = text => text.replace(/[^\n]/g, " ");
  for (let index = 0; index < source.length;) {
    if (source.startsWith("//", index)) {
      const end = source.indexOf("\n", index);
      const next = end < 0 ? source.length : end;
      result += blank(source.slice(index, next));
      index = next;
    } else if (source.startsWith("/*", index)) {
      const start = index;
      index += 2;
      let depth = 1;
      while (index < source.length && depth) {
        if (source.startsWith("/*", index)) { depth++; index += 2; }
        else if (source.startsWith("*/", index)) { depth--; index += 2; }
        else index++;
      }
      result += blank(source.slice(start, index));
    } else {
      const raw = (source[index] === "r" || source[index] === "b") && source.slice(index).match(/^(?:b)?r(#{0,255})"/);
      const character = source[index] === "'" && source.slice(index).match(/^'(?:\\(?:u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|[^\n])|[^'\\\n])'/);
      if (raw) {
        const end = source.indexOf('"' + raw[1], index + raw[0].length);
        const next = end < 0 ? source.length : end + 1 + raw[1].length;
        result += blank(source.slice(index, next));
        index = next;
      } else if (character) {
        result += blank(character[0]);
        index += character[0].length;
      } else if (source[index] === '"') {
        const start = index++;
        while (index < source.length) {
          if (source[index] === "\\") index += 2;
          else if (source[index++] === '"') break;
        }
        const literal = source.slice(start, index);
        result += /#\[path\s*=\s*$/.test(result) ? literal : '"' + blank(literal.slice(1, -1)) + '"';
      } else {
        result += source[index++];
      }
    }
  }
  return result;
}

function unitImports(source) {
  return [...rustCode(source).matchAll(/#\[path\s*=\s*"([^"/]+_test\.rs)"\]\s*(?:#\[(?:[^\[\]"]|"(?:\\.|[^"\\])*"|\[[^\]]*\])*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;/g)];
}

function placementErrors(path, source) {
  if (/^(?:\.github|\.ast-grep)\//.test(path)) return [];
  if (path.endsWith(".rs")) source = rustCode(source);
  const testFile = /(?:_test\.rs|\.(?:test|spec)\.[cm]?[jt]sx?)$/.test(path);
  const testAttribute = TEST_ATTRIBUTE.test(source);
  const rustSource = /^(?:server\/src\/|clients\/cli\/src\/|clients\/desktop\/src-native\/)/.test(path);
  if (rustSource && /(?:^|\/)(?:test_helpers[^/]+|[^/]+_test_helpers)\.rs$/.test(path)) return [`${path}: テスト補助は test_helpers.rs にまとめてください`];
  const auxiliary = /(?:^|\/)test_support\//.test(path) || /(?:^|\/)test_helpers\.rs$/.test(path) || /^(?:(?:server|clients\/(?:cli|desktop))\/tests\/(?:support|helpers|fixtures)\/|clients\/desktop\/src\/test\/)/.test(path);
  if (auxiliary) return testFile || testAttribute ? [`${path}: 補助ファイルの置き場所にテストを置かないでください`] : [];
  const rustIntegration = /^(?:server|clients\/(?:desktop|cli))\/tests\//.test(path);
  const errors = [];
  if (/\.(?:test|spec)\.[cm]?[jt]sx?$/.test(path) && !(path.startsWith("clients/desktop/src/") && /\.test\.tsx?$/.test(path)) && !/^clients\/desktop\/tests\/(?:integration|behavior)\//.test(path)) errors.push("フロントのテストの置き場所が規約と一致しません");
  if (path.endsWith("_test.rs") && !rustSource && !rustIntegration) errors.push("Rust テストの置き場所が規約と一致しません");
  if (path.endsWith(".rs") && testAttribute && !rustIntegration && !(rustSource && path.endsWith("_test.rs"))) errors.push("#[test] は *_test.rs または Rust の tests/ に置いてください");
  if (rustSource && path.endsWith("_test.rs")) {
    const implementation = path.replace(/_test\.rs$/, ".rs");
    if (!existsSync(implementation)) errors.push("同じディレクトリに対応する実装がありません");
    else if (!unitImports(readFileSync(implementation, "utf8")).some(([, file]) => file === basename(path))) errors.push("対応する実装から #[path] で取り込まれていません");
  }
  if (rustSource && path.endsWith(".rs") && !path.endsWith("_test.rs")) {
    const imports = unitImports(source);
    if (imports.length > 1) errors.push("一つの実装に取り込む単体テストファイルは一つだけにしてください");
    for (const [, file, module] of imports) {
      if (file !== basename(path, ".rs") + "_test.rs" || module !== file.replace(/_test\.rs$/, "_tests")) errors.push(`${file} の取り込み先とモジュール名が実装に対応していません`);
      if (!existsSync(join(dirname(path), file))) errors.push(`${file} が存在しません`);
    }
  }
  return errors.map(message => `${path}: ${message}`);
}

function integrationErrors(sources) {
  const errors = [];
  for (const root of ["server", "clients/desktop", "clients/cli"]) {
    const directory = `${root}/tests/`;
    const manifest = sources.get(`${root}/Cargo.toml`) ?? "";
    const targets = [...manifest.matchAll(/^\[\[test\]\]\s*\n([\s\S]*?)(?=^\[|$(?![\s\S]))/gm)].map(match => match[1]);
    const entries = [...sources.keys()].filter(path => path.startsWith(directory) && !path.slice(directory.length).includes("/") && path.endsWith(".rs"));
    for (const target of targets) {
      const path = target.match(/^path\s*=\s*"([^"]+)"/m)?.[1];
      if (/^test\s*=\s*false\s*$/m.test(target)) {
        errors.push(`${root}/Cargo.toml: [[test]] の test = false はテストを実行しません`);
        if (path) entries.splice(entries.indexOf(`${root}/${path}`), entries.includes(`${root}/${path}`) ? 1 : 0);
      } else if (path) entries.push(`${root}/${path}`);
    }
    const reachable = new Set();
    function visit(path) {
      if (reachable.has(path) || !sources.has(path)) return;
      reachable.add(path);
      const source = rustCode(sources.get(path));
      const moduleDirectory = path.endsWith("/mod.rs") || entries.includes(path) ? dirname(path) : join(dirname(path), basename(path, ".rs"));
      for (const match of source.matchAll(/(?:#\[[^\]\n]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;/g)) {
        const explicit = match[0].match(/#\[path\s*=\s*"([^"]+)"\]/)?.[1];
        if (explicit) visit(normalize(join(dirname(path), explicit)));
        else {
          visit(join(moduleDirectory, `${match[1]}.rs`));
          visit(join(moduleDirectory, match[1], "mod.rs"));
        }
      }
    }
    entries.forEach(visit);
    for (const [path, source] of sources) {
      if (path.startsWith(directory) && TEST_ATTRIBUTE.test(rustCode(source)) && !reachable.has(path)) errors.push(`${path}: 統合テストの入口から取り込まれていません`);
    }
  }
  return errors;
}

const paths = execFileSync("git", ["ls-files", "--cached", "--others", "--exclude-standard", "-z"]).toString().split("\0").filter(path => path && existsSync(path));
const sources = new Map(paths.map(path => [path, /\.(?:rs|toml|[cm]?[jt]sx?)$/.test(path) ? readFileSync(path, "utf8") : ""]));
const errors = [...paths.flatMap(path => placementErrors(path, sources.get(path))), ...integrationErrors(sources)];
for (const error of errors) console.error(error);
if (errors.length) process.exitCode = 1;
