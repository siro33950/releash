import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

test("loads references with the installed CLI and masks multiline secrets", () => {
  const directory = mkdtempSync(join(tmpdir(), "releash-secrets-"));
  try {
    writeFileSync(join(directory, "op"), '#!/bin/sh\n[ "$1" = read ] && [ "$2" = --no-newline ] && [ "$3" = op://vault/item/field ] || exit 1\nprintf "first%%line\\nsecond"\n', { mode: 0o700 });
    const output = join(directory, "env");
    const result = spawnSync(process.execPath, [new URL("./load-secrets.mjs", import.meta.url).pathname], {
      env: { PATH: directory, GITHUB_ENV: output, SIGNING_KEY: "op://vault/item/field", PLAIN: "not a reference" },
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, "::add-mask::first%25line%0Asecond\n");
    const lines = readFileSync(output, "utf8").split("\n");
    const delimiter = lines[0].slice("SIGNING_KEY<<".length);
    assert.deepEqual(lines, [`SIGNING_KEY<<${delimiter}`, "first%line", "second", delimiter, ""]);
    writeFileSync(join(directory, "op"), "#!/bin/sh\nexit 1\n", { mode: 0o700 });
    const failed = spawnSync(process.execPath, [new URL("./load-secrets.mjs", import.meta.url).pathname], {
      env: { PATH: directory, GITHUB_ENV: output, SIGNING_KEY: "op://vault/item/field" },
      encoding: "utf8",
    });
    assert.notEqual(failed.status, 0);
    assert.equal(readFileSync(output, "utf8"), lines.join("\n"));
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
