import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";

const require = createRequire(new URL("../../.nx/installation/package.json", import.meta.url));
const { parse } = require("yaml");

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
  assert.deepEqual(dependencies["server:test-integration"], ["cli:build"]);
  assert.deepEqual(dependencies["cli:test-integration"].sort(), ["cli:build", "server:build"]);
  assert.deepEqual(dependencies["desktop:rust-test-integration"].sort(), ["cli:build", "server:build"]);
  for (const target of ["server:test-unit", "cli:test-unit", "desktop:rust-test-unit",
    "desktop:frontend-lint", "desktop:frontend-test-unit", "desktop:frontend-test-integration"]) {
    assert.deepEqual(dependencies[target], [], `${target} must not wait for a binary build`);
  }
});

test("native selection propagates the Release configuration to helpers", () => {
  const { tasks, dependencies } = graph("clients/macos/project.yml",
    "--exclude=server,cli,desktop,workspace", "--configuration=release");
  assert.deepEqual(Object.keys(tasks).sort(), [
    "macos:build:release", "macos:generate", "macos:helpers:release", "macos:test:release",
  ]);
  for (const target of ["build", "test"]) {
    assert.deepEqual(dependencies[`macos:${target}:release`].sort(), ["macos:generate", "macos:helpers:release"]);
  }
});

test("server changes also select CLI integration tests", () => {
  assert.ok(graph("server/src/lib.rs", "--exclude=macos").tasks["cli:test"]);
});

test("CI lanes cover the aggregate checks without sharing a runner or building macOS twice", () => {
  const { jobs } = parse(readFileSync(".github/workflows/ci.yml", "utf8"));
  const checks = parse(readFileSync(".github/workflows/ci-checks.yml", "utf8"));
  assert.equal(checks.jobs.check.strategy["fail-fast"], false);
  const selected = new Set();
  for (const group of ["frontend", "server", "shell", "cli", "quality"]) {
    const lane = jobs[`${group}-checks`];
    for (const target of JSON.parse(lane.with.targets)) selected.add(`${lane.with.project}:${target}`);
    assert.deepEqual(jobs[group].needs, ["plan", `${group}-checks`]);
  }
  const { tasks } = graph("proto/client.proto", "--exclude=macos");
  for (const [id, task] of Object.entries(tasks)) {
    const project = JSON.parse(readFileSync(`${task.projectRoot}/project.json`, "utf8"));
    if (project.targets[task.target.target].executor === "nx:noop" ||
        ["server:build", "cli:build"].includes(id)) continue;
    assert.ok(selected.has(id), `${id} missing from CI`);
  }
  const native = jobs["macos-native"].steps.find(step => step.name === "Build and test native client");
  assert.match(native.run, /nx run macos:test/);
  assert.doesNotMatch(native.run, /macos:build/);
});

test("required checks reject failures, cancellations, and unexpected skips", () => {
  const { jobs } = parse(readFileSync(".github/workflows/ci.yml", "utf8"));
  for (const group of ["frontend", "server", "shell", "cli", "quality"]) {
    const script = jobs[group].steps[0].run;
    for (const plan of ["success", "failure", "cancelled", "skipped"]) {
      for (const result of ["success", "failure", "cancelled", "skipped"]) {
        for (const expected of ["true", "false"]) {
          const pass = plan === "success" && result === (expected === "true" ? "success" : "skipped");
          let succeeded = true;
          try {
            execFileSync("bash", ["-e", "-c", script], {
              env: { ...process.env, PLAN_RESULT: plan, CHECK_RESULT: result, EXPECTED: expected },
              stdio: "pipe",
            });
          } catch { succeeded = false; }
          assert.equal(succeeded, pass, `${group}: plan=${plan} check=${result} expected=${expected}`);
        }
      }
    }
  }
});
