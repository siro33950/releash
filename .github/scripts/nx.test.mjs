import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

const require = createRequire(new URL("../../.nx/installation/package.json", import.meta.url));
const { parse } = require("yaml");

test("protocol baseline preparation works with a detached CI checkout and no local main", () => {
  const directory = mkdtempSync(join(tmpdir(), "releash-ci-baseline-"));
  const git = (...args) => execFileSync("git", args, { cwd: directory, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  try {
    git("init", "--initial-branch=feature");
    git("config", "user.name", "CI test");
    git("config", "user.email", "ci@example.invalid");
    writeFileSync(join(directory, "schema"), "baseline");
    git("add", ".");
    git("-c", "commit.gpgsign=false", "commit", "-m", "baseline");
    const baseline = git("rev-parse", "HEAD");
    git("update-ref", "refs/remotes/origin/main", baseline);
    writeFileSync(join(directory, "schema"), "changed");
    git("-c", "commit.gpgsign=false", "commit", "-am", "change");
    const head = git("rev-parse", "HEAD");
    git("checkout", "--detach");
    const workflow = parse(readFileSync(".github/workflows/ci-checks.yml", "utf8"));
    const prepare = workflow.jobs.check.steps.find(step => step.name === "Prepare protocol comparison baseline");
    execFileSync("bash", ["-e", "-c", prepare.run], { cwd: directory, stdio: "pipe" });
    assert.equal(git("rev-parse", "main"), baseline);
    assert.equal(git("rev-parse", "HEAD"), head);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

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
  assert.deepEqual(dependencies["server:test-integration"].sort(), ["cli:build", "server:test-fixture-build"]);
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
    assert.equal(lane.needs, "plan");
  }
  const { tasks } = graph("proto/client.proto", "--exclude=macos");
  for (const [id, task] of Object.entries(tasks)) {
    const project = JSON.parse(readFileSync(`${task.projectRoot}/project.json`, "utf8"));
    if (project.targets[task.target.target].executor === "nx:noop" ||
        ["server:build", "server:test-fixture-build", "cli:build"].includes(id)) continue;
    assert.ok(selected.has(id), `${id} missing from CI`);
  }
  const native = jobs["macos-checks"].steps.find(step => step.name === "Build and test native client");
  assert.match(native.run, /nx run macos:test/);
  assert.doesNotMatch(native.run, /macos:build/);
});

test("unaffected projects retain their check names without executing validation", () => {
  const { jobs } = parse(readFileSync(".github/workflows/ci.yml", "utf8"));
  const checks = parse(readFileSync(".github/workflows/ci-checks.yml", "utf8"));
  assert.equal(checks.on.workflow_call.inputs.affected.type, "boolean");
  assert.equal(checks.on.workflow_call.inputs.affected.required, true);
  assert.equal(checks.jobs.check.if, undefined);
  assert.equal(checks.jobs.check.name, "${{ matrix.target }}");
  for (const group of ["frontend", "server", "shell", "cli", "quality"]) {
    const lane = jobs[`${group}-checks`];
    assert.equal(lane.if, undefined, "expand matrix checks even when unaffected");
    assert.equal(lane.needs, "plan", "a failed plan must prevent validation");
    assert.equal(lane.with.affected,
      `\${{ contains(fromJSON(needs.plan.outputs.projects), '${lane.with.project}') }}`);
  }
  for (const step of checks.jobs.check.steps) {
    assert.match(step.if, /^(?:\$\{\{ )?inputs\.affected(?:$| &&)/,
      `${step.name || step.uses} must not execute for unaffected projects`);
  }
  assert.equal(jobs["macos-checks"].needs, "plan");
});

test("changes inside every project select the workspace quality checks", () => {
  for (const file of ["server/src/lib.rs", "server/Cargo.toml", "clients/cli/src/lib.rs",
    "clients/desktop/src/App.tsx", "clients/macos/project.yml"]) {
    const projects = JSON.parse(execFileSync(process.execPath, [
      ".nx/nxw.js", "show", "projects", "--affected", `--files=${file}`, "--json",
    ], {
      encoding: "utf8",
      env: { ...process.env, NX_DAEMON: "false" },
    }));
    assert.ok(projects.includes("workspace"), `${file} must select the quality job`);
    assert.ok(graph(file).tasks["workspace:lint"], `${file} must select the quality target`);
  }
});
