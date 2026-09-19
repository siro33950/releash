import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test } from "node:test";
import { startPerformanceDaemon, connectPerformanceClient } from "./performance-daemon.mjs";

test("performance harness starts an isolated external daemon with fixture environment and stops it", { timeout: 45_000 }, async () => {
    const directory = await mkdtemp(join(tmpdir(), "releash-performance-test-"));
    let stop;
    try {
        const fixture = resolve("tests/fixtures/terminal-launch-provider-fixture");
        stop = await startPerformanceDaemon("src-tauri/target/release/releash-backend", directory, {
            HOME: directory, XDG_CONFIG_HOME: join(directory, "config"),
            CLAUDE_CONFIG_DIR: join(directory, ".claude"), CODEX_HOME: join(directory, ".codex"),
            RELEASH_DATA_DIR: directory, SHELL: "/bin/bash",
            RELEASH_PERFORMANCE_LAUNCH_PROVIDER: "fixture",
            RELEASH_PERFORMANCE_PROVIDER_FIXTURE_EXECUTABLE: fixture,
        });
        const discovery = JSON.parse(await readFile(join(directory, "client-api.json"), "utf8"));
        const client = await connectPerformanceClient(discovery);
        const result = await client.getProviderAvailability({});
        const providers = result.providers.items;
        assert.equal(providers.length, 2);
        for (const provider of providers) {
            assert.equal(provider.effectiveExecutable, fixture);
            assert.equal(provider.available, true);
        }
        await stop();
        stop = undefined;
        await assert.rejects(readFile(join(directory, "client-api.json")), { code: "ENOENT" });
    } finally {
        await stop?.();
        await rm(directory, { recursive: true, force: true });
    }
});

test("performance harness reports spawn failure", async () => {
    const directory = await mkdtemp(join(tmpdir(), "releash-performance-missing-"));
    try {
        await assert.rejects(startPerformanceDaemon(join(directory, "missing-backend"), directory, {}), { code: "ENOENT" });
    } finally {
        await rm(directory, { recursive: true, force: true });
    }
});
