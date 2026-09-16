import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test } from "node:test";
import { fromBinary, fromJson, toBinary } from "@bufbuild/protobuf";
import { build } from "esbuild";
import { startPerformanceDaemon } from "./performance-daemon.mjs";

test("performance harness starts an isolated external daemon with fixture environment and stops it", { timeout: 45_000 }, async () => {
    const directory = await mkdtemp(join(tmpdir(), "releash-performance-test-"));
    let stop;
    let socket;
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
        const bundle = await build({ entryPoints: ["src/generated/client_pb.ts"], bundle: true, platform: "node", format: "esm", write: false });
        const { EnvelopeSchema } = await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].contents).toString("base64")}`);
        socket = new WebSocket(`ws://127.0.0.1:${discovery.port}/v1/client`, `releash-bearer.${discovery.token}`);
        socket.binaryType = "arraybuffer";
        const result = await new Promise((done, reject) => {
            const send = (body) => socket.send(toBinary(EnvelopeSchema, fromJson(EnvelopeSchema, body)));
            socket.onerror = reject;
            socket.onopen = () => send({ hello: {} });
            socket.onmessage = ({ data }) => {
                const { body } = fromBinary(EnvelopeSchema, new Uint8Array(data));
                if (body.case === "hello") send({ request: { requestId: "providers", getProviderAvailability: {} } });
                if (body.case === "response") done(body.value);
            };
        });
        assert.equal(result.outcome.case, "result");
        const providers = result.outcome.value.command.value.providers.items;
        assert.equal(providers.length, 2);
        for (const provider of providers) {
            assert.equal(provider.effectiveExecutable, fixture);
            assert.equal(provider.available, true);
        }
        socket.close();
        await stop();
        stop = undefined;
        await assert.rejects(readFile(join(directory, "client-api.json")), { code: "ENOENT" });
    } finally {
        socket?.close();
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
