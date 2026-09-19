import assert from "node:assert/strict";
import { createInterface } from "node:readline";
import { setTimeout } from "node:timers/promises";
import { fromBinary } from "@bufbuild/protobuf";
import { build } from "esbuild";

const input = createInterface({ input: process.stdin });
const pending = new Map();
let nextId = 0;
let channel;
let originalId;
let originalSends = 0;
let dropped = false;
let unknownCount = 0;
const generations = new Set();
const restored = process.argv.includes("--restored");
let EnvelopeSchema;
input.on("line", (line) => {
    const message = JSON.parse(line);
    if ("frame" in message) {
        if (message.frame) {
            const { body } = fromBinary(EnvelopeSchema, new Uint8Array(message.frame));
            if (!restored && body.case === "response" && body.value.requestId === originalId) { dropped = true; return; }
            if (body.case === "operationStatus" && body.value.requestId === originalId && body.value.state === "restored_unknown") unknownCount++;
        }
        channel?.onmessage(message.frame);
        return;
    }
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
    __TAURI_INTERNALS__: {
        transformCallback: () => ++nextId,
        unregisterCallback: () => {},
        invoke: async (command, args) => {
            if (command === "attach_desktop_client") {
                channel = args.channel;
                const hello = await invokeHost(command, {attachmentId: args.attachmentId});
                generations.add(fromBinary(EnvelopeSchema, new Uint8Array(hello)).body.value.instanceId);
                return hello;
            }
            if (command === "send_desktop_client_frame") {
                const {body} = fromBinary(EnvelopeSchema, new Uint8Array(args.bytes));
                if (body.case === "request" && body.value.command.case === "updateExternalEditor") { originalId ??= body.value.requestId; originalSends++; }
            }
            const result = await invokeHost(command, args);
            return result;
        },
    },
});
const bundle = await build({
    stdin: { contents: 'export * from "./src/lib/clientSocket.ts"; export { EnvelopeSchema } from "./src/generated/client_pb.ts";', resolveDir: process.cwd() },
    bundle: true, platform: "node", format: "esm", write: false,
});
const client = await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].contents).toString("base64")}`);
EnvelopeSchema = client.EnvelopeSchema;
const {invokeClient, getClientStatus, onClientRefresh, retryClientOperation, dismissClientOperation, completeClientRestoration} = client;
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
    void restore().then(() => invokeClient("get_app_settings")).then(settings => { refreshedSettings = settings; }).catch(error => { console.error(error); });
});
try {
    await restore();
    if (restored) {
        const settings = await invokeClient("get_app_settings");
        assert.equal(settings.external_editor, "desktop-recovery");
        const unresolved = getClientStatus().operations.find(op => op.command === "update_external_editor");
        assert.equal(unresolved?.state, "unknown");
        assert.equal(unresolved.canQuery, false);
        assert.equal(unresolved.canDismiss, true);
        retryClientOperation(unresolved.id);
        await setTimeout(50);
        assert.equal(originalSends, 0);
        assert.equal(getClientStatus().operations.find(op => op.id === unresolved.id)?.state, "unknown");
        await dismissClientOperation(unresolved.id);
        assert.equal(getClientStatus().operations.length, 0);
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
        void invokeClient("update_external_editor", { editor: "desktop-recovery" }).catch(() => {});
        await waitFor(() => dropped);
        // When
        await invokeHost("restart");
        await waitFor(() => generations.size === 2 && unknownCount > 0 && refreshedSettings);
        // Then
        assert.equal(refreshedSettings.external_editor, "desktop-recovery");
        assert.equal(getClientStatus().connected, true);
        await waitFor(() => getClientStatus().operations.some(op => op.id === originalId && op.canDismiss));
        retryClientOperation(originalId);
        await setTimeout(50);
        assert.ok(getClientStatus().operations.some(op => op.id === originalId && op.state === "unknown"));
        assert.equal(originalSends, 1);
    }
} finally {
    stopRefresh();
    window.dispatchEvent(new Event("pagehide"));
    input.close();
    process.stdin.destroy();
}
