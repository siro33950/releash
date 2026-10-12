import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import test from "node:test";

function graph(file, ...args) {
  return JSON.parse(execFileSync(process.execPath, [
    ".nx/nxw.js", "affected", "-t", "lint", "test", "build",
    `--files=${file}`, "--graph=stdout", ...args,
  ], {
    encoding: "utf8",
    env: { ...process.env, NX_DAEMON: "false" },
    maxBuffer: 4 * 1024 * 1024,
  })).tasks;
}

test("protocol changes select all clients and build integration prerequisites", () => {
  const { tasks, dependencies } = graph("proto/client.proto", "--exclude=macos");
  for (const project of ["server", "cli", "desktop"]) {
    for (const target of ["lint", "test", "build"]) assert.ok(tasks[`${project}:${target}`]);
  }
  assert.ok(tasks["workspace:lint"]);
  assert.ok(Object.values(tasks).every(task => task.target.project !== "macos"));
  assert.deepEqual(dependencies["server:test"], ["cli:build"]);
  assert.deepEqual(dependencies["cli:test"].sort(), ["cli:build", "server:build"]);
  assert.deepEqual(dependencies["desktop:test"].sort(), ["cli:build", "server:build"]);
});

test("native selection propagates the Release configuration to helpers", () => {
  const { tasks, dependencies } = graph("clients/macos/project.yml",
    "--exclude=server,cli,desktop,workspace", "--configuration=release");
  assert.deepEqual(Object.keys(tasks).sort(), [
    "cli:build", "macos:build:release", "macos:generate", "macos:helpers:release",
    "macos:test:release", "server:build",
  ]);
  for (const target of ["build", "test"]) {
    assert.deepEqual(dependencies[`macos:${target}:release`].sort(), ["macos:generate", "macos:helpers:release"]);
  }
});

test("server changes also select CLI integration tests", () => {
  assert.ok(graph("server/src/lib.rs", "--exclude=macos").tasks["cli:test"]);
});
