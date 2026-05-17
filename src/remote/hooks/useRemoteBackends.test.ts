import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { BackendInfoMsg, WsMessage } from "@/types/protocol";
import { useRemoteBackends } from "./useRemoteBackends";

const makeBackend = (
	overrides: Partial<BackendInfoMsg> = {},
): BackendInfoMsg => ({
	id: "backend-1",
	name: "Claude",
	available: true,
	available_models: [],
	...overrides,
});

describe("useRemoteBackends", () => {
	it("初期状態で loading が false、backends が空", () => {
		const subscribe = vi.fn(() => vi.fn());
		const send = vi.fn();

		const { result } = renderHook(() =>
			useRemoteBackends({ subscribe, send, connected: false }),
		);

		expect(result.current.loading).toBe(false);
		expect(result.current.backends).toEqual([]);
		expect(result.current.defaultId).toBeNull();
		expect(result.current.selectedBackendId).toBeNull();
	});

	it("connected が true になったら backend_list_request を送信する", () => {
		const subscribe = vi.fn(() => vi.fn());
		const send = vi.fn();

		renderHook(() => useRemoteBackends({ subscribe, send, connected: true }));

		expect(send).toHaveBeenCalledWith({
			type: "backend_list_request",
			payload: {},
		});
	});

	it("backend_list_response を受信したら backends と defaultId が更新される", () => {
		let handler: ((msg: WsMessage) => void) | null = null;
		const subscribe = vi.fn((cb: (msg: WsMessage) => void) => {
			handler = cb;
			return vi.fn();
		});
		const send = vi.fn();

		const backends = [
			makeBackend({ id: "b1", name: "Claude" }),
			makeBackend({ id: "b2", name: "GPT" }),
		];

		const { result } = renderHook(() =>
			useRemoteBackends({ subscribe, send, connected: true }),
		);

		act(() => {
			handler?.({
				type: "backend_list_response",
				payload: { backends, default_id: "b1" },
			});
		});

		expect(result.current.backends).toEqual(backends);
		expect(result.current.defaultId).toBe("b1");
		expect(result.current.loading).toBe(false);
	});

	it("selectedBackendId が null の場合、defaultId で初期化される", () => {
		let handler: ((msg: WsMessage) => void) | null = null;
		const subscribe = vi.fn((cb: (msg: WsMessage) => void) => {
			handler = cb;
			return vi.fn();
		});
		const send = vi.fn();

		const backends = [
			makeBackend({ id: "b1", name: "Claude" }),
			makeBackend({ id: "b2", name: "GPT" }),
		];

		const { result } = renderHook(() =>
			useRemoteBackends({ subscribe, send, connected: true }),
		);

		act(() => {
			handler?.({
				type: "backend_list_response",
				payload: { backends, default_id: "b2" },
			});
		});

		expect(result.current.selectedBackendId).toBe("b2");
	});

	it("defaultId が null の場合、最初のバックエンドで初期化される", () => {
		let handler: ((msg: WsMessage) => void) | null = null;
		const subscribe = vi.fn((cb: (msg: WsMessage) => void) => {
			handler = cb;
			return vi.fn();
		});
		const send = vi.fn();

		const backends = [
			makeBackend({ id: "b1", name: "Claude" }),
			makeBackend({ id: "b2", name: "GPT" }),
		];

		const { result } = renderHook(() =>
			useRemoteBackends({ subscribe, send, connected: true }),
		);

		act(() => {
			handler?.({
				type: "backend_list_response",
				payload: { backends, default_id: null },
			});
		});

		expect(result.current.selectedBackendId).toBe("b1");
	});

	it("selectedBackendId が既に設定済みの場合、レスポンスで上書きされない", () => {
		let handler: ((msg: WsMessage) => void) | null = null;
		const subscribe = vi.fn((cb: (msg: WsMessage) => void) => {
			handler = cb;
			return vi.fn();
		});
		const send = vi.fn();

		const backends = [
			makeBackend({ id: "b1", name: "Claude" }),
			makeBackend({ id: "b2", name: "GPT" }),
		];

		const { result } = renderHook(() =>
			useRemoteBackends({ subscribe, send, connected: true }),
		);

		act(() => {
			handler?.({
				type: "backend_list_response",
				payload: { backends, default_id: "b1" },
			});
		});

		act(() => {
			result.current.setSelectedBackendId("b2");
		});

		act(() => {
			handler?.({
				type: "backend_list_response",
				payload: { backends, default_id: "b1" },
			});
		});

		expect(result.current.selectedBackendId).toBe("b2");
	});

	it("connected が false の場合、refresh が何もしない", () => {
		const subscribe = vi.fn(() => vi.fn());
		const send = vi.fn();

		const { result } = renderHook(() =>
			useRemoteBackends({ subscribe, send, connected: false }),
		);

		act(() => {
			result.current.refresh();
		});

		expect(send).not.toHaveBeenCalled();
		expect(result.current.loading).toBe(false);
	});

	it("connected が true から false に変わったら loading が false になる", () => {
		const subscribe = vi.fn((_cb: (msg: WsMessage) => void) => vi.fn());
		const send = vi.fn();

		const { result, rerender } = renderHook(
			({ connected }) => useRemoteBackends({ subscribe, send, connected }),
			{ initialProps: { connected: true } },
		);

		expect(result.current.loading).toBe(true);

		rerender({ connected: false });

		expect(result.current.loading).toBe(false);
	});

	it("refresh を呼ぶと backend_list_request が再送信される", () => {
		const subscribe = vi.fn(() => vi.fn());
		const send = vi.fn();

		const { result } = renderHook(() =>
			useRemoteBackends({ subscribe, send, connected: true }),
		);

		send.mockClear();

		act(() => {
			result.current.refresh();
		});

		expect(send).toHaveBeenCalledWith({
			type: "backend_list_request",
			payload: {},
		});
		expect(result.current.loading).toBe(true);
	});

	it("backend_models_updated を受信したら対象 backend の候補だけ更新される", () => {
		let handler: ((msg: WsMessage) => void) | null = null;
		const subscribe = vi.fn((cb: (msg: WsMessage) => void) => {
			handler = cb;
			return vi.fn();
		});
		const send = vi.fn();
		const backends = [
			makeBackend({
				id: "claude",
				available_models: [{ value: "old-claude" }],
			}),
			makeBackend({
				id: "codex",
				available_models: [{ value: "old-codex" }],
			}),
		];

		const { result } = renderHook(() =>
			useRemoteBackends({ subscribe, send, connected: true }),
		);

		act(() => {
			handler?.({
				type: "backend_list_response",
				payload: { backends, default_id: "claude" },
			});
		});
		act(() => {
			handler?.({
				type: "backend_models_updated",
				payload: {
					backend_id: "codex",
					available_models: [{ value: "gpt-5.5" }],
				},
			});
		});

		expect(result.current.backends).toEqual([
			backends[0],
			{ ...backends[1], available_models: [{ value: "gpt-5.5" }] },
		]);
	});

	it("アンマウント時に unsubscribe が呼ばれる", () => {
		const unsubscribe = vi.fn();
		const subscribe = vi.fn(() => unsubscribe);
		const send = vi.fn();

		const { unmount } = renderHook(() =>
			useRemoteBackends({ subscribe, send, connected: false }),
		);

		unmount();
		expect(unsubscribe).toHaveBeenCalled();
	});

	it("backends が空で defaultId も null の場合、selectedBackendId は null のまま", () => {
		let handler: ((msg: WsMessage) => void) | null = null;
		const subscribe = vi.fn((cb: (msg: WsMessage) => void) => {
			handler = cb;
			return vi.fn();
		});
		const send = vi.fn();

		const { result } = renderHook(() =>
			useRemoteBackends({ subscribe, send, connected: true }),
		);

		act(() => {
			handler?.({
				type: "backend_list_response",
				payload: { backends: [], default_id: null },
			});
		});

		expect(result.current.selectedBackendId).toBeNull();
		expect(result.current.backends).toEqual([]);
	});
});
