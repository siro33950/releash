import assert from "node:assert/strict";
import { setTimeout } from "node:timers/promises";
import { build } from "esbuild";
import { fromBinary } from "@bufbuild/protobuf";

const bundle = await build({
    stdin: { contents: 'export * from "./src/lib/clientSocket.ts"; export { EnvelopeSchema } from "./src/generated/client_pb.ts";', resolveDir: process.cwd() },
    plugins: [{ name: "protocol-client-socket", setup(build) {
        build.onResolve({filter: /\/desktopClientSocket$/}, () => ({path: "fixture-socket", namespace: "fixture"}));
        build.onLoad({filter: /.*/, namespace: "fixture"}, () => ({contents: 'export class DesktopClientSocket { constructor() { return new globalThis.WebSocket(globalThis.__clientEndpoint.url, [globalThis.__clientEndpoint.authSubprotocol]); } }', loader: "js"}));
    }}],
    bundle: true, platform: "node", format: "esm", write: false,
});
globalThis.__clientEndpoint = { url: process.argv[2], authSubprotocol: "releash-bearer.acceptance" };
const unresolved = new Map();
globalThis.window = Object.assign(new EventTarget(), {
    __TAURI_INTERNALS__: { invoke: async (command, args) => {
        if (command === "list_client_handoff") return [...unresolved.values()];
        if (command === "admit_client_command") return;
        if (command === "forget_client_operation") { unresolved.delete(args.id); return; }
        if (command === "validate_daemon_connection") return;
        throw new Error(`Unexpected shell command: ${command}`);
    } },
});
const { invokeClient, getClientStatus, retryClientOperation, EnvelopeSchema } = await import(
    `data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].contents).toString("base64")}`
);
const NativeWebSocket = globalThis.WebSocket;
let originalId;
let dropped = false;
let unknownCount = 0;
let socket;
const generations = new Set();
globalThis.WebSocket = class {
    constructor(url, protocols) {
        socket = this;
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
        if (body.case === "request" && body.value.command.case === "updateCrashReporting" && body.value.command.value.enabled) originalId ??= body.value.requestId;
        this.native.send(bytes);
    }
    close() { this.native.close(); }
};
async function waitFor(predicate) {
    const deadline = Date.now() + 5_000;
    while (!predicate()) {
        assert.ok(Date.now() < deadline, "client state deadline");
        await setTimeout(10);
    }
}
try {
    void invokeClient("update_crash_reporting", { enabled: true }).catch(() => {});
    await waitFor(() => dropped);
    socket.close();
    await waitFor(() => getClientStatus().operations.some(op => op.id === originalId && op.state === "unknown"));
    await invokeClient("update_crash_reporting", { enabled: false });
    await invokeClient("report_mounted_xterm_count", { count: 3 });
    globalThis.__clientEndpoint.url = process.argv[3];
    socket.close();
    await waitFor(() => unknownCount > 0);
    assert.equal(generations.size, 2);
    assert.ok(getClientStatus().operations.some(op => op.id === originalId && op.state === "unknown"));
    const beforeRetry = unknownCount;
    retryClientOperation(originalId);
    await waitFor(() => unknownCount > beforeRetry);
    assert.ok(getClientStatus().operations.some(op => op.id === originalId && op.state === "unknown"));
} finally {
    window.dispatchEvent(new Event("pagehide"));
}
