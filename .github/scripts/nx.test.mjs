import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
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
  assert.deepEqual(dependencies["server:test-integration"], ["cli:build"]);
  assert.deepEqual(dependencies["cli:test-integration"].sort(), ["cli:build", "server:build"]);
  assert.deepEqual(dependencies["desktop:rust-test-integration"].sort(), ["cli:build", "server:build"]);
  for (const target of ["server:test-unit", "cli:test-unit", "desktop:rust-test-unit",
    "desktop:frontend-lint", "desktop:frontend-test-unit", "desktop:frontend-test-integration"]) {
    assert.deepEqual(dependencies[target], [], `${target} must not wait for a binary build`);
  }
});

test("native selection propagates Release through preparation and separates test outputs", () => {
  const { tasks, dependencies } = graph("clients/macos/project.yml",
    "--exclude=server,cli,desktop,workspace", "--configuration=release");
  for (const target of ["build", "test-unit", "test-integration"]) {
    assert.deepEqual(dependencies[`macos:${target}:release`], ["macos:prepare:release"]);
  }
  assert.deepEqual(dependencies["macos:prepare:release"].sort(),
    ["macos:file-viewer", "macos:generate", "macos:helpers:release"]);
  assert.deepEqual(dependencies["macos:test:release"].sort(),
    ["macos:test-integration:release", "macos:test-unit:release"]);
  const native = JSON.parse(readFileSync("clients/macos/project.json", "utf8"));
  assert.notDeepEqual(native.targets["test-unit"].outputs, native.targets["test-integration"].outputs);
  const lint = Object.values(tasks).find(task => task.target.target === "lint");
  assert.deepEqual(dependencies[lint.id], []);
});

test("server changes also select CLI integration tests", () => {
  assert.ok(graph("server/src/lib.rs", "--exclude=macos").tasks["cli:test"]);
});

test("CI lanes cover the aggregate checks without a redundant native build job", () => {
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
        ["server:build", "cli:build"].includes(id)) continue;
    assert.ok(selected.has(id), `${id} missing from CI`);
  }
  const native = jobs["macos-checks"];
  assert.deepEqual(native.strategy.matrix.target, ["lint", "test-unit", "test-integration"]);
  assert.equal(native.strategy["fail-fast"], false);
  assert.equal(native.if, undefined);
  const run = native.steps.find(step => step.name === "Run check");
  assert.match(run.run, /macos:\$NX_TARGET/);

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
  for (const step of jobs["macos-checks"].steps) {
    assert.match(step.if, /^contains\(fromJSON\(needs.plan.outputs.projects\), 'macos'\)/);
  }
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

test("each required group rejects failed, cancelled or skipped planning and checks", () => {
  const { jobs } = parse(readFileSync(".github/workflows/ci.yml", "utf8"));
  for (const group of ["frontend", "server", "shell", "cli", "quality", "macos"]) {
    const gate = jobs[`${group}-required`];
    assert.deepEqual(gate.needs, ["plan", `${group}-checks`]);
    assert.equal(gate.if, "${{ always() }}");
    const step = gate.steps[0];
    assert.equal(step.env.PLAN_RESULT, "${{ needs.plan.result }}");
    assert.equal(step.env.CHECK_RESULT, "${{ needs." + group + "-checks.result }}");
    for (const plan of ["success", "failure", "cancelled", "skipped"]) {
      for (const check of ["success", "failure", "cancelled", "skipped"]) {
        const result = spawnSync("bash", ["-e", "-c", step.run], {
          env: { ...process.env, PLAN_RESULT: plan, CHECK_RESULT: check },
        });
        assert.equal(result.status === 0, plan === "success" && check === "success", `${group}: ${plan}/${check}`);
      }
    }
  }
});
