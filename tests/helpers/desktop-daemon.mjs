import assert from "node:assert/strict";
import { createInterface } from "node:readline";
import { setTimeout } from "node:timers/promises";
import { fromBinary } from "@bufbuild/protobuf";
import { build } from "esbuild";

const input = createInterface({ input: process.stdin });
const pending = new Map();
let nextId = 0;
input.on("line", (line) => {
    const { id, result } = JSON.parse(line);
    assert.ok(pending.has(id));
    pending.get(id)(result);
    pending.delete(id);
});
function invokeHost(command, args) {
    return new Promise((resolve) => {
        const id = ++nextId;
        pending.set(id, resolve);
        process.stdout.write(`${JSON.stringify({ id, command, args })}\n`);
    });
}
globalThis.window = Object.assign(new EventTarget(), {
    __TAURI_INTERNALS__: { invoke: (command, args) => {
        assert.ok(["get_client_endpoint", "apply_desktop_settings"].includes(command));
        return invokeHost(command, args);
    } },
});
const bundle = await build({
    stdin: { contents: 'export * from "./src/lib/clientSocket.ts"; export { EnvelopeSchema } from "./src/generated/client_pb.ts";', resolveDir: process.cwd() },
    bundle: true, platform: "node", format: "esm", write: false,
});
const { invokeClient, getClientStatus, onClientRefresh, retryClientOperation, EnvelopeSchema } = await import(
    `data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].contents).toString("base64")}`
);
const NativeWebSocket = globalThis.WebSocket;
const connections = [];
const generations = new Set();
let originalId;
let originalSends = 0;
let dropped = false;
let unknownCount = 0;
globalThis.WebSocket = class {
    constructor(url, protocols) {
        connections.push({ url, protocols });
        this.native = new NativeWebSocket(url, protocols);
        this.native.binaryType = "arraybuffer";
        this.native.onopen = () => this.onopen?.();
        this.native.onerror = () => this.onerror?.();
        this.native.onclose = () => this.onclose?.();
        this.native.onmessage = (event) => {
            const { body } = fromBinary(EnvelopeSchema, new Uint8Array(event.data));
            if (body.case === "hello") generations.add(body.value.instanceId);
            if (body.case === "response" && body.value.requestId === originalId) {
                dropped = true;
                return;
            }
            if (body.case === "operationStatus" && body.value.requestId === originalId && body.value.state === "unknown") unknownCount++;
            this.onmessage?.(event);
        };
    }
    send(bytes) {
        const { body } = fromBinary(EnvelopeSchema, bytes);
        if (body.case === "request" && body.value.command.case === "updateExternalEditor") {
            originalId ??= body.value.requestId;
            originalSends++;
        }
        this.native.send(bytes);
    }
    close() { this.native.close(); }
};
async function waitFor(predicate) {
    const deadline = Date.now() + 15_000;
    while (!predicate()) {
        assert.ok(Date.now() < deadline, "desktop recovery deadline");
        await setTimeout(10);
    }
}
let refreshedSettings;
const stopRefresh = onClientRefresh(() => {
    void invokeClient("get_app_settings").then((settings) => { refreshedSettings = settings; });
});
try {
    // Given
    const initial = await invokeClient("get_app_settings");
    assert.equal(initial.external_editor, "");
    assert.equal(initial.close_to_tray, false);
    assert.equal(initial.start_minimized, true);
    assert.equal(generations.size, 1);
    assert.equal(connections.length, 1);
    await invokeClient("update_app_settings", { app: { close_to_tray: true, auto_launch: false, start_minimized: false } });
    await invokeClient("update_app_settings", { app: { close_to_tray: false, auto_launch: false, start_minimized: true } });
    await invokeClient("update_crash_reporting", { enabled: false });
    assert.equal(await invokeClient("get_crash_reporting_enabled"), false);
    await invokeClient("update_crash_reporting", { enabled: true });
    assert.equal(await invokeClient("get_crash_reporting_enabled"), true);
    await invokeHost("damage_settings");
    const cached = await invokeClient("get_app_settings");
    assert.equal(cached.close_to_tray, false);
    assert.equal(cached.start_minimized, true);
    void invokeClient("update_external_editor", { editor: "desktop-recovery" }).catch(() => {});
    await waitFor(() => dropped);
    // When
    await invokeHost("restart");
    await waitFor(() => generations.size === 2 && unknownCount > 0 && refreshedSettings);
    // Then
    assert.equal(refreshedSettings.external_editor, "desktop-recovery");
    assert.equal(getClientStatus().connected, true);
    assert.notDeepEqual(connections[0].protocols, connections.at(-1).protocols);
    assert.ok(getClientStatus().operations.some(op => op.id === originalId && op.state === "unknown"));
    const beforeRetry = unknownCount;
    retryClientOperation(originalId);
    await waitFor(() => unknownCount > beforeRetry);
    assert.ok(getClientStatus().operations.some(op => op.id === originalId && op.state === "unknown"));
    assert.equal(originalSends, 1);
} finally {
    stopRefresh();
    window.dispatchEvent(new Event("pagehide"));
    input.close();
    process.stdin.destroy();
}

