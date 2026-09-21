import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { test } from "node:test";

function block(file, marker, key) {
  const text = readFileSync(new URL(`../workflows/${file}`, import.meta.url), "utf8").split(marker)[1];
  const lines = text.split("\n");
  const start = lines.findIndex(line => line.trim() === `${key}: |`) + 1;
  assert.ok(start > 0, `${marker}: ${key} block exists`);
  const indent = lines[start].length - lines[start].trimStart().length;
  const end = lines.findIndex((line, index) => index >= start && line.trim() && !line.startsWith(" ".repeat(indent)));
  return lines.slice(start, end < 0 ? undefined : end).map(line => line.slice(indent)).join("\n");
}

const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const changes = new AsyncFunction("context", "core", "require", block("ci.yml", "  rust-lint:", "script"));
const nightly = new AsyncFunction("context", "core", "github", block("nightly.yml", "  check:", "script"));

const ciConfig = readFileSync(new URL("../workflows/ci.yml", import.meta.url), "utf8");
const nightlyConfig = readFileSync(new URL("../workflows/nightly.yml", import.meta.url), "utf8");

function section(text, heading) {
  const lines = text.split("\n");
  const start = lines.indexOf(heading);
  assert.ok(start >= 0, `${heading} exists`);
  const indent = heading.length - heading.trimStart().length;
  const end = lines.findIndex((line, index) => index > start && line.trim() && !line.trimStart().startsWith("#") && line.length - line.trimStart().length <= indent);
  return lines.slice(start + 1, end < 0 ? undefined : end).join("\n");
}

function value(text, key) {
  return text.match(new RegExp(`^ *(?:- )?${key}: (.+)$`, "m"))?.[1];
}

function jobs(config) {
  return Object.fromEntries([...section(config, "jobs:").matchAll(/^  ([\w-]+):$/gm)].map(([, name]) => [name, section(config, `  ${name}:`)]));
}

function steps(job) {
  return job.split(/^      - /m).slice(1);
}

function commands(job) {
  return steps(job).map(step => value(step, "run")).filter(command => /^(cargo |pnpm (lint|test|build|exec vitest)\b|node --test |python3 |qlty check )/.test(command));
}

const rustCommands = {
  "rust-lint": [
    "cargo fmt --check",
    "cargo clippy --locked -- -D warnings",
    "cargo deny --locked check",
    "cargo clippy --locked --no-default-features --bin releash-backend -- -D warnings",
  ],
  "rust-test-desktop": ["cargo test --locked"],
  "rust-test-headless": [
    "cargo test --locked --no-default-features --lib",
    "cargo test --locked --no-default-features --test daemon_smoke",
  ],
};

function assertPrChecks(config) {
  for (const step of Object.values(jobs(config)).flatMap(steps)) {
    let executable = false;
    for (const line of step.split("\n")) {
      if (line.trimStart().startsWith("#")) continue;
      if (/^\S|^ {8}\S/.test(line)) executable = /^(?: {8})?(run|uses|with|env):/.test(line);
      if (executable) assert.doesNotMatch(line, /performance|llvm-cov|coverage|cargo build|agent_tui_harness/);
    }
  }
}

test("PR and main push share the required parallel jobs and command allocation", () => {
  assert.equal(section(ciConfig, "on:").trim(), "push:\n    branches: [main]\n  pull_request:\n    branches: [main]");
  const ciJobs = jobs(ciConfig);
  assert.deepEqual(Object.keys(ciJobs).sort(), ["frontend", "integration", "quality", "workflow-tests", "rust", ...Object.keys(rustCommands)].sort());
  for (const [name, job] of Object.entries(ciJobs)) {
    if (name !== "rust") assert.doesNotMatch(job, /^    (if|needs|strategy):/m, `${name} starts for both events independently`);
  }
  assert.deepEqual(commands(ciJobs.frontend), ["pnpm lint", "pnpm test", "pnpm build"]);
  assert.deepEqual(commands(ciJobs.integration), ["pnpm test:integration"]);
  assert.deepEqual(commands(ciJobs.quality), ["qlty check --no-progress --all"]);
  for (const [name, expected] of Object.entries(rustCommands)) {
    assert.deepEqual(commands(ciJobs[name]), expected);
    assert.equal(value(ciJobs[name], "working-directory"), "src-tauri");
  }
  assertPrChecks(ciConfig);
  assert.equal(value(ciJobs.rust, "needs"), "[rust-lint, rust-test-desktop, rust-test-headless]");
  assert.equal(value(ciJobs.rust, "if"), "always()");
});

test("PR check restrictions ignore explanatory comments and step names", () => {
  const note = "performance llvm-cov coverage cargo build agent_tui_harness";
  const config = `# ${note}\n${ciConfig}`.replace("      - run: pnpm lint", `      # ${note}\n      - name: ${note}\n        # ${note}\n        run: pnpm lint`);
  assert.doesNotThrow(() => assertPrChecks(config));
});

test("PR check restrictions still reject prohibited commands, actions, and action inputs", () => {
  for (const step of [
    "run: cargo test --features performance",
    "run: cargo llvm-cov",
    "run: pnpm exec vitest run --coverage",
    "run: cargo build --locked",
    "run: cargo test --test agent_tui_harness",
    "run: |\n          cargo build --locked",
    "uses: example/coverage-action@v1",
    "uses: actions/github-script@v9\n        with:\n          script: |\n            await exec.exec('cargo build --locked');",
  ]) {
    const config = ciConfig.replace("      - run: pnpm lint", `      - ${step}`);
    assert.throws(() => assertPrChecks(config), assert.AssertionError, step);
  }
});

test("workflow decision tests run independently with only Node, including docs-only changes", () => {
  const job = jobs(ciConfig)["workflow-tests"];
  assert.ok(job);
  assert.doesNotMatch(job, /\b(if|needs|continue-on-error):|pnpm|cargo/);
  assert.deepEqual(commands(job), ["node --test .github/scripts/workflows-test.mjs"]);
  assert.deepEqual(steps(job).map(step => value(step, "uses")).filter(Boolean), ["actions/checkout@v7", "actions/setup-node@v7"]);
  assert.equal(value(job, "node-version"), "24");
});

test("integration comments skip cancelled runs and report failures on both create and update", async () => {
  const step = steps(jobs(ciConfig).integration).find(step => value(step, "name") === "Add PR comment with test results");
  assert.equal(value(step, "if"), "${{ !cancelled() && github.event_name == 'pull_request' }}");
  const condition = new Function("cancelled", "github", `return ${value(step, "if").slice(3, -2)}`);
  const script = block("ci.yml", "      - name: Add PR comment with test results", "script");
  for (const cancelled of [true, false]) {
    for (const existing of [true, false]) {
      const calls = [];
      const comments = existing ? [{ id: 10, user: { type: "Bot" }, body: "Integration Tests: All tests passed" }] : [];
      if (condition(() => cancelled, { event_name: "pull_request" })) {
        const comment = new AsyncFunction("context", "github", script.replace("${{ steps.test.outcome }}", "failure"));
        await comment({ payload: { pull_request: { number: 1 } }, repo: { owner: "owner", repo: "repo" }, runId: 2 }, {
          rest: { issues: {
            listComments: async () => ({ data: comments }),
            createComment: async args => calls.push(["create", args]),
            updateComment: async args => calls.push(["update", args]),
          } },
        });
      }
      assert.equal(calls.length, cancelled ? 0 : 1);
      if (!cancelled) {
        const [operation, args] = calls[0];
        assert.equal(operation, existing ? "update" : "create");
        assert.equal(existing ? args.comment_id : args.issue_number, existing ? 10 : 1);
        assert.match(args.body, /^## ❌ Integration Tests: Some tests failed\n/);
      }
    }
  }
  assert.equal(condition(() => false, { event_name: "push" }), false);
});

test("headless tests retain Tauri system dependencies without frontend setup", () => {
  const job = jobs(ciConfig)["rust-test-headless"];
  assert.doesNotMatch(job, /pnpm|setup-node|frontend/);
  assert.match(job, /sudo apt-get install -y libwebkit2gtk-4\.1-dev/);
  assert.deepEqual(commands(job), rustCommands["rust-test-headless"]);
});

test("every Rust check requires successful setup, survives check failures, and respects docs-only/cancellation", () => {
  for (const [name, expected] of Object.entries(rustCommands)) {
    const job = jobs(ciConfig)[name];
    assert.doesNotMatch(job, /continue-on-error:/);
    assert.match(job, name === "rust-lint" ? /      - &rust-changes\n/ : /      - \*rust-changes\n/);
    const jobSteps = steps(job);
    const firstCheck = jobSteps.findIndex(step => expected.includes(value(step, "run")));
    const preparation = jobSteps.slice(0, firstCheck);
    for (const step of preparation) {
      assert.ok([undefined, "steps.changes.outputs.rust == 'true'"].includes(value(step, "if")), `${name}: preparation requires prior success`);
    }
    const checks = jobSteps.slice(firstCheck);
    assert.equal(checks.length, expected.length);
    for (const step of checks) {
      const condition = new Function("cancelled", "steps", "success", `return ${value(step, "if").slice(3, -2)}`);
      for (const [outcome, rust, cancelled, earlierChecksSucceeded, shouldRun] of [
        ["success", "true", false, true, true],
        ["success", "true", false, false, true],
        ["failure", "true", false, false, false],
        ["skipped", "true", false, false, false],
        ["cancelled", "true", true, false, false],
        ["success", "true", true, true, false],
        ["skipped", "false", false, true, false],
      ]) {
        assert.equal(condition(() => cancelled, { setup: { outcome }, changes: { outputs: { rust } } }, () => earlierChecksSucceeded), shouldRun, `${name}: ${value(step, "run")}, ${outcome}, rust=${rust}, cancelled=${cancelled}, earlierChecksSucceeded=${earlierChecksSucceeded}`);
      }
      assert.equal(value(step, "if"), "${{ !cancelled() && steps.changes.outputs.rust == 'true' && steps.setup.outcome == 'success' }}", value(step, "run"));
    }
    assert.equal(value(preparation.at(-1), "id"), "setup", `${name}: setup marks the last preparation step`);
  }
});

test("PR Rust disables debuginfo and saves separate job caches only on main push", () => {
  assert.equal(value(section(ciConfig, "env:"), "CARGO_PROFILE_DEV_DEBUG"), '"0"');
  const keys = [];
  for (const name of Object.keys(rustCommands)) {
    const job = jobs(ciConfig)[name];
    assert.doesNotMatch(job, /CARGO_PROFILE_(DEV|TEST)_DEBUG|--profile|--release/);
    const caches = steps(job).filter(step => value(step, "uses")?.startsWith("Swatinem/rust-cache@"));
    assert.equal(caches.length, 1);
    assert.equal(value(caches[0], "save-if"), "${{ github.event_name == 'push' && github.ref == 'refs/heads/main' }}");
    assert.equal(value(caches[0], "workspaces"), "src-tauri");
    keys.push(value(caches[0], "key"));
  }
  assert.ok(keys.every(Boolean));
  assert.equal(new Set(keys).size, keys.length);
});

test("concurrency cancels the same PR only and keeps main runs in separate groups", () => {
  const concurrency = section(ciConfig, "concurrency:");
  assert.equal(value(concurrency, "group"), "${{ github.workflow }}-${{ github.event_name == 'pull_request' && github.event.pull_request.number || github.run_id }}");
  assert.equal(value(concurrency, "cancel-in-progress"), "${{ github.event_name == 'pull_request' }}");
});

test("nightly runs daily and manually, gates both jobs, and checks out main for both events", () => {
  const triggers = section(nightlyConfig, "on:");
  assert.deepEqual([...triggers.matchAll(/^  ([\w-]+):$/gm)].map(([, name]) => name).sort(), ["schedule", "workflow_dispatch"]);
  assert.match(triggers, /^    - cron: '\d{1,2} \d{1,2} \* \* \*'$/m);
  const nightlyJobs = jobs(nightlyConfig);
  assert.deepEqual(Object.keys(nightlyJobs).sort(), ["check", "coverage", "performance"]);
  assert.equal(value(section(nightlyJobs.check, "    outputs:"), "run"), "${{ steps.changes.outputs.run }}");
  for (const name of ["performance", "coverage"]) {
    const job = nightlyJobs[name];
    assert.equal(value(job, "needs"), "check");
    assert.equal(value(job, "if"), "needs.check.outputs.run == 'true'");
    const checkouts = steps(job).filter(step => value(step, "uses")?.startsWith("actions/checkout@"));
    assert.equal(checkouts.length, 1);
    assert.equal(value(checkouts[0], "ref"), "main");
    assert.equal(value(checkouts[0], "if"), undefined);
    for (const step of steps(job).filter(step => commands(job).includes(value(step, "run")))) {
      assert.equal(value(step, "if"), undefined, `${name}: validation runs for both events`);
    }
  }
});

test("nightly runs all performance lib tests, the release daemon, and desktop CLI installation", () => {
  const performance = jobs(nightlyConfig).performance;
  assert.deepEqual(commands(performance), [
    "cargo test --locked --no-default-features --features performance --lib",
    "pnpm test:performance:daemon",
    "cargo test --locked --features performance --test desktop_cli_install",
  ]);
  assert.equal(value(performance, "working-directory"), "src-tauri");
  const daemon = steps(performance).find(step => value(step, "run") === "pnpm test:performance:daemon");
  assert.equal(value(daemon, "working-directory"), ".");
  const packageJson = JSON.parse(readFileSync(new URL("../../package.json", import.meta.url), "utf8"));
  assert.equal(packageJson.scripts["test:performance:daemon"], "cargo build --manifest-path src-tauri/Cargo.toml --locked --release --no-default-features --features performance --bin releash-backend && node --test tests/helpers/performance-daemon.test.mjs");
});

test("nightly measures both coverages with profile retention and uploads the checked-out main commit", () => {
  const coverage = jobs(nightlyConfig).coverage;
  assert.doesNotMatch(coverage, /continue-on-error:/);
  assert.deepEqual(commands(coverage), [
    "pnpm exec vitest run --coverage",
    "python3 .github/scripts/coverage.test.py",
    "cargo llvm-cov --locked --codecov --output-path rust-codecov.json",
  ]);
  const environment = section(coverage, "    env:");
  assert.equal(value(environment, "RUSTFLAGS"), "-C llvm-args=-runtime-counter-relocation");
  assert.equal(value(environment, "LLVM_PROFILE_FILE_NAME"), "releash-%m%c.profraw");
  assert.equal(value(environment, "RUST_TEST_THREADS"), '"2"');
  assert.match(coverage, /components: llvm-tools-preview/);
  assert.match(coverage, /uses: taiki-e\/install-action@cargo-llvm-cov/);
  const checkout = steps(coverage).find(step => value(step, "uses")?.startsWith("actions/checkout@"));
  assert.equal(value(checkout, "id"), "checkout");
  const uploads = steps(coverage).filter(step => value(step, "uses")?.startsWith("codecov/codecov-action@"));
  assert.deepEqual(uploads.map(step => [value(step, "flags"), value(step, "files")]), [
    ["typescript", "coverage/coverage-final.json"],
    ["rust", "src-tauri/rust-codecov.json"],
  ]);
  for (const step of uploads) {
    assert.equal(value(step, "fail_ci_if_error"), "true", `${value(step, "flags")} upload errors must fail nightly`);
    assert.equal(value(step, "token"), "${{ secrets.CODECOV_TOKEN }}");
    assert.equal(value(step, "override_branch"), "main");
    assert.equal(value(step, "override_commit"), "${{ steps.checkout.outputs.commit }}");
    assert.equal(value(step, "if"), undefined);
  }
});

test("AGENTS validation commands, directories, layers, and coverage environment match CI", () => {
  const agents = readFileSync(new URL("../../AGENTS.md", import.meta.url), "utf8");
  const instructions = agents.split("## ビルド・テスト・Lint\n")[1]?.split("\n## ")[0];
  assert.ok(instructions);
  const blocks = [...instructions.matchAll(/```bash\n([\s\S]*?)```/g)].map(([, script]) => script);
  assert.equal(blocks.length, 6);
  const documented = blocks.map((script, index) => {
    let directory = [1, 4].includes(index) ? "src-tauri" : ".";
    return script.split("\n").map(line => line.trim()).filter(Boolean).flatMap(line => {
      if (line.startsWith("cd ")) { directory = line.slice(3); return []; }
      if (line.startsWith("export ") || line === "(" || line === ")") return [];
      return [[directory, line]];
    });
  });
  for (const [config, expected] of [[ciConfig, documented.slice(0, 3).flat()], [nightlyConfig, documented.slice(3).flat()]]) {
    const actual = Object.values(jobs(config)).flatMap(job => steps(job).filter(step => commands(job).includes(value(step, "run"))).map(step => [
      value(step, "working-directory") ?? value(job.split("    steps:\n")[0], "working-directory") ?? ".",
      value(step, "run"),
    ]));
    assert.deepEqual(actual.sort(), expected.sort());
  }
  assert.ok(instructions.includes(`CARGO_PROFILE_DEV_DEBUG=${value(section(ciConfig, "env:"), "CARGO_PROFILE_DEV_DEBUG")}`));
  const exported = Object.fromEntries([...blocks[5].matchAll(/^  export (\w+)="(.*)"$/gm)].map(([, key, setting]) => [key, setting]));
  const configured = Object.fromEntries([...section(jobs(nightlyConfig).coverage, "    env:").matchAll(/^      (\w+): (.+)$/gm)].map(([, key, setting]) => [key, setting.replace(/^"(.*)"$/, "$1")]));
  assert.deepEqual(exported, configured);
});

for (const isPr of [true, false]) {
  test(`${isPr ? "PR" : "push"}: docs only skips, facets/code/renames run Rust`, async () => {
    for (const [paths, expected] of [
      [["README.md", "AGENTS.md", "docs/a.md", "docs/nested/code.rs"], false],
      [["workflows/facets/instructions/review.md"], true],
      [["src/main.tsx"], true],
      [["nested/README.md"], true],
      [["old.rs", "docs/renamed.md"], true],
      [["docs/a.md", "source\nfile.rs"], true],
    ]) {
      let result;
      await changes({
        sha: "push-head",
        payload: isPr ? { pull_request: { base: { sha: "base" }, head: { sha: "pr-head" } } } : { before: "base" },
      }, { setOutput: (name, value) => { assert.equal(name, "rust"); result = value; } }, name => {
        assert.equal(name, "node:child_process");
        return { execFileSync: (command, args) => {
          assert.equal(command, "git");
          assert.deepEqual(args, ["diff", "--no-renames", "--name-only", "-z", ...(isPr ? ["base...pr-head"] : ["base", "push-head"])]);
          return Buffer.from(`${paths.join("\0")}\0`);
        } };
      });
      assert.equal(result, expected, paths.join(", "));
    }
  });
}

test("initial push runs Rust; failed diff is not treated as docs only", async () => {
  const failure = new Error("missing base commit");
  const require = () => ({ execFileSync: () => { throw failure; } });
  let result;
  await changes({ payload: { before: "0".repeat(40) }, sha: "head" }, { setOutput: (_, value) => { result = value; } }, require);
  assert.equal(result, true);
  await assert.rejects(changes({ payload: { before: "base" }, sha: "head" }, { setOutput: () => assert.fail("failed diff has no output") }, require), failure);
});

test("nightly skips only scheduled runs with the same last successful main SHA", async () => {
  for (const [eventName, previous, expected] of [
    ["schedule", "head", false],
    ["schedule", "previous", true],
    ["schedule", undefined, true],
    ["workflow_dispatch", "head", true],
  ]) {
    let result;
    await nightly({ eventName, sha: "head", repo: { owner: "owner", repo: "repo" } }, {
      setOutput: (name, value) => { assert.equal(name, "run"); result = value; },
    }, { rest: { actions: { listWorkflowRuns: async args => {
      assert.equal(eventName, "schedule", "manual runs do not depend on the API");
      assert.deepEqual(args, { owner: "owner", repo: "repo", workflow_id: "nightly.yml", branch: "main", status: "success", per_page: 1 });
      return { data: { workflow_runs: previous ? [{ head_sha: previous }] : [] } };
    } } } });
    assert.equal(result, expected);
  }
  const failure = new Error("GitHub API unavailable");
  await assert.rejects(nightly({ eventName: "schedule" }, { setOutput: () => assert.fail("failed API has no output") }, {
    rest: { actions: { listWorkflowRuns: async () => { throw failure; } } },
  }), failure);
});

test("rust aggregate accepts success/skips and rejects every failure/cancellation combination", () => {
  const script = block("ci.yml", "  rust:", "run");
  for (const lint of ["success", "skipped", "failure", "cancelled"]) {
    for (const desktop of ["success", "skipped", "failure", "cancelled"]) {
      for (const headless of ["success", "skipped", "failure", "cancelled"]) {
        const results = { "rust-lint": lint, "rust-test-desktop": desktop, "rust-test-headless": headless };
        const run = () => execFileSync("bash", ["-e", "-c", script.replace(/\$\{\{ needs\.(\S+)\.result \}\}/g, (_, job) => results[job])], { stdio: "pipe" });
        if (Object.values(results).every(result => ["success", "skipped"].includes(result))) run();
        else assert.throws(run);
      }
    }
  }
});
