import assert from "node:assert/strict";
import { createInterface } from "node:readline";
import { setTimeout } from "node:timers/promises";
import { build } from "esbuild";

console.debug = (...args) => console.error(...args.map(value => value instanceof Error ? value.message : value));
const input = createInterface({ input: process.stdin });
const pending = new Map();
let nextId = 0;
let originalSends = 0;
let dropped = false;
const generations = new Set();
const restored = process.argv.includes("--restored");
input.on("line", (line) => {
    const message = JSON.parse(line);
    assert.ok(pending.has(message.id));
    const {resolve, reject} = pending.get(message.id);
    pending.delete(message.id);
    if (message.error !== undefined) reject(message.error);
    else resolve(message.result);
});
function invokeHost(command, args = {}) {
    return new Promise((resolve, reject) => {
        const id = ++nextId;
        pending.set(id, {resolve, reject});
        process.stdout.write(`${JSON.stringify({ id, command, args })}\n`);
    });
}
globalThis.window = Object.assign(new EventTarget(), {
    __TAURI_INTERNALS__: { invoke: async (command, args) => {
        const result = await invokeHost(command, args);
        if (command === "get_client_endpoint") generations.add(result.launchId);
        return result;
    } },
});
const nativeFetch = globalThis.fetch;
globalThis.fetch = async (input, init) => {
    const request = new Request(input, init);
    request.headers.set("origin", "tauri://localhost");
    const response = await nativeFetch(request);
    if (request.url.endsWith("/UpdateExternalEditor")) {
        originalSends++;
        if (!restored && !dropped) {
            dropped = true;
            await response.arrayBuffer();
            throw new TypeError("response lost");
        }
    }
    return response;
};
const bundle = await build({
    stdin: { contents: 'export * from "./src/lib/client.ts";', resolveDir: process.cwd() },
    bundle: true, platform: "node", format: "esm", write: false,
});
const {invokeClient, onClientRefresh, completeClientRestoration, refreshClient} = await import(`data:text/javascript;base64,${Buffer.from(`${bundle.outputFiles[0].text}\n//# sourceURL=releash-client-fixture.mjs`).toString("base64")}`);
async function waitFor(predicate) {
    const deadline = Date.now() + 15_000;
    while (!(await predicate())) { assert.ok(Date.now() < deadline, "desktop recovery deadline"); await setTimeout(10); }
}
let refreshedSettings;
const restore = async () => {
    await Promise.all([invokeClient("get_repo_paths"), invokeClient("get_performance_telemetry_enabled")]);
    await completeClientRestoration((await invokeHost("get_daemon_status")).connectionGeneration);
    await waitFor(async () => (await invokeHost("get_daemon_status")).phase === "ready");
};
const stopRefresh = onClientRefresh(() => {
    void invokeClient("get_app_settings").then(settings => { refreshedSettings = settings; }).catch(error => { console.error(error instanceof Error ? error.message : error); });
});
try {
    await restore();
    if (restored) {
        const settings = await invokeClient("get_app_settings");
        assert.equal(settings.external_editor, "desktop-recovery");
        assert.equal(originalSends, 0);
        await invokeClient("update_external_editor", {editor: "confirmed-after-restart"});
        assert.equal((await invokeClient("get_app_settings")).external_editor, "confirmed-after-restart");
    } else {
        // Given
        const initial = await invokeClient("get_app_settings");
        assert.equal(initial.external_editor, "");
        assert.equal(initial.close_to_tray, false);
        assert.equal(initial.start_minimized, true);
        assert.equal(generations.size, 1);
        await invokeClient("update_app_settings", { app: { close_to_tray: true, start_minimized: false } });
        await invokeClient("update_app_settings", { app: { close_to_tray: false, start_minimized: true } });
        await invokeClient("update_crash_reporting", { enabled: false });
        assert.equal(await invokeClient("get_crash_reporting_enabled"), false);
        await invokeClient("update_crash_reporting", { enabled: true });
        assert.equal(await invokeClient("get_crash_reporting_enabled"), true);
        await invokeHost("damage_settings");
        const cached = await invokeClient("get_app_settings");
        assert.equal(cached.close_to_tray, false);
        assert.equal(cached.start_minimized, true);
        await assert.rejects(invokeClient("update_external_editor", { editor: "desktop-recovery" }));
        await waitFor(() => dropped);
        // When
        await invokeHost("restart");
        refreshedSettings = undefined;
        refreshClient();
        await waitFor(() => generations.size === 2 && refreshedSettings);
        await restore();
        assert.equal(refreshedSettings.external_editor, "desktop-recovery");
        assert.equal(originalSends, 1);
    }
} finally {
    stopRefresh();
    window.dispatchEvent(new Event("pagehide"));
    input.close();
    process.stdin.destroy();
}
