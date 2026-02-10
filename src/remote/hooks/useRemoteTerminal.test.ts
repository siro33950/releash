import { renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WsMessage } from "@/types/protocol";
import type { Subscribe } from "./useMessageBus";
import { useRemoteTerminal } from "./useRemoteTerminal";

type MessageHandler = (msg: WsMessage) => void;

function createMockSubscribe(): {
	subscribe: Subscribe;
	handler: () => MessageHandler | null;
} {
	let captured: MessageHandler | null = null;
	const subscribe: Subscribe = vi.fn((h: MessageHandler) => {
		captured = h;
		return () => {
			captured = null;
		};
	});
	return { subscribe, handler: () => captured };
}

function createContainerRef() {
	const div = document.createElement("div");
	Object.defineProperty(div, "clientWidth", { value: 800, writable: true });
	Object.defineProperty(div, "clientHeight", { value: 600, writable: true });
	return { current: div } as React.RefObject<HTMLDivElement>;
}

describe("useRemoteTerminal", () => {
	let mockSend: (msg: WsMessage) => void;
	let mockSubscribe: ReturnType<typeof createMockSubscribe>;
	let containerRef: ReturnType<typeof createContainerRef>;

	beforeEach(() => {
		vi.useFakeTimers();
		mockSend = vi.fn();
		mockSubscribe = createMockSubscribe();
		containerRef = createContainerRef();
	});

	afterEach(() => {
		vi.useRealTimers();
	});

	it("subscribes to message bus on mount", () => {
		renderHook(() =>
			useRemoteTerminal({
				containerRef,
				ptyId: 1,
				ptyCols: 80,
				send: mockSend,
				subscribe: mockSubscribe.subscribe,
				visible: true,
			}),
		);

		expect(mockSubscribe.subscribe).toHaveBeenCalled();
	});

	it("sends pty_output_request on initial mount", () => {
		renderHook(() =>
			useRemoteTerminal({
				containerRef,
				ptyId: 42,
				ptyCols: 80,
				send: mockSend,
				subscribe: mockSubscribe.subscribe,
				visible: true,
			}),
		);

		vi.advanceTimersByTime(100);

		expect(mockSend).toHaveBeenCalledWith({
			type: "pty_output_request",
			payload: { pty_id: 42 },
		});
	});

	it("writes pty_output to terminal", () => {
		const { result } = renderHook(() =>
			useRemoteTerminal({
				containerRef,
				ptyId: 1,
				ptyCols: 80,
				send: mockSend,
				subscribe: mockSubscribe.subscribe,
				visible: true,
			}),
		);

		const handler = mockSubscribe.handler();
		expect(handler).toBeTruthy();

		handler?.({
			type: "pty_output",
			payload: { pty_id: 1, data: "hello world" },
		});

		const terminal = result.current.terminalRef.current;
		expect(terminal?.write).toHaveBeenCalledWith("hello world");
	});

	it("ignores pty_output for different pty_id", () => {
		const { result } = renderHook(() =>
			useRemoteTerminal({
				containerRef,
				ptyId: 1,
				ptyCols: 80,
				send: mockSend,
				subscribe: mockSubscribe.subscribe,
				visible: true,
			}),
		);

		const handler = mockSubscribe.handler();
		handler?.({
			type: "pty_output",
			payload: { pty_id: 999, data: "other pty" },
		});

		const terminal = result.current.terminalRef.current;
		expect(terminal?.write).not.toHaveBeenCalled();
	});

	it("writes exit message on pty_exit", () => {
		const { result } = renderHook(() =>
			useRemoteTerminal({
				containerRef,
				ptyId: 1,
				ptyCols: 80,
				send: mockSend,
				subscribe: mockSubscribe.subscribe,
				visible: true,
			}),
		);

		const handler = mockSubscribe.handler();
		handler?.({
			type: "pty_exit",
			payload: { pty_id: 1, exit_code: 0 },
		});

		const terminal = result.current.terminalRef.current;
		expect(terminal?.write).toHaveBeenCalledWith(
			expect.stringContaining("Process exited with code 0"),
		);
	});

	it("cleans up on unmount", () => {
		const { unmount, result } = renderHook(() =>
			useRemoteTerminal({
				containerRef,
				ptyId: 1,
				ptyCols: 80,
				send: mockSend,
				subscribe: mockSubscribe.subscribe,
				visible: true,
			}),
		);

		const terminal = result.current.terminalRef.current;
		unmount();

		expect(terminal?.dispose).toHaveBeenCalled();
	});
});
