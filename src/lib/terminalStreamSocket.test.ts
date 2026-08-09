import { afterEach, describe, expect, it, vi } from "vitest";

import { openTerminalStreamSocket } from "./terminalStreamSocket";

class FakeWebSocket {
	static latest: FakeWebSocket | null = null;

	onopen: (() => void) | null = null;
	onerror: (() => void) | null = null;
	onclose: (() => void) | null = null;
	onmessage: ((event: { data: string }) => void) | null = null;
	readonly send = vi.fn();
	readonly close = vi.fn();

	constructor() {
		FakeWebSocket.latest = this;
	}

	emitOpen() {
		this.onopen?.();
	}

	emitMessage(payload: unknown) {
		this.onmessage?.({ data: JSON.stringify(payload) });
	}

	emitClose() {
		this.onclose?.();
	}
}

const callbacks = {
	isClosedByUs: vi.fn(() => false),
	onUnexpectedClose: vi.fn(),
	onStreamItem: vi.fn(),
	onStreamError: vi.fn(),
};

describe("openTerminalStreamSocket", () => {
	afterEach(() => {
		vi.unstubAllGlobals();
		FakeWebSocket.latest = null;
		vi.clearAllMocks();
	});

	it("backendのattach受理応答までは接続完了にしない", async () => {
		vi.stubGlobal("WebSocket", FakeWebSocket);
		let resolved = false;
		const promise = openTerminalStreamSocket(
			{ url: "ws://terminal", authSubprotocol: "token" },
			{
				attachmentId: "attachment-1",
				owner: { kind: "workspace", workspacePath: "/repo" },
			},
			callbacks,
		).then((socket) => {
			resolved = true;
			return socket;
		});
		const socket = FakeWebSocket.latest;
		expect(socket).not.toBeNull();
		socket?.emitOpen();
		await Promise.resolve();

		expect(resolved).toBe(false);

		socket?.emitMessage({ status: "attached", id: "attachment-1" });
		await expect(promise).resolves.toBe(socket);
	});

	it("attach受理前のbackend errorは接続失敗として返す", async () => {
		vi.stubGlobal("WebSocket", FakeWebSocket);
		const promise = openTerminalStreamSocket(
			{ url: "ws://terminal", authSubprotocol: "token" },
			{
				attachmentId: "attachment-1",
				owner: { kind: "workspace", workspacePath: "/repo" },
			},
			callbacks,
		);
		const socket = FakeWebSocket.latest;
		socket?.emitOpen();
		socket?.emitMessage({
			status: "error",
			id: "attachment-1",
			error: { message: "surface not found" },
		});

		await expect(promise).rejects.toThrow("surface not found");
		expect(socket?.close).toHaveBeenCalledOnce();
		socket?.emitClose();
		expect(callbacks.onStreamError).not.toHaveBeenCalled();
		expect(callbacks.onUnexpectedClose).not.toHaveBeenCalled();
	});
});
