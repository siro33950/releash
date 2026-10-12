import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";

const require = createRequire(new URL("../../.nx/installation/package.json", import.meta.url));
const { parse } = require("yaml");
const config = JSON.parse(readFileSync("renovate.json", "utf8"));
const workflow = parse(readFileSync(".github/workflows/renovate-postprocess.yml", "utf8"));
const steps = workflow.jobs.postprocess.steps;

function fixture(run) {
  const directory = mkdtempSync(join(tmpdir(), "releash-renovate-"));
  const write = (path, body) => {
    const target = join(directory, path);
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, body, { mode: 0o700 });
  };
  const execute = (name, env = {}) => execFileSync("bash", ["-e", "-o", "pipefail", "-c",
    steps.find(step => step.name === name).run], {
    cwd: directory,
    env: { ...process.env, PATH: `${directory}:${dirname(process.execPath)}:${process.env.PATH}`,
      GITHUB_OUTPUT: join(directory, "output"), BASE_SHA: "base", ...env },
    stdio: "pipe",
  });
  try {
    run({ directory, write, execute });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

test("Renovate uses default managers and update timing, pins Rust, and retires Dependabot", () => {
  assert.deepEqual(config.extends, ["config:recommended"]);
  assert.deepEqual(config.labels, ["dependencies"]);
  for (const key of ["enabledManagers", "schedule", "prConcurrentLimit", "prHourlyLimit", "vulnerabilityAlerts"]) {
    assert.equal(config[key], undefined);
  }
  assert.equal(config.automerge, true);
  assert.equal(config.automergeType, "pr");
  assert.equal(config.packageRules.find(rule => rule.matchManagers?.includes("rust-toolchain")).rangeStrategy, "pin");
  assert.equal(existsSync(".github/dependabot.yml"), false);
  assert.equal(existsSync(".github/workflows/dependabot-auto-merge.yml"), false);
});

test("Swift dependencies group both managers by package name independently", () => {
  for (const name of ["apple/swift-protobuf", "connectrpc/connect-swift"]) {
    const rule = config.packageRules.find(rule => rule.matchPackageNames?.includes(name));
    assert.deepEqual(rule.matchManagers, ["xcodegen", "swift"]);
    assert.deepEqual(rule.matchPackageNames, [name]);
    assert.ok(rule.groupName);
  }
});

test("Nx extraction updates both installation versions together and separates major upgrades without automerging", () => {
  const manager = config.customManagers[0];
  assert.equal(manager.datasourceTemplate, "npm");
  assert.match("nx.json", new RegExp(manager.managerFilePatterns[0].slice(1, -1)));
  const text = readFileSync("nx.json", "utf8");
  const matches = manager.matchStrings.map(pattern => new RegExp(pattern).exec(text));
  assert.equal(matches[0].groups.currentValue, JSON.parse(text).installation.version);
  assert.equal(matches[1].groups.depName, "@nx/js");
  assert.equal(matches[1].groups.currentValue, JSON.parse(text).installation.plugins["@nx/js"]);
  let updated = text;
  for (const pattern of manager.matchStrings) {
    const regex = new RegExp(pattern);
    const match = regex.exec(updated);
    updated = updated.replace(regex, match[0].replace(match.groups.currentValue, "999.0.0"));
  }
  assert.equal(JSON.parse(updated).installation.version, "999.0.0");
  assert.equal(JSON.parse(updated).installation.plugins["@nx/js"], "999.0.0");
  const rule = config.packageRules.find(rule => rule.matchPackageNames?.includes("nx"));
  assert.deepEqual(rule.matchPackageNames, ["nx", "@nx/js"]);
  assert.equal(rule.separateMultipleMajor, true);
  assert.equal(rule.automerge, false);
});

test("postprocessing uses base credentials, same-repository Renovate events, and an App token for push", () => {
  assert.ok(workflow.on.pull_request_target);
  assert.match(workflow.jobs.postprocess.if, /pull_request.user.login == 'renovate\[bot\]'/);
  assert.match(workflow.jobs.postprocess.if, /github.actor == 'renovate\[bot\]'/);
  assert.match(workflow.jobs.postprocess.if, /head.repo.full_name == github.repository/);
  assert.equal(steps[0].with.ref, "${{ github.event.pull_request.base.sha }}");
  const credentials = steps.findIndex(step => step.name === "Load App credentials");
  assert.match(steps[credentials].run, /load-secrets.mjs/);
  const token = steps.findIndex(step => step.uses === "actions/create-github-app-token@v3");
  assert.ok(token > credentials);
  const head = steps.findIndex(step => step.with?.ref === "${{ github.event.pull_request.head.sha }}");
  assert.ok(head > steps.findIndex(step => step.name === "Clear App credentials"));
  assert.ok(steps.filter(step => step.uses === "actions/checkout@v7").every(step => step.with["persist-credentials"] === false));
  assert.equal(steps.find(step => step.name === "Push postprocessing").env.GH_TOKEN, "${{ steps.app-token.outputs.token }}");
});

test("postprocessing selects Swift, Nx, both, or neither from the PR diff", () => {
  for (const [files, expected] of [
    ["clients/macos/project.yml", "swift=true\n"],
    ["clients/macos/BuildTools/Package.swift", "swift=true\n"],
    ["nx.json", "nx=true\n"],
    ["nx.json\nclients/macos/project.yml", "swift=true\nnx=true\n"],
    ["clients/desktop/package.json", ""],
  ]) {
    fixture(({ directory, write, execute }) => {
      write("git", '#!/bin/sh\nprintf "%s\\n" "$FILES"\n');
      execute("Select postprocessing", { FILES: files });
      assert.equal(existsSync(join(directory, "output")) ? readFileSync(join(directory, "output"), "utf8") : "", expected);
    });
  }
});

test("App identity uses the bot user ID and stops when the API fails", () => {
  for (const failure of [false, true]) {
    fixture(({ directory, write, execute }) => {
      write("gh", '#!/bin/sh\n[ "$1" = api ] && [ "$2" = "/users/app[bot]" ] || exit 2\n[ "$FAIL" = false ] || exit 1\necho 123\n');
      if (failure) {
        assert.throws(() => execute("Get App identity", { APP_SLUG: "app", FAIL: "true" }));
        assert.equal(existsSync(join(directory, "output")), false);
      } else {
        execute("Get App identity", { APP_SLUG: "app", FAIL: "false" });
        assert.equal(readFileSync(join(directory, "output"), "utf8"),
          "name=app[bot]\nemail=123+app[bot]@users.noreply.github.com\n");
      }
    });
  }
});

test("Renovate ignores the postprocessing App author for any bot ID but not other Apps", () => {
  assert.ok(config.gitIgnoredAuthors.includes("/^\\d+\\+releash-dependency-updates\\[bot\\]@users\\.noreply\\.github\\.com$/"));
  for (const slug of ["releash-dependency-updates", "app"]) {
    for (const id of ["1", "123456789"]) {
      fixture(({ directory, write, execute }) => {
        write("gh", '#!/bin/sh\n[ "$1" = api ] && [ "$2" = "/users/${APP_SLUG}[bot]" ] || exit 2\nprintf "%s\\n" "$BOT_ID"\n');
        execute("Get App identity", { APP_SLUG: slug, BOT_ID: id });
        const output = readFileSync(join(directory, "output"), "utf8");
        const email = output.split("\n").find(line => line.startsWith("email=")).slice("email=".length);
        assert.equal(email, `${id}+${slug}[bot]@users.noreply.github.com`);
        assert.equal(config.gitIgnoredAuthors.some(pattern => new RegExp(pattern.slice(1, -1)).test(email)),
          slug === "releash-dependency-updates");
      });
    }
  }
});

test("Nx migrates from the base installation, keeps PR config, installs dependencies, and runs generated migrations", () => {
  for (const migrations of [true, false]) {
    fixture(({ directory, write, execute }) => {
      write("nx.json", JSON.stringify({ installation: { version: "24.0.0", plugins: { "@nx/js": "24.0.0" } }, other: "PR setting" }));
      write("migrations.json", '{"migrations":["old migration"]}');
      write("git", '#!/bin/sh\nprintf \'{"installation":{"version":"23.3.0","plugins":{"@nx/js":"23.3.0"}}}\\n\'\n');
      write("nx", `#!/bin/sh\necho "nx $*" >> calls\nif [ "$2" != --run-migrations ]; then\n  node -e 'const fs = require("node:fs"); const x = require("./nx.json"); if (x.installation.version !== "23.3.0" || x.installation.plugins["@nx/js"] !== "23.3.0" || x.other !== "PR setting") process.exit(1); x.installation.version = "24.0.0"; x.installation.plugins["@nx/js"] = "24.0.0"; fs.writeFileSync("nx.json", JSON.stringify(x));'\n  ${migrations ? "echo '{}' > migrations.json" : ":"}\nfi\n`);
      write("pnpm", '#!/bin/sh\necho "pnpm $*" >> calls\n');
      execute("Migrate Nx");
      assert.equal(readFileSync(join(directory, "calls"), "utf8"),
        "nx migrate 24.0.0 --interactive=false\npnpm --dir clients/desktop install --no-frozen-lockfile\n" +
        (migrations ? "nx migrate --run-migrations --interactive=false\n" : ""));
      assert.equal(JSON.parse(readFileSync(join(directory, "nx.json"))).installation.version, "24.0.0");
    });
  }
});

test("Nx skips unchanged versions and fails instead of hiding migration errors", () => {
  for (const version of ["23.3.0", "24.0.0"]) {
    fixture(({ directory, write, execute }) => {
      write("nx.json", JSON.stringify({ installation: { version } }));
      write("git", '#!/bin/sh\necho \'{"installation":{"version":"23.3.0"}}\'\n');
      write("nx", '#!/bin/sh\necho nx >> calls\nexit 1\n');
      write("pnpm", '#!/bin/sh\necho pnpm >> calls\n');
      if (version === "23.3.0") {
        execute("Migrate Nx");
        assert.equal(existsSync(join(directory, "calls")), false);
      } else {
        assert.throws(() => execute("Migrate Nx"));
        assert.equal(readFileSync(join(directory, "calls"), "utf8"), "nx\n");
      }
    });
  }
});

test("postprocessing pushes all generated files only when there are changes and never force-pushes", () => {
  for (const changed of [true, false]) {
    fixture(({ directory, write, execute }) => {
      write("git", '#!/bin/sh\necho "git $*" >> calls\nif [ "$1" = diff ]; then exit "$CHANGED"; fi\n');
      write("gh", '#!/bin/sh\necho "gh $*" >> calls\n');
      execute("Push postprocessing", { CHANGED: changed ? "1" : "0", BOT_NAME: "app[bot]", BOT_EMAIL: "1+app[bot]@users.noreply.github.com", PR_BRANCH: "renovate/nx" });
      const calls = readFileSync(join(directory, "calls"), "utf8");
      assert.match(calls, /git add --all/);
      if (changed) {
        assert.match(calls, /git config user.email 1\+app\[bot\]@users.noreply.github.com/);
        assert.match(calls, /gh auth setup-git\ngit push origin HEAD:refs\/heads\/renovate\/nx/);
      } else {
        assert.doesNotMatch(calls, /git commit|git push|gh auth/);
      }
      assert.doesNotMatch(calls, /--force/);
    });
  }
});
