import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
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
const stableConfig = readFileSync(new URL("../workflows/stable.yml", import.meta.url), "utf8");
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
  assert.equal(section(ciConfig, "on:").trim(), "push:\n    branches: [main]\n  pull_request:\n    branches: [main]\n  workflow_call:\n    inputs:\n      ref:\n        required: true\n        type: string");
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

test("nightly runs daily and manually with performance pinned and coverage tracking main", () => {
  const triggers = section(nightlyConfig, "on:");
  assert.deepEqual([...triggers.matchAll(/^  ([\w-]+):$/gm)].map(([, name]) => name).sort(), ["schedule", "workflow_dispatch"]);
  assert.match(triggers, /^    - cron: '\d{1,2} \d{1,2} \* \* \*'$/m);
  const nightlyJobs = jobs(nightlyConfig);
  assert.deepEqual(Object.keys(nightlyJobs).sort(), ["check", "cleanup", "coverage", "performance", "release", "validation"]);
  assert.equal(value(section(nightlyJobs.check, "    outputs:"), "run"), "${{ steps.changes.outputs.run }}");
  for (const [name, ref] of [["performance", "${{ needs.check.outputs.sha }}"], ["coverage", "main"]]) {
    const job = nightlyJobs[name];
    assert.equal(value(job, "needs"), "check");
    assert.equal(value(job, "if"), "needs.check.outputs.run == 'true'");
    const checkouts = steps(job).filter(step => value(step, "uses")?.startsWith("actions/checkout@"));
    assert.equal(checkouts.length, 1);
    assert.equal(value(checkouts[0], "ref"), ref);
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
        eventName: isPr ? "pull_request" : "push",
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
  await changes({ eventName: "push", payload: { before: "0".repeat(40) }, sha: "head" }, { setOutput: (_, value) => { result = value; } }, require);
  assert.equal(result, true);
  await assert.rejects(changes({ eventName: "push", payload: { before: "base" }, sha: "head" }, { setOutput: () => assert.fail("failed diff has no output") }, require), failure);
});

test("nightly compares the latest published nightly tag's commit with main, never the dispatch ref", async () => {
  for (const [eventName, previous, expected] of [
    ["schedule", "main-head", false],
    ["schedule", "previous", true],
    ["schedule", undefined, true],
    ["workflow_dispatch", "main-head", true],
  ]) {
    const output = {};
    const latest = { id: 2, tag_name: "v0.4.15-nightly.20260921.2", published_at: "2026-09-21T01:00:00Z", prerelease: true, draft: false };
    const draft = { ...latest, id: 4, tag_name: "v0.4.15-nightly.20260921.3", published_at: null, draft: true };
    const releases = previous ? [
      { ...latest, id: 1, tag_name: "v0.4.15-nightly.20260921.1", published_at: "2026-09-20T01:00:00Z" },
      { ...latest, id: 3, tag_name: "v0.4.15", prerelease: false },
      draft,
      { ...latest, id: 5, tag_name: "v0.4.15-beta.1" },
      latest,
    ] : [draft];
    const repo = { owner: "owner", repo: "repo" };
    const listReleases = () => {};
    await nightly({ eventName, sha: "dispatch-head", repo }, {
      setOutput: (name, value) => { output[name] = value; },
    }, {
      paginate: async (method, args) => {
        assert.equal(eventName, "schedule");
        assert.equal(method, listReleases);
        assert.deepEqual(args, { ...repo, per_page: 100 });
        return releases;
      },
      rest: { repos: { listReleases, getCommit: async args => {
        assert.deepEqual(args, { ...repo, ref: args.ref });
        if (args.ref === "heads/main") return { data: { sha: "main-head" } };
        assert.equal(args.ref, `tags/${latest.tag_name}`);
        return { data: { sha: previous } };
      } } },
    });
    assert.deepEqual(output, { sha: "main-head", run: expected });
  }
});

test("nightly API failures stop the run instead of treating them as no prior release", async () => {
  const failure = new Error("GitHub API unavailable");
  for (const failing of ["main", "releases", "tag"]) {
    await assert.rejects(nightly({ eventName: "schedule", repo: {} }, {
      setOutput: name => assert.equal(name, "sha", "failed lookup must not enable validation"),
    }, {
      paginate: async () => {
        if (failing === "releases") throw failure;
        return [{ tag_name: "v1.0.0-nightly.20260921.1", prerelease: true, draft: false }];
      },
      rest: { repos: { getCommit: async ({ ref }) => {
        if (failing === "main" || ref.startsWith("tags/")) throw failure;
        return { data: { sha: "main" } };
      } } },
    }), failure);
  }
});

test("nightly rejects a workflow commit different from the release target while preserving schedule skips", () => {
  const checkSteps = steps(jobs(nightlyConfig).check);
  const guard = checkSteps.find(step => value(step, "name") === "Verify workflow commit");
  assert.ok(guard, "the check job must reject mismatched workflow definitions");
  assert.ok(checkSteps.indexOf(guard) > checkSteps.findIndex(step => value(step, "id") === "changes"));
  const reject = new Function("steps", "github", `return ${value(guard, "if")}`);
  for (const event_name of ["schedule", "workflow_dispatch"]) {
    for (const [run, workflow_sha, expected] of [
      ["true", "main-head", false],
      ["true", "older-main", true],
      ["true", "feature-head", true],
      ["true", "", true],
      ["false", "main-head", false],
      ["false", "older-main", false],
    ]) {
      assert.equal(reject({ changes: { outputs: { run, sha: "main-head" } } }, { event_name, workflow_sha }), expected,
        `${event_name}: run=${run}, workflow_sha=${workflow_sha}`);
    }
  }
  assert.throws(() => execFileSync("bash", ["-e", "-c", block("nightly.yml", "      - name: Verify workflow commit", "run")], { stdio: "pipe" }),
    error => error.status === 1 && error.stdout.toString().includes("::error::"));
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

test("nightly reuses every PR check on the pinned SHA and always enables Rust", async () => {
  const validation = jobs(nightlyConfig).validation;
  assert.equal(value(validation, "needs"), "check");
  assert.equal(value(validation, "if"), "needs.check.outputs.run == 'true'");
  assert.equal(value(validation, "uses"), "./.github/workflows/ci.yml");
  assert.equal(value(validation, "ref"), "${{ needs.check.outputs.sha }}");
  for (const job of Object.values(jobs(ciConfig))) {
    for (const step of steps(job).filter(step => value(step, "uses")?.startsWith("actions/checkout@"))) {
      assert.equal(value(step, "ref"), "${{ inputs.ref || github.sha }}");
    }
  }
  for (const eventName of ["schedule", "workflow_dispatch"]) {
    const output = {};
    await changes({ eventName }, { setOutput: (key, value) => { output[key] = value; } }, () => assert.fail("nightly must not diff a push or PR"));
    assert.deepEqual(output, { rust: true });
  }
});

test("only successful gates can create a release; coverage cannot block publication", () => {
  const nightlyJobs = jobs(nightlyConfig);
  for (const [name, job] of [
    ...Object.entries(jobs(ciConfig)),
    ...["check", "validation", "performance"].map(name => [name, nightlyJobs[name]]),
  ]) {
    assert.doesNotMatch(job, /continue-on-error:/, `${name}: job and step failures must fail the gate`);
  }
  assert.equal(value(nightlyJobs.release, "needs"), "[check, validation, performance]");
  assert.doesNotMatch(nightlyJobs.release, /continue-on-error:|^    if:/m);
  const releaseSteps = steps(nightlyJobs.release);
  const createIndex = releaseSteps.findIndex(step => value(step, "id") === "create");
  const buildIndex = releaseSteps.findIndex(step => value(step, "uses") === "tauri-apps/tauri-action@v1");
  const publishIndex = releaseSteps.findIndex(step => value(step, "name") === "Publish nightly");
  assert.ok(createIndex >= 0 && createIndex < buildIndex && buildIndex < publishIndex);
  for (const step of releaseSteps) assert.equal(value(step, "if"), undefined, "release steps require previous success");
  assert.equal(value(section(nightlyJobs.check, "    outputs:"), "sha"), "${{ steps.changes.outputs.sha }}");
  const stableJobs = jobs(stableConfig);
  for (const name of ["create-release", "build", "publish"]) {
    assert.doesNotMatch(stableJobs[name], /continue-on-error:|^    if:/m);
    assert.doesNotMatch(value(stableJobs[name], "needs"), /coverage/);
  }
  assert.equal(value(stableJobs.build, "needs"), "create-release");
  assert.equal(value(stableJobs.publish, "needs"), "[create-release, build]");
  assert.equal(value(stableJobs.build, "ref"), "${{ needs.create-release.outputs.sha }}");
  for (const config of [nightlyConfig, stableConfig]) assert.equal(value(section(config, "concurrency:"), "cancel-in-progress"), "false");
  assert.equal(value(section(nightlyConfig, "concurrency:"), "group"), "nightly-release");
  assert.equal(value(section(stableConfig, "concurrency:"), "group"), "stable-release");
});

test("resolved commits and release IDs reach creation, build and publication through YAML outputs and env", () => {
  for (const [config, source, resolver] of [[nightlyConfig, "check", "changes"], [stableConfig, "resolve", "nightly"]]) {
    const workflowJobs = jobs(config);
    assert.equal(value(section(workflowJobs[source], "    outputs:"), "sha"), `\${{ steps.${resolver}.outputs.sha }}`);
    assert.ok(steps(workflowJobs[source]).some(step => value(step, "id") === resolver));
    const isNightly = config === nightlyConfig;
    const create = isNightly ? workflowJobs.release : workflowJobs["create-release"];
    const sha = `\${{ needs.${source}.outputs.sha }}`;
    if (!isNightly) {
      assert.equal(value(section(create, "    outputs:"), "sha"), sha);
      assert.equal(value(section(create, "    outputs:"), "release-id"), "${{ steps.create.outputs.release-id }}");
    }
    const checkout = steps(create).find(step => value(step, "uses")?.startsWith("actions/checkout@"));
    assert.equal(value(checkout, "ref"), sha);
    assert.equal(value(checkout, "if"), undefined);
    const createStep = steps(create).find(step => value(step, "id") === "create");
    assert.equal(value(section(createStep, "        env:"), "RELEASE_SHA"), sha);
    const publishStep = steps(isNightly ? workflowJobs.release : workflowJobs.publish).find(step => value(step, "name")?.startsWith("Publish "));
    assert.equal(value(section(publishStep, "        env:"), "RELEASE_ID"), isNightly ? "${{ steps.create.outputs.release-id }}" : "${{ needs.create-release.outputs.release-id }}");
  }
  const stableJobs = jobs(stableConfig);
  assert.equal(value(stableJobs["create-release"], "needs"), "resolve");
  for (const name of ["resolve", "create-release"]) {
    const step = steps(stableJobs[name]).find(step => value(step, "id") === (name === "resolve" ? "nightly" : "create"));
    assert.equal(value(section(step, "        env:"), "NIGHTLY_TAG"), "${{ inputs.nightly }}");
  }
});

function script(file, marker, ...args) {
  return new AsyncFunction("context", "core", "github", "require", "process", "Date", block(file, marker, "script"))(...args);
}

const repo = { owner: "owner", repo: "repo" };
const fixedDate = class extends Date { constructor() { super("2026-09-21T15:00:00Z"); } };

test("nightly tags use the build date and the next daily ordinal across versions and retained tags", async () => {
  for (const [tags, ordinal] of [
    [[], 1],
    [["v0.4.15-nightly.20260920.20", "v0.4.15", "v0.4.15-beta.1"], 1],
    [["v0.4.14-nightly.20260921.9", "v0.4.15-nightly.20260921.2", "v0.4.15-nightly.20260921.10"], 11],
  ]) {
    const calls = [];
    const output = {};
    const listTags = () => {};
    await script("nightly.yml", "      - id: create", { repo }, { setOutput: (key, value) => { output[key] = value; } }, {
      paginate: async (method, args) => {
        assert.equal(method, listTags);
        assert.deepEqual(args, { ...repo, per_page: 100 });
        return tags.map(name => ({ name }));
      },
      rest: {
        git: { createRef: async args => calls.push(["tag", args]) },
        repos: { listTags, createRelease: async args => { calls.push(["release", args]); return { data: { id: 42 } }; } },
      },
    }, name => { assert.equal(name, "./package.json"); return { version: "0.4.15" }; }, { env: { RELEASE_SHA: "verified-main" } }, fixedDate);
    const tag = `v0.4.15-nightly.20260921.${ordinal}`;
    assert.deepEqual(calls, [
      ["tag", { ...repo, ref: `refs/tags/${tag}`, sha: "verified-main" }],
      ["release", { ...repo, tag_name: tag, target_commitish: "verified-main", name: tag, draft: true, prerelease: true, make_latest: "false", generate_release_notes: true }],
    ]);
    assert.deepEqual(output, { "release-id": 42 });
  }
});

test("repository versions agree across all four files and contain only three integers", () => {
  const packageJson = JSON.parse(readFileSync(new URL("../../package.json", import.meta.url), "utf8"));
  const tauriConfig = JSON.parse(readFileSync(new URL("../../src-tauri/tauri.conf.json", import.meta.url), "utf8"));
  const cargoPackage = readFileSync(new URL("../../src-tauri/Cargo.toml", import.meta.url), "utf8").split("[package]\n")[1].split("\n[")[0];
  const lockedPackage = readFileSync(new URL("../../src-tauri/Cargo.lock", import.meta.url), "utf8").split("[[package]]\n").find(entry => /^name = "releash"$/m.test(entry));
  for (const [file, version] of [
    ["package.json", packageJson.version],
    ["src-tauri/tauri.conf.json", tauriConfig.version],
    ["src-tauri/Cargo.toml", cargoPackage.match(/^version = "([^"]+)"$/m)?.[1]],
    ["src-tauri/Cargo.lock", lockedPackage?.match(/^version = "([^"]+)"$/m)?.[1]],
  ]) {
    assert.match(version, /^\d+\.\d+\.\d+$/, `${file}: expected an X.Y.Z version`);
    assert.equal(version, packageJson.version, `${file}: must match package.json`);
  }
});

test("invalid repository versions and conflicting new tags fail before creating a release", async () => {
  for (const file of ["nightly.yml", "stable.yml"]) {
    for (const version of ["1.2.3-beta.1", "v1.2.3", "1.2", "1.2.3\n"]) {
      await assert.rejects(script(file, "      - id: create", { repo }, {}, {}, () => ({ version }), { env: {} }, fixedDate), /Expected an X.Y.Z version/);
    }
    const failure = new Error("Tag already exists");
    await assert.rejects(script(file, "      - id: create", { repo }, {}, {
      paginate: async () => [],
      rest: { repos: { getCommit: async () => { throw Object.assign(new Error("Missing tag"), { status: 404 }); } }, git: { createRef: async () => { throw failure; } } },
    }, () => ({ version: "1.2.3" }), { env: { NIGHTLY_TAG: "v1.2.3-nightly.20260921.1" } }, fixedDate), failure);
  }
});

test("nightly publishes as prerelease and retains 14 published nightlies without drafts", async () => {
  const calls = [];
  await script("nightly.yml", "      - name: Publish nightly", { repo }, {}, {
    rest: { repos: { updateRelease: async args => calls.push(args) } },
  }, null, { env: { RELEASE_ID: "42" } });
  assert.deepEqual(calls, [{ ...repo, release_id: 42, draft: false, prerelease: true, make_latest: "false" }]);
  for (const count of [0, 14, 15, 105]) {
    const deleted = [];
    const listReleases = () => {};
    const nightlies = Array.from({ length: count }, (_, i) => ({
      id: i + 1, tag_name: `v1.2.3-nightly.20260921.${i + 1}`, prerelease: true, draft: false,
      published_at: new Date(Date.UTC(2026, 8, 21, 0, i)).toISOString(),
      created_at: new Date(Date.UTC(2026, 8, 21, 0, i)).toISOString(),
    }));
    const drafts = [
      { id: 200, tag_name: "v1.2.3-nightly.20260920.1", prerelease: true, draft: true, published_at: null, created_at: "2026-09-20T00:00:00Z" },
      { id: 201, tag_name: "v1.2.3-nightly.20260922.1", prerelease: true, draft: true, published_at: null, created_at: "2026-09-22T00:00:00Z" },
    ];
    await script("nightly.yml", "      - name: Keep the latest 14 nightlies", { repo }, {}, {
      paginate: async (method, args) => {
        assert.equal(method, listReleases);
        assert.deepEqual(args, { ...repo, per_page: 100 });
        return [
          { id: -1, tag_name: "v1.2.3", prerelease: false, draft: false },
          { id: -3, tag_name: "v1.2.3-beta.1", prerelease: true, draft: false },
          { id: -4, tag_name: "v1.2.4", prerelease: false, draft: true },
          ...drafts,
          ...nightlies,
        ];
      },
      rest: { repos: { listReleases, deleteRelease: async args => deleted.push(args) } },
    });
    assert.deepEqual(deleted, [...drafts.toReversed(), ...nightlies.slice(0, Math.max(0, count - 14)).reverse()].map(({ id }) => ({ ...repo, release_id: id })));
  }
});

test("nightly cleans failed drafts in a separate job and retries creation, build and publication with a new release", async () => {
  const cleanup = jobs(nightlyConfig).cleanup;
  assert.equal(value(cleanup, "needs"), "release");
  assert.equal(value(cleanup, "runs-on"), "ubuntu-latest");
  const cleanupCondition = new Function("always", "needs", `return ${value(cleanup, "if").slice(3, -2)}`);
  for (const result of ["success", "failure", "cancelled", "skipped"]) {
    assert.equal(cleanupCondition(() => true, { release: { result } }), result !== "skipped");
  }
  for (const failing of ["creation", "build", "timeout", "publication"]) {
    const releases = Array.from({ length: 14 }, (_, i) => ({
      id: i + 1, tag_name: `v1.2.3-nightly.20260921.${i + 1}`, prerelease: true, draft: false,
      published_at: new Date(Date.UTC(2026, 8, 21, 0, i)).toISOString(),
    }));
    const tags = releases.map(release => ({ name: release.tag_name }));
    const builds = [];
    let nextId = 15;
    let failAt = failing;
    const failure = new Error(`Failed ${failing}`);
    const listTags = () => {};
    const listReleases = () => {};
    const github = {
      paginate: async method => {
        assert.ok(method === listTags || method === listReleases);
        return method === listTags ? tags : releases;
      },
      rest: { git: { createRef: async ({ ref, sha }) => {
        assert.equal(sha, "verified-main");
        assert.ok(!tags.some(tag => `refs/tags/${tag.name}` === ref));
        tags.push({ name: ref.slice("refs/tags/".length) });
      } }, repos: {
        listTags,
        listReleases,
        createRelease: async args => {
          assert.equal(args.target_commitish, "verified-main");
          if (failAt === "creation") throw failure;
          const current = { ...args, id: nextId++, published_at: null, created_at: "2026-09-21T15:00:00Z", assets: [] };
          releases.push(current);
          return { data: current };
        },
        updateRelease: async ({ release_id }) => {
          const current = releases.find(release => release.id === release_id);
          assert.ok(current, "publication uses an existing release");
          assert.deepEqual(current.assets, ["signed-universal-build"]);
          if (failAt === "publication") throw failure;
          current.draft = false;
          current.published_at = current.created_at;
        },
        deleteRelease: async ({ release_id }) => releases.splice(releases.findIndex(release => release.id === release_id), 1),
      } },
    };
    const attempt = async () => {
      const output = {};
      await script("nightly.yml", "      - id: create", { repo }, { setOutput: (key, value) => { output[key] = value; } }, github,
        () => ({ version: "1.2.3" }), { env: { RELEASE_SHA: "verified-main" } }, fixedDate);
      const releaseId = output["release-id"];
      builds.push(releaseId);
      if (["build", "timeout"].includes(failAt)) throw failure;
      const current = releases.find(release => release.id === releaseId);
      assert.ok(current, "build uploads to the release created in this attempt");
      current.assets.push("signed-universal-build");
      await script("nightly.yml", "      - name: Publish nightly", { repo }, {}, github, null, { env: { RELEASE_ID: String(releaseId) } });
    };
    await assert.rejects(attempt(), failure);
    await script("nightly.yml", "      - name: Keep the latest 14 nightlies", { repo }, {}, github);
    assert.deepEqual(releases.map(release => release.id), Array.from({ length: 14 }, (_, i) => i + 1));
    failAt = null;
    await attempt();
    await script("nightly.yml", "      - name: Keep the latest 14 nightlies", { repo }, {}, github);
    assert.equal(tags.at(-1).name, "v1.2.3-nightly.20260921.16");
    assert.equal(new Set(builds).size, builds.length, "retries never reuse a deleted release ID");
    assert.equal(releases.length, 14);
    assert.ok(releases.every(release => !release.draft));
    assert.equal(releases.at(-1).tag_name, "v1.2.3-nightly.20260921.16");
    assert.deepEqual(releases.slice(0, 13).map(release => release.id), Array.from({ length: 13 }, (_, i) => i + 2));
  }
});

test("stable accepts only a published nightly and resolves its tag to a commit", async () => {
  const resolve = (tag, release, output = {}) => script("stable.yml", "  resolve:", { repo }, {
    setOutput: (key, value) => { output[key] = value; },
  }, { rest: { repos: {
    getReleaseByTag: async args => { assert.deepEqual(args, { ...repo, tag }); return { data: release }; },
    getCommit: async args => { assert.deepEqual(args, { ...repo, ref: `tags/${tag}` }); return { data: { sha: "nightly-commit" } }; },
  } } }, null, { env: { NIGHTLY_TAG: tag } });
  const output = {};
  await resolve("v1.2.3-nightly.20260921.1", { prerelease: true, draft: false }, output);
  assert.deepEqual(output, { sha: "nightly-commit" });
  for (const tag of ["main", "v1.2.3", "v1.2.3-nightly.20260921.0", "$(echo secret)", "v1.2.3-nightly.20260921.1\n"]) {
    await assert.rejects(resolve(tag), /Expected a nightly tag/);
  }
  for (const release of [{ draft: true, prerelease: true }, { draft: false, prerelease: false }]) {
    await assert.rejects(resolve("v1.2.3-nightly.20260921.1", release), /Expected a published nightly prerelease/);
  }
  const failure = new Error("Missing nightly release");
  await assert.rejects(script("stable.yml", "  resolve:", { repo }, {}, {
    rest: { repos: { getReleaseByTag: async () => { throw failure; } } },
  }, null, { env: { NIGHTLY_TAG: "v1.2.3-nightly.20260921.1" } }), failure);
});

test("stable resumes after tag creation, release creation or output fails without duplicating either resource", async () => {
  for (const failing of ["tag-response", "release-request", "release-response", "output"]) {
    const failure = new Error(`Failed ${failing}`);
    let failed = false;
    const fail = point => {
      if (point === failing && !failed) { failed = true; throw failure; }
    };
    let tagSha;
    let release;
    const writes = [];
    const output = {};
    const listReleases = () => {};
    const github = {
      paginate: async (method, args) => {
        assert.equal(method, listReleases);
        assert.deepEqual(args, { ...repo, per_page: 100 });
        return [
          { id: 1, tag_name: "v1.2.2", draft: false, prerelease: false },
          { id: 2, tag_name: "v1.2.3-nightly.20260921.1", draft: false, prerelease: true },
          ...(release ? [release] : []),
        ];
      },
      rest: {
        git: { createRef: async args => {
          assert.equal(tagSha, undefined, "an existing tag must not be recreated");
          assert.deepEqual(args, { ...repo, ref: "refs/tags/v1.2.3", sha: "nightly-commit" });
          tagSha = args.sha;
          writes.push("tag");
          fail("tag-response");
        } },
        repos: {
          listReleases,
          getCommit: async args => {
            assert.deepEqual(args, { ...repo, ref: "tags/v1.2.3" });
            if (!tagSha) throw Object.assign(new Error("Missing tag"), { status: 404 });
            return { data: { sha: tagSha } };
          },
          createRelease: async args => {
            assert.equal(release, undefined, "an existing draft must not be recreated");
            fail("release-request");
            release = { ...args, id: 42 };
            writes.push("release");
            fail("release-response");
            return { data: release };
          },
        },
      },
    };
    const attempt = () => script("stable.yml", "  create-release:", { repo }, {
      setOutput: (key, value) => { fail("output"); output[key] = value; },
    }, github, () => ({ version: "1.2.3" }), { env: { RELEASE_SHA: "nightly-commit", NIGHTLY_TAG: "v1.2.3-nightly.20260921.1" } });
    await assert.rejects(attempt(), failure);
    await attempt();
    assert.deepEqual(output, { "release-id": 42 });
    assert.equal(release.draft, true);
    assert.equal(release.prerelease, false);
    assert.equal(release.target_commitish, "nightly-commit");
    await attempt();
    assert.deepEqual(writes, ["tag", "release"]);
  }
});

test("stable rejects different commits, published releases and incompatible drafts before writing", async () => {
  for (const [sha, release, expected] of [
    ["other-commit", undefined, /different commit/],
    ["other-commit", { draft: true, prerelease: false }, /different commit/],
    ["nightly-commit", { draft: false, prerelease: false }, /unpublished stable release/],
    ["nightly-commit", { draft: true, prerelease: true }, /unpublished stable release/],
    [undefined, { draft: true, prerelease: false }, /unpublished stable release/],
  ]) {
    const noWrite = () => assert.fail("a conflicting release must not be changed");
    await assert.rejects(script("stable.yml", "  create-release:", { repo }, { setOutput: noWrite }, {
      paginate: async () => release ? [{ ...release, tag_name: "v1.2.3" }] : [],
      rest: {
        git: { createRef: noWrite },
        repos: {
          getCommit: async () => {
            if (!sha) throw Object.assign(new Error("Missing tag"), { status: 404 });
            return { data: { sha } };
          },
          createRelease: noWrite,
        },
      },
    }, () => ({ version: "1.2.3" }), { env: { RELEASE_SHA: "nightly-commit", NIGHTLY_TAG: "v1.2.3-nightly.20260921.1" } }), expected);
  }
});

test("stable propagates lookup failures instead of treating them as absent tags or releases", async () => {
  for (const [failing, status] of [["tag", 401], ["tag", 403], ["tag", 500], ["releases", 404], ["releases", 403], ["releases", 500]]) {
    const failure = Object.assign(new Error("GitHub API failure"), { status });
    const noWrite = () => assert.fail("lookup failures must stop before writing");
    await assert.rejects(script("stable.yml", "  create-release:", { repo }, { setOutput: noWrite }, {
      paginate: async () => { throw failure; },
      rest: {
        git: { createRef: noWrite },
        repos: {
          getCommit: async () => {
            if (failing === "tag") throw failure;
            throw Object.assign(new Error("Missing tag"), { status: 404 });
          },
          createRelease: noWrite,
        },
      },
    }, () => ({ version: "1.2.3" }), { env: { RELEASE_SHA: "nightly-commit", NIGHTLY_TAG: "v1.2.3-nightly.20260921.1" } }), failure);
  }
});

test("stable creates its version tag at the selected nightly commit and requires the updater manifest to publish", async () => {
  const calls = [];
  const output = {};
  const env = { NIGHTLY_TAG: "v1.2.3-nightly.20260921.1", RELEASE_SHA: "nightly-commit", RELEASE_ID: "42" };
  await script("stable.yml", "  create-release:", { repo }, { setOutput: (key, value) => { output[key] = value; } }, {
    paginate: async () => [],
    rest: {
      git: { createRef: async args => calls.push(["tag", args]) },
      repos: {
        getCommit: async () => { throw Object.assign(new Error("Missing tag"), { status: 422 }); },
        createRelease: async args => { calls.push(["release", args]); return { data: { id: 42 } }; },
      },
    },
  }, () => ({ version: "1.2.3" }), { env });
  assert.deepEqual(calls, [
    ["tag", { ...repo, ref: "refs/tags/v1.2.3", sha: "nightly-commit" }],
    ["release", { ...repo, tag_name: "v1.2.3", target_commitish: "nightly-commit", name: "v1.2.3", draft: true, prerelease: false, make_latest: "false", generate_release_notes: true }],
  ]);
  assert.deepEqual(output, { "release-id": 42 });
  await assert.rejects(script("stable.yml", "  create-release:", { repo }, {}, {}, () => ({ version: "1.2.4" }), { env }), /Nightly tag and repository version differ/);
  for (const [assets, canPublish] of [
    [[], false],
    [[{ name: "latest.json", size: 0 }], false],
    [[{ name: "Releash.dmg", size: 123 }], false],
    [[{ name: "Releash.dmg", size: 123 }, { name: "latest.json", size: 0 }], false],
    [[{ name: "latest.json", size: 123 }], true],
    [[{ name: "Releash.dmg", size: 123 }, { name: "latest.json", size: 123 }], true],
  ]) {
    const published = [];
    const listReleaseAssets = () => {};
    const result = script("stable.yml", "  publish:", { repo }, {}, {
      paginate: async (method, args) => {
        assert.equal(method, listReleaseAssets);
        assert.deepEqual(args, { ...repo, release_id: 42, per_page: 100 });
        return assets;
      },
      rest: { repos: { listReleaseAssets, updateRelease: async args => published.push(args) } },
    }, null, { env });
    if (canPublish) {
      await result;
      assert.deepEqual(published, [{ ...repo, release_id: 42, draft: false, prerelease: false, make_latest: "true" }]);
    } else {
      await assert.rejects(result, /Missing updater manifest/);
      assert.deepEqual(published, []);
    }
  }
});

test("stable creates its tag with a workflows-capable token instead of GITHUB_TOKEN", () => {
  const create = jobs(stableConfig)["create-release"];
  assert.doesNotMatch(create, /^    permissions:$/m);
  const secrets = steps(create).find(step => value(step, "id") === "secrets");
  assert.ok(value(secrets, "uses").startsWith("1password/load-secrets-action@"));
  assert.equal(value(section(secrets, "        env:"), "OP_SERVICE_ACCOUNT_TOKEN"), "${{ secrets.OP_SERVICE_ACCOUNT_TOKEN }}");
  assert.equal(value(section(secrets, "        env:"), "STABLE_RELEASE_TOKEN"), "op://releash/releash-stable-release/token");
  assert.equal(value(secrets, "export-env"), "false");
  const createStep = steps(create).find(step => value(step, "id") === "create");
  assert.equal(value(createStep, "github-token"), "${{ steps.secrets.outputs.STABLE_RELEASE_TOKEN }}");
  assert.ok(steps(create).indexOf(secrets) < steps(create).indexOf(createStep));
});

test("both builds preserve signing, notarization, updater artifacts, telemetry and universal Tauri build settings", () => {
  for (const config of [nightlyConfig, stableConfig]) {
    const build = config === nightlyConfig ? jobs(config).release : jobs(config).build;
    assert.equal(value(build, "runs-on"), "macos-latest");
    const secrets = steps(build).find(step => value(step, "uses")?.startsWith("1password/load-secrets-action@"));
    const environment = Object.fromEntries([...section(secrets, "        env:").matchAll(/^          (\w+): (.+)$/gm)].map(([, key, value]) => [key, value]));
    assert.deepEqual(environment, {
      OP_SERVICE_ACCOUNT_TOKEN: "${{ secrets.OP_SERVICE_ACCOUNT_TOKEN }}",
      APPLE_CERTIFICATE: "op://releash/apple-signing/certificate",
      APPLE_CERTIFICATE_PASSWORD: "op://releash/apple-signing/certificate-password",
      APPLE_SIGNING_IDENTITY: "op://releash/apple-signing/signing-identity",
      APPLE_ID: "op://releash/apple-signing/apple-id",
      APPLE_PASSWORD: "op://releash/apple-signing/app-password",
      APPLE_TEAM_ID: "op://releash/apple-signing/team-id",
      TAURI_SIGNING_PRIVATE_KEY: "op://releash/tauri-updater/private-key",
      TAURI_SIGNING_PRIVATE_KEY_PASSWORD: "op://releash/tauri-updater/password",
      OTLP_ENDPOINT: "op://releash/new-relic/otlp-endpoint",
      NEW_RELIC_LICENSE_KEY: "op://releash/new-relic/license-key",
    });
    assert.equal(value(secrets, "export-env"), "true");
    const tauri = steps(build).find(step => value(step, "uses") === "tauri-apps/tauri-action@v1");
    assert.equal(value(tauri, "releaseId"), config === nightlyConfig ? "${{ steps.create.outputs.release-id }}" : "${{ needs.create-release.outputs.release-id }}");
    assert.equal(value(tauri, "args"), "--target universal-apple-darwin -- --features vendored-openssl");
    assert.equal(value(tauri, "uploadUpdaterJson"), "true");
    assert.equal(value(tauri, "tauriScript"), "pnpm tauri");
    assert.equal(value(tauri, "OTLP_ENDPOINT"), "${{ env.OTLP_ENDPOINT }}");
    assert.equal(value(tauri, "NEW_RELIC_LICENSE_KEY"), "${{ env.NEW_RELIC_LICENSE_KEY }}");
  }
  const config = JSON.parse(readFileSync(new URL("../../src-tauri/tauri.conf.json", import.meta.url), "utf8"));
  assert.deepEqual(config.plugins.updater.endpoints, ["https://github.com/siro33950/releash/releases/latest/download/latest.json"]);
  assert.equal(config.bundle.createUpdaterArtifacts, true);
});

test("stable bumps main's patch after publication and reuses Bump Version's four-file update", t => {
  const directory = mkdtempSync(join(tmpdir(), "releash-version-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const githubEnv = join(directory, "github-env");
  const stableJobs = jobs(stableConfig);
  assert.equal(value(stableJobs.bump, "needs"), "publish");
  assert.equal(value(stableJobs.bump, "ref"), "main");
  assert.equal(block("stable.yml", "      - name: Bump version in files", "run"), block("bump-version.yml", "      - name: Bump version in files", "run"));
  const bumpScript = block("stable.yml", "      - name: Calculate next patch version", "run");
  for (const [current, next] of [["0.4.15", "0.4.16"], ["1.9.99", "1.9.100"], ["0.0.0", "0.0.1"]]) {
    writeFileSync(githubEnv, "");
    execFileSync("bash", ["-e", "-c", `jq() { echo "$TEST_CURRENT"; }\n${bumpScript}`], {
      env: { ...process.env, TEST_CURRENT: current, GITHUB_ENV: githubEnv },
    });
    assert.equal(readFileSync(githubEnv, "utf8").trim(), `VERSION=${next}`);
  }
  assert.throws(() => execFileSync("bash", ["-e", "-c", `jq() { echo '1.2.3-beta'; }\n${bumpScript}`], { env: { ...process.env, GITHUB_ENV: githubEnv }, stdio: "pipe" }));
  const pr = steps(stableJobs.bump).find(step => value(step, "uses")?.startsWith("peter-evans/create-pull-request@"));
  assert.equal(value(pr, "base"), "main");
  assert.equal(value(pr, "branch"), "release/v${{ env.VERSION }}");
  assert.equal(value(pr, "title"), '"release: v${{ env.VERSION }}"');
  assert.match(pr, /add-paths: \|\n            package.json\n            src-tauri\/tauri.conf.json\n            src-tauri\/Cargo.toml\n            src-tauri\/Cargo.lock/);
});

test("Bump Version calculates each bump and updates all four files without changing dependencies", t => {
  const directory = mkdtempSync(join(tmpdir(), "releash-bump-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  mkdirSync(join(directory, "src-tauri"));
  const githubEnv = join(directory, "github-env");
  const calculate = block("bump-version.yml", "      - name: Calculate new version", "run");
  const update = block("bump-version.yml", "      - name: Bump version in files", "run");
  const portableUpdate = process.platform === "darwin" ? update.replaceAll("sed -i ", "sed -i '' ").replaceAll("/}'", "/;}'") : update;
  for (const [bump, current, expected] of [
    ["patch", "1.9.99", "1.9.100"],
    ["minor", "1.9.99", "1.10.0"],
    ["major", "1.9.99", "2.0.0"],
    ["patch", "0.0.0", "0.0.1"],
  ]) {
    const packageJson = { name: "releash", version: current, dependencies: { example: current } };
    const tauriConfig = { version: current, identifier: "com.releash.app" };
    const cargoToml = `[package]\nname = "releash"\nversion = "${current}"\n\n[dependencies]\nexample = "${current}"\n`;
    const cargoLock = `version = 4\n\n[[package]]\nname = "before"\nversion = "${current}"\n\n[[package]]\nname = "releash"\nversion = "${current}"\ndependencies = ["before", "after"]\n\n[[package]]\nname = "after"\nversion = "${current}"\n`;
    for (const [file, contents] of [
      ["package.json", JSON.stringify(packageJson)],
      ["src-tauri/tauri.conf.json", JSON.stringify(tauriConfig)],
      ["src-tauri/Cargo.toml", cargoToml],
      ["src-tauri/Cargo.lock", cargoLock],
    ]) writeFileSync(join(directory, file), contents);
    writeFileSync(githubEnv, "");
    execFileSync("bash", ["-e", "-c", calculate.replaceAll("${{ inputs.bump }}", bump)], {
      cwd: directory, env: { ...process.env, GITHUB_ENV: githubEnv }, stdio: "pipe",
    });
    const output = readFileSync(githubEnv, "utf8").trim();
    assert.equal(output, `VERSION=${expected}`);
    execFileSync("bash", ["-e", "-c", portableUpdate], {
      cwd: directory, env: { ...process.env, VERSION: output.slice("VERSION=".length) }, stdio: "pipe",
    });
    assert.deepEqual(JSON.parse(readFileSync(join(directory, "package.json"), "utf8")), { ...packageJson, version: expected });
    assert.deepEqual(JSON.parse(readFileSync(join(directory, "src-tauri/tauri.conf.json"), "utf8")), { ...tauriConfig, version: expected });
    for (const [file, original] of [["Cargo.toml", cargoToml], ["Cargo.lock", cargoLock]]) {
      assert.equal(readFileSync(join(directory, "src-tauri", file), "utf8"), original.replace(`name = "releash"\nversion = "${current}"`, `name = "releash"\nversion = "${expected}"`));
    }
  }
});

test("old release triggers are removed and AGENTS describes nightly, stable and version-only bumps", () => {
  assert.equal(existsSync(new URL("../workflows/auto-tag.yml", import.meta.url)), false);
  assert.equal(existsSync(new URL("../workflows/release.yml", import.meta.url)), false);
  assert.deepEqual([...section(stableConfig, "on:").matchAll(/^  ([\w-]+):$/gm)].map(([, name]) => name), ["workflow_dispatch"]);
  assert.equal(value(section(stableConfig, "      nightly:"), "type"), "string");
  assert.equal(value(section(stableConfig, "      nightly:"), "required"), "true");
  const agents = readFileSync(new URL("../../AGENTS.md", import.meta.url), "utf8");
  const release = agents.split("## リリース\n")[1].split("\n## ")[0];
  for (const text of ["Nightly", "main", "スキップ", "手動起動は常に", "coverage", "14", "Stable", "nightly", "latest.json", "patch", "Bump Version", "patch / minor / major"]) assert.ok(release.includes(text), text);
  assert.match(agents, /版を上げるコミットは `release: vX.Y.Z`/);
});
