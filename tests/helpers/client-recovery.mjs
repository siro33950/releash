import assert from "node:assert/strict";
import { setTimeout } from "node:timers/promises";
import { build } from "esbuild";
let url = process.argv[2];
const requests = [];
const nativeFetch = globalThis.fetch;
let drop = true;
globalThis.fetch = async (input, init) => {
    const request = new Request(input, init);
    requests.push(new URL(request.url).pathname);
    const response = await nativeFetch(request);
    if (drop && request.url.endsWith("/UpdateCrashReporting")) {
        drop = false;
        await response.arrayBuffer();
        throw new TypeError("response lost");
    }
    return response;
};
globalThis.window = Object.assign(new EventTarget(), {
    __TAURI_INTERNALS__: { invoke: async (command) => {
        if (command === "validate_daemon_connection") return;
        assert.equal(command, "get_client_endpoint");
        return { url, token: "acceptance", launchId: "" };
    } },
});
const bundle = await build({
    stdin: { contents: 'export * from "./src/lib/client.ts";', resolveDir: process.cwd() },
    bundle: true, platform: "node", format: "esm", write: false,
});
const { invokeClient, onClientRefresh, refreshClient } = await import(`data:text/javascript;base64,${Buffer.from(`${bundle.outputFiles[0].text}\n//# sourceURL=releash-client-fixture.mjs`).toString("base64")}`);
let refreshes = 0;
const stop = onClientRefresh(() => { refreshes++; });
try {
    await assert.rejects(invokeClient("update_crash_reporting", { enabled: true }));
    await invokeClient("report_mounted_xterm_count", { count: 3 });
    await invokeClient("update_crash_reporting", { enabled: false });
    const before = refreshes;
    url = process.argv[3];
    refreshClient();
    const deadline = Date.now() + 5000;
    while (refreshes <= before) { assert.ok(Date.now() < deadline); await setTimeout(10); }
    assert.equal(requests.filter(path => path.endsWith("/UpdateCrashReporting")).length, 2);
    assert.ok(requests.every(path => !/Operation|Ack|Retry/.test(path)));
} finally {
    stop();
    window.dispatchEvent(new Event("pagehide"));
}
