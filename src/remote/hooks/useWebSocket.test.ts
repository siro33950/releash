import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { serializeMessage } from "@/types/protocol";
import { useWebSocket } from "./useWebSocket";

let mockInstances: MockWebSocket[];

class MockWebSocket {
	static CONNECTING = 0;
	static OPEN = 1;
	static CLOSING = 2;
	static CLOSED = 3;

	readyState = MockWebSocket.CONNECTING;
	onopen: (() => void) | null = null;
	onclose: (() => void) | null = null;
	onmessage: ((event: { data: string }) => void) | null = null;
	onerror: (() => void) | null = null;
	send = vi.fn();
	close = vi.fn(() => {
		this.readyState = MockWebSocket.CLOSED;
	});

	constructor(public url: string) {
		mockInstances.push(this);
	}

	simulateOpen() {
		this.readyState = MockWebSocket.OPEN;
		this.onopen?.();
	}

	simulateMessage(data: string) {
		this.onmessage?.({ data });
	}

	simulateClose() {
		this.readyState = MockWebSocket.CLOSED;
		this.onclose?.();
	}
}

const OriginalWebSocket = globalThis.WebSocket;

describe("useWebSocket", () => {
	beforeEach(() => {
		mockInstances = [];
		globalThis.WebSocket = MockWebSocket as unknown as typeof WebSocket;
		vi.useFakeTimers();
	});

	afterEach(() => {
		vi.useRealTimers();
		globalThis.WebSocket = OriginalWebSocket;
	});

	it("starts as disconnected when no url/token", () => {
		const { result } = renderHook(() => useWebSocket({ url: "", token: "" }));
		expect(result.current.status).toBe("disconnected");
	});

	it("transitions to connecting then authenticating on open", () => {
		const { result } = renderHook(() =>
			useWebSocket({ url: "wss://localhost:9443", token: "secret" }),
		);

		expect(result.current.status).toBe("connecting");

		act(() => {
			mockInstances[0].simulateOpen();
		});

		expect(result.current.status).toBe("authenticating");
	});

	it("transitions to connected on auth success", () => {
		const { result } = renderHook(() =>
			useWebSocket({ url: "wss://localhost:9443", token: "secret" }),
		);

		act(() => {
			mockInstances[0].simulateOpen();
		});

		act(() => {
			mockInstances[0].simulateMessage(
				serializeMessage({
					type: "auth_result",
					payload: { success: true },
				}),
			);
		});

		expect(result.current.status).toBe("connected");
	});

	it("closes socket on auth failure", () => {
		renderHook(() =>
			useWebSocket({ url: "wss://localhost:9443", token: "secret" }),
		);

		act(() => {
			mockInstances[0].simulateOpen();
		});

		act(() => {
			mockInstances[0].simulateMessage(
				serializeMessage({
					type: "auth_result",
					payload: { success: false, message: "bad token" },
				}),
			);
		});

		expect(mockInstances[0].close).toHaveBeenCalled();
	});

	it("calls onMessage for non-auth messages after auth", () => {
		const onMessage = vi.fn();
		renderHook(() =>
			useWebSocket({
				url: "wss://localhost:9443",
				token: "secret",
				onMessage,
			}),
		);

		act(() => {
			mockInstances[0].simulateOpen();
		});

		act(() => {
			mockInstances[0].simulateMessage(
				serializeMessage({
					type: "auth_result",
					payload: { success: true },
				}),
			);
		});

		act(() => {
			mockInstances[0].simulateMessage(
				serializeMessage({
					type: "worktree_list_request",
					payload: {},
				}),
			);
		});

		expect(onMessage).toHaveBeenCalledWith(
			expect.objectContaining({ type: "worktree_list_request" }),
		);
	});

	it("sends message via send when connected", () => {
		const { result } = renderHook(() =>
			useWebSocket({ url: "wss://localhost:9443", token: "secret" }),
		);

		act(() => {
			mockInstances[0].simulateOpen();
			mockInstances[0].readyState = MockWebSocket.OPEN;
		});

		act(() => {
			result.current.send({
				type: "worktree_list_request",
				payload: {} as Record<string, never>,
			});
		});

		expect(mockInstances[0].send).toHaveBeenCalled();
	});

	it("reconnects with backoff on unintentional close", () => {
		renderHook(() =>
			useWebSocket({ url: "wss://localhost:9443", token: "secret" }),
		);

		const ws1 = mockInstances[0];

		act(() => {
			ws1.simulateOpen();
		});
		act(() => {
			ws1.simulateMessage(
				serializeMessage({
					type: "auth_result",
					payload: { success: true },
				}),
			);
		});

		act(() => {
			ws1.simulateClose();
		});

		expect(mockInstances.length).toBe(1);

		act(() => {
			vi.advanceTimersByTime(1000);
		});

		expect(mockInstances.length).toBe(2);
	});

	it("does not reconnect on intentional disconnect", () => {
		const { result } = renderHook(() =>
			useWebSocket({ url: "wss://localhost:9443", token: "secret" }),
		);

		act(() => {
			mockInstances[0].simulateOpen();
		});

		act(() => {
			result.current.disconnect();
		});

		expect(result.current.status).toBe("disconnected");

		act(() => {
			vi.advanceTimersByTime(5000);
		});

		expect(mockInstances.length).toBe(1);
	});

	it("does not reconnect after auth failure close", () => {
		renderHook(() =>
			useWebSocket({ url: "wss://localhost:9443", token: "secret" }),
		);

		act(() => {
			mockInstances[0].simulateOpen();
		});

		act(() => {
			mockInstances[0].simulateMessage(
				serializeMessage({
					type: "auth_result",
					payload: { success: false, message: "denied" },
				}),
			);
		});

		act(() => {
			mockInstances[0].simulateClose();
		});

		act(() => {
			vi.advanceTimersByTime(60000);
		});

		expect(mockInstances.length).toBe(1);
	});
});
